//! Serialization-only shell. All behavior is in the core service.
use std::sync::Arc;
use tauri::State;
use tunnelwarden_core::{dto::*, service::AppService};
use uuid::Uuid;
type Service<'a> = State<'a, Arc<AppService>>;

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
pub fn get_logs(service: Service<'_>, filter: Option<LogFilter>) -> Vec<LogEntry> {
    service.get_logs(filter)
}
#[tauri::command]
pub fn get_settings(service: Service<'_>) -> Settings {
    service.get_settings()
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
pub async fn save_tunnel(service: Service<'_>, tunnel: TunnelDto) -> Result<TunnelDto, String> {
    service.save_tunnel(tunnel).await
}
#[tauri::command]
pub async fn delete_tunnel(service: Service<'_>, tunnel_id: Uuid) -> Result<(), String> {
    service.delete_tunnel(tunnel_id).await
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
pub async fn set_settings(service: Service<'_>, settings: Settings) -> Result<(), String> {
    service.set_settings(settings).await
}
#[tauri::command]
pub fn get_host_key_prompts(service: Service<'_>) -> Vec<HostKeyPrompt> {
    service.prompts.snapshot()
}
#[tauri::command]
pub fn respond_host_key(service: Service<'_>, prompt_id: Uuid, trust: bool) -> Result<(), String> {
    service.prompts.respond(prompt_id, trust)
}
