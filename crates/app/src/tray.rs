use config_store::DomainConfig;
use tokio::sync::mpsc;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
use tunnel_core::{ManagerSnapshot, SupervisorState};
use tunnel_domain::TunnelId;

pub enum TrayAction {
    Open,
    StartAll,
    StopAll,
    Toggle(TunnelId),
    Quit,
}

pub struct TrayController {
    icon: TrayIcon,
    presentation: Option<TrayPresentation>,
}

#[derive(PartialEq, Eq)]
struct TrayPresentation {
    tunnels: Vec<(TunnelId, String, bool)>,
    remaining: usize,
    healthy: usize,
    reconnecting: usize,
}

impl TrayPresentation {
    fn new(config: &DomainConfig, snapshot: &ManagerSnapshot) -> Self {
        let tunnels = config
            .tunnels
            .iter()
            .take(25)
            .map(|tunnel| {
                let active = snapshot.get(&tunnel.id).is_some_and(|view| {
                    !matches!(
                        view.state,
                        SupervisorState::Stopped | SupervisorState::Blocked { .. }
                    )
                });
                (tunnel.id.clone(), tunnel.name.clone(), active)
            })
            .collect();
        Self {
            tunnels,
            remaining: config.tunnels.len().saturating_sub(25),
            healthy: snapshot
                .values()
                .filter(|view| matches!(view.state, SupervisorState::Healthy { .. }))
                .count(),
            reconnecting: snapshot
                .values()
                .filter(|view| matches!(view.state, SupervisorState::Reconnecting { .. }))
                .count(),
        }
    }
}

impl Drop for TrayController {
    fn drop(&mut self) {
        MenuEvent::set_event_handler::<fn(MenuEvent)>(None);
        TrayIconEvent::set_event_handler::<fn(TrayIconEvent)>(None);
    }
}

impl TrayController {
    pub fn new(
        actions: mpsc::Sender<TrayAction>,
        config: &DomainConfig,
        snapshot: &ManagerSnapshot,
    ) -> Result<Self, String> {
        let menu_sender = actions.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id().0.as_str() {
                "open" => TrayAction::Open,
                "start-all" => TrayAction::StartAll,
                "stop-all" => TrayAction::StopAll,
                "quit" => TrayAction::Quit,
                id if id.starts_with("tunnel:") => TrayAction::Toggle(TunnelId(id[7..].to_owned())),
                _ => return,
            };
            let _ = menu_sender.try_send(action);
        }));
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                let _ = actions.try_send(TrayAction::Open);
            }
        }));
        let icon = TrayIconBuilder::new()
            .with_icon(make_icon()?)
            .with_tooltip("TunnelWarden")
            .with_menu_on_left_click(false)
            .build()
            .map_err(|error| error.to_string())?;
        let mut controller = Self {
            icon,
            presentation: None,
        };
        controller.update(config, snapshot)?;
        Ok(controller)
    }

    pub fn update(
        &mut self,
        config: &DomainConfig,
        snapshot: &ManagerSnapshot,
    ) -> Result<(), String> {
        let presentation = TrayPresentation::new(config, snapshot);
        if self.presentation.as_ref() == Some(&presentation) {
            return Ok(());
        }
        let menu = Menu::new();
        menu.append(&MenuItem::with_id("open", "Open TunnelWarden", true, None))
            .map_err(|error| error.to_string())?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|error| error.to_string())?;
        for (id, name, active) in &presentation.tunnels {
            let marker = if *active { "●" } else { "○" };
            menu.append(&MenuItem::with_id(
                format!("tunnel:{}", id.0),
                format!("{name}   {marker}"),
                true,
                None,
            ))
            .map_err(|error| error.to_string())?;
        }
        if presentation.remaining > 0 {
            menu.append(&MenuItem::new(
                format!("{} more tunnels in the app", presentation.remaining),
                false,
                None,
            ))
            .map_err(|error| error.to_string())?;
        }
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|error| error.to_string())?;
        menu.append(&MenuItem::with_id("start-all", "Start All", true, None))
            .map_err(|error| error.to_string())?;
        menu.append(&MenuItem::with_id("stop-all", "Stop All", true, None))
            .map_err(|error| error.to_string())?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|error| error.to_string())?;
        menu.append(&MenuItem::with_id("quit", "Quit", true, None))
            .map_err(|error| error.to_string())?;
        self.icon.set_menu(Some(Box::new(menu)));
        self.icon
            .set_tooltip(Some(format!(
                "TunnelWarden\n{} connected · {} reconnecting",
                presentation.healthy, presentation.reconnecting
            )))
            .map_err(|error| error.to_string())?;
        self.presentation = Some(presentation);
        Ok(())
    }
}

fn make_icon() -> Result<Icon, String> {
    let mut rgba = vec![0u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let center = (x as i32 - 16).abs();
            let inside = (3..=27).contains(&y) && center <= (13 - (y as i32 / 5)).max(5);
            if inside {
                let offset = (y * 32 + x) * 4;
                rgba[offset..offset + 4].copy_from_slice(&[35, 142, 190, 255]);
            }
        }
    }
    Icon::from_rgba(rgba, 32, 32).map_err(|error| error.to_string())
}
