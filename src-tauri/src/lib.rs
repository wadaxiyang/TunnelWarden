//! Thin Tauri shell.
//!
//! Do not implement SSH, forwarding, reconnect, or channel lifecycle here.
//! Those belong to `tunnelwarden-core` and ultimately the pinned Oryxis engine.

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .run(tauri::generate_context!())
        .expect("error while running TunnelWarden");
}
