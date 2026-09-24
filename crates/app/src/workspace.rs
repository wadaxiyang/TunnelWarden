use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use config_store::{ConfigStoreError, DomainConfig};
use gpui_kit::base::{Disableable, Selectable, StyledExt};
use gpui_kit::component::{
    ActiveTheme,
    button::{Button, ButtonVariants},
    scroll::ScrollableElement,
};
use gpui_kit::{
    AsyncApp, Context, IntoElement, ParentElement as _, Render, Styled as _, Task, Window, div,
    rems,
};
use tunnel_core::{CoreCommand, ManagerHandle, ManagerSnapshot, SupervisorState};
use tunnel_domain::TunnelMode;

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
}

impl Workspace {
    pub fn new(
        startup: Result<(PathBuf, DomainConfig), ConfigStoreError>,
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
        Self {
            page: Page::Overview,
            page_index: 0,
            startup: startup.map_err(|error| error.to_string()),
            manager,
            snapshot,
            command_error: None,
            runtime_error,
            _updates: updates,
        }
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
        let body = match &self.startup {
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
                        "{healthy} Healthy  ·  {reconnecting} Reconnecting  ·  {blocked} Blocked"
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
            Page::Settings => div()
                .p_6()
                .flex()
                .flex_col()
                .gap_3()
                .child("Configuration folder")
                .child(directory.display().to_string())
                .into_any_element(),
        }
    }

    fn render_hosts(&self, config: &DomainConfig, cx: &mut Context<Self>) -> impl IntoElement {
        let start = self.page_index.saturating_mul(PAGE_SIZE);
        let mut rows = div().flex().flex_col().gap_1();
        for host in config.hosts.iter().skip(start).take(PAGE_SIZE) {
            rows = rows.child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(host.name.clone())
                    .child(format!("{}@{}:{}", host.username, host.hostname, host.port)),
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
            let state = self
                .snapshot
                .get(&tunnel.id)
                .map(|view| &view.state)
                .unwrap_or(&SupervisorState::Stopped);
            let label = state_label(state);
            let id = tunnel.id.clone();
            let (button_label, command) = match state {
                SupervisorState::Stopped => ("Start", CoreCommand::StartTunnel(id.clone())),
                SupervisorState::Blocked { .. } => {
                    ("Retry", CoreCommand::RestartTunnel(id.clone()))
                }
                _ => ("Stop", CoreCommand::StopTunnel(id.clone())),
            };
            let action = Button::new(format!("{}-action", id.0))
                .label(button_label)
                .disabled(self.manager.is_none())
                .on_click(cx.listener(move |this, _, _, cx| this.request(command.clone(), cx)));
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
                            .gap_1()
                            .child(tunnel.name.clone())
                            .child(format!(
                                "{mode} · {}:{} · {label}",
                                tunnel.local.host, tunnel.local.port
                            )),
                    )
                    .child(action),
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

fn state_label(state: &SupervisorState) -> &'static str {
    match state {
        SupervisorState::Connecting { .. } => "Connecting",
        SupervisorState::Reconnecting { .. } => "Reconnecting",
        SupervisorState::Healthy { .. } => "Healthy",
        SupervisorState::Blocked { .. } => "Blocked",
        SupervisorState::Stopped => "Stopped",
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
