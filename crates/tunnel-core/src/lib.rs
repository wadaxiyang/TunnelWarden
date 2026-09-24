//! Single-writer tunnel lifecycle model. Network resources will be owned by a
//! separate runtime, which must execute the effects returned by this model.

mod health;
mod local_forward;
mod manager;
mod remote_forward;
mod remote_supervisor;
mod retry;
mod runtime;
mod state_machine;
mod supervisor;

pub use health::WorkerHealth;
pub use local_forward::{LocalForwardError, LocalForwardWorker};
pub use manager::{
    CoreCommand, CredentialSource, HostKeyPromptView, LogEvent, LogLevel, LogSnapshot, LogSource,
    ManagerError, ManagerHandle, ManagerSnapshot, SecretUpdate, TunnelManager, TunnelView,
};
pub use remote_forward::{RemoteForwardError, RemoteForwardWorker};
pub use remote_supervisor::RemoteForwardSupervisor;
pub use retry::RetrySchedule;
pub use runtime::{ListenerRuntime, ListenerRuntimeError};
pub use ssh_engine::HostKeyDecision;
pub use state_machine::{
    Event, EventOutcome, Lifecycle, MachineError, RuntimePhase, StateMachine, StateTransition,
    TransitionEffect,
};
pub use supervisor::{LocalForwardSupervisor, SupervisorError, SupervisorMode, SupervisorState};
