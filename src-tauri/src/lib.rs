//! Thin Tauri shell.
//!
//! Do not implement SSH, forwarding, reconnect, or channel lifecycle here.
//! Those belong to `tunnelwarden-core` and ultimately the pinned Oryxis engine.

mod commands;
mod tray;
mod ui;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tauri::{Emitter, Manager};
use tunnelwarden_core::{credentials::PlatformSecrets, service::AppService, storage::ConfigStore};

#[derive(Default)]
struct ExitState {
    requested: AtomicBool,
    ready: AtomicBool,
}

pub fn run() {
    let context = tauri::generate_context!();
    let startup_name = context
        .config()
        .product_name
        .clone()
        .unwrap_or_else(|| "TunnelWarden".into());
    let app = tauri::Builder::default()
        .manage(ExitState::default())
        .manage(ui::UiState::default())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            tray::show(app)
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name(startup_name)
                .arg("--autostart")
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .max_file_size(1_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
                .build(),
        )
        .setup(|app| {
            let path = app.path().app_data_dir()?.join("config.json");
            let store = Arc::new(ConfigStore::open(path).map_err(std::io::Error::other)?);
            let service = tauri::async_runtime::block_on(async {
                AppService::new(store, Arc::new(PlatformSecrets))
            });
            let mut runtime = service.runtime.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while runtime.changed().await.is_ok() {
                    let _ = handle.emit(
                        "tunnelwarden://runtime",
                        runtime.borrow_and_update().clone(),
                    );
                }
            });
            let mut prompts = service.prompts.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while prompts.changed().await.is_ok() {
                    ui::show(&handle, true);
                    let _ = handle.emit(
                        "tunnelwarden://host-key",
                        prompts.borrow_and_update().clone(),
                    );
                }
            });
            let mut logs = service.logs.subscribe();
            let mut key_prompts = service.prompts.key_subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while key_prompts.changed().await.is_ok() {
                    ui::show(&handle, true);
                    let _ = handle.emit(
                        "tunnelwarden://key-passphrase",
                        key_prompts.borrow_and_update().clone(),
                    );
                }
            });
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match logs.recv().await {
                        Ok(entry) => {
                            if let Ok(line) = serde_json::to_string(&entry) {
                                log::info!(target: "tunnelwarden", "{line}");
                            }
                            let _ = handle.emit("tunnelwarden://log", entry);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });
            app.manage(service.clone());
            tray::install(app.handle())?;
            if !std::env::args().any(|arg| arg == "--autostart") {
                tray::show(app.handle());
            }
            log::info!("TunnelWarden 0.1.0 starting; Oryxis 16ad2af");
            tauri::async_runtime::spawn(async move {
                if let Err(e) = service.runtime.auto_start().await {
                    service.logs.push("error", None, e);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if window.state::<ui::UiState>().ready.load(Ordering::SeqCst) {
                    let _ = window.emit("tunnelwarden://close-requested", ());
                } else {
                    let app = window.app_handle().clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = ui::close(&app).await {
                            log::error!("Close interface: {e}");
                        }
                    });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_hosts,
            commands::list_tunnels,
            commands::get_runtime_snapshot,
            commands::get_logs,
            commands::get_settings,
            commands::save_host,
            commands::delete_host,
            commands::save_tunnel,
            commands::delete_tunnel,
            commands::start_tunnel,
            commands::stop_tunnel,
            commands::restart_tunnel,
            commands::set_settings,
            commands::get_host_key_prompts,
            commands::respond_host_key,
            commands::open_source,
            commands::quit_app,
            commands::test_host,
            commands::cancel_host_test,
            commands::get_key_passphrase_prompts,
            commands::respond_key_passphrase,
            commands::export_error_logs,
            commands::ui_ready,
            commands::close_ui
        ])
        .build(context)
        .expect("error while building TunnelWarden");
    app.run(|handle, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            // Destroying the last UI window is not an instruction to stop SSH.
            if code.is_none() {
                api.prevent_exit();
                return;
            }
            let exiting = handle.state::<ExitState>();
            if !exiting.ready.load(Ordering::SeqCst) {
                api.prevent_exit();
                if !exiting.requested.swap(true, Ordering::SeqCst) {
                    let service = handle.state::<Arc<AppService>>().inner().clone();
                    let handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        service.cancel_ui_questions();
                        if let Err(e) = service.runtime.shutdown().await {
                            log::error!("Shutdown: {e}");
                        }
                        log::info!("All tunnels stopped; exiting");
                        handle
                            .state::<ExitState>()
                            .ready
                            .store(true, Ordering::SeqCst);
                        handle.exit(0);
                    });
                }
            }
        }
    });
}
