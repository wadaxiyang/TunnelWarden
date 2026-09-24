use std::path::{Path, PathBuf};

use config_store::{ConfigStoreError, DomainConfig};
use gpui_kit::base::{Disableable, Selectable, StyledExt};
use gpui_kit::component::{
    ActiveTheme,
    button::{Button, ButtonVariants},
    scroll::ScrollableElement,
};
use gpui_kit::{Context, IntoElement, ParentElement as _, Render, Styled as _, Window, div, rems};
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
}

impl Workspace {
    pub fn new(
        startup: Result<(PathBuf, DomainConfig), ConfigStoreError>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            page: Page::Overview,
            page_index: 0,
            startup: startup.map_err(|error| error.to_string()),
        }
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
        content.child(div().flex_1().min_h_0().overflow_y_scrollbar().child(body))
    }

    fn render_page(
        &self,
        directory: &Path,
        config: &DomainConfig,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        match self.page {
            Page::Overview => div()
                .p_6()
                .gap_4()
                .flex()
                .flex_col()
                .child(format!(
                    "{} tunnels · {} jumpers",
                    config.tunnels.len(),
                    config.hosts.len()
                ))
                .child("Open Tunnels to inspect your saved connections.")
                .into_any_element(),
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
            rows = rows.child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(tunnel.name.clone())
                    .child(format!(
                        "{mode} · {}:{} · Stopped",
                        tunnel.local.host, tunnel.local.port
                    )),
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
