use std::{io, sync::Arc, time::Duration};

use forwarding::{TrafficCounters, relay_bidirectional};
use ssh_engine::{
    DirectSshSession, DirectTcpStream, RemoteForwardRegistration, SshChain, SshChainError,
    SshConnectError,
};
use thiserror::Error;
use tokio::{
    net::TcpStream,
    sync::mpsc,
    task::JoinSet,
    time::{interval, timeout},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use tunnel_domain::LocalEndpoint;

const MAX_ACTIVE_CONNECTIONS: usize = 64;
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const LOCAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3);
const HEALTH_INTERVAL: Duration = Duration::from_secs(10);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum RemoteForwardError {
    #[error("invalid local target")]
    InvalidTarget,
    #[error("SSH session closed")]
    SessionClosed,
    #[error("SSH health check failed: {0}")]
    Health(#[source] Box<SshChainError>),
    #[error("remote forward registration failed: {0}")]
    Registration(#[source] SshConnectError),
    #[error("remote forward cancellation failed: {0}")]
    Cancellation(#[source] SshConnectError),
}

/// Owns one SSH remote forward registration and its bounded relay task tree.
pub struct RemoteForwardWorker {
    session: Option<SshChain>,
    channels: mpsc::Receiver<DirectTcpStream>,
    local_target: LocalEndpoint,
    bound_port: u16,
    cancellation: CancellationToken,
    connections: JoinSet<io::Result<()>>,
    counters: Arc<TrafficCounters>,
}

impl RemoteForwardWorker {
    pub async fn start(
        session: DirectSshSession,
        local_target: LocalEndpoint,
        cancellation: CancellationToken,
    ) -> Result<Self, RemoteForwardError> {
        Self::start_chain(SshChain::single(session), local_target, cancellation).await
    }

    pub async fn start_chain(
        mut session: SshChain,
        local_target: LocalEndpoint,
        cancellation: CancellationToken,
    ) -> Result<Self, RemoteForwardError> {
        if local_target.host.is_empty() || local_target.host.len() > 255 || local_target.port == 0 {
            session.disconnect().await;
            return Err(RemoteForwardError::InvalidTarget);
        }
        let registration = session
            .final_session_mut()
            .request_remote_forward(REMOTE_REQUEST_TIMEOUT)
            .await;
        let RemoteForwardRegistration { port, channels } = match registration {
            Ok(registration) => registration,
            Err(error) => {
                session.disconnect().await;
                return Err(RemoteForwardError::Registration(error));
            }
        };
        Ok(Self {
            session: Some(session),
            channels,
            local_target,
            bound_port: port,
            cancellation,
            connections: JoinSet::new(),
            counters: Arc::new(TrafficCounters::default()),
        })
    }

    pub fn bound_port(&self) -> u16 {
        self.bound_port
    }

    pub fn counters(&self) -> &TrafficCounters {
        &self.counters
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub async fn run(&mut self) -> Result<(), RemoteForwardError> {
        let Some(session) = self.session.as_ref() else {
            return Err(RemoteForwardError::SessionClosed);
        };
        let mut session_tick = interval(Duration::from_secs(1));
        let mut health_tick = interval(HEALTH_INTERVAL);
        let mut ping_failures = 0u8;
        loop {
            tokio::select! {
                biased;
                _ = self.cancellation.cancelled() => return Ok(()),
                _ = session_tick.tick() => {
                    if session.is_closed() { return Err(RemoteForwardError::SessionClosed); }
                }
                _ = health_tick.tick() => {
                    match session.ping_all(HEALTH_TIMEOUT).await {
                        Ok(_) => ping_failures = 0,
                        Err(error) => {
                            ping_failures = ping_failures.saturating_add(1);
                            if ping_failures >= 3 { return Err(RemoteForwardError::Health(Box::new(error))); }
                            warn!(%error, ping_failures, "remote SSH health check failed");
                        }
                    }
                }
                Some(result) = self.connections.join_next(), if !self.connections.is_empty() => {
                    match result {
                        Ok(Ok(())) => debug!("remote relay completed"),
                        Ok(Err(error)) => warn!(%error, "remote relay failed"),
                        Err(error) => warn!(%error, "remote relay task failed"),
                    }
                }
                channel = self.channels.recv(), if self.connections.len() < MAX_ACTIVE_CONNECTIONS => {
                    let Some(channel) = channel else {
                        return Err(RemoteForwardError::SessionClosed);
                    };
                    let target = self.local_target.clone();
                    let cancellation = self.cancellation.child_token();
                    let counters = Arc::clone(&self.counters);
                    self.connections.spawn(async move {
                        let local = tokio::select! {
                            biased;
                            _ = cancellation.cancelled() => return Ok(()),
                            result = timeout(LOCAL_CONNECT_TIMEOUT, TcpStream::connect((target.host.as_str(), target.port))) => {
                                result.map_err(io::Error::other)??
                            }
                        };
                        relay_bidirectional(local, channel, &cancellation, &counters).await
                    });
                }
            }
        }
    }

    pub async fn stop(mut self) -> Result<(), RemoteForwardError> {
        // The server registration must be cancelled while the SSH session is
        // still live. Closing the session afterward also removes it if cancel
        // was denied or timed out.
        let cancel_result = if let Some(session) = self.session.as_mut() {
            session
                .final_session_mut()
                .cancel_remote_forward(REMOTE_REQUEST_TIMEOUT)
                .await
        } else {
            Ok(())
        };
        self.cancellation.cancel();
        let mut connections = std::mem::take(&mut self.connections);
        if timeout(SHUTDOWN_DEADLINE, async {
            while connections.join_next().await.is_some() {}
        })
        .await
        .is_err()
        {
            connections.abort_all();
            while connections.join_next().await.is_some() {}
        }
        if let Some(session) = self.session.take() {
            session.disconnect().await;
        }
        cancel_result.map_err(RemoteForwardError::Cancellation)
    }
}
