use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use config_store::{ConfigDocument, DomainConfig, SshImportPreview};
use gpui_kit::base::{Disableable, Selectable, StyledExt};
use gpui_kit::component::{
    ActiveTheme,
    button::{Button, ButtonVariants},
    input::{Input, InputContentType, InputEvent, InputState},
    scroll::ScrollableElement,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext as _, AsyncApp, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, PathPromptOptions, Render, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, TestSupportExt as _, Window, div, rems,
};
use tokio::sync::oneshot;
use tunnel_core::{
    CoreCommand, HostKeyDecision, HostKeyPromptView, LogEvent, LogLevel, LogSnapshot, LogSource,
    ManagerHandle, ManagerSnapshot, SaveReceipt, SecretUpdate, SupervisorState, TunnelAction,
    actions_for,
};
use tunnel_domain::TunnelMode;
use tunnel_domain::{AuthConfig, GroupId, HostId, HostKeyPolicy, SecretRef, TunnelGroup, TunnelId};

use crate::editor::{Editor, HostEditor, TunnelEditor};

const PAGE_SIZE: usize = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogFilter {
    All,
    Debug,
    Info,
    Warning,
    Error,
    Tunnel,
    Host,
}

impl LogFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Debug => "Debug",
            Self::Info => "Info",
            Self::Warning => "Warning",
            Self::Error => "Error",
            Self::Tunnel => "Tunnel",
            Self::Host => "Host",
        }
    }

    fn matches(self, event: &LogEvent) -> bool {
        match self {
            Self::All => true,
            Self::Debug => event.level == LogLevel::Debug,
            Self::Info => event.level == LogLevel::Info,
            Self::Warning => event.level == LogLevel::Warning,
            Self::Error => event.level == LogLevel::Error,
            Self::Tunnel => event.source == LogSource::Tunnel,
            Self::Host => event.source == LogSource::Host,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Overview,
    Jumpers,
    Tunnels,
    Groups,
    Logs,
    Settings,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Jumpers => "Jumpers",
            Self::Tunnels => "Tunnels",
            Self::Groups => "Groups",
            Self::Logs => "Logs",
            Self::Settings => "Settings",
        }
    }
}

struct GroupEditor {
    id: GroupId,
    name: Entity<InputState>,
}

pub struct Workspace {
    page: Page,
    page_index: usize,
    startup: Result<(PathBuf, DomainConfig), String>,
    manager: Option<ManagerHandle>,
    snapshot: Arc<ManagerSnapshot>,
    command_error: Option<String>,
    save_warning: Option<String>,
    runtime_error: Option<String>,
    _updates: Option<Task<()>>,
    _host_key_updates: Option<Task<()>>,
    _log_updates: Option<Task<()>>,
    host_key_prompts: Arc<Vec<HostKeyPromptView>>,
    logs: Arc<LogSnapshot>,
    log_filter: LogFilter,
    log_search: Entity<InputState>,
    _log_search_subscription: Subscription,
    editor: Option<Editor>,
    context_session: u64,
    pending_page: Option<Page>,
    group_editor: Option<GroupEditor>,
    pending_group_delete: Option<GroupId>,
    saving: bool,
    _save_task: Option<Task<()>>,
    _file_task: Option<Task<()>>,
    file_busy: bool,
    import_preview: Option<DomainConfig>,
    ssh_preview: Option<SshImportPreview>,
    ssh_selected: Vec<bool>,
    file_message: Option<String>,
}

impl Workspace {
    fn advance_context(&mut self) {
        self.context_session = self.context_session.wrapping_add(1).max(1);
    }

    fn apply_save_receipt(
        &mut self,
        directory: PathBuf,
        receipt: SaveReceipt,
        submitted_context: u64,
    ) {
        self.startup = Ok((directory, receipt.canonical_config));
        if self.context_session == submitted_context {
            self.editor = None;
            self.group_editor = None;
            self.pending_group_delete = None;
            self.import_preview = None;
            self.ssh_preview = None;
            if let Some(page) = self.pending_page.take() {
                self.page = page;
                self.page_index = 0;
                self.advance_context();
            }
        }
        self.command_error = None;
        self.save_warning = (!receipt.warnings.is_empty()).then(|| {
            format!(
                "Configuration saved with maintenance warnings: {}",
                receipt.warnings.join("; ")
            )
        });
    }

    pub fn new(
        startup: Result<(PathBuf, DomainConfig), String>,
        manager: Option<ManagerHandle>,
        runtime_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (snapshot, updates) = match &manager {
            Some(manager) => {
                let mut state = manager.subscribe();
                let snapshot = Arc::clone(&state.borrow_and_update());
                let updates = cx.spawn(async move |this, cx: &mut AsyncApp| {
                    while state.changed().await.is_ok() {
                        let latest = Arc::clone(&state.borrow_and_update());
                        if this
                            .update(cx, |view, cx| {
                                view.snapshot = latest;
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                (snapshot, Some(updates))
            }
            None => (Arc::new(ManagerSnapshot::new()), None),
        };
        let (host_key_prompts, host_key_updates) = match &manager {
            Some(manager) => {
                let mut state = manager.subscribe_host_keys();
                let prompts = Arc::clone(&state.borrow_and_update());
                let updates = cx.spawn(async move |this, cx: &mut AsyncApp| {
                    while state.changed().await.is_ok() {
                        let latest = Arc::clone(&state.borrow_and_update());
                        if this
                            .update(cx, |view, cx| {
                                view.host_key_prompts = latest;
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                (prompts, Some(updates))
            }
            None => (Arc::new(Vec::new()), None),
        };
        let (logs, log_updates) = match &manager {
            Some(manager) => {
                let mut state = manager.subscribe_logs();
                let logs = Arc::clone(&state.borrow_and_update());
                let updates = cx.spawn(async move |this, cx: &mut AsyncApp| {
                    while state.changed().await.is_ok() {
                        let latest = Arc::clone(&state.borrow_and_update());
                        if this
                            .update(cx, |view, cx| {
                                view.logs = latest;
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                (logs, Some(updates))
            }
            None => (Arc::new(LogSnapshot::new()), None),
        };
        let log_search = cx.new(|cx| InputState::new(window, cx));
        let log_search_subscription = cx.subscribe(&log_search, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.page_index = 0;
                cx.notify();
            }
        });
        Self {
            page: Page::Overview,
            page_index: 0,
            startup,
            manager,
            snapshot,
            command_error: None,
            save_warning: None,
            runtime_error,
            _updates: updates,
            _host_key_updates: host_key_updates,
            _log_updates: log_updates,
            host_key_prompts,
            logs,
            log_filter: LogFilter::All,
            log_search,
            _log_search_subscription: log_search_subscription,
            editor: None,
            context_session: 1,
            pending_page: None,
            group_editor: None,
            pending_group_delete: None,
            saving: false,
            _save_task: None,
            _file_task: None,
            file_busy: false,
            import_preview: None,
            ssh_preview: None,
            ssh_selected: Vec::new(),
            file_message: None,
        }
    }

    fn new_host(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok((_, config)) = &self.startup else {
            return;
        };
        let mut n = config.hosts.len() + 1;
        let id = loop {
            let id = HostId(format!("host-{n}"));
            if config.hosts.iter().all(|host| host.id != id) {
                break id;
            }
            n += 1;
        };
        self.editor = Some(Editor::Host(Box::new(HostEditor::new(
            id, None, window, cx,
        ))));
        self.advance_context();
        cx.notify();
    }

    fn edit_host(&mut self, id: &HostId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self
            .startup
            .as_ref()
            .ok()
            .and_then(|(_, config)| config.hosts.iter().find(|host| &host.id == id))
        else {
            return;
        };
        self.editor = Some(Editor::Host(Box::new(HostEditor::new(
            id.clone(),
            Some(host),
            window,
            cx,
        ))));
        self.advance_context();
        cx.notify();
    }

    fn new_tunnel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok((_, config)) = &self.startup else {
            return;
        };
        let mut n = config.tunnels.len() + 1;
        let id = loop {
            let id = TunnelId(format!("tunnel-{n}"));
            if config.tunnels.iter().all(|tunnel| tunnel.id != id) {
                break id;
            }
            n += 1;
        };
        self.editor = Some(Editor::Tunnel(Box::new(TunnelEditor::new(
            id, None, window, cx,
        ))));
        self.advance_context();
        cx.notify();
    }

    fn edit_tunnel(&mut self, id: &TunnelId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tunnel) = self
            .startup
            .as_ref()
            .ok()
            .and_then(|(_, config)| config.tunnels.iter().find(|tunnel| &tunnel.id == id))
        else {
            return;
        };
        self.editor = Some(Editor::Tunnel(Box::new(TunnelEditor::new(
            id.clone(),
            Some(tunnel),
            window,
            cx,
        ))));
        self.advance_context();
        cx.notify();
    }

    fn new_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok((_, config)) = &self.startup else {
            return;
        };
        let mut number = config.groups.len() + 1;
        let id = loop {
            let id = GroupId(format!("group-{number}"));
            if config.groups.iter().all(|group| group.id != id) {
                break id;
            }
            number += 1;
        };
        self.group_editor = Some(GroupEditor {
            id,
            name: cx.new(|cx| InputState::new(window, cx)),
        });
        self.advance_context();
        cx.notify();
    }

    fn edit_group(&mut self, id: &GroupId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group) = self
            .startup
            .as_ref()
            .ok()
            .and_then(|(_, config)| config.groups.iter().find(|group| &group.id == id))
        else {
            return;
        };
        self.group_editor = Some(GroupEditor {
            id: id.clone(),
            name: cx.new(|cx| InputState::new(window, cx).default_value(&group.name)),
        });
        self.advance_context();
        cx.notify();
    }

    fn save_group(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let (Some(editor), Ok((directory, current))) = (&self.group_editor, &self.startup) else {
            return;
        };
        let name = editor.name.read(cx).value().to_string().trim().to_owned();
        if name.is_empty() {
            self.command_error = Some("Group name is required".into());
            cx.notify();
            return;
        }
        if current
            .groups
            .iter()
            .any(|group| group.id != editor.id && group.name.eq_ignore_ascii_case(&name))
        {
            self.command_error = Some("A group with this name already exists".into());
            cx.notify();
            return;
        }
        let mut config = current.clone();
        if let Some(group) = config.groups.iter_mut().find(|group| group.id == editor.id) {
            group.name = name;
        } else {
            let order = config
                .groups
                .iter()
                .map(|group| group.sort_order)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            config.groups.push(TunnelGroup {
                id: editor.id.clone(),
                name,
                sort_order: order,
            });
        }
        self.persist_config(directory.clone(), config, None, cx);
    }

    fn delete_group(&mut self, id: &GroupId, cx: &mut Context<Self>) {
        let Ok((directory, current)) = &self.startup else {
            return;
        };
        let mut config = current.clone();
        config.groups.retain(|group| &group.id != id);
        for tunnel in &mut config.tunnels {
            if tunnel.group_id.as_ref() == Some(id) {
                tunnel.group_id = None;
            }
        }
        self.persist_config(directory.clone(), config, None, cx);
    }

    fn move_group(&mut self, id: &GroupId, direction: isize, cx: &mut Context<Self>) {
        let Ok((directory, current)) = &self.startup else {
            return;
        };
        let mut config = current.clone();
        config.groups.sort_by_key(|group| group.sort_order);
        let Some(index) = config.groups.iter().position(|group| &group.id == id) else {
            return;
        };
        let target = index as isize + direction;
        if target < 0 || target >= config.groups.len() as isize {
            return;
        }
        config.groups.swap(index, target as usize);
        for (order, group) in config.groups.iter_mut().enumerate() {
            group.sort_order = order as u32;
        }
        self.persist_config(directory.clone(), config, None, cx);
    }

    fn save_editor(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some(editor) = &self.editor else {
            return;
        };
        let Ok((directory, current)) = &self.startup else {
            return;
        };
        let directory = directory.clone();
        let mut config = current.clone();
        let mut secret = None;
        let edit = match editor {
            Editor::Host(editor) => editor.collect(cx).map(|(host, update)| {
                secret = update;
                if let Some(index) = config.hosts.iter().position(|old| old.id == host.id) {
                    config.hosts[index] = host;
                } else {
                    config.hosts.push(host);
                }
            }),
            Editor::Tunnel(editor) => editor.collect(cx).map(|tunnel| {
                if let Some(index) = config.tunnels.iter().position(|old| old.id == tunnel.id) {
                    config.tunnels[index] = tunnel;
                } else {
                    config.tunnels.push(tunnel);
                }
            }),
        };
        if let Err(error) = edit {
            self.command_error = Some(error);
            cx.notify();
            return;
        }
        if let Err(error) =
            ConfigDocument::try_from(config.clone()).and_then(ConfigDocument::into_domain)
        {
            self.command_error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.persist_config(directory, config, secret, cx);
    }

    fn persist_config(
        &mut self,
        directory: PathBuf,
        config: DomainConfig,
        secret: Option<SecretUpdate>,
        cx: &mut Context<Self>,
    ) {
        let Some(manager) = &self.manager else {
            self.command_error = Some("Tunnel runtime is unavailable".into());
            cx.notify();
            return;
        };
        let (reply, answer) = oneshot::channel();
        if let Err(error) = manager.try_send(CoreCommand::SaveConfig {
            directory: directory.clone(),
            config,
            secret,
            reply,
        }) {
            self.command_error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.saving = true;
        self.command_error = None;
        self.save_warning = None;
        let submitted_context = self.context_session;
        self._save_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = answer
                .await
                .unwrap_or_else(|_| Err("Configuration manager stopped".into()));
            let _ = this.update(cx, |view, cx| {
                view.saving = false;
                match result {
                    Ok(receipt) => view.apply_save_receipt(directory, receipt, submitted_context),
                    Err(error) => view.command_error = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn request(&mut self, command: CoreCommand, cx: &mut Context<Self>) {
        self.command_error = self.manager.as_ref().and_then(|manager| {
            manager
                .try_send(command)
                .err()
                .map(|error| error.to_string())
        });
        cx.notify();
    }

    fn choose_import(&mut self, cx: &mut Context<Self>) {
        if self.file_busy {
            return;
        }
        let Some(manager) = self.manager.clone() else {
            self.file_message = Some("Tunnel runtime is unavailable".into());
            cx.notify();
            return;
        };
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import TunnelWarden configuration".into()),
        });
        self.file_busy = true;
        self._file_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = match picker.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(path) => {
                        let (reply, answer) = oneshot::channel();
                        match manager.try_send(CoreCommand::ImportConfig { path, reply }) {
                            Ok(()) => answer.await.unwrap_or_else(|_| Err("Configuration manager stopped".into())).map(Some),
                            Err(error) => Err(error.to_owned()),
                        }
                    }
                    None => Ok(None),
                },
                Ok(Ok(None)) => Ok(None),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |view, cx| {
                view.file_busy = false;
                match result {
                    Ok(Some(config)) => {
                        view.file_message = Some(format!("Import preview: {} jumpers, {} tunnels. Existing configuration will be replaced after confirmation.", config.hosts.len(), config.tunnels.len()));
                        view.import_preview = Some(config);
                    }
                    Ok(None) => {},
                    Err(error) => view.file_message = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn confirm_import(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some(config) = self.import_preview.clone() else {
            return;
        };
        let Ok((directory, _)) = &self.startup else {
            return;
        };
        self.persist_config(directory.clone(), config, None, cx);
    }

    fn choose_ssh_import(&mut self, cx: &mut Context<Self>) {
        if self.file_busy {
            return;
        }
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import OpenSSH config".into()),
        });
        self.file_busy = true;
        self._file_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = match picker.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(path) => {
                        let (reply, answer) = oneshot::channel();
                        match manager.try_send(CoreCommand::PreviewSshConfig { path, reply }) {
                            Ok(()) => answer.await.unwrap_or_else(|_| Err("Configuration manager stopped".into())).map(Some),
                            Err(error) => Err(error.to_owned()),
                        }
                    }
                    None => Ok(None),
                },
                Ok(Ok(None)) => Ok(None),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |view, cx| {
                view.file_busy = false;
                match result {
                    Ok(Some(preview)) => {
                        view.ssh_selected = preview.entries.iter().map(|entry| {
                            let conflict = view.startup.as_ref().ok().is_some_and(|(_, config)| config.hosts.iter().any(|host| host.id == entry.host.id || host.name == entry.host.name));
                            !conflict && !entry.host.username.is_empty()
                        }).collect();
                        view.ssh_preview = Some(preview);
                        view.page_index = 0;
                        view.import_preview = None;
                        view.file_message = Some("Select jumpers to add, then confirm. Existing jumpers will be preserved.".into());
                    }
                    Ok(None) => {},
                    Err(error) => view.file_message = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn confirm_ssh_import(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let (Some(preview), Ok((directory, current))) = (&self.ssh_preview, &self.startup) else {
            return;
        };
        let mut config = current.clone();
        for (entry, selected) in preview.entries.iter().zip(&self.ssh_selected) {
            if !selected {
                continue;
            }
            if entry.host.username.is_empty() {
                self.command_error = Some(format!("Missing user for {}", entry.alias));
                cx.notify();
                return;
            }
            let mut host = entry.host.clone();
            if config.hosts.iter().any(|existing| {
                existing.id == host.id || existing.name.eq_ignore_ascii_case(&host.name)
            }) {
                let Some((id, name)) = (2..=1024)
                    .map(|suffix| {
                        (
                            HostId(format!("{}-{suffix}", entry.host.id.0)),
                            format!("{} ({suffix})", entry.host.name),
                        )
                    })
                    .find(|(id, name)| {
                        config.hosts.iter().all(|existing| {
                            existing.id != *id && !existing.name.eq_ignore_ascii_case(name)
                        })
                    })
                else {
                    self.command_error =
                        Some(format!("Could not find a unique name for {}", entry.alias));
                    cx.notify();
                    return;
                };
                host.id = id;
                host.name = name;
            }
            config.hosts.push(host);
        }
        if let Err(error) =
            ConfigDocument::try_from(config.clone()).and_then(ConfigDocument::into_domain)
        {
            self.command_error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.persist_config(directory.clone(), config, None, cx);
    }

    fn choose_export(&mut self, cx: &mut Context<Self>) {
        if self.file_busy {
            return;
        }
        let Some(manager) = self.manager.clone() else {
            self.file_message = Some("Tunnel runtime is unavailable".into());
            cx.notify();
            return;
        };
        let Ok((directory, config)) = &self.startup else {
            return;
        };
        let config = config.clone();
        let picker = cx.prompt_for_new_path(directory, Some("tunnelwarden-config.toml"));
        self.file_busy = true;
        self._file_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = match picker.await {
                Ok(Ok(Some(path))) => {
                    let (reply, answer) = oneshot::channel();
                    match manager.try_send(CoreCommand::ExportConfig {
                        path: path.clone(),
                        config,
                        reply,
                    }) {
                        Ok(()) => answer
                            .await
                            .unwrap_or_else(|_| Err("Configuration manager stopped".into()))
                            .map(|_| Some(path)),
                        Err(error) => Err(error.to_owned()),
                    }
                }
                Ok(Ok(None)) => Ok(None),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |view, cx| {
                view.file_busy = false;
                match result {
                    Ok(Some(path)) => {
                        view.file_message = Some(format!(
                            "Exported to {}. Passwords and passphrases were not included.",
                            path.display()
                        ))
                    }
                    Ok(None) => {}
                    Err(error) => view.file_message = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn select_page(&mut self, page: Page, cx: &mut Context<Self>) {
        if (self.editor.is_some() || self.group_editor.is_some()) && self.page != page {
            self.pending_page = Some(page);
            cx.notify();
            return;
        }
        self.advance_context();
        self.page = page;
        self.page_index = 0;
        self.editor = None;
        self.group_editor = None;
        self.pending_page = None;
        self.pending_group_delete = None;
        cx.notify();
    }

    fn discard_draft_and_navigate(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.pending_page.take() else {
            return;
        };
        self.editor = None;
        self.group_editor = None;
        self.command_error = None;
        self.page = page;
        self.page_index = 0;
        self.advance_context();
        cx.notify();
    }

    fn save_draft_and_navigate(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if self.editor.is_some() {
            self.save_editor(cx);
        } else if self.group_editor.is_some() {
            self.save_group(cx);
        }
    }

    fn page_count(&self) -> usize {
        let Ok((_, config)) = &self.startup else {
            return 0;
        };
        match self.page {
            Page::Jumpers => config.hosts.len(),
            Page::Tunnels => config.tunnels.len(),
            Page::Groups => config.groups.len(),
            _ => 0,
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut sidebar = div()
            .flex()
            .flex_col()
            .w(rems(13.))
            .h_full()
            .p_3()
            .gap_1()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(div().px_2().py_3().font_semibold().child("TunnelWarden"));
        for page in [
            Page::Overview,
            Page::Jumpers,
            Page::Tunnels,
            Page::Groups,
            Page::Logs,
        ] {
            sidebar = sidebar.child(
                Button::new(page.title())
                    .ghost()
                    .label(page.title())
                    .selected(self.page == page)
                    .toggled(self.page == page)
                    .on_click(cx.listener(move |this, _, _, cx| this.select_page(page, cx))),
            );
        }
        sidebar.child(div().flex_1()).child(
            Button::new("Settings")
                .ghost()
                .label("Settings")
                .selected(self.page == Page::Settings)
                .toggled(self.page == Page::Settings)
                .on_click(cx.listener(|this, _, _, cx| this.select_page(Page::Settings, cx))),
        )
    }

    fn render_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .px_6()
                    .py_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .text_lg()
                    .font_semibold()
                    .child(self.page.title()),
            );
        let content = if let Some(prompt) = self.host_key_prompts.first() {
            content.child(self.render_host_key_prompt(prompt, cx))
        } else {
            content
        };
        let content = if let Some(page) = self.pending_page {
            content.child(
                div()
                    .px_6()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().warning)
                    .child(format!("Leave this draft for {}?", page.title()))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .pt_2()
                            .child(
                                Button::new("save-draft-and-leave")
                                    .primary()
                                    .label("Save")
                                    .disabled(self.saving)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save_draft_and_navigate(cx)
                                    })),
                            )
                            .child(
                                Button::new("discard-draft-and-leave")
                                    .label("Discard")
                                    .disabled(self.saving)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.discard_draft_and_navigate(cx)
                                    })),
                            )
                            .child(
                                Button::new("stay-with-draft")
                                    .ghost()
                                    .label("Stay")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.pending_page = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
        } else {
            content
        };
        let body = match &self.startup {
            Ok((_, config)) if self.editor.is_some() => self.render_editor(config, cx),
            Ok((_, _)) if self.group_editor.is_some() => self.render_group_editor(cx),
            Ok((directory, config)) => self.render_page(directory, config, cx),
            Err(error) => div()
                .p_6()
                .text_color(cx.theme().danger)
                .child(format!("Could not load configuration: {error}"))
                .into_any_element(),
        };
        let content = content.child(div().flex_1().min_h_0().overflow_y_scrollbar().child(body));
        if let Some(error) = &self.command_error {
            content.child(
                div()
                    .px_6()
                    .py_2()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            )
        } else if let Some(warning) = &self.save_warning {
            content.child(
                div()
                    .px_6()
                    .py_2()
                    .text_color(cx.theme().warning)
                    .child(warning.clone()),
            )
        } else if let Some(error) = &self.runtime_error {
            content.child(
                div()
                    .px_6()
                    .py_2()
                    .text_color(cx.theme().danger)
                    .child(format!("Could not start tunnel runtime: {error}")),
            )
        } else {
            content
        }
    }

    fn render_host_key_prompt(
        &self,
        prompt: &HostKeyPromptView,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = prompt.id;
        div()
            .p_4()
            .mx_6()
            .mt_3()
            .flex()
            .flex_col()
            .gap_2()
            .border_1()
            .border_color(cx.theme().warning)
            .child(
                div()
                    .font_semibold()
                    .child("The authenticity of this SSH host cannot be established"),
            )
            .child(format!(
                "{}:{} · {} · {}",
                prompt.host, prompt.port, prompt.algorithm, prompt.fingerprint
            ))
            .child(format!(
                "{} pending host key confirmation(s)",
                self.host_key_prompts.len()
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new(format!("trust-once-{id}"))
                            .label("Trust Once")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.request(
                                    CoreCommand::ResolveHostKey {
                                        id,
                                        decision: HostKeyDecision::TrustOnce,
                                    },
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new(format!("trust-save-{id}"))
                            .primary()
                            .label("Trust & Save")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.request(
                                    CoreCommand::ResolveHostKey {
                                        id,
                                        decision: HostKeyDecision::TrustAndSave,
                                    },
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new(format!("trust-cancel-{id}"))
                            .label("Cancel")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.request(
                                    CoreCommand::ResolveHostKey {
                                        id,
                                        decision: HostKeyDecision::Cancel,
                                    },
                                    cx,
                                )
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_page(
        &self,
        directory: &Path,
        config: &DomainConfig,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        match self.page {
            Page::Overview => {
                let healthy = self
                    .snapshot
                    .values()
                    .filter(|view| matches!(view.state, SupervisorState::Healthy { .. }))
                    .count();
                let reconnecting = self
                    .snapshot
                    .values()
                    .filter(|view| matches!(view.state, SupervisorState::Reconnecting { .. }))
                    .count();
                let degraded = self
                    .snapshot
                    .values()
                    .filter(|view| matches!(view.state, SupervisorState::Degraded { .. }))
                    .count();
                let blocked = self
                    .snapshot
                    .values()
                    .filter(|view| matches!(view.state, SupervisorState::Blocked { .. }))
                    .count();
                div()
                    .p_6()
                    .gap_4()
                    .flex()
                    .flex_col()
                    .child(format!(
                        "{healthy} Healthy  ·  {degraded} Degraded  ·  {reconnecting} Reconnecting  ·  {blocked} Blocked"
                    ))
                    .child(format!(
                        "{} saved tunnels · {} jumpers",
                        config.tunnels.len(),
                        config.hosts.len()
                    ))
                    .child("Open Tunnels to start or inspect a connection.")
                    .into_any_element()
            }
            Page::Jumpers => self.render_hosts(config, cx).into_any_element(),
            Page::Tunnels => self.render_tunnels(config, cx).into_any_element(),
            Page::Groups => self.render_groups(config, cx).into_any_element(),
            Page::Logs => self.render_logs(cx),
            Page::Settings => self.render_settings(directory, config, cx),
        }
    }

    fn render_logs(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let query = self.log_search.read(cx).value().trim().to_ascii_lowercase();
        let matches = |event: &LogEvent| {
            self.log_filter.matches(event)
                && (query.is_empty()
                    || event.subject.to_ascii_lowercase().contains(&query)
                    || event.stage.to_ascii_lowercase().contains(&query)
                    || event.message.to_ascii_lowercase().contains(&query))
        };
        let total = self.logs.iter().filter(|event| matches(event)).count();
        let start = self.page_index.saturating_mul(PAGE_SIZE);
        let mut filters = div().flex().flex_wrap().gap_1();
        for filter in [
            LogFilter::All,
            LogFilter::Debug,
            LogFilter::Info,
            LogFilter::Warning,
            LogFilter::Error,
            LogFilter::Tunnel,
            LogFilter::Host,
        ] {
            filters = filters.child(
                Button::new(format!("log-filter-{}", filter.label()))
                    .label(filter.label())
                    .selected(self.log_filter == filter)
                    .toggled(self.log_filter == filter)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.log_filter = filter;
                        this.page_index = 0;
                        cx.notify();
                    })),
            );
        }
        let mut rows = div().flex().flex_col().gap_1();
        for (index, event) in self
            .logs
            .iter()
            .rev()
            .filter(|event| matches(event))
            .skip(start)
            .take(PAGE_SIZE)
            .enumerate()
        {
            let time = event.timestamp_unix % 86_400;
            let level = match event.level {
                LogLevel::Debug => "DEBUG",
                LogLevel::Info => "INFO",
                LogLevel::Warning => "WARN",
                LogLevel::Error => "ERROR",
            };
            rows = rows.child(
                div()
                    .id(format!("log-row-{}", start + index))
                    .test_support()
                    .aria_label(format!(
                        "{} {} {} {}",
                        level, event.subject, event.stage, event.message
                    ))
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(format!(
                        "{:02}:{:02}:{:02} UTC  {level}  {}  {}  {}",
                        time / 3600,
                        (time / 60) % 60,
                        time % 60,
                        event.subject,
                        event.stage,
                        event.message
                    )),
            );
        }
        div()
            .p_6()
            .flex()
            .flex_col()
            .gap_3()
            .child(filters)
            .child("Search events")
            .child(Input::new(&self.log_search).id("log-search"))
            .child(if total == 0 {
                "No matching runtime events"
            } else {
                "Recent runtime events (newest first)"
            })
            .child(rows)
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(format!(
                        "{}–{} of {}",
                        if total == 0 { 0 } else { start + 1 },
                        (start + PAGE_SIZE).min(total),
                        total
                    ))
                    .child(
                        Button::new("logs-previous")
                            .label("Previous")
                            .disabled(start == 0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.page_index = this.page_index.saturating_sub(1);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("logs-next")
                            .label("Next")
                            .disabled(start + PAGE_SIZE >= total)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.page_index += 1;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_settings(
        &self,
        directory: &Path,
        config: &DomainConfig,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let mut body = div()
            .p_6()
            .flex()
            .flex_col()
            .gap_3()
            .child("Configuration folder")
            .child(directory.display().to_string())
            .child(
                Button::new("run-at-login")
                    .label(if config.app.run_at_startup {
                        "Launch at Windows login: on"
                    } else {
                        "Launch at Windows login: off"
                    })
                    .disabled(self.saving)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let Ok((directory, current)) = &this.startup else {
                            return;
                        };
                        let mut config = current.clone();
                        config.app.run_at_startup = !config.app.run_at_startup;
                        this.persist_config(directory.clone(), config, None, cx);
                    })),
            )
            .child(
                Button::new("close-mode")
                    .label(if config.app.minimize_to_tray {
                        "Close window: keep tunnels in tray"
                    } else {
                        "Close window: quit application"
                    })
                    .disabled(self.saving)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let Ok((directory, current)) = &this.startup else {
                            return;
                        };
                        let mut config = current.clone();
                        config.app.minimize_to_tray = !config.app.minimize_to_tray;
                        this.persist_config(directory.clone(), config, None, cx);
                    })),
            )
            .child(
                Button::new("import-config")
                    .label("Import TunnelWarden config…")
                    .disabled(self.file_busy || self.saving)
                    .on_click(cx.listener(|this, _, _, cx| this.choose_import(cx))),
            )
            .child(
                Button::new("export-config")
                    .label("Export config…")
                    .disabled(self.file_busy || self.saving)
                    .on_click(cx.listener(|this, _, _, cx| this.choose_export(cx))),
            )
            .child(
                Button::new("import-ssh-config")
                    .label("Import OpenSSH config…")
                    .disabled(self.file_busy || self.saving)
                    .on_click(cx.listener(|this, _, _, cx| this.choose_ssh_import(cx))),
            );
        if let Some(message) = &self.file_message {
            body = body.child(message.clone());
        }
        if !config.app.minimize_to_tray {
            body = body.child(
                div()
                    .text_color(cx.theme().danger)
                    .child("Closing the window will stop all tunnels and quit TunnelWarden."),
            );
        }
        if let Some(preview) = &self.import_preview {
            body = body.child(div().font_semibold().child("Import preview"));
            for host in preview.hosts.iter().take(PAGE_SIZE) {
                body = body.child(format!(
                    "{} · {}@{}:{}",
                    host.name, host.username, host.hostname, host.port
                ));
            }
            if preview.hosts.len() > PAGE_SIZE {
                body = body.child(format!(
                    "… {} more jumpers",
                    preview.hosts.len() - PAGE_SIZE
                ));
            }
            body = body
                .child(format!(
                    "{} tunnels will be imported",
                    preview.tunnels.len()
                ))
                .child(
                    Button::new("confirm-import")
                        .primary()
                        .label("Replace configuration")
                        .disabled(self.saving)
                        .on_click(cx.listener(|this, _, _, cx| this.confirm_import(cx))),
                )
                .child(
                    Button::new("cancel-import")
                        .label("Cancel import")
                        .disabled(self.saving)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.import_preview = None;
                            this.file_message = None;
                            cx.notify();
                        })),
                );
        }
        if let Some(preview) = &self.ssh_preview {
            body = body.child(div().font_semibold().child("OpenSSH import preview"));
            for warning in preview.warnings.iter().take(8) {
                body = body.child(div().text_color(cx.theme().warning).child(warning.clone()));
            }
            for (index, entry) in preview
                .entries
                .iter()
                .enumerate()
                .skip(self.page_index * PAGE_SIZE)
                .take(PAGE_SIZE)
            {
                let conflict = config
                    .hosts
                    .iter()
                    .any(|host| host.id == entry.host.id || host.name == entry.host.name);
                let selected = self.ssh_selected.get(index).copied().unwrap_or(false);
                let status = if conflict {
                    "Duplicate · rename on import"
                } else if entry.host.username.is_empty() {
                    "Needs user"
                } else if !entry.warnings.is_empty() {
                    "Warnings"
                } else {
                    "Ready"
                };
                body = body.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new(format!("ssh-import-{index}"))
                                .label(if selected {
                                    "✓ Import"
                                } else if conflict {
                                    "Rename & add"
                                } else {
                                    "Skip"
                                })
                                .selected(selected)
                                .disabled(entry.host.username.is_empty() || self.saving)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(selected) = this.ssh_selected.get_mut(index) {
                                        *selected = !*selected;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(format!(
                            "{} → {}@{}:{} · {} · {}",
                            entry.alias,
                            entry.host.username,
                            entry.host.hostname,
                            entry.host.port,
                            entry.proxy_jump.as_deref().unwrap_or("direct"),
                            status
                        )),
                );
                for warning in entry.warnings.iter().take(3) {
                    body = body.child(div().text_color(cx.theme().warning).child(warning.clone()));
                }
            }
            if preview.entries.len() > PAGE_SIZE {
                let pages = preview.entries.len().div_ceil(PAGE_SIZE);
                body = body.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("ssh-prev")
                                .label("Previous")
                                .disabled(self.page_index == 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.page_index = this.page_index.saturating_sub(1);
                                    cx.notify();
                                })),
                        )
                        .child(format!("Page {} of {}", self.page_index + 1, pages))
                        .child(
                            Button::new("ssh-next")
                                .label("Next")
                                .disabled(self.page_index + 1 >= pages)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.page_index += 1;
                                    cx.notify();
                                })),
                        ),
                );
            }
            body = body
                .child(
                    Button::new("confirm-ssh-import")
                        .primary()
                        .label("Add selected jumpers")
                        .disabled(
                            self.saving || !self.ssh_selected.iter().any(|selected| *selected),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.confirm_ssh_import(cx))),
                )
                .child(
                    Button::new("cancel-ssh-import")
                        .label("Cancel import")
                        .disabled(self.saving)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ssh_preview = None;
                            this.ssh_selected.clear();
                            this.file_message = None;
                            cx.notify();
                        })),
                );
        }
        body.into_any_element()
    }

    fn render_editor(&self, config: &DomainConfig, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut form = div().p_6().flex().flex_col().gap_3().max_w(rems(42.));
        match self.editor.as_ref() {
            Some(Editor::Host(editor)) => {
                form = form
                    .child(div().text_lg().font_semibold().child("Jumper connection"))
                    .child(input_row("Name", &editor.name))
                    .child(input_row("Host", &editor.hostname))
                    .child(input_row("Port", &editor.port))
                    .child(input_row("User", &editor.username))
                    .child(div().font_semibold().child("Authentication"));
                for (label, kind) in [
                    ("SSH Agent", 0),
                    ("Private Key", 1),
                    ("Password", 2),
                    ("Keyboard Interactive", 3),
                ] {
                    let selected = matches!(
                        (&editor.auth, kind),
                        (AuthConfig::Agent { .. }, 0)
                            | (AuthConfig::PrivateKey { .. }, 1)
                            | (AuthConfig::Password { .. }, 2)
                            | (AuthConfig::KeyboardInteractive, 3)
                    );
                    form = form.child(
                        Button::new(format!("auth-{kind}"))
                            .label(label)
                            .selected(selected)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(Editor::Host(editor)) = &mut this.editor {
                                    editor.auth = match kind {
                                        0 => AuthConfig::Agent { socket: None },
                                        1 => match &editor.original_auth {
                                            AuthConfig::PrivateKey {
                                                key_path,
                                                passphrase_ref,
                                            } => AuthConfig::PrivateKey {
                                                key_path: key_path.clone(),
                                                passphrase_ref: passphrase_ref.clone(),
                                            },
                                            _ => AuthConfig::PrivateKey {
                                                key_path: PathBuf::new(),
                                                passphrase_ref: None,
                                            },
                                        },
                                        2 => match &editor.original_auth {
                                            AuthConfig::Password { credential_ref } => {
                                                AuthConfig::Password {
                                                    credential_ref: credential_ref.clone(),
                                                }
                                            }
                                            _ => AuthConfig::Password {
                                                credential_ref: SecretRef("pending".into()),
                                            },
                                        },
                                        _ => AuthConfig::KeyboardInteractive,
                                    };
                                    cx.notify();
                                }
                            })),
                    );
                }
                if matches!(editor.auth, AuthConfig::PrivateKey { .. }) {
                    form = form
                        .child(input_row("Identity file", &editor.identity_file))
                        .child(password_row(
                            "Key passphrase (leave blank to keep saved value)",
                            &editor.passphrase,
                        ));
                }
                if matches!(editor.auth, AuthConfig::Password { .. }) {
                    form = form.child(password_row(
                        "Password (leave blank to keep saved value)",
                        &editor.password,
                    ));
                }
                if matches!(editor.auth, AuthConfig::Agent { .. }) {
                    form = form.child(input_row("Agent socket (optional)", &editor.agent_socket));
                }
                form = form.child(div().font_semibold().child("Security"));
                for (label, policy) in [
                    ("Strict host key", HostKeyPolicy::Strict),
                    ("Bypass verification (unsafe)", HostKeyPolicy::Bypass),
                ] {
                    form = form.child(
                        Button::new(format!("policy-{label}"))
                            .label(label)
                            .selected(editor.policy == policy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(Editor::Host(editor)) = &mut this.editor {
                                    editor.policy = policy;
                                    cx.notify();
                                }
                            })),
                    );
                }
                if editor.policy == HostKeyPolicy::Bypass {
                    form = form.child(
                        div()
                            .text_color(cx.theme().danger)
                            .child("SSH host identity will not be verified for this jumper."),
                    );
                }
                form = form
                    .child(div().font_semibold().child("Reliability"))
                    .child(input_row(
                        "Connect timeout (ms)",
                        &editor.connect_timeout_ms,
                    ))
                    .child(input_row(
                        "Keepalive interval (ms)",
                        &editor.keepalive_interval_ms,
                    ))
                    .child(input_row("Keepalive max failures", &editor.keepalive_max))
                    .child(div().font_semibold().child("Notes"))
                    .child(input_row("Notes", &editor.notes));
            }
            Some(Editor::Tunnel(editor)) => {
                form = form
                    .child(div().text_lg().font_semibold().child("Tunnel editor"))
                    .child(input_row("Name", &editor.name))
                    .child(input_row("Description", &editor.description))
                    .child(div().font_semibold().child("Group"))
                    .child(
                        Button::new("group-ungrouped")
                            .label("Ungrouped")
                            .selected(editor.group_id.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(Editor::Tunnel(editor)) = &mut this.editor {
                                    editor.group_id = None;
                                    cx.notify();
                                }
                            })),
                    );
                let mut groups: Vec<_> = config.groups.iter().collect();
                groups.sort_by_key(|group| group.sort_order);
                for group in groups {
                    let id = group.id.clone();
                    form = form.child(
                        Button::new(format!("group-option-{}", id.0))
                            .label(group.name.clone())
                            .selected(editor.group_id.as_ref() == Some(&id))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(Editor::Tunnel(editor)) = &mut this.editor {
                                    editor.group_id = Some(id.clone());
                                    cx.notify();
                                }
                            })),
                    );
                }
                form = form.child(div().font_semibold().child("Mode"));
                for (label, mode) in [
                    ("Local", TunnelMode::Local),
                    ("Remote", TunnelMode::Remote),
                    ("Dynamic SOCKS5", TunnelMode::Dynamic),
                ] {
                    form = form.child(
                        Button::new(format!("mode-{label}"))
                            .label(label)
                            .selected(editor.mode == mode)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(Editor::Tunnel(editor)) = &mut this.editor {
                                    editor.mode = mode;
                                    cx.notify();
                                }
                            })),
                    );
                }
                let local_label = if editor.mode == TunnelMode::Remote {
                    "Local destination host"
                } else {
                    "Listen host"
                };
                let port_label = if editor.mode == TunnelMode::Remote {
                    "Local destination port"
                } else {
                    "Listen port"
                };
                form = form
                    .child(input_row(local_label, &editor.local_host))
                    .child(input_row(port_label, &editor.local_port));
                if editor.mode != TunnelMode::Dynamic {
                    form = form
                        .child(input_row(
                            if editor.mode == TunnelMode::Remote {
                                "Remote listen host"
                            } else {
                                "Destination host"
                            },
                            &editor.remote_host,
                        ))
                        .child(input_row(
                            if editor.mode == TunnelMode::Remote {
                                "Remote listen port"
                            } else {
                                "Destination port"
                            },
                            &editor.remote_port,
                        ));
                }
                if editor.mode == TunnelMode::Dynamic
                    && editor.local_host.read(cx).value().as_ref() == "0.0.0.0"
                {
                    form = form.child(
                        div()
                            .text_color(cx.theme().danger)
                            .child("This exposes the proxy to other devices on the network."),
                    );
                }
                form = form.child(
                    div()
                        .font_semibold()
                        .child("Jump chain (first hop to final host)"),
                );
                for (index, id) in editor.jump_chain.iter().enumerate() {
                    let name = config
                        .hosts
                        .iter()
                        .find(|host| &host.id == id)
                        .map_or(id.0.as_str(), |host| host.name.as_str())
                        .to_owned();
                    let up = Button::new(format!("hop-up-{index}"))
                        .label("↑")
                        .disabled(index == 0)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(Editor::Tunnel(editor)) = &mut this.editor
                                && index > 0
                                && index < editor.jump_chain.len()
                            {
                                editor.jump_chain.swap(index, index - 1);
                                cx.notify();
                            }
                        }));
                    let down = Button::new(format!("hop-down-{index}"))
                        .label("↓")
                        .disabled(index + 1 == editor.jump_chain.len())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(Editor::Tunnel(editor)) = &mut this.editor
                                && index + 1 < editor.jump_chain.len()
                            {
                                editor.jump_chain.swap(index, index + 1);
                                cx.notify();
                            }
                        }));
                    let remove = Button::new(format!("hop-remove-{index}"))
                        .label("Remove")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(Editor::Tunnel(editor)) = &mut this.editor
                                && index < editor.jump_chain.len()
                            {
                                editor.jump_chain.remove(index);
                                cx.notify();
                            }
                        }));
                    form = form.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(format!("{}. {name}", index + 1))
                            .child(up)
                            .child(down)
                            .child(remove),
                    );
                }
                for host in &config.hosts {
                    if editor.jump_chain.contains(&host.id) {
                        continue;
                    }
                    let id = host.id.clone();
                    form = form.child(
                        Button::new(format!("hop-add-{}", id.0))
                            .label(format!("Add {}", host.name))
                            .disabled(editor.jump_chain.len() >= 16)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(Editor::Tunnel(editor)) = &mut this.editor
                                    && editor.jump_chain.len() < 16
                                    && !editor.jump_chain.contains(&id)
                                {
                                    editor.jump_chain.push(id.clone());
                                    cx.notify();
                                }
                            })),
                    );
                }
                form = form.child(
                    Button::new("auto-start")
                        .label(if editor.auto_start {
                            "Auto start: On"
                        } else {
                            "Auto start: Off"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(Editor::Tunnel(editor)) = &mut this.editor {
                                editor.auto_start = !editor.auto_start;
                                cx.notify();
                            }
                        })),
                );
            }
            None => {}
        }
        form.child(
            div()
                .flex()
                .gap_2()
                .pt_3()
                .child(
                    Button::new("save-editor")
                        .primary()
                        .label("Save")
                        .disabled(self.saving)
                        .on_click(cx.listener(|this, _, _, cx| this.save_editor(cx))),
                )
                .child(
                    Button::new("cancel-editor")
                        .label("Cancel")
                        .disabled(self.saving)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.editor = None;
                            this.command_error = None;
                            cx.notify();
                        })),
                ),
        )
        .into_any_element()
    }

    fn render_group_editor(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(editor) = &self.group_editor else {
            return div().into_any_element();
        };
        div()
            .p_6()
            .flex()
            .flex_col()
            .gap_3()
            .max_w(rems(32.))
            .child(div().text_lg().font_semibold().child("Tunnel group"))
            .child(input_row("Name", &editor.name))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("save-group")
                            .primary()
                            .label("Save Group")
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, _, cx| this.save_group(cx))),
                    )
                    .child(
                        Button::new("cancel-group")
                            .label("Cancel")
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.group_editor = None;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_groups(&self, config: &DomainConfig, cx: &mut Context<Self>) -> impl IntoElement {
        let mut groups: Vec<_> = config.groups.iter().collect();
        groups.sort_by_key(|group| group.sort_order);
        let start = self.page_index.saturating_mul(PAGE_SIZE);
        let mut rows = div().flex().flex_col().gap_2();
        for group in groups.into_iter().skip(start).take(PAGE_SIZE) {
            let id = group.id.clone();
            let tunnel_count = config
                .tunnels
                .iter()
                .filter(|tunnel| tunnel.group_id.as_ref() == Some(&id))
                .count();
            let pending = self.pending_group_delete.as_ref() == Some(&id);
            let edit_id = id.clone();
            let start_id = id.clone();
            let stop_id = id.clone();
            let up_id = id.clone();
            let down_id = id.clone();
            let delete_id = id.clone();
            rows = rows.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(format!("{} · {} tunnels", group.name, tunnel_count))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new(format!("group-edit-{}", id.0))
                                    .label("Rename")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.edit_group(&edit_id, window, cx)
                                    })),
                            )
                            .child(
                                Button::new(format!("group-start-{}", id.0))
                                    .label("Start All")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.request(CoreCommand::StartGroup(start_id.clone()), cx)
                                    })),
                            )
                            .child(
                                Button::new(format!("group-stop-{}", id.0))
                                    .label("Stop All")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.request(CoreCommand::StopGroup(stop_id.clone()), cx)
                                    })),
                            )
                            .child(
                                Button::new(format!("group-up-{}", id.0))
                                    .label("Up")
                                    .disabled(self.saving)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.move_group(&up_id, -1, cx)
                                    })),
                            )
                            .child(
                                Button::new(format!("group-down-{}", id.0))
                                    .label("Down")
                                    .disabled(self.saving)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.move_group(&down_id, 1, cx)
                                    })),
                            )
                            .child(
                                Button::new(format!("group-delete-{}", id.0))
                                    .label(if pending { "Confirm delete" } else { "Delete" })
                                    .disabled(self.saving)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.pending_group_delete.as_ref() == Some(&delete_id) {
                                            this.delete_group(&delete_id, cx);
                                        } else {
                                            this.pending_group_delete = Some(delete_id.clone());
                                            cx.notify();
                                        }
                                    })),
                            ),
                    )
                    .when(pending, |row| {
                        row.child("Deleting this group moves its tunnels to Ungrouped.")
                    }),
            );
        }
        div()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(if config.groups.is_empty() {
                "No groups configured"
            } else {
                "Tunnel groups"
            })
            .child(
                Button::new("new-group")
                    .primary()
                    .label("New Group")
                    .on_click(cx.listener(|this, _, window, cx| this.new_group(window, cx))),
            )
            .child(rows)
            .child(self.render_pager(cx))
    }

    fn render_hosts(&self, config: &DomainConfig, cx: &mut Context<Self>) -> impl IntoElement {
        let start = self.page_index.saturating_mul(PAGE_SIZE);
        let mut rows = div().flex().flex_col().gap_1();
        for host in config.hosts.iter().skip(start).take(PAGE_SIZE) {
            let id = host.id.clone();
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(host.name.clone())
                            .child(format!("{}@{}:{}", host.username, host.hostname, host.port)),
                    )
                    .child(
                        Button::new(format!("edit-host-{}", id.0))
                            .label("Edit")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit_host(&id, window, cx)
                            })),
                    ),
            );
        }
        div()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(if config.hosts.is_empty() {
                "No jumpers configured"
            } else {
                "Saved jumpers"
            })
            .child(
                Button::new("new-host")
                    .primary()
                    .label("New Jumper")
                    .on_click(cx.listener(|this, _, window, cx| this.new_host(window, cx))),
            )
            .child(rows)
            .child(self.render_pager(cx))
    }

    fn render_tunnels(&self, config: &DomainConfig, cx: &mut Context<Self>) -> impl IntoElement {
        let start = self.page_index.saturating_mul(PAGE_SIZE);
        let mut rows = div().flex().flex_col().gap_1();
        for tunnel in config.tunnels.iter().skip(start).take(PAGE_SIZE) {
            let mode = match tunnel.mode {
                TunnelMode::Local => "Local",
                TunnelMode::Remote => "Remote",
                TunnelMode::Dynamic => "SOCKS5",
            };
            let view = self.snapshot.get(&tunnel.id);
            let state = view
                .map(|view| &view.state)
                .unwrap_or(&SupervisorState::Stopped);
            let label = state_label(state);
            let details = status_details(state, view.and_then(|view| view.local_addr).is_some());
            let id = tunnel.id.clone();
            let available = actions_for(state);
            let button_label = match available.primary {
                TunnelAction::Start => "Start",
                TunnelAction::Retry => "Retry",
                TunnelAction::Stop => "Stop",
            };
            let action_kind = available.primary;
            let action = Button::new(format!("{}-action", id.0))
                .label(button_label)
                .disabled(self.manager.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let command = match action_kind {
                        TunnelAction::Start => CoreCommand::StartTunnel(id.clone()),
                        TunnelAction::Retry => CoreCommand::RetryTunnel(id.clone()),
                        TunnelAction::Stop => CoreCommand::StopTunnel(id.clone()),
                    };
                    this.request(command, cx);
                }));
            let mut controls = div()
                .flex()
                .gap_2()
                .child(
                    Button::new(format!("edit-tunnel-{}", tunnel.id.0))
                        .label("Edit")
                        .on_click({
                            let id = tunnel.id.clone();
                            cx.listener(move |this, _, window, cx| {
                                this.edit_tunnel(&id, window, cx)
                            })
                        }),
                )
                .child(action);
            if available.primary == TunnelAction::Retry && available.can_stop {
                let id = tunnel.id.clone();
                controls = controls.child(
                    Button::new(format!("stop-blocked-{}", id.0))
                        .label("Stop")
                        .disabled(self.manager.is_none())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.request(CoreCommand::StopTunnel(id.clone()), cx)
                        })),
                );
            }
            let mut info = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(tunnel.name.clone())
                .child(
                    div()
                        .id(format!("tunnel-status-{}", tunnel.id.0))
                        .test_support()
                        .aria_label(format!("{} {label}", tunnel.name))
                        .child(format!(
                            "{mode} · {}:{} · {label}",
                            tunnel.local.host, tunnel.local.port
                        )),
                );
            if let Some(details) = details {
                info = info.child(
                    div()
                        .id(format!("tunnel-details-{}", tunnel.id.0))
                        .test_support()
                        .aria_label(details.clone())
                        .text_color(cx.theme().muted_foreground)
                        .child(details),
                );
            }
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(info)
                    .child(controls),
            );
        }
        div()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .child(if config.tunnels.is_empty() {
                "No tunnels configured"
            } else {
                "Saved tunnels"
            })
            .child(
                Button::new("new-tunnel")
                    .primary()
                    .label("New Tunnel")
                    .on_click(cx.listener(|this, _, window, cx| this.new_tunnel(window, cx))),
            )
            .child(rows)
            .child(self.render_pager(cx))
    }

    fn render_pager(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let total = self.page_count();
        let page = self.page_index;
        div()
            .flex()
            .gap_2()
            .child(format!(
                "{}–{} of {}",
                if total == 0 { 0 } else { page * PAGE_SIZE + 1 },
                ((page + 1) * PAGE_SIZE).min(total),
                total
            ))
            .child(
                Button::new("previous-page")
                    .label("Previous")
                    .disabled(page == 0)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.page_index = this.page_index.saturating_sub(1);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("next-page")
                    .label("Next")
                    .disabled((page + 1) * PAGE_SIZE >= total)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.page_index += 1;
                        cx.notify();
                    })),
            )
    }
}

fn input_row(
    label: &'static str,
    input: &gpui_kit::Entity<gpui_kit::component::input::InputState>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(label)
        .child(Input::new(input).id(label))
}

fn password_row(
    label: &'static str,
    input: &gpui_kit::Entity<gpui_kit::component::input::InputState>,
) -> impl IntoElement {
    div().flex().flex_col().gap_1().child(label).child(
        Input::new(input)
            .id(label)
            .content_type(InputContentType::Password),
    )
}

fn state_label(state: &SupervisorState) -> &'static str {
    match state {
        SupervisorState::Connecting { .. } => "Connecting",
        SupervisorState::Reconnecting { .. } => "Reconnecting",
        SupervisorState::Healthy { .. } => "Healthy",
        SupervisorState::Degraded { .. } => "Degraded",
        SupervisorState::Blocked { .. } => "Blocked",
        SupervisorState::Stopped => "Stopped",
    }
}

fn status_details(state: &SupervisorState, listener_reserved: bool) -> Option<String> {
    match state {
        SupervisorState::Healthy { rtts, remote_port } => {
            let latency = rtts.last().map(|rtt| format!("{} ms", rtt.as_millis()));
            match (latency, remote_port) {
                (Some(latency), Some(port)) => Some(format!("{latency} · Remote port {port}")),
                (Some(latency), None) => Some(latency),
                (None, Some(port)) => Some(format!("Remote port {port}")),
                (None, None) => None,
            }
        }
        SupervisorState::Degraded {
            failed_pings,
            reason,
        } => Some(format!("Ping failure {failed_pings}/3 · {reason}")),
        SupervisorState::Reconnecting {
            attempt,
            delay,
            reason,
        } => {
            let listener = if listener_reserved {
                " · Local listener remains reserved"
            } else {
                ""
            };
            Some(format!(
                "Retry {attempt} after {}s · {reason}{listener}",
                delay.as_secs_f32()
            ))
        }
        SupervisorState::Blocked { reason } => Some(reason.clone()),
        SupervisorState::Connecting { attempt } => Some(format!("SSH attempt {attempt}")),
        SupervisorState::Stopped => None,
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_stretch()
            .size_full()
            .child(self.render_sidebar(cx))
            .child(self.render_content(cx))
    }
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt;
    use std::{collections::VecDeque, net::SocketAddr, time::Duration};
    use tunnel_domain::{LocalEndpoint, RetryPolicy, TunnelConfig};

    struct NoCredentials;
    impl tunnel_core::CredentialSource for NoCredentials {
        fn load(&self, _: &SecretRef) -> Result<zeroize::Zeroizing<String>, String> {
            Err("no credentials".into())
        }
    }

    #[gpui_kit::test]
    fn sidebar_opens_group_editor_and_cancel_restores_list(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let config = ConfigDocument::default()
            .into_domain()
            .expect("default config");
        let handle = cx.add_window(move |window, cx| {
            Workspace::new(Ok((PathBuf::new(), config)), None, None, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click("Groups", cx);
            window.click("new-group", cx);
            assert!(window.find("save-group").visible());
            window.click("cancel-group", cx);
            assert!(window.find("new-group").visible());
        })
        .expect("workspace window");
    }

    #[gpui_kit::test]
    fn stale_save_receipt_keeps_later_editor_and_its_draft(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let config = ConfigDocument::default()
            .into_domain()
            .expect("default config");
        let handle = cx.add_window(move |window, cx| {
            Workspace::new(Ok((PathBuf::new(), config)), None, None, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("Tunnels", cx);
            window.click("new-tunnel", cx);
        })
        .expect("first editor");
        let mut submitted_context = 0;
        cx.update(|app| {
            handle
                .update(app, |view, _, _| submitted_context = view.context_session)
                .expect("capture edit session");
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("Jumpers", cx);
            window.click("discard-draft-and-leave", cx);
            window.click("new-host", cx);
            window.click("Name", cx);
            window.input("Later draft", cx);
        })
        .expect("second editor");
        let mut committed = ConfigDocument::default()
            .into_domain()
            .expect("committed config");
        committed.app.theme = config_store::ThemePreference::Dark;
        cx.update(|app| {
            handle
                .update(app, |view, _, cx| {
                    view.apply_save_receipt(
                        PathBuf::new(),
                        SaveReceipt {
                            canonical_config: committed,
                            warnings: Vec::new(),
                        },
                        submitted_context,
                    );
                    assert!(matches!(&view.editor, Some(Editor::Host(editor)) if editor.name.read(cx).value() == "Later draft"));
                    assert_eq!(
                        view.startup.as_ref().expect("saved config").1.app.theme,
                        config_store::ThemePreference::Dark
                    );
                })
                .expect("later editor survives");
        });
    }

    #[gpui_kit::test]
    fn navigation_keeps_draft_until_user_saves_or_discards_it(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let config = ConfigDocument::default()
            .into_domain()
            .expect("default config");
        let handle = cx.add_window(move |window, cx| {
            Workspace::new(Ok((PathBuf::new(), config)), None, None, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("Tunnels", cx);
            window.click("new-tunnel", cx);
            window.click("Name", cx);
            window.input("Unfinished tunnel", cx);
            window.click("Jumpers", cx);
            assert!(window.find("save-draft-and-leave").visible());
            assert!(window.find("discard-draft-and-leave").visible());
            window.click("stay-with-draft", cx);
            assert_eq!(window.find("Name").value(), Some("Unfinished tunnel"));
            window.click("Jumpers", cx);
            window.click("save-draft-and-leave", cx);
            assert!(window.find("discard-draft-and-leave").visible());
            window.click("discard-draft-and-leave", cx);
            assert!(window.find("new-host").visible());
        })
        .expect("draft navigation");
        cx.update(|app| {
            handle
                .update(app, |view, _, _| {
                    assert_eq!(view.page, Page::Jumpers);
                    assert!(view.editor.is_none());
                })
                .expect("draft discarded");
        });
    }

    #[gpui_kit::test]
    fn tunnel_mode_host_key_prompt_and_settings_are_interactive(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let config = ConfigDocument::default()
            .into_domain()
            .expect("default config");
        let handle = cx.add_window(move |window, cx| {
            Workspace::new(Ok((PathBuf::new(), config)), None, None, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("Tunnels", cx);
            window.click("new-tunnel", cx);
            window.click("Name", cx);
            window.input("Demo SOCKS", cx);
            assert_eq!(window.find("Name").focused(), Some(true));
            assert_eq!(window.find("Name").value(), Some("Demo SOCKS"));
            assert!(window.find("mode-Dynamic SOCKS5").visible());
            window.click("mode-Local", cx);
            window.click("mode-Remote", cx);
            assert!(window.find("mode-Remote").visible());
        })
        .expect("tunnel editor window");
        cx.update(|app| {
            handle.update(app, |view, _, _| {
                assert!(matches!(&view.editor, Some(Editor::Tunnel(editor)) if editor.mode == TunnelMode::Remote));
            }).expect("tunnel editor state");
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("cancel-editor", cx);
            window.click("Settings", cx);
            assert!(window.find("import-ssh-config").visible());
            assert!(window.find("run-at-login").visible());
            assert_eq!(
                window.find("run-at-login").label(),
                Some("Launch at Windows login: off")
            );
            window.click("close-mode", cx);
        })
        .expect("workspace window");
        cx.update(|app| {
            handle
                .update(app, |view, _, _| {
                    assert_eq!(
                        view.command_error.as_deref(),
                        Some("Tunnel runtime is unavailable")
                    );
                })
                .expect("settings interaction");
        });
        let (closed_manager, manager_handle) = tunnel_core::TunnelManager::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Arc::new(NoCredentials),
        )
        .expect("test manager");
        drop(closed_manager);
        cx.update(|app| {
            handle
                .update(app, |view, _, cx| {
                    view.manager = Some(manager_handle);
                    view.host_key_prompts = Arc::new(vec![HostKeyPromptView {
                        id: 7,
                        host: "example.test".into(),
                        port: 22,
                        algorithm: "ssh-ed25519".into(),
                        fingerprint: "SHA256:test".into(),
                    }]);
                    cx.notify();
                })
                .expect("workspace update");
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("trust-once-7").visible());
            assert!(window.find("trust-save-7").visible());
            assert!(window.find("trust-cancel-7").visible());
            window.click("trust-once-7", cx);
        })
        .expect("workspace window");
        cx.update(|app| {
            handle
                .update(app, |view, _, _| {
                    assert_eq!(
                        view.command_error.as_deref(),
                        Some("Tunnel manager has stopped")
                    );
                })
                .expect("host key action");
        });
    }

    #[gpui_kit::test]
    fn blocked_reconnecting_and_sidebar_states_are_visible(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut config = ConfigDocument::default()
            .into_domain()
            .expect("default config");
        let id = TunnelId("demo".into());
        config.tunnels.push(TunnelConfig {
            id: id.clone(),
            name: "Demo".into(),
            group_id: None,
            mode: TunnelMode::Dynamic,
            jump_chain: Vec::new(),
            local: LocalEndpoint {
                host: "127.0.0.1".into(),
                port: 1080,
            },
            remote: None,
            auto_start: false,
            reconnect: RetryPolicy::default(),
            description: String::new(),
        });
        let handle = cx.add_window(move |window, cx| {
            Workspace::new(Ok((PathBuf::new(), config)), None, None, window, cx)
        });
        cx.update_window(handle.into(), |_, window, cx| {
            for page in ["Jumpers", "Groups", "Overview", "Tunnels"] {
                window.click(page, cx);
                assert!(window.find(page).visible());
            }
        })
        .expect("sidebar navigation");
        cx.update(|app| {
            handle
                .update(app, |view, _, _| assert_eq!(view.page, Page::Tunnels))
                .expect("selected page");
        });
        cx.update(|app| {
            handle
                .update(app, |view, _, cx| {
                    view.snapshot = Arc::new(ManagerSnapshot::from([(
                        id.clone(),
                        tunnel_core::TunnelView {
                            state: SupervisorState::Blocked {
                                reason: "host key rejected".into(),
                            },
                            local_addr: None,
                        },
                    )]));
                    cx.notify();
                })
                .expect("blocked snapshot");
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("tunnel-status-demo").label(),
                Some("Demo Blocked")
            );
            assert_eq!(
                window.find("tunnel-details-demo").label(),
                Some("host key rejected")
            );
            assert_eq!(window.find("demo-action").label(), Some("Retry"));
            assert_eq!(window.find("stop-blocked-demo").label(), Some("Stop"));
        })
        .expect("blocked UI");
        cx.update(|app| {
            handle
                .update(app, |view, _, cx| {
                    view.snapshot = Arc::new(ManagerSnapshot::from([(
                        id,
                        tunnel_core::TunnelView {
                            state: SupervisorState::Reconnecting {
                                attempt: 2,
                                delay: Duration::from_secs(3),
                                reason: "network unavailable".into(),
                            },
                            local_addr: Some(SocketAddr::from(([127, 0, 0, 1], 1080))),
                        },
                    )]));
                    cx.notify();
                })
                .expect("reconnecting snapshot");
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("tunnel-status-demo").label(),
                Some("Demo Reconnecting")
            );
            assert!(
                window
                    .find("tunnel-details-demo")
                    .label()
                    .expect("details")
                    .contains("Local listener remains reserved")
            );
            assert_eq!(window.find("demo-action").label(), Some("Stop"));
        })
        .expect("reconnecting UI");
    }

    #[gpui_kit::test]
    fn logs_filter_and_search_change_visible_rows(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let config = ConfigDocument::default()
            .into_domain()
            .expect("default config");
        let handle = cx.add_window(move |window, cx| {
            Workspace::new(Ok((PathBuf::new(), config)), None, None, window, cx)
        });
        cx.update(|app| {
            handle
                .update(app, |view, _, cx| {
                    view.logs = Arc::new(VecDeque::from([
                        Arc::new(LogEvent {
                            timestamp_unix: 0,
                            level: LogLevel::Info,
                            source: LogSource::Tunnel,
                            subject: "alpha".into(),
                            stage: "Lifecycle",
                            message: "Connected",
                        }),
                        Arc::new(LogEvent {
                            timestamp_unix: 1,
                            level: LogLevel::Error,
                            source: LogSource::Host,
                            subject: "beta".into(),
                            stage: "Host key",
                            message: "Confirmation required",
                        }),
                    ]));
                    cx.notify();
                })
                .expect("log snapshot");
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.click("Logs", cx);
            assert!(window.find("log-row-1").visible());
            window.click("log-filter-Error", cx);
            assert!(
                window
                    .find("log-row-0")
                    .label()
                    .expect("row label")
                    .contains("beta")
            );
            assert!(window.try_find("log-row-1").is_none());
            window.click("log-filter-Tunnel", cx);
            assert!(
                window
                    .find("log-row-0")
                    .label()
                    .expect("row label")
                    .contains("alpha")
            );
            window.click("log-search", cx);
            window.input("missing", cx);
            assert_eq!(window.find("log-search").focused(), Some(true));
            assert!(window.try_find("log-row-0").is_none());
        })
        .expect("logs UI");
        cx.update(|app| {
            handle
                .update(app, |view, _, _| {
                    assert_eq!(view.page, Page::Logs);
                    assert_eq!(view.log_filter, LogFilter::Tunnel);
                })
                .expect("log filter state");
        });
    }
}
