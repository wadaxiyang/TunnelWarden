use crate::{GroupId, HostId};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TunnelId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelMode {
    Local,
    Remote,
    Dynamic,
}

/// Local listener for Local/Dynamic; local target for Remote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalEndpoint {
    pub host: String,
    pub port: u16,
}

/// Target for Local; remote listener for Remote. Dynamic does not use it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteEndpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub reset_after_healthy: Duration,
    pub jitter_percent: u8,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            reset_after_healthy: Duration::from_secs(60),
            jitter_percent: 20,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelConfig {
    pub id: TunnelId,
    pub name: String,
    pub group_id: Option<GroupId>,
    pub mode: TunnelMode,
    /// Ordered hops, including the final target host.
    pub jump_chain: Vec<HostId>,
    pub local: LocalEndpoint,
    pub remote: Option<RemoteEndpoint>,
    pub auto_start: bool,
    pub reconnect: RetryPolicy,
    pub description: String,
}
