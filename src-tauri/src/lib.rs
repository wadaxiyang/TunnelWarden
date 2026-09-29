//! Thin Tauri shell.
//!
//! Do not implement SSH, forwarding, reconnect, or channel lifecycle here.
//! Those belong to `tunnelwarden-core` and ultimately the pinned Oryxis engine.

mod commands;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tunnelwarden_core::{credentials::PlatformSecrets, service::AppService, storage::ConfigStore};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
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
                    let _ = handle.emit(
                        "tunnelwarden://host-key",
                        prompts.borrow_and_update().clone(),
                    );
                }
            });
            let mut logs = service.logs.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match logs.recv().await {
                        Ok(entry) => {
                            let _ = handle.emit("tunnelwarden://log", entry);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });
            app.manage(service);
            Ok(())
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
            commands::respond_host_key
        ])
        .run(tauri::generate_context!())
        .expect("error while running TunnelWarden");
}
