use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use config_store::{ConfigDocument, DomainConfig, SshImportPreview};
use gpui_kit::base::{Disableable, Selectable, StyledExt};
use gpui_kit::component::{
    ActiveTheme,
    button::{Button, ButtonVariants},
    input::{Input, InputContentType},
    scroll::ScrollableElement,
};
use gpui_kit::{
    AsyncApp, Context, IntoElement, ParentElement as _, PathPromptOptions, Render, Styled as _,
    Task, Window, div, rems,
};
use tokio::sync::oneshot;
use tunnel_core::{
    CoreCommand, HostKeyDecision, HostKeyPromptView, ManagerHandle, ManagerSnapshot, SecretUpdate,
    SupervisorState,
};
use tunnel_domain::TunnelMode;
use tunnel_domain::{AuthConfig, HostId, HostKeyPolicy, SecretRef, TunnelId};

use crate::editor::{Editor, HostEditor, TunnelEditor};

const PAGE_SIZE: usize = 25;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Jumpers,
    Tunnels,
    Logs,
    Settings,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Jumpers => "Jumpers",
            Self::Tunnels => "Tunnels",
            Self::Logs => "Logs",
            Self::Settings => "Settings",
        }
    }
}

pub struct Workspace {
    page: Page,
    page_index: usize,
    startup: Result<(PathBuf, DomainConfig), String>,
    manager: Option<ManagerHandle>,
    snapshot: Arc<ManagerSnapshot>,
    command_error: Option<String>,
    runtime_error: Option<String>,
    _updates: Option<Task<()>>,
    _host_key_updates: Option<Task<()>>,
    host_key_prompts: Arc<Vec<HostKeyPromptView>>,
    editor: Option<Editor>,
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
    pub fn new(
        startup: Result<(PathBuf, DomainConfig), String>,
        manager: Option<ManagerHandle>,
        runtime_error: Option<String>,
        _window: &mut Window,
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
        Self {
            page: Page::Overview,
            page_index: 0,
            startup,
            manager,
            snapshot,
            command_error: None,
            runtime_error,
            _updates: updates,
            _host_key_updates: host_key_updates,
            host_key_prompts,
            editor: None,
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
        self.editor = Some(Editor::Host(HostEditor::new(id, None, window, cx)));
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
        self.editor = Some(Editor::Host(HostEditor::new(
            id.clone(),
            Some(host),
            window,
            cx,
        )));
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
        self.editor = Some(Editor::Tunnel(TunnelEditor::new(id, None, window, cx)));
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
        self.editor = Some(Editor::Tunnel(TunnelEditor::new(
            id.clone(),
            Some(tunnel),
            window,
            cx,
        )));
        cx.notify();
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
            config: config.clone(),
            secret,
            reply,
        }) {
            self.command_error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.saving = true;
        self.command_error = None;
        self._save_task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = answer
                .await
                .unwrap_or_else(|_| Err("Configuration manager stopped".into()));
            let _ = this.update(cx, |view, cx| {
                view.saving = false;
                match result {
                    Ok(()) => {
                        view.startup = Ok((directory, config));
                        view.editor = None;
                        view.import_preview = None;
                        view.ssh_preview = None;
                        view.command_error = None;
                    }
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
            if entry.host.username.is_empty()
                || config
                    .hosts
                    .iter()
                    .any(|host| host.id == entry.host.id || host.name == entry.host.name)
            {
                self.command_error = Some(format!(
                    "Resolve conflict or missing user for {}",
                    entry.alias
                ));
                cx.notify();
                return;
            }
            config.hosts.push(entry.host.clone());
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
        self.page = page;
        self.page_index = 0;
        cx.notify();
    }

    fn page_count(&self) -> usize {
        let Ok((_, config)) = &self.startup else {
            return 0;
        };
        match self.page {
            Page::Jumpers => config.hosts.len(),
            Page::Tunnels => config.tunnels.len(),
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
        for page in [Page::Overview, Page::Jumpers, Page::Tunnels, Page::Logs] {
            sidebar = sidebar.child(
                Button::new(page.title())
                    .ghost()
                    .label(page.title())
                    .selected(self.page == page)
                    .on_click(cx.listener(move |this, _, _, cx| this.select_page(page, cx))),
            );
        }
        sidebar.child(div().flex_1()).child(
            Button::new("Settings")
                .ghost()
                .label("Settings")
                .selected(self.page == Page::Settings)
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
        let body = match &self.startup {
            Ok((_, config)) if self.editor.is_some() => self.render_editor(config, cx),
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
            Page::Logs => div()
                .p_6()
                .child("No runtime events yet")
                .into_any_element(),
            Page::Settings => self.render_settings(directory, config, cx),
        }
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
                    "Duplicate"
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
                                .label(if selected { "✓ Import" } else { "Skip" })
                                .selected(selected)
                                .disabled(conflict || entry.host.username.is_empty() || self.saving)
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
                    .child(div().font_semibold().child("Mode"));
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
            let (button_label, action_kind) = match state {
                SupervisorState::Stopped => ("Start", 0),
                SupervisorState::Blocked { .. } => ("Retry", 1),
                _ => ("Stop", 2),
            };
            let action = Button::new(format!("{}-action", id.0))
                .label(button_label)
                .disabled(self.manager.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let command = match action_kind {
                        0 => CoreCommand::StartTunnel(id.clone()),
                        1 => CoreCommand::RetryTunnel(id.clone()),
                        _ => CoreCommand::StopTunnel(id.clone()),
                    };
                    this.request(command, cx);
                }));
            let mut info = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(tunnel.name.clone())
                .child(format!(
                    "{mode} · {}:{} · {label}",
                    tunnel.local.host, tunnel.local.port
                ));
            if let Some(details) = details {
                info = info.child(div().text_color(cx.theme().muted_foreground).child(details));
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
                    .child(
                        div()
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
                            .child(action),
                    ),
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
        .child(Input::new(input))
}

fn password_row(
    label: &'static str,
    input: &gpui_kit::Entity<gpui_kit::component::input::InputState>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(label)
        .child(Input::new(input).content_type(InputContentType::Password))
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
