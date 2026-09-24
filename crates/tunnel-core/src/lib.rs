//! Single-writer tunnel lifecycle model. Network resources will be owned by a
//! separate runtime, which must execute the effects returned by this model.

mod local_forward;
mod retry;
mod runtime;
mod state_machine;

pub use local_forward::{LocalForwardError, LocalForwardWorker};
pub use retry::RetrySchedule;
pub use runtime::{ListenerRuntime, ListenerRuntimeError};
pub use state_machine::{
    Event, EventOutcome, Lifecycle, MachineError, RuntimePhase, StateMachine, StateTransition,
    TransitionEffect,
};
