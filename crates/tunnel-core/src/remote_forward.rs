use std::{io, sync::Arc, time::Duration};

use forwarding::{TrafficCounters, relay_bidirectional};
use ssh_engine::{DirectSshSession, DirectTcpStream, RemoteForwardRegistration, SshConnectError};
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

#[derive(Debug, Error)]
pub enum RemoteForwardError {
    #[error("invalid local target")]
    InvalidTarget,
    #[error("SSH session closed")]
    SessionClosed,
    #[error("remote forward registration failed: {0}")]
    Registration(#[source] SshConnectError),
    #[error("remote forward cancellation failed: {0}")]
    Cancellation(#[source] SshConnectError),
}

/// Owns one SSH remote forward registration and its bounded relay task tree.
pub struct RemoteForwardWorker {
    session: Option<DirectSshSession>,
    channels: mpsc::Receiver<DirectTcpStream>,
    local_target: LocalEndpoint,
    bound_port: u16,
    cancellation: CancellationToken,
    connections: JoinSet<io::Result<()>>,
    counters: Arc<TrafficCounters>,
}

impl RemoteForwardWorker {
    pub async fn start(
        mut session: DirectSshSession,
        local_target: LocalEndpoint,
        cancellation: CancellationToken,
    ) -> Result<Self, RemoteForwardError> {
        if local_target.host.is_empty() || local_target.host.len() > 255 || local_target.port == 0 {
            return Err(RemoteForwardError::InvalidTarget);
        }
        let RemoteForwardRegistration { port, channels } = session
            .request_remote_forward(REMOTE_REQUEST_TIMEOUT)
            .await
            .map_err(RemoteForwardError::Registration)?;
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
        let mut health_tick = interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                biased;
                _ = self.cancellation.cancelled() => return Ok(()),
                _ = health_tick.tick() => {
                    if session.is_closed() { return Err(RemoteForwardError::SessionClosed); }
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
            session.cancel_remote_forward(REMOTE_REQUEST_TIMEOUT).await
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
        if let Some(session) = self.session.take()
            && let Err(error) = session.disconnect().await
        {
            debug!(%error, "SSH session closed during remote forward stop");
        }
        cancel_result.map_err(RemoteForwardError::Cancellation)
    }
}
