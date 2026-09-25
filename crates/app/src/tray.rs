use config_store::DomainConfig;
use tokio::sync::mpsc;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
use tunnel_core::{AvailableActions, ManagerSnapshot, SupervisorState, TunnelAction, actions_for};
use tunnel_domain::TunnelId;

pub enum TrayAction {
    Open,
    StartAll,
    StopAll,
    Tunnel(TunnelId, TunnelAction),
    Quit,
}

pub struct TrayController {
    icon: TrayIcon,
    presentation: Option<TrayPresentation>,
}

#[derive(PartialEq, Eq)]
struct TrayPresentation {
    tunnels: Vec<(TunnelId, String, AvailableActions)>,
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
                let actions = snapshot
                    .get(&tunnel.id)
                    .map(|view| actions_for(&view.state))
                    .unwrap_or_else(|| actions_for(&SupervisorState::Stopped));
                (tunnel.id.clone(), tunnel.name.clone(), actions)
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
            let Some(action) = menu_action(event.id().0.as_str()) else {
                return;
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
        for (id, name, actions) in &presentation.tunnels {
            let (label, action_id) = match actions.primary {
                TunnelAction::Start => ("Start", "start"),
                TunnelAction::Stop => ("Stop", "stop"),
                TunnelAction::Retry => ("Retry", "retry"),
            };
            menu.append(&MenuItem::with_id(
                format!("tunnel:{action_id}:{}", id.0),
                format!("{name}   {label}"),
                true,
                None,
            ))
            .map_err(|error| error.to_string())?;
            if actions.primary == TunnelAction::Retry && actions.can_stop {
                menu.append(&MenuItem::with_id(
                    format!("tunnel:stop:{}", id.0),
                    format!("{name}   Stop"),
                    true,
                    None,
                ))
                .map_err(|error| error.to_string())?;
            }
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

fn menu_action(id: &str) -> Option<TrayAction> {
    match id {
        "open" => Some(TrayAction::Open),
        "start-all" => Some(TrayAction::StartAll),
        "stop-all" => Some(TrayAction::StopAll),
        "quit" => Some(TrayAction::Quit),
        _ => {
            for (prefix, action) in [
                ("tunnel:start:", TunnelAction::Start),
                ("tunnel:stop:", TunnelAction::Stop),
                ("tunnel:retry:", TunnelAction::Retry),
            ] {
                if let Some(tunnel_id) = id.strip_prefix(prefix)
                    && !tunnel_id.is_empty()
                {
                    return Some(TrayAction::Tunnel(TunnelId(tunnel_id.into()), action));
                }
            }
            None
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_menu_entries_dispatch_distinct_retry_and_stop_actions() {
        assert!(matches!(
            menu_action("tunnel:retry:demo"),
            Some(TrayAction::Tunnel(TunnelId(id), TunnelAction::Retry)) if id == "demo"
        ));
        assert!(matches!(
            menu_action("tunnel:stop:demo"),
            Some(TrayAction::Tunnel(TunnelId(id), TunnelAction::Stop)) if id == "demo"
        ));
    }
}
