//! Single-writer tunnel lifecycle model. Network resources will be owned by a
//! separate runtime, which must execute the effects returned by this model.

mod retry;
mod state_machine;

pub use retry::RetrySchedule;
pub use state_machine::{
    Event, EventOutcome, Lifecycle, MachineError, RuntimePhase, StateMachine, StateTransition,
    TransitionEffect,
};
