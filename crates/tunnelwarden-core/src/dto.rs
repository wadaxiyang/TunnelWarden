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
