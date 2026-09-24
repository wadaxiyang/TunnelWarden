//! Pure configuration and state types shared by the runtime and user interface.

mod error;
mod group;
mod host;
mod state;
mod tunnel;

pub use error::{FailureStage, Retryability, TunnelError, TunnelErrorKind};
pub use group::{GroupId, TunnelGroup};
pub use host::{AuthConfig, HostId, HostKeyPolicy, SecretRef, SshHost};
pub use state::{
    DesiredState, HealthIssue, HealthState, ListenerState, RuntimeState, StartPhase, TunnelSnapshot,
};
pub use tunnel::{LocalEndpoint, RemoteEndpoint, RetryPolicy, TunnelConfig, TunnelId, TunnelMode};
