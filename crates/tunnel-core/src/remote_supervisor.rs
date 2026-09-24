use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use rand::RngExt;
use ssh_engine::{SshChain, SshChainError};
use tokio::{
    sync::{Notify, watch},
    time::sleep,
};
use tokio_util::sync::CancellationToken;
use tracing::warn;
use tunnel_domain::{LocalEndpoint, RetryPolicy, Retryability};

use crate::{
    RemoteForwardError, RemoteForwardWorker, RetrySchedule, SupervisorState, WorkerHealth,
};

const INITIAL_PING_TIMEOUT: Duration = Duration::from_secs(5);

/// Owns remote registration attempts and the active forwarding worker. The
/// connector receives an attempt-scoped token independent of Stop, allowing
/// cancel-tcpip-forward to finish before the SSH transport is closed.
pub struct RemoteForwardSupervisor {
    local_target: LocalEndpoint,
    cancellation: CancellationToken,
    retries: RetrySchedule,
    state: watch::Sender<SupervisorState>,
    retry_hint: Arc<Notify>,
}

impl RemoteForwardSupervisor {
    pub fn new(
        local_target: LocalEndpoint,
        cancellation: CancellationToken,
        retry: RetryPolicy,
    ) -> Self {
        let (state, _) = watch::channel(SupervisorState::Connecting { attempt: 1 });
        Self {
            local_target,
            cancellation,
            retries: RetrySchedule::new(retry),
            state,
            retry_hint: Arc::new(Notify::new()),
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<SupervisorState> {
        self.state.subscribe()
    }

    pub fn retry_handle(&self) -> Arc<Notify> {
        Arc::clone(&self.retry_hint)
    }

    pub async fn run<F, Fut>(self, mut connect: F)
    where
        F: FnMut(CancellationToken) -> Fut,
        Fut: Future<Output = Result<SshChain, SshChainError>>,
    {
        let mut attempt = 1u32;
        loop {
            if self.cancellation.is_cancelled() {
                break;
            }
            self.state
                .send_replace(SupervisorState::Connecting { attempt });
            let session_token = CancellationToken::new();
            let result = tokio::select! {
                biased;
                _ = self.cancellation.cancelled() => {
                    session_token.cancel();
                    break;
                }
                result = connect(session_token.clone()) => result,
            };
            let chain = match result {
                Ok(chain) => chain,
                Err(error) => {
                    session_token.cancel();
                    let blocked =
                        super::supervisor::chain_retryability(&error) == Retryability::Blocked;
                    if !self
                        .after_failure(attempt, error.to_string(), blocked)
                        .await
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
                    session_token.cancel();
                    if !self.after_failure(attempt, error.to_string(), false).await {
                        break;
                    }
                    attempt = attempt.saturating_add(1);
                    continue;
                }
            };
            let worker_token = CancellationToken::new();
            let mut worker = match RemoteForwardWorker::start_chain(
                chain,
                self.local_target.clone(),
                worker_token.clone(),
            )
            .await
            {
                Ok(worker) => worker,
                Err(error) => {
                    let blocked = remote_retryability(&error) == Retryability::Blocked;
                    session_token.cancel();
                    if !self
                        .after_failure(attempt, error.to_string(), blocked)
                        .await
                    {
                        break;
                    }
                    attempt = attempt.saturating_add(1);
                    continue;
                }
            };
            self.state.send_replace(SupervisorState::Healthy {
                rtts,
                remote_port: Some(worker.bound_port()),
            });
            let healthy_since = Instant::now();
            let remote_port = worker.bound_port();
            let mut health = worker.subscribe_health();
            let mut run = Box::pin(worker.run());
            let outcome = loop {
                tokio::select! {
                    biased;
                    _ = self.cancellation.cancelled() => {
                        worker_token.cancel();
                        break Ok(());
                    }
                    result = &mut run => break result,
                    changed = health.changed() => {
                        if changed.is_err() { break Err(RemoteForwardError::SessionClosed); }
                        match &*health.borrow_and_update() {
                            Some(WorkerHealth::Healthy { rtts }) => {
                                self.state.send_replace(SupervisorState::Healthy { rtts: rtts.clone(), remote_port: Some(remote_port) });
                            }
                            Some(WorkerHealth::Degraded { failed_pings, reason }) => {
                                self.state.send_replace(SupervisorState::Degraded { failed_pings: *failed_pings, reason: reason.clone() });
                            }
                            None => {}
                        }
                    }
                }
            };
            drop(run);
            if let Err(error) = worker.stop().await {
                warn!(%error, "remote forward cleanup failed");
            }
            session_token.cancel();
            if self.cancellation.is_cancelled() {
                break;
            }
            let reason = match outcome {
                Ok(()) => "remote SSH worker stopped unexpectedly".to_owned(),
                Err(error) => error.to_string(),
            };
            if self.retries.should_reset(healthy_since.elapsed()) {
                attempt = 1;
            } else {
                attempt = attempt.saturating_add(1);
            }
            if !self.after_failure(attempt, reason, false).await {
                break;
            }
        }
        self.state.send_replace(SupervisorState::Stopped);
    }

    async fn after_failure(&self, attempt: u32, reason: String, blocked: bool) -> bool {
        let reason: String = reason.chars().take(512).collect();
        if blocked {
            self.state.send_replace(SupervisorState::Blocked { reason });
            return tokio::select! {
                biased;
                _ = self.cancellation.cancelled() => false,
                _ = self.retry_hint.notified() => true,
            };
        }
        let limit = self.retries.jitter_limit();
        let jitter = rand::rng().random_range(-limit..=limit);
        let delay = self.retries.delay(attempt, jitter);
        self.state.send_replace(SupervisorState::Reconnecting {
            attempt,
            delay,
            reason,
        });
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => false,
            _ = sleep(delay) => true,
            _ = self.retry_hint.notified() => true,
        }
    }
}

fn remote_retryability(error: &RemoteForwardError) -> Retryability {
    match error {
        RemoteForwardError::InvalidTarget => Retryability::Blocked,
        RemoteForwardError::Registration(source) | RemoteForwardError::Cancellation(source) => {
            source.summary().retryability
        }
        RemoteForwardError::SessionClosed | RemoteForwardError::Health(_) => {
            Retryability::Transient
        }
    }
}
