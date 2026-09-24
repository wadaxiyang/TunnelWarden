use crate::{TunnelError, TunnelId};
use std::{net::SocketAddr, time::Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DesiredState {
    #[default]
    Stopped,
    Running,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartPhase {
    Validating,
    AcquiringListener,
    BuildingJumpChain,
    RegisteringRemoteForward,
    StartingHealthMonitor,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HealthIssue {
    PingTimeout,
    SessionLost,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeState {
    Stopped,
    Starting {
        phase: StartPhase,
    },
    Connecting {
        hop: usize,
        total_hops: usize,
        attempt: u32,
    },
    Authenticating {
        hop: usize,
        total_hops: usize,
    },
    EstablishingForward,
    Healthy {
        since: Instant,
    },
    Degraded {
        since: Instant,
        reason: HealthIssue,
    },
    Reconnecting {
        attempt: u32,
        next_retry_at: Instant,
        reason: TunnelError,
    },
    Blocked {
        reason: TunnelError,
    },
    Stopping,
    Failed {
        reason: TunnelError,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListenerState {
    NotRequired,
    Unbound,
    Bound {
        address: SocketAddr,
    },
    Conflict {
        address: SocketAddr,
        owner_hint: Option<String>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HealthState {
    pub authenticated: bool,
    pub forward_ready: bool,
    pub last_rtt_ms: Option<u32>,
    pub consecutive_ping_failures: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelSnapshot {
    pub id: TunnelId,
    pub desired_state: DesiredState,
    pub runtime_state: RuntimeState,
    pub listener_state: ListenerState,
    pub health: HealthState,
    pub generation: u64,
    pub upload_bps: u64,
    pub download_bps: u64,
    pub upload_total: u64,
    pub download_total: u64,
    pub active_connections: u32,
    pub last_error: Option<TunnelError>,
}
