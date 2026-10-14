//! Serialization-only shell. All behavior is in the core service.
use std::sync::Arc;
use tauri::State;
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tunnelwarden_core::{dto::*, service::AppService};
use uuid::Uuid;
type Service<'a> = State<'a, Arc<AppService>>;

#[tauri::command]
pub fn get_system_language() -> tunnelwarden_core::i18n::Language {
    tunnelwarden_core::i18n::detect_os_language()
}

#[tauri::command]
pub fn list_hosts(service: Service<'_>) -> Result<Vec<HostDto>, String> {
    service.list_hosts()
}
#[tauri::command]
pub fn list_tunnels(service: Service<'_>) -> Vec<TunnelDto> {
    service.list_tunnels()
}
#[tauri::command]
pub fn get_runtime_snapshot(service: Service<'_>) -> RuntimeSnapshot {
    service.get_runtime_snapshot()
}
#[tauri::command]
pub async fn get_tunnel_traffic(
    service: Service<'_>,
    tunnel_id: Uuid,
) -> Result<Option<TrafficSample>, String> {
    service.get_tunnel_traffic(tunnel_id).await
}
#[tauri::command]
pub fn get_logs(service: Service<'_>, filter: Option<LogFilter>) -> Vec<LogEntry> {
    service.get_logs(filter)
}
#[tauri::command]
pub fn get_settings(app: tauri::AppHandle, service: Service<'_>) -> Result<Settings, String> {
    let mut settings = service.get_settings();
    settings.launch_at_login = app.autolaunch().is_enabled().map_err(|e| e.to_string())?;
    Ok(settings)
}
#[tauri::command]
pub async fn save_host(service: Service<'_>, host: SaveHost) -> Result<HostDto, String> {
    service.save_host(host).await
}
#[tauri::command]
pub async fn delete_host(service: Service<'_>, host_id: Uuid) -> Result<(), String> {
    service.delete_host(host_id).await
}
#[tauri::command]
pub async fn save_tunnel(
    app: tauri::AppHandle,
    service: Service<'_>,
    tunnel: TunnelDto,
) -> Result<TunnelDto, String> {
    let result = service.save_tunnel(tunnel).await;
    crate::tray::refresh(&app);
    result
}
#[tauri::command]
pub async fn delete_tunnel(
    app: tauri::AppHandle,
    service: Service<'_>,
    tunnel_id: Uuid,
) -> Result<(), String> {
    let result = service.delete_tunnel(tunnel_id).await;
    crate::tray::refresh(&app);
    result
}
#[tauri::command]
pub async fn start_tunnel(service: Service<'_>, tunnel_id: Uuid) -> Result<(), String> {
    service.start_tunnel(tunnel_id).await
}
#[tauri::command]
pub async fn stop_tunnel(service: Service<'_>, tunnel_id: Uuid) -> Result<(), String> {
    service.stop_tunnel(tunnel_id).await
}
#[tauri::command]
pub async fn restart_tunnel(service: Service<'_>, tunnel_id: Uuid) -> Result<(), String> {
    service.restart_tunnel(tunnel_id).await
}
#[tauri::command]
pub async fn set_settings(
    app: tauri::AppHandle,
    service: Service<'_>,
    settings: Settings,
) -> Result<(), String> {
    let launch = app.autolaunch();
    let previous = launch.is_enabled().map_err(|e| e.to_string())?;
    if settings.launch_at_login {
        launch.enable()
    } else {
        launch.disable()
    }
    .map_err(|e| e.to_string())?;
    if let Err(e) = service.set_settings(settings).await {
        let rollback = if previous {
            launch.enable()
        } else {
            launch.disable()
        };
        rollback.map_err(|r| format!("{e}; restoring launch-at-login failed: {r}"))?;
        return Err(e);
    }
    crate::tray::refresh(&app);
    Ok(())
}
#[tauri::command]
pub fn get_host_key_prompts(service: Service<'_>) -> Vec<HostKeyPrompt> {
    service.prompts.snapshot()
}
#[tauri::command]
pub fn respond_host_key(service: Service<'_>, prompt_id: Uuid, trust: bool) -> Result<(), String> {
    service.prompts.respond(prompt_id, trust)
}

#[tauri::command]
pub fn open_source(app: tauri::AppHandle) -> Result<(), String> {
    app.opener()
        .open_url("https://github.com/wadaxiyang/TunnelWarden", None::<&str>)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn ui_ready(app: tauri::AppHandle) {
    use tauri::Manager;
    app.state::<crate::ui::UiState>()
        .ready
        .store(true, std::sync::atomic::Ordering::SeqCst);
}
#[tauri::command]
pub async fn close_ui(app: tauri::AppHandle) -> Result<(), String> {
    crate::ui::close(&app).await
}

#[tauri::command]
pub async fn test_host(service: Service<'_>, host_id: Uuid) -> Result<HostTestResult, String> {
    service.test_host(host_id).await
}
#[tauri::command]
pub fn cancel_host_test(service: Service<'_>, host_id: Uuid) {
    service.cancel_host_test(host_id);
}
#[tauri::command]
pub fn get_key_passphrase_prompts(service: Service<'_>) -> Vec<KeyPassphrasePrompt> {
    service.prompts.key_snapshot()
}
#[tauri::command]
pub fn respond_key_passphrase(
    service: Service<'_>,
    prompt_id: Uuid,
    passphrase: Option<String>,
) -> Result<(), String> {
    service.prompts.respond_key(prompt_id, passphrase)
}
#[tauri::command]
pub async fn export_error_logs(
    app: tauri::AppHandle,
    service: Service<'_>,
) -> Result<Option<String>, String> {
    let report = service.diagnostic_report()?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(
            tunnelwarden_core::i18n::text(service.get_settings().language, "Diagnostic report"),
            &["json"],
        )
        .set_file_name("TunnelWarden-diagnostics.json")
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        std::fs::write(&path, report).map_err(|e| format!("Cannot export report: {e}"))?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}
