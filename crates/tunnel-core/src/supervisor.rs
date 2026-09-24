use std::{
    future::Future,
    io,
    net::SocketAddr,
    time::{Duration, Instant},
};

use forwarding::{Socks5Frontend, SocksFrontend, SocksReply};
use rand::RngExt;
use ssh_engine::{SshChain, SshChainError};
use thiserror::Error;
use tokio::{
    net::TcpStream,
    sync::watch,
    task::JoinSet,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use tunnel_domain::{RemoteEndpoint, RetryPolicy, Retryability};

use crate::{
    ListenerRuntime, ListenerRuntimeError, LocalForwardError, LocalForwardWorker, RetrySchedule,
};

const MAX_UNHEALTHY_CLIENTS: usize = 64;
const SOCKS_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3);
const INITIAL_PING_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub enum SupervisorState {
    Connecting {
        attempt: u32,
    },
    Reconnecting {
        attempt: u32,
        delay: Duration,
        reason: String,
    },
    Healthy {
        rtts: Vec<Duration>,
        remote_port: Option<u16>,
    },
    Blocked {
        reason: String,
    },
    Stopped,
}

#[derive(Clone, Debug)]
pub enum SupervisorMode {
    Local(RemoteEndpoint),
    Dynamic,
}

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error("local listener is not bound")]
    NoListener,
    #[error("local accept failed: {0}")]
    Accept(#[source] io::Error),
    #[error("local worker failed: {0}")]
    Worker(#[from] LocalForwardError),
    #[error("listener shutdown failed: {0}")]
    Shutdown(#[from] ListenerRuntimeError),
}

/// Runs one Local or Dynamic tunnel until cancelled. The local listener stays
/// bound across every SSH attempt; a slow/unhealthy SOCKS client receives a
/// bounded standard failure response. A single watch value is the latest
/// runtime status, so slow observers cannot accumulate an event backlog.
pub struct LocalForwardSupervisor {
    listener: Option<ListenerRuntime>,
    mode: SupervisorMode,
    cancellation: CancellationToken,
    retries: RetrySchedule,
    state: watch::Sender<SupervisorState>,
    unhealthy_clients: JoinSet<()>,
}

impl LocalForwardSupervisor {
    pub fn new(
        listener: ListenerRuntime,
        mode: SupervisorMode,
        cancellation: CancellationToken,
        retry: RetryPolicy,
    ) -> Result<Self, SupervisorError> {
        if listener.listener().is_none() {
            return Err(SupervisorError::NoListener);
        }
        let (state, _) = watch::channel(SupervisorState::Connecting { attempt: 1 });
        Ok(Self {
            listener: Some(listener),
            mode,
            cancellation,
            retries: RetrySchedule::new(retry),
            state,
            unhealthy_clients: JoinSet::new(),
        })
    }

    pub fn subscribe(&self) -> watch::Receiver<SupervisorState> {
        self.state.subscribe()
    }

    pub fn local_addr(&self) -> io::Result<Option<SocketAddr>> {
        self.listener
            .as_ref()
            .map(ListenerRuntime::local_addr)
            .transpose()
            .map(Option::flatten)
    }

    /// The connector reloads credentials and builds a fresh chain on every
    /// call. Its future is dropped on Stop, and any sessions it created are
    /// cancelled by their owners.
    pub async fn run<F, Fut>(mut self, mut connect: F) -> Result<(), SupervisorError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<SshChain, SshChainError>>,
    {
        let result = self.run_until_stopped(&mut connect).await;
        self.cancellation.cancel();
        self.shutdown_unhealthy_clients().await;
        let stopped = match self.listener.take() {
            Some(mut listener) => listener.stop().map_err(SupervisorError::Shutdown),
            None => Ok(()),
        };
        self.state.send_replace(SupervisorState::Stopped);
        result.and(stopped)
    }

    async fn run_until_stopped<F, Fut>(&mut self, connect: &mut F) -> Result<(), SupervisorError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<SshChain, SshChainError>>,
    {
        let mut attempt = 1u32;
        loop {
            if self.cancellation.is_cancelled() {
                break;
            }
            self.state
                .send_replace(SupervisorState::Connecting { attempt });
            let Some(result) = self.while_unhealthy(connect()).await? else {
                break;
            };
            let chain = match result {
                Ok(chain) => chain,
                Err(error) => {
                    let blocked = chain_retryability(&error) == Retryability::Blocked;
                    if !self
                        .after_failure(attempt, error.to_string(), blocked)
                        .await?
                    {
                        break;
                    }
                    attempt = attempt.saturating_add(1);
                    continue;
                }
            };
            let rtts = match chain.ping_all(INITIAL_PING_TIMEOUT).await {
                Ok(rtts) => rtts,
                Err(error) => {
                    chain.disconnect().await;
                    if !self
                        .after_failure(attempt, error.to_string(), false)
                        .await?
                    {
                        break;
                    }
                    attempt = attempt.saturating_add(1);
                    continue;
                }
            };
            let healthy_since = Instant::now();
            let listener = self.listener.take().ok_or(SupervisorError::NoListener)?;
            let mut worker = match &self.mode {
                SupervisorMode::Local(remote) => LocalForwardWorker::new_chain(
                    listener,
                    chain,
                    remote.clone(),
                    self.cancellation.clone(),
                )?,
                SupervisorMode::Dynamic => LocalForwardWorker::new_dynamic_chain(
                    listener,
                    chain,
                    self.cancellation.clone(),
                )?,
            };
            self.state.send_replace(SupervisorState::Healthy {
                rtts,
                remote_port: None,
            });
            let outcome = worker.run().await;
            self.listener = Some(worker.release_listener().await);
            if self.cancellation.is_cancelled() {
                break;
            }
            let reason = match outcome {
                Ok(()) => "SSH worker stopped unexpectedly".to_owned(),
                Err(error) => error.to_string(),
            };
            if self.retries.should_reset(healthy_since.elapsed()) {
                attempt = 1;
            } else {
                attempt = attempt.saturating_add(1);
            }
            if !self.after_failure(attempt, reason, false).await? {
                break;
            }
        }
        Ok(())
    }

    async fn after_failure(
        &mut self,
        attempt: u32,
        reason: String,
        blocked: bool,
    ) -> Result<bool, SupervisorError> {
        let reason: String = reason.chars().take(512).collect();
        if blocked {
            self.state.send_replace(SupervisorState::Blocked { reason });
            let stop = self.cancellation.clone();
            return Ok(self.while_unhealthy(stop.cancelled()).await?.is_some());
        }
        let limit = self.retries.jitter_limit();
        let jitter = rand::rng().random_range(-limit..=limit);
        let delay = self.retries.delay(attempt, jitter);
        self.state.send_replace(SupervisorState::Reconnecting {
            attempt,
            delay,
            reason,
        });
        Ok(self.while_unhealthy(sleep(delay)).await?.is_some())
    }

    async fn while_unhealthy<T>(
        &mut self,
        future: impl Future<Output = T>,
    ) -> Result<Option<T>, SupervisorError> {
        tokio::pin!(future);
        loop {
            let listener = self
                .listener
                .as_ref()
                .and_then(ListenerRuntime::listener)
                .ok_or(SupervisorError::NoListener)?;
            tokio::select! {
                biased;
                _ = self.cancellation.cancelled() => return Ok(None),
                result = &mut future => return Ok(Some(result)),
                Some(result) = self.unhealthy_clients.join_next(), if !self.unhealthy_clients.is_empty() => {
                    if let Err(error) = result { warn!(%error, "unhealthy SOCKS client task failed"); }
                }
                accepted = listener.accept(), if self.unhealthy_clients.len() < MAX_UNHEALTHY_CLIENTS => {
                    let (socket, _) = accepted.map_err(SupervisorError::Accept)?;
                    self.reject_unhealthy(socket);
                }
            }
        }
    }

    fn reject_unhealthy(&mut self, mut socket: TcpStream) {
        if matches!(self.mode, SupervisorMode::Dynamic) {
            let cancellation = self.cancellation.child_token();
            self.unhealthy_clients.spawn(async move {
                let parsed = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return,
                    result = timeout(SOCKS_TIMEOUT, Socks5Frontend.read_connect(&mut socket)) => result,
                };
                if matches!(parsed, Ok(Ok(_))) {
                    let _ = timeout(SOCKS_TIMEOUT, Socks5Frontend.reply(&mut socket, SocksReply::HostUnreachable)).await;
                }
            });
        } else {
            // Raw Local clients receive a prompt close during reconnect.
            drop(socket);
        }
    }

    async fn shutdown_unhealthy_clients(&mut self) {
        if timeout(SHUTDOWN_DEADLINE, async {
            while self.unhealthy_clients.join_next().await.is_some() {}
        })
        .await
        .is_err()
        {
            debug!("unhealthy SOCKS client shutdown exceeded deadline");
            self.unhealthy_clients.abort_all();
            while self.unhealthy_clients.join_next().await.is_some() {}
        }
    }
}

pub(crate) fn chain_retryability(error: &SshChainError) -> Retryability {
    match error {
        SshChainError::Empty | SshChainError::TooManyHops => Retryability::Blocked,
        SshChainError::Hop { source, .. } => source.summary().retryability,
    }
}
