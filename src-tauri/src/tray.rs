//! Platform controls only; all start/stop operations reuse AppService.
use std::sync::Arc;
use tauri::{
    Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tunnelwarden_core::service::AppService;

pub fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
pub fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "Open TunnelWarden", true, None::<&str>)?;
    let start = MenuItem::with_id(app, "start", "Start all tunnels", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stop all tunnels", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit and stop tunnels", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &start, &stop, &separator, &quit])?;
    let mut builder = TrayIconBuilder::with_id("tunnelwarden")
        .tooltip("TunnelWarden — SSH tunnels")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show(app),
            "quit" => app.exit(0),
            id @ ("start" | "stop") => {
                let start = id == "start";
                let service = app.state::<Arc<AppService>>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    let result = if start {
                        service.start_all().await
                    } else {
                        service.stop_all().await
                    };
                    if let Err(e) = result {
                        service.logs.push("error", None, e);
                    }
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}
