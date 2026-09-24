use std::{
    collections::VecDeque,
    net::SocketAddr,
    time::{Duration, Instant, SystemTime},
};
use thiserror::Error;
use tunnel_domain::{
    DesiredState, FailureStage, HealthIssue, HealthState, ListenerState, Retryability,
    RuntimeState, StartPhase, TunnelError, TunnelErrorKind,
};

const JOURNAL_LIMIT: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimePhase {
    Stopped,
    Starting,
    Connecting,
    Authenticating,
    EstablishingForward,
    Healthy,
    Degraded,
    Reconnecting,
    Blocked,
    Stopping,
    Failed,
}

impl From<&RuntimeState> for RuntimePhase {
    fn from(value: &RuntimeState) -> Self {
        match value {
            RuntimeState::Stopped => Self::Stopped,
            RuntimeState::Starting { .. } => Self::Starting,
            RuntimeState::Connecting { .. } => Self::Connecting,
            RuntimeState::Authenticating { .. } => Self::Authenticating,
            RuntimeState::EstablishingForward => Self::EstablishingForward,
            RuntimeState::Healthy { .. } => Self::Healthy,
            RuntimeState::Degraded { .. } => Self::Degraded,
            RuntimeState::Reconnecting { .. } => Self::Reconnecting,
            RuntimeState::Blocked { .. } => Self::Blocked,
            RuntimeState::Stopping => Self::Stopping,
            RuntimeState::Failed { .. } => Self::Failed,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Lifecycle {
    pub desired: DesiredState,
    pub runtime: RuntimeState,
    pub listener: ListenerState,
    pub health: HealthState,
    pub generation: u64,
}

#[derive(Clone, Debug)]
pub enum Event {
    Start,
    Stop,
    Restart,
    ConfigChanged,
    ShutdownComplete {
        generation: u64,
    },
    Phase {
        generation: u64,
        phase: StartPhase,
    },
    ListenerBound {
        generation: u64,
        address: SocketAddr,
    },
    ListenerConflict {
        generation: u64,
        address: SocketAddr,
    },
    Connecting {
        generation: u64,
        hop: usize,
        total_hops: usize,
        attempt: u32,
    },
    Authenticating {
        generation: u64,
        hop: usize,
        total_hops: usize,
    },
    Authenticated {
        generation: u64,
    },
    HopAuthenticated {
        generation: u64,
    },
    ForwardReady {
        generation: u64,
    },
    PingSucceeded {
        generation: u64,
        at: Instant,
        rtt: Duration,
    },
    PingFailed {
        generation: u64,
        at: Instant,
        attempt: u32,
        next_retry_at: Instant,
    },
    Disconnected {
        generation: u64,
        error: TunnelError,
        attempt: u32,
        next_retry_at: Instant,
    },
}

impl Event {
    fn generation(&self) -> Option<u64> {
        match self {
            Self::Start | Self::Stop | Self::Restart | Self::ConfigChanged => None,
            Self::ShutdownComplete { generation }
            | Self::Phase { generation, .. }
            | Self::ListenerBound { generation, .. }
            | Self::ListenerConflict { generation, .. }
            | Self::Connecting { generation, .. }
            | Self::Authenticating { generation, .. }
            | Self::Authenticated { generation }
            | Self::HopAuthenticated { generation }
            | Self::ForwardReady { generation }
            | Self::PingSucceeded { generation, .. }
            | Self::PingFailed { generation, .. }
            | Self::Disconnected { generation, .. } => Some(*generation),
        }
    }

    fn cause(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::ConfigChanged => "config_changed",
            Self::ShutdownComplete { .. } => "shutdown_complete",
            Self::Phase { .. } => "start_phase",
            Self::ListenerBound { .. } => "listener_bound",
            Self::ListenerConflict { .. } => "listener_conflict",
            Self::Connecting { .. } => "connecting",
            Self::Authenticating { .. } => "authenticating",
            Self::Authenticated { .. } => "authenticated",
            Self::HopAuthenticated { .. } => "hop_authenticated",
            Self::ForwardReady { .. } => "forward_ready",
            Self::PingSucceeded { .. } => "ping_succeeded",
            Self::PingFailed { .. } => "ping_failed",
            Self::Disconnected { .. } => "disconnected",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionEffect {
    /// The manager may begin work for this generation.
    BeginGeneration(u64),
    /// Cancel and join all work from the previous generation, then report ShutdownComplete.
    CancelAndJoin(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateTransition {
    pub from: RuntimePhase,
    pub to: RuntimePhase,
    pub at: SystemTime,
    pub cause: &'static str,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventOutcome {
    Applied(Option<TransitionEffect>),
    IgnoredStale,
    NoChange,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MachineError {
    #[error("event {event} is invalid while tunnel is {phase:?}")]
    InvalidTransition {
        event: &'static str,
        phase: RuntimePhase,
    },
    #[error("generation counter is exhausted")]
    GenerationExhausted,
    #[error("hop index must be within a nonempty chain")]
    InvalidHop,
}

/// All lifecycle mutation passes through `transition`. This type owns no sockets
/// and does not claim that a listener or session is healthy on its own.
pub struct StateMachine {
    state: Lifecycle,
    journal: VecDeque<StateTransition>,
    restart_after_shutdown: bool,
    listener_required: bool,
}

impl StateMachine {
    pub fn new(listener_required: bool) -> Self {
        Self {
            state: Lifecycle {
                desired: DesiredState::Stopped,
                runtime: RuntimeState::Stopped,
                listener: if listener_required {
                    ListenerState::Unbound
                } else {
                    ListenerState::NotRequired
                },
                health: HealthState::default(),
                generation: 0,
            },
            journal: VecDeque::with_capacity(JOURNAL_LIMIT),
            restart_after_shutdown: false,
            listener_required,
        }
    }

    pub fn state(&self) -> &Lifecycle {
        &self.state
    }

    pub fn journal(&self) -> &VecDeque<StateTransition> {
        &self.journal
    }

    pub fn transition(&mut self, event: Event) -> Result<EventOutcome, MachineError> {
        if let Some(generation) = event.generation()
            && generation != self.state.generation
        {
            return Ok(EventOutcome::IgnoredStale);
        }

        let old = RuntimePhase::from(&self.state.runtime);
        let cause = event.cause();
        let effect = match event {
            Event::Start => {
                if !matches!(self.state.runtime, RuntimeState::Stopped) {
                    return Err(Self::invalid(cause, old));
                }
                self.next_generation()?;
                self.begin();
                Some(TransitionEffect::BeginGeneration(self.state.generation))
            }
            Event::Stop => {
                if matches!(
                    self.state.runtime,
                    RuntimeState::Stopped | RuntimeState::Stopping
                ) {
                    return Ok(EventOutcome::NoChange);
                }
                let old_generation = self.state.generation;
                self.next_generation()?;
                self.state.desired = DesiredState::Stopped;
                self.state.runtime = RuntimeState::Stopping;
                self.state.health = HealthState::default();
                self.restart_after_shutdown = false;
                Some(TransitionEffect::CancelAndJoin(old_generation))
            }
            Event::Restart => {
                if matches!(self.state.runtime, RuntimeState::Stopping) {
                    // Cleanup is already owned by the in-flight Stop. Its completion
                    // event must retain the current generation.
                    return Ok(EventOutcome::NoChange);
                }
                let old_generation = self.state.generation;
                self.next_generation()?;
                if matches!(self.state.runtime, RuntimeState::Stopped) {
                    self.begin();
                    Some(TransitionEffect::BeginGeneration(self.state.generation))
                } else {
                    self.state.runtime = RuntimeState::Stopping;
                    self.state.health = HealthState::default();
                    self.restart_after_shutdown = true;
                    Some(TransitionEffect::CancelAndJoin(old_generation))
                }
            }
            Event::ConfigChanged => {
                if matches!(self.state.runtime, RuntimeState::Stopping) {
                    return Ok(EventOutcome::NoChange);
                }
                let old_generation = self.state.generation;
                self.next_generation()?;
                if matches!(self.state.runtime, RuntimeState::Stopped) {
                    None
                } else {
                    self.state.runtime = RuntimeState::Stopping;
                    self.state.health = HealthState::default();
                    self.restart_after_shutdown = self.state.desired == DesiredState::Running;
                    Some(TransitionEffect::CancelAndJoin(old_generation))
                }
            }
            Event::ShutdownComplete { .. } => {
                if !matches!(self.state.runtime, RuntimeState::Stopping) {
                    return Err(Self::invalid(cause, old));
                }
                self.state.listener = if self.listener_required {
                    ListenerState::Unbound
                } else {
                    ListenerState::NotRequired
                };
                self.state.health = HealthState::default();
                if self.restart_after_shutdown {
                    self.restart_after_shutdown = false;
                    self.begin();
                    Some(TransitionEffect::BeginGeneration(self.state.generation))
                } else {
                    self.state.runtime = RuntimeState::Stopped;
                    None
                }
            }
            Event::Phase { phase, .. } => {
                self.require_phase(cause, old, &[RuntimePhase::Starting])?;
                self.state.runtime = RuntimeState::Starting { phase };
                None
            }
            Event::ListenerBound { address, .. } => {
                self.require_start_phase(cause, StartPhase::AcquiringListener)?;
                if !self.listener_required || !matches!(self.state.listener, ListenerState::Unbound)
                {
                    return Err(Self::invalid(cause, old));
                }
                self.state.listener = ListenerState::Bound { address };
                None
            }
            Event::ListenerConflict { address, .. } => {
                self.require_start_phase(cause, StartPhase::AcquiringListener)?;
                if !self.listener_required || !matches!(self.state.listener, ListenerState::Unbound)
                {
                    return Err(Self::invalid(cause, old));
                }
                self.state.listener = ListenerState::Conflict {
                    address,
                    owner_hint: None,
                };
                self.state.runtime = RuntimeState::Blocked {
                    reason: TunnelError {
                        kind: TunnelErrorKind::PortConflict,
                        stage: FailureStage::BindLocalPort,
                        message: format!("Port {address} is already in use"),
                        retryability: Retryability::Blocked,
                        source_chain: Vec::new(),
                    },
                };
                None
            }
            Event::Connecting {
                hop,
                total_hops,
                attempt,
                ..
            } => {
                self.require_phase(
                    cause,
                    old,
                    &[RuntimePhase::Starting, RuntimePhase::Reconnecting],
                )?;
                if old == RuntimePhase::Starting {
                    self.require_start_phase(cause, StartPhase::BuildingJumpChain)?;
                }
                Self::check_hop(hop, total_hops)?;
                self.state.health = HealthState::default();
                self.state.runtime = RuntimeState::Connecting {
                    hop,
                    total_hops,
                    attempt,
                };
                None
            }
            Event::Authenticating {
                hop, total_hops, ..
            } => {
                self.require_phase(cause, old, &[RuntimePhase::Connecting])?;
                Self::check_hop(hop, total_hops)?;
                if !matches!(self.state.runtime, RuntimeState::Connecting { hop: current_hop, total_hops: current_total, .. } if hop == current_hop && total_hops == current_total)
                {
                    return Err(Self::invalid(cause, old));
                }
                self.state.runtime = RuntimeState::Authenticating { hop, total_hops };
                None
            }
            Event::Authenticated { .. } => {
                self.require_phase(cause, old, &[RuntimePhase::Authenticating])?;
                if !matches!(self.state.runtime, RuntimeState::Authenticating { hop, total_hops } if hop == total_hops)
                {
                    return Err(Self::invalid(cause, old));
                }
                self.state.health.authenticated = true;
                self.state.runtime = RuntimeState::EstablishingForward;
                None
            }
            Event::HopAuthenticated { .. } => {
                self.require_phase(cause, old, &[RuntimePhase::Authenticating])?;
                let RuntimeState::Authenticating { hop, total_hops } = self.state.runtime else {
                    return Err(Self::invalid(cause, old));
                };
                if hop >= total_hops {
                    return Err(Self::invalid(cause, old));
                }
                self.state.runtime = RuntimeState::Connecting {
                    hop: hop + 1,
                    total_hops,
                    attempt: 0,
                };
                None
            }
            Event::ForwardReady { .. } => {
                self.require_phase(cause, old, &[RuntimePhase::EstablishingForward])?;
                self.state.health.forward_ready = true;
                self.state.runtime = RuntimeState::Starting {
                    phase: StartPhase::StartingHealthMonitor,
                };
                None
            }
            Event::PingSucceeded { at, rtt, .. } => {
                self.require_phase(
                    cause,
                    old,
                    &[
                        RuntimePhase::Starting,
                        RuntimePhase::Healthy,
                        RuntimePhase::Degraded,
                    ],
                )?;
                if !self.state.health.authenticated || !self.state.health.forward_ready {
                    return Err(Self::invalid(cause, old));
                }
                self.state.health.last_rtt_ms = Some(rtt.as_millis().min(u32::MAX as u128) as u32);
                self.state.health.consecutive_ping_failures = 0;
                if !matches!(self.state.runtime, RuntimeState::Healthy { .. }) {
                    self.state.runtime = RuntimeState::Healthy { since: at };
                }
                None
            }
            Event::PingFailed {
                at,
                attempt,
                next_retry_at,
                ..
            } => {
                self.require_phase(
                    cause,
                    old,
                    &[
                        RuntimePhase::Starting,
                        RuntimePhase::Healthy,
                        RuntimePhase::Degraded,
                    ],
                )?;
                if !self.state.health.authenticated || !self.state.health.forward_ready {
                    return Err(Self::invalid(cause, old));
                }
                let failures = self
                    .state
                    .health
                    .consecutive_ping_failures
                    .saturating_add(1);
                self.state.health.consecutive_ping_failures = failures;
                if failures >= 3 {
                    self.state.health.authenticated = false;
                    self.state.health.forward_ready = false;
                    self.state.runtime = RuntimeState::Reconnecting {
                        attempt,
                        next_retry_at,
                        reason: TunnelError {
                            kind: TunnelErrorKind::Timeout,
                            stage: FailureStage::HealthCheck,
                            message: "SSH session ping timed out".into(),
                            retryability: Retryability::Transient,
                            source_chain: Vec::new(),
                        },
                    };
                } else if !matches!(self.state.runtime, RuntimeState::Degraded { .. }) {
                    self.state.runtime = RuntimeState::Degraded {
                        since: at,
                        reason: HealthIssue::PingTimeout,
                    };
                }
                None
            }
            Event::Disconnected {
                error,
                attempt,
                next_retry_at,
                ..
            } => {
                self.require_running(cause, old)?;
                self.state.health = HealthState::default();
                self.state.runtime = match error.retryability {
                    Retryability::Transient => RuntimeState::Reconnecting {
                        attempt,
                        next_retry_at,
                        reason: error,
                    },
                    Retryability::Blocked => RuntimeState::Blocked { reason: error },
                };
                None
            }
        };

        self.record(old, cause);
        Ok(EventOutcome::Applied(effect))
    }

    fn next_generation(&mut self) -> Result<(), MachineError> {
        self.state.generation = self
            .state
            .generation
            .checked_add(1)
            .ok_or(MachineError::GenerationExhausted)?;
        Ok(())
    }

    fn begin(&mut self) {
        self.state.desired = DesiredState::Running;
        self.state.runtime = RuntimeState::Starting {
            phase: StartPhase::Validating,
        };
        self.state.health = HealthState::default();
        self.state.listener = if self.listener_required {
            ListenerState::Unbound
        } else {
            ListenerState::NotRequired
        };
    }

    fn require_running(
        &self,
        cause: &'static str,
        phase: RuntimePhase,
    ) -> Result<(), MachineError> {
        if self.state.desired == DesiredState::Running
            && !matches!(
                phase,
                RuntimePhase::Stopping
                    | RuntimePhase::Stopped
                    | RuntimePhase::Blocked
                    | RuntimePhase::Failed
            )
        {
            Ok(())
        } else {
            Err(Self::invalid(cause, phase))
        }
    }

    fn require_phase(
        &self,
        cause: &'static str,
        phase: RuntimePhase,
        valid: &[RuntimePhase],
    ) -> Result<(), MachineError> {
        self.require_running(cause, phase)?;
        if valid.contains(&phase) {
            Ok(())
        } else {
            Err(Self::invalid(cause, phase))
        }
    }

    fn require_start_phase(
        &self,
        cause: &'static str,
        expected: StartPhase,
    ) -> Result<(), MachineError> {
        let phase = RuntimePhase::from(&self.state.runtime);
        self.require_running(cause, phase)?;
        if matches!(self.state.runtime, RuntimeState::Starting { phase } if phase == expected) {
            Ok(())
        } else {
            Err(Self::invalid(cause, phase))
        }
    }

    fn check_hop(hop: usize, total_hops: usize) -> Result<(), MachineError> {
        if total_hops > 0 && (1..=total_hops).contains(&hop) {
            Ok(())
        } else {
            Err(MachineError::InvalidHop)
        }
    }

    fn invalid(event: &'static str, phase: RuntimePhase) -> MachineError {
        MachineError::InvalidTransition { event, phase }
    }

    fn record(&mut self, from: RuntimePhase, cause: &'static str) {
        if self.journal.len() == JOURNAL_LIMIT {
            self.journal.pop_front();
        }
        self.journal.push_back(StateTransition {
            from,
            to: RuntimePhase::from(&self.state.runtime),
            at: SystemTime::now(),
            cause,
            generation: self.state.generation,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn address() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1080)
    }

    fn transient_error() -> TunnelError {
        TunnelError {
            kind: TunnelErrorKind::Network,
            stage: FailureStage::ConnectTcp,
            message: "connection reset".into(),
            retryability: Retryability::Transient,
            source_chain: Vec::new(),
        }
    }

    fn start_to_forward_ready(machine: &mut StateMachine) -> u64 {
        assert!(machine.transition(Event::Start).is_ok());
        let generation = machine.state().generation;
        assert!(
            machine
                .transition(Event::Phase {
                    generation,
                    phase: StartPhase::AcquiringListener
                })
                .is_ok()
        );
        assert!(
            machine
                .transition(Event::ListenerBound {
                    generation,
                    address: address()
                })
                .is_ok()
        );
        assert!(
            machine
                .transition(Event::Phase {
                    generation,
                    phase: StartPhase::BuildingJumpChain
                })
                .is_ok()
        );
        assert!(
            machine
                .transition(Event::Connecting {
                    generation,
                    hop: 1,
                    total_hops: 1,
                    attempt: 0
                })
                .is_ok()
        );
        assert!(
            machine
                .transition(Event::Authenticating {
                    generation,
                    hop: 1,
                    total_hops: 1
                })
                .is_ok()
        );
        assert!(
            machine
                .transition(Event::Authenticated { generation })
                .is_ok()
        );
        assert!(
            machine
                .transition(Event::ForwardReady { generation })
                .is_ok()
        );
        generation
    }

    #[test]
    fn bound_listener_never_implies_healthy() {
        let mut machine = StateMachine::new(true);
        let started = machine.transition(Event::Start);
        assert!(matches!(
            started,
            Ok(EventOutcome::Applied(Some(
                TransitionEffect::BeginGeneration(1)
            )))
        ));
        assert!(
            machine
                .transition(Event::Phase {
                    generation: 1,
                    phase: StartPhase::AcquiringListener
                })
                .is_ok()
        );
        assert!(
            machine
                .transition(Event::ListenerBound {
                    generation: 1,
                    address: address()
                })
                .is_ok()
        );
        assert!(matches!(
            machine.state().listener,
            ListenerState::Bound { .. }
        ));
        assert!(matches!(
            machine.state().runtime,
            RuntimeState::Starting { .. }
        ));
        assert!(
            machine
                .transition(Event::PingSucceeded {
                    generation: 1,
                    at: Instant::now(),
                    rtt: Duration::from_millis(20)
                })
                .is_err()
        );
    }

    #[test]
    fn verified_session_becomes_healthy_only_after_ping() {
        let mut machine = StateMachine::new(true);
        let generation = start_to_forward_ready(&mut machine);
        assert!(matches!(
            machine.state().runtime,
            RuntimeState::Starting {
                phase: StartPhase::StartingHealthMonitor
            }
        ));
        assert!(
            machine
                .transition(Event::PingSucceeded {
                    generation,
                    at: Instant::now(),
                    rtt: Duration::from_millis(38)
                })
                .is_ok()
        );
        assert!(matches!(
            machine.state().runtime,
            RuntimeState::Healthy { .. }
        ));
        assert_eq!(machine.state().health.last_rtt_ms, Some(38));
    }

    #[test]
    fn stop_during_reconnect_invalidates_old_completion_and_releases_listener() {
        let mut machine = StateMachine::new(true);
        let generation = start_to_forward_ready(&mut machine);
        let later = Instant::now() + Duration::from_secs(5);
        assert!(
            machine
                .transition(Event::Disconnected {
                    generation,
                    error: transient_error(),
                    attempt: 1,
                    next_retry_at: later
                })
                .is_ok()
        );
        assert!(matches!(
            machine.state().runtime,
            RuntimeState::Reconnecting { .. }
        ));
        assert!(matches!(
            machine.state().listener,
            ListenerState::Bound { .. }
        ));
        let stopped = machine.transition(Event::Stop);
        assert!(matches!(
            stopped,
            Ok(EventOutcome::Applied(Some(
                TransitionEffect::CancelAndJoin(1)
            )))
        ));
        assert_eq!(machine.state().generation, 2);
        assert!(matches!(
            machine.transition(Event::PingSucceeded {
                generation,
                at: Instant::now(),
                rtt: Duration::from_millis(10)
            }),
            Ok(EventOutcome::IgnoredStale)
        ));
        assert!(
            machine
                .transition(Event::ShutdownComplete { generation: 2 })
                .is_ok()
        );
        assert!(matches!(machine.state().runtime, RuntimeState::Stopped));
        assert!(matches!(machine.state().listener, ListenerState::Unbound));
        assert_eq!(machine.state().desired, DesiredState::Stopped);
    }

    #[test]
    fn three_ping_failures_reconnect_but_one_degrades() {
        let mut machine = StateMachine::new(true);
        let generation = start_to_forward_ready(&mut machine);
        let now = Instant::now();
        assert!(
            machine
                .transition(Event::PingSucceeded {
                    generation,
                    at: now,
                    rtt: Duration::from_millis(30)
                })
                .is_ok()
        );
        for failure in 1..=3 {
            assert!(
                machine
                    .transition(Event::PingFailed {
                        generation,
                        at: now,
                        attempt: 1,
                        next_retry_at: now + Duration::from_secs(1)
                    })
                    .is_ok()
            );
            if failure < 3 {
                assert!(matches!(
                    machine.state().runtime,
                    RuntimeState::Degraded { .. }
                ));
            }
        }
        assert!(matches!(
            machine.state().runtime,
            RuntimeState::Reconnecting { .. }
        ));
        assert!(matches!(
            machine.state().listener,
            ListenerState::Bound { .. }
        ));
    }

    #[test]
    fn blocked_error_does_not_schedule_retry() {
        let mut machine = StateMachine::new(true);
        assert!(machine.transition(Event::Start).is_ok());
        let error = TunnelError {
            kind: TunnelErrorKind::Authentication,
            stage: FailureStage::Authentication,
            message: "public key rejected".into(),
            retryability: Retryability::Blocked,
            source_chain: Vec::new(),
        };
        assert!(
            machine
                .transition(Event::Disconnected {
                    generation: 1,
                    error,
                    attempt: 1,
                    next_retry_at: Instant::now()
                })
                .is_ok()
        );
        assert!(matches!(
            machine.state().runtime,
            RuntimeState::Blocked { .. }
        ));
        assert!(
            machine
                .transition(Event::Connecting {
                    generation: 1,
                    hop: 1,
                    total_hops: 1,
                    attempt: 2
                })
                .is_err()
        );
    }

    #[test]
    fn intermediate_hop_does_not_authenticate_final_session() {
        let mut machine = StateMachine::new(false);
        assert!(machine.transition(Event::Start).is_ok());
        assert!(
            machine
                .transition(Event::Phase {
                    generation: 1,
                    phase: StartPhase::BuildingJumpChain
                })
                .is_ok()
        );
        assert!(
            machine
                .transition(Event::Connecting {
                    generation: 1,
                    hop: 1,
                    total_hops: 2,
                    attempt: 0
                })
                .is_ok()
        );
        assert!(
            machine
                .transition(Event::Authenticating {
                    generation: 1,
                    hop: 1,
                    total_hops: 2
                })
                .is_ok()
        );
        assert!(
            machine
                .transition(Event::Authenticated { generation: 1 })
                .is_err()
        );
        assert!(
            machine
                .transition(Event::HopAuthenticated { generation: 1 })
                .is_ok()
        );
        assert!(!machine.state().health.authenticated);
        assert!(
            machine
                .transition(Event::Authenticating {
                    generation: 1,
                    hop: 2,
                    total_hops: 2
                })
                .is_ok()
        );
        assert!(
            machine
                .transition(Event::Authenticated { generation: 1 })
                .is_ok()
        );
        assert!(machine.state().health.authenticated);
    }

    #[test]
    fn restart_waits_for_cleanup_before_starting_next_generation() {
        let mut machine = StateMachine::new(true);
        assert!(machine.transition(Event::Start).is_ok());
        assert!(
            machine
                .transition(Event::Phase {
                    generation: 1,
                    phase: StartPhase::AcquiringListener
                })
                .is_ok()
        );
        assert!(
            machine
                .transition(Event::ListenerBound {
                    generation: 1,
                    address: address()
                })
                .is_ok()
        );
        assert!(matches!(
            machine.transition(Event::Restart),
            Ok(EventOutcome::Applied(Some(
                TransitionEffect::CancelAndJoin(1)
            )))
        ));
        assert!(matches!(machine.state().runtime, RuntimeState::Stopping));
        assert!(matches!(
            machine.transition(Event::ShutdownComplete { generation: 1 }),
            Ok(EventOutcome::IgnoredStale)
        ));
        assert!(matches!(
            machine.transition(Event::ShutdownComplete { generation: 2 }),
            Ok(EventOutcome::Applied(Some(
                TransitionEffect::BeginGeneration(2)
            )))
        ));
        assert!(matches!(machine.state().listener, ListenerState::Unbound));
    }

    #[test]
    fn journal_is_bounded() {
        let mut machine = StateMachine::new(false);
        for _ in 0..250 {
            assert!(machine.transition(Event::ConfigChanged).is_ok());
        }
        assert_eq!(machine.journal().len(), JOURNAL_LIMIT);
        assert_eq!(machine.state().generation, 250);
    }

    proptest! {
        #[test]
        fn stopped_never_accepts_old_health_events(actions in proptest::collection::vec(0u8..4, 0..200)) {
            let mut machine = StateMachine::new(true);
            let mut old_generation = 0;
            for action in actions {
                match action {
                    0 => {
                        if matches!(machine.state().runtime, RuntimeState::Stopped) {
                            let _ = machine.transition(Event::Start);
                            old_generation = machine.state().generation;
                        }
                    }
                    1 => {
                        let _ = machine.transition(Event::Stop);
                    }
                    2 => {
                        if matches!(machine.state().runtime, RuntimeState::Stopping) {
                            let generation = machine.state().generation;
                            let _ = machine.transition(Event::ShutdownComplete { generation });
                        }
                    }
                    _ => {
                        let _ = machine.transition(Event::PingSucceeded {
                            generation: old_generation,
                            at: Instant::now(),
                            rtt: Duration::from_millis(1),
                        });
                    }
                }
                if machine.state().desired == DesiredState::Stopped {
                    prop_assert!(!matches!(machine.state().runtime, RuntimeState::Healthy { .. }), "stopped intent became healthy");
                }
                if matches!(machine.state().runtime, RuntimeState::Stopped) {
                    prop_assert!(!matches!(machine.state().listener, ListenerState::Bound { .. }), "stopped runtime retained listener state");
                }
            }
        }
    }
}
