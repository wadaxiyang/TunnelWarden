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
