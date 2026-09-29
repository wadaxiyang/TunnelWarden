//! Serializable application views; network state comes only from the runtime.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelStatus {
    Stopped,
    Connecting,
    Running,
    Retrying,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTunnel {
    pub id: Uuid,
    pub status: TunnelStatus,
    pub last_error: Option<String>,
    /// Unix milliseconds derived from the monotonic retry deadline.
    pub next_retry_at: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSnapshot {
    pub tunnels: Vec<RuntimeTunnel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub auto_reconnect: bool,
    pub launch_at_login: bool,
    pub close_to_tray: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_reconnect: true,
            launch_at_login: false,
            close_to_tray: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostKeyPrompt {
    pub id: Uuid,
    pub host_id: Uuid,
    pub hostname: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    pub old_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostDto {
    pub id: Uuid,
    pub name: String,
    pub hostname: String,
    pub ssh_port: u16,
    pub username: String,
    pub auth_method: oryxis_core::models::connection::AuthMethod,
    pub identity_file: String,
    pub jump_chain: Vec<Uuid>,
    pub keepalive_seconds: u32,
    #[serde(default)]
    pub has_password: bool,
}
// Deliberately no Debug: a mutation may contain a credential.
#[derive(Deserialize)]
pub struct SaveHost {
    pub profile: HostDto,
    pub password: Option<String>,
    #[serde(default)]
    pub clear_password: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TunnelDto {
    pub id: Uuid,
    pub name: String,
    pub kind: oryxis_core::models::port_forward_rule::ForwardKind,
    pub host_id: Uuid,
    pub listen_address: String,
    pub listen_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub auto_start: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: Uuid,
    pub timestamp: String,
    pub level: String,
    pub tunnel_id: Option<Uuid>,
    pub message: String,
}
#[derive(Clone, Debug, Default, Deserialize)]
pub struct LogFilter {
    pub tunnel_id: Option<Uuid>,
    pub level: Option<String>,
}
