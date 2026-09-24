use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TunnelErrorKind {
    Network,
    Dns,
    Timeout,
    Authentication,
    HostKeyUnknown,
    HostKeyMismatch,
    PortConflict,
    PermissionDenied,
    RemoteForwardRejected,
    InvalidConfig,
    Cancelled,
    Internal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureStage {
    ConfigValidation,
    BindLocalPort,
    ResolveHost,
    ConnectTcp,
    SshHandshake,
    HostKeyVerification,
    Authentication,
    BuildJumpChain,
    RegisterForward,
    Relay,
    HealthCheck,
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Retryability {
    Transient,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelError {
    pub kind: TunnelErrorKind,
    pub stage: FailureStage,
    pub message: String,
    pub retryability: Retryability,
    /// Keep only safe, bounded diagnostic summaries here; never secret values.
    pub source_chain: Vec<String>,
}
