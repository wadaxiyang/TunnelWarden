use std::{io, net::SocketAddr, time::Instant};
use thiserror::Error;
use tokio::net::TcpListener;
use tunnel_domain::{FailureStage, Retryability, StartPhase, TunnelError, TunnelErrorKind};

use crate::{Event, EventOutcome, Lifecycle, MachineError, StateMachine, TransitionEffect};

#[derive(Debug, Error)]
pub enum ListenerRuntimeError {
    #[error(transparent)]
    State(#[from] MachineError),
    #[error("cannot bind {address}: {source}")]
    Bind {
        address: SocketAddr,
        #[source]
        source: io::Error,
    },
}

/// Owns one Local/Dynamic listener. This is the first resource-owning runtime
/// boundary; it deliberately does not accept connections or claim SSH health.
/// The future supervisor will own this value along with its SSH chain and tasks.
pub struct ListenerRuntime {
    machine: StateMachine,
    listener: Option<TcpListener>,
}

impl ListenerRuntime {
    pub fn new() -> Self {
        Self {
            machine: StateMachine::new(true),
            listener: None,
        }
    }

    pub fn state(&self) -> &Lifecycle {
        self.machine.state()
    }

    pub fn local_addr(&self) -> io::Result<Option<SocketAddr>> {
        self.listener
            .as_ref()
            .map(TcpListener::local_addr)
            .transpose()
    }

    /// Bind is performed by Tokio's worker, never by the UI render thread.
    /// The listener remains owned across a later SSH reconnect until `stop`.
    pub async fn start_listener(
        &mut self,
        address: SocketAddr,
    ) -> Result<u64, ListenerRuntimeError> {
        match self.machine.transition(Event::Start)? {
            EventOutcome::Applied(Some(TransitionEffect::BeginGeneration(generation))) => {
                self.machine.transition(Event::Phase {
                    generation,
                    phase: StartPhase::AcquiringListener,
                })?;
                match TcpListener::bind(address).await {
                    Ok(listener) => {
                        let bound_address = match listener.local_addr() {
                            Ok(bound_address) => bound_address,
                            Err(source) => {
                                self.record_bind_failure(generation, address, &source)?;
                                return Err(ListenerRuntimeError::Bind { address, source });
                            }
                        };
                        self.machine.transition(Event::ListenerBound {
                            generation,
                            address: bound_address,
                        })?;
                        self.listener = Some(listener);
                        self.machine.transition(Event::Phase {
                            generation,
                            phase: StartPhase::BuildingJumpChain,
                        })?;
                        Ok(generation)
                    }
                    Err(source) => {
                        self.record_bind_failure(generation, address, &source)?;
                        Err(ListenerRuntimeError::Bind { address, source })
                    }
                }
            }
            _ => Err(ListenerRuntimeError::State(
                MachineError::InvalidTransition {
                    event: "start",
                    phase: crate::RuntimePhase::from(&self.machine.state().runtime),
                },
            )),
        }
    }

    /// For this stage there are no child tasks to join. Drop the listener
    /// before acknowledging ShutdownComplete, so `Stopped` means no owned port.
    pub fn stop(&mut self) -> Result<(), ListenerRuntimeError> {
        match self.machine.transition(Event::Stop)? {
            EventOutcome::Applied(Some(TransitionEffect::CancelAndJoin(_))) => {
                self.listener = None;
                let generation = self.machine.state().generation;
                self.machine
                    .transition(Event::ShutdownComplete { generation })?;
            }
            EventOutcome::NoChange => {}
            _ => {
                return Err(ListenerRuntimeError::State(
                    MachineError::InvalidTransition {
                        event: "stop",
                        phase: crate::RuntimePhase::from(&self.machine.state().runtime),
                    },
                ));
            }
        }
        Ok(())
    }

    fn record_bind_failure(
        &mut self,
        generation: u64,
        address: SocketAddr,
        source: &io::Error,
    ) -> Result<(), MachineError> {
        if source.kind() == io::ErrorKind::AddrInUse {
            self.machine.transition(Event::ListenerConflict {
                generation,
                address,
            })?;
        } else {
            self.machine.transition(Event::Disconnected {
                generation,
                error: bind_error(source),
                attempt: 0,
                next_retry_at: Instant::now(),
            })?;
        }
        Ok(())
    }
}

impl Default for ListenerRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn bind_error(source: &io::Error) -> TunnelError {
    let (kind, retryability) = match source.kind() {
        io::ErrorKind::PermissionDenied => {
            (TunnelErrorKind::PermissionDenied, Retryability::Blocked)
        }
        io::ErrorKind::AddrNotAvailable | io::ErrorKind::InvalidInput => {
            (TunnelErrorKind::InvalidConfig, Retryability::Blocked)
        }
        _ => (TunnelErrorKind::Network, Retryability::Transient),
    };
    TunnelError {
        kind,
        stage: FailureStage::BindLocalPort,
        message: source.to_string(),
        retryability,
        source_chain: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, TcpListener as StdTcpListener};
    use tunnel_domain::{DesiredState, ListenerState, RuntimeState};

    fn loopback_ephemeral() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
    }

    #[tokio::test]
    async fn stop_releases_real_listener_port() {
        let mut runtime = ListenerRuntime::new();
        assert!(runtime.start_listener(loopback_ephemeral()).await.is_ok());
        let address = runtime.local_addr().expect("local_addr").expect("listener");
        assert!(StdTcpListener::bind(address).is_err());
        assert!(matches!(
            runtime.state().runtime,
            RuntimeState::Starting { .. }
        ));
        assert!(runtime.stop().is_ok());
        assert_eq!(runtime.state().desired, DesiredState::Stopped);
        assert!(matches!(runtime.state().runtime, RuntimeState::Stopped));
        assert!(matches!(runtime.state().listener, ListenerState::Unbound));
        assert!(StdTcpListener::bind(address).is_ok());
    }

    #[tokio::test]
    async fn port_conflict_is_blocked_and_retry_can_bind_after_release() {
        let occupied = StdTcpListener::bind(loopback_ephemeral()).expect("test bind");
        let address = occupied.local_addr().expect("test address");
        let mut runtime = ListenerRuntime::new();
        assert!(runtime.start_listener(address).await.is_err());
        assert!(matches!(
            runtime.state().runtime,
            RuntimeState::Blocked { .. }
        ));
        assert!(matches!(
            runtime.state().listener,
            ListenerState::Conflict { .. }
        ));
        assert!(runtime.stop().is_ok());
        drop(occupied);
        assert!(runtime.start_listener(address).await.is_ok());
        assert!(runtime.stop().is_ok());
    }
}
