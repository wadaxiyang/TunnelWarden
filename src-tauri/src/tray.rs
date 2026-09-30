//! Platform controls only; all start/stop operations reuse AppService.
use crate::tray_view::{can_stop, menu_name, status_text};
use std::sync::{Arc, Mutex};
use tauri::{
    Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tunnelwarden_core::{dto::TunnelStatus, service::AppService};
use uuid::Uuid;

struct RuleMenu {
    id: Uuid,
    name: String,
    submenu: Submenu<tauri::Wry>,
    start: MenuItem<tauri::Wry>,
    stop: MenuItem<tauri::Wry>,
}
struct TrayMenus {
    tunnels: Submenu<tauri::Wry>,
    rules: Mutex<Vec<RuleMenu>>,
}

pub fn show(app: &tauri::AppHandle) {
    hide_panel(app);
    crate::ui::show(app, false);
}
fn hide_panel(app: &tauri::AppHandle) {
    #[cfg(windows)]
    if let Some(panel) = app.try_state::<crate::tray_panel::TrayPanel>() {
        panel.hide();
    }
}
pub fn shutdown(app: &tauri::AppHandle) {
    #[cfg(windows)]
    if let Some(panel) = app.try_state::<crate::tray_panel::TrayPanel>() {
        panel.shutdown();
    }
}
/// Run on the UI thread; retain native menu handles while their labels and
/// enabled states change. Structure changes only when saved profiles change.
pub fn refresh(app: &tauri::AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Err(e) = refresh_now(&handle) {
            log::error!("Refresh tray menu: {e}");
        }
    });
}
fn refresh_now(app: &tauri::AppHandle) -> tauri::Result<()> {
    let Some(menus) = app.try_state::<TrayMenus>() else {
        return Ok(());
    };
    let service = app.state::<Arc<AppService>>();
    let tunnels = service.list_tunnels();
    let runtime = service.get_runtime_snapshot();
    let mut rules = menus.rules.lock().unwrap_or_else(|p| p.into_inner());
    if rules.len() != tunnels.len()
        || !rules
            .iter()
            .zip(&tunnels)
            .all(|(m, t)| m.id == t.id && m.name == t.name)
    {
        for item in menus.tunnels.items()? {
            menus.tunnels.remove(&item)?;
        }
        rules.clear();
        if tunnels.is_empty() {
            menus.tunnels.append(&MenuItem::new(
                app,
                "No tunnels configured",
                false,
                None::<&str>,
            )?)?;
        }
        for t in &tunnels {
            let start = MenuItem::with_id(
                app,
                format!("tunnel-start:{}", t.id),
                "Start",
                true,
                None::<&str>,
            )?;
            let stop = MenuItem::with_id(
                app,
                format!("tunnel-stop:{}", t.id),
                "Stop",
                false,
                None::<&str>,
            )?;
            let submenu = Submenu::with_items(app, menu_name(&t.name), true, &[&start, &stop])?;
            menus.tunnels.append(&submenu)?;
            rules.push(RuleMenu {
                id: t.id,
                name: t.name.clone(),
                submenu,
                start,
                stop,
            });
        }
    }
    for item in rules.iter() {
        let status = runtime
            .tunnels
            .iter()
            .find(|s| s.id == item.id)
            .map(|s| s.status)
            .unwrap_or(TunnelStatus::Stopped);
        item.submenu.set_text(format!(
            "{} — {}",
            menu_name(&item.name),
            status_text(status)
        ))?;
        item.start.set_enabled(!can_stop(status))?;
        item.stop.set_enabled(can_stop(status))?;
    }
    Ok(())
}
fn handle_menu(app: &tauri::AppHandle, id: &str) {
    hide_panel(app);
    match id {
        "show" => show(app),
        "quit" => app.exit(0),
        _ => {
            let rule = id.split_once(':').and_then(|(action, id)| {
                let start = match action {
                    "tunnel-start" => true,
                    "tunnel-stop" => false,
                    _ => return None,
                };
                Some((start, Uuid::parse_str(id).ok()?))
            });
            if rule.is_none() && id != "start" && id != "stop" {
                return;
            }
            let start_all = id == "start";
            let service = app.state::<Arc<AppService>>().inner().clone();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let result = match rule {
                    Some((true, id)) => service.start_tunnel(id).await,
                    Some((false, id)) => service.stop_tunnel(id).await,
                    None if start_all => service.start_all().await,
                    None => service.stop_all().await,
                };
                if let Err(e) = result {
                    service.logs.push("error", rule.map(|(_, id)| id), e);
                }
                refresh(&app);
            });
        }
    }
}
pub fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "Open TunnelWarden", true, None::<&str>)?;
    let tunnels = Submenu::new(app, "Tunnels", true)?;
    // A placeholder is needed even for an initially empty configuration.
    tunnels.append(&MenuItem::new(
        app,
        "No tunnels configured",
        false,
        None::<&str>,
    )?)?;
    let start = MenuItem::with_id(app, "start", "Start all tunnels", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stop all tunnels", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit and stop tunnels", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&show_item, &tunnels, &start, &stop, &separator, &quit],
    )?;
    app.manage(TrayMenus {
        tunnels,
        rules: Mutex::new(Vec::new()),
    });
    #[cfg(windows)]
    app.manage(crate::tray_panel::TrayPanel::new(
        app.state::<Arc<AppService>>().inner().clone(),
    )?);
    let mut builder = TrayIconBuilder::with_id("tunnelwarden")
        .tooltip("TunnelWarden — SSH tunnels")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu(app, event.id.as_ref()))
        .on_tray_icon_event(|tray, event| {
            let app = tray.app_handle();
            match event {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => show(app),
                TrayIconEvent::Click { .. } | TrayIconEvent::DoubleClick { .. } => hide_panel(app),
                #[cfg(windows)]
                TrayIconEvent::Enter { rect, .. } => {
                    // Preserve a name at rest; avoid a competing Shell tooltip.
                    let _ = tray.set_tooltip(None::<&str>);
                    app.state::<crate::tray_panel::TrayPanel>().enter(rect)
                }
                #[cfg(windows)]
                TrayIconEvent::Leave { .. } => {
                    let _ = tray.set_tooltip(Some("TunnelWarden — SSH tunnels"));
                    app.state::<crate::tray_panel::TrayPanel>().leave();
                }
                _ => {}
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    refresh_now(app)?;
    Ok(())
}
