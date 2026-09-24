use std::{io, net::SocketAddr, sync::Arc, time::Duration};

use forwarding::{Socks5Frontend, SocksFrontend, SocksReply, TrafficCounters, relay_bidirectional};
use ssh_engine::{
    DirectSshSession, DirectTcpStream, ForwardingFailure, SshChain, SshChainError, SshConnectError,
};
use thiserror::Error;
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot},
    task::JoinSet,
    time::{interval, timeout},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use tunnel_domain::RemoteEndpoint;

use crate::{ListenerRuntime, ListenerRuntimeError};

const MAX_ACTIVE_CONNECTIONS: usize = 64;
const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const SOCKS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3);
const HEALTH_INTERVAL: Duration = Duration::from_secs(10);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum LocalForwardError {
    #[error("local listener is not bound")]
    NoListener,
    #[error("SSH session closed")]
    SessionClosed,
    #[error("SSH health check failed: {0}")]
    Health(#[source] Box<SshChainError>),
    #[error("local accept failed: {0}")]
    Accept(#[source] io::Error),
    #[error("listener shutdown failed: {0}")]
    Shutdown(#[from] ListenerRuntimeError),
}

#[derive(Clone)]
enum ForwardMode {
    Fixed(RemoteEndpoint),
    Dynamic,
}

struct OpenRequest {
    destination_host: String,
    destination_port: u16,
    originator: SocketAddr,
    reply: oneshot::Sender<Result<DirectTcpStream, SshConnectError>>,
}

/// Owns the local listener, SSH session, and bounded connection task tree for
/// either Local or Dynamic forwarding. The caller drives `run`, then `stop`.
pub struct LocalForwardWorker {
    listener: ListenerRuntime,
    session: Option<SshChain>,
    mode: ForwardMode,
    cancellation: CancellationToken,
    session_cancellation: CancellationToken,
    connections: JoinSet<io::Result<()>>,
    counters: Arc<TrafficCounters>,
}

impl LocalForwardWorker {
    pub fn new(
        listener: ListenerRuntime,
        session: DirectSshSession,
        destination: RemoteEndpoint,
        cancellation: CancellationToken,
    ) -> Result<Self, LocalForwardError> {
        Self::build(
            listener,
            SshChain::single(session),
            ForwardMode::Fixed(destination),
            cancellation,
        )
    }

    pub fn new_dynamic(
        listener: ListenerRuntime,
        session: DirectSshSession,
        cancellation: CancellationToken,
    ) -> Result<Self, LocalForwardError> {
        Self::build(
            listener,
            SshChain::single(session),
            ForwardMode::Dynamic,
            cancellation,
        )
    }

    pub fn new_chain(
        listener: ListenerRuntime,
        session: SshChain,
        destination: RemoteEndpoint,
        cancellation: CancellationToken,
    ) -> Result<Self, LocalForwardError> {
        Self::build(
            listener,
            session,
            ForwardMode::Fixed(destination),
            cancellation,
        )
    }

    pub fn new_dynamic_chain(
        listener: ListenerRuntime,
        session: SshChain,
        cancellation: CancellationToken,
    ) -> Result<Self, LocalForwardError> {
        Self::build(listener, session, ForwardMode::Dynamic, cancellation)
    }

    fn build(
        listener: ListenerRuntime,
        session: SshChain,
        mode: ForwardMode,
        cancellation: CancellationToken,
    ) -> Result<Self, LocalForwardError> {
        if listener.listener().is_none() {
            return Err(LocalForwardError::NoListener);
        }
        Ok(Self {
            listener,
            session: Some(session),
            mode,
            session_cancellation: cancellation.child_token(),
            cancellation,
            connections: JoinSet::new(),
            counters: Arc::new(TrafficCounters::default()),
        })
    }

    pub fn local_addr(&self) -> io::Result<Option<SocketAddr>> {
        self.listener.local_addr()
    }

    pub fn counters(&self) -> &TrafficCounters {
        &self.counters
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub async fn run(&mut self) -> Result<(), LocalForwardError> {
        let Some(session) = self.session.as_ref() else {
            return Err(LocalForwardError::SessionClosed);
        };
        let Some(listener) = self.listener.listener() else {
            return Err(LocalForwardError::NoListener);
        };
        // One pending open per accepted connection at most; requests cannot
        // accumulate after the connection task limit is reached.
        let (open_tx, mut open_rx) = mpsc::channel::<OpenRequest>(MAX_ACTIVE_CONNECTIONS);
        let mut health_tick = interval(HEALTH_INTERVAL);
        let mut session_tick = interval(Duration::from_secs(1));
        let mut ping_failures = 0u8;
        loop {
            tokio::select! {
                biased;
                _ = self.cancellation.cancelled() => return Ok(()),
                _ = session_tick.tick() => {
                    if session.is_closed() { return Err(LocalForwardError::SessionClosed); }
                }
                _ = health_tick.tick() => {
                    match session.ping_all(HEALTH_TIMEOUT).await {
                        Ok(_) => ping_failures = 0,
                        Err(error) => {
                            ping_failures = ping_failures.saturating_add(1);
                            if ping_failures >= 3 { return Err(LocalForwardError::Health(Box::new(error))); }
                            warn!(%error, ping_failures, "SSH health check failed");
                        }
                    }
                }
                Some(result) = self.connections.join_next(), if !self.connections.is_empty() => {
                    match result {
                        Ok(Ok(())) => debug!("forwarded connection completed"),
                        Ok(Err(error)) => warn!(%error, "forwarded connection failed"),
                        Err(error) => warn!(%error, "forwarded connection task failed"),
                    }
                }
                Some(request) = open_rx.recv() => {
                    let result = tokio::select! {
                        biased;
                        _ = self.cancellation.cancelled() => Err(SshConnectError::Cancelled),
                        result = session.final_session().open_direct_tcpip(
                            &request.destination_host,
                            request.destination_port,
                            request.originator,
                            CHANNEL_OPEN_TIMEOUT,
                        ) => result,
                    };
                    let _ = request.reply.send(result);
                }
                accepted = listener.accept(), if self.connections.len() < MAX_ACTIVE_CONNECTIONS => {
                    let (local, originator) = accepted.map_err(LocalForwardError::Accept)?;
                    let mode = self.mode.clone();
                    let requests = open_tx.clone();
                    let cancellation = self.session_cancellation.child_token();
                    let counters = Arc::clone(&self.counters);
                    self.connections.spawn(async move {
                        handle_connection(local, originator, mode, requests, cancellation, counters).await
                    });
                }
            }
        }
    }

    pub async fn stop(self) -> Result<(), LocalForwardError> {
        self.cancellation.cancel();
        let mut listener = self.release_listener().await;
        listener.stop()?;
        Ok(())
    }

    /// Ends one SSH attempt and its relay tasks while keeping the listener
    /// bound for a reconnecting supervisor.
    pub async fn release_listener(mut self) -> ListenerRuntime {
        self.session_cancellation.cancel();
        let mut connections = std::mem::take(&mut self.connections);
        if timeout(SHUTDOWN_DEADLINE, async {
            while connections.join_next().await.is_some() {}
        })
        .await
        .is_err()
        {
            warn!("connection shutdown exceeded deadline; remaining tasks aborted");
            connections.abort_all();
            while connections.join_next().await.is_some() {}
        }
        if let Some(session) = self.session.take() {
            session.disconnect().await;
        }
        std::mem::take(&mut self.listener)
    }
}

async fn handle_connection(
    mut local: TcpStream,
    originator: SocketAddr,
    mode: ForwardMode,
    requests: mpsc::Sender<OpenRequest>,
    cancellation: CancellationToken,
    counters: Arc<TrafficCounters>,
) -> io::Result<()> {
    let destination = match &mode {
        ForwardMode::Fixed(destination) => (destination.host.clone(), destination.port),
        ForwardMode::Dynamic => {
            let parsed = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Ok(()),
                result = timeout(SOCKS_HANDSHAKE_TIMEOUT, Socks5Frontend.read_connect(&mut local)) => result,
            };
            match parsed {
                Ok(Ok(request)) => (request.destination.host(), request.destination.port()),
                Ok(Err(error)) => {
                    debug!(%error, %originator, "SOCKS5 request rejected");
                    return Ok(());
                }
                Err(_) => {
                    debug!(%originator, "SOCKS5 handshake timed out");
                    return Ok(());
                }
            }
        }
    };
    let channel = request_channel(&requests, destination, originator, &cancellation).await;
    let channel = match channel {
        Ok(channel) => channel,
        Err(error) => {
            if matches!(mode, ForwardMode::Dynamic) && !cancellation.is_cancelled() {
                let reply = map_socks_failure(&error);
                let _ = timeout(
                    CHANNEL_OPEN_TIMEOUT,
                    Socks5Frontend.reply(&mut local, reply),
                )
                .await;
            }
            warn!(%error, %originator, "direct-tcpip channel rejected");
            return Ok(());
        }
    };
    if matches!(mode, ForwardMode::Dynamic) {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Ok(()),
            result = Socks5Frontend.reply(&mut local, SocksReply::Succeeded) => {
                result.map_err(io::Error::other)?;
            }
        }
    }
    relay_bidirectional(local, channel, &cancellation, &counters).await
}

async fn request_channel(
    requests: &mpsc::Sender<OpenRequest>,
    destination: (String, u16),
    originator: SocketAddr,
    cancellation: &CancellationToken,
) -> Result<DirectTcpStream, SshConnectError> {
    let (reply, result) = oneshot::channel();
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(SshConnectError::Cancelled),
        sent = requests.send(OpenRequest {
            destination_host: destination.0,
            destination_port: destination.1,
            originator,
            reply,
        }) => if sent.is_err() { return Err(SshConnectError::Cancelled); },
    }
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => Err(SshConnectError::Cancelled),
        response = result => response.unwrap_or(Err(SshConnectError::Cancelled)),
    }
}

fn map_socks_failure(error: &SshConnectError) -> SocksReply {
    match error.forwarding_failure() {
        ForwardingFailure::NotAllowed => SocksReply::NotAllowed,
        ForwardingFailure::ConnectFailed => SocksReply::ConnectionRefused,
        ForwardingFailure::Timeout => SocksReply::TtlExpired,
        ForwardingFailure::SessionUnavailable => SocksReply::HostUnreachable,
        ForwardingFailure::ResourceShortage | ForwardingFailure::Other => {
            SocksReply::GeneralFailure
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::{ChannelOpenFailure, Error};

    #[test]
    fn channel_failure_produces_socks_failure_instead_of_success() {
        let refused = SshConnectError::ChannelOpen(Error::ChannelOpenFailure(
            ChannelOpenFailure::ConnectFailed,
        ));
        assert_eq!(map_socks_failure(&refused), SocksReply::ConnectionRefused);
        let denied = SshConnectError::ChannelOpen(Error::ChannelOpenFailure(
            ChannelOpenFailure::AdministrativelyProhibited,
        ));
        assert_eq!(map_socks_failure(&denied), SocksReply::NotAllowed);
        assert_eq!(
            map_socks_failure(&SshConnectError::Timeout("direct-tcpip channel")),
            SocksReply::TtlExpired
        );
    }
}
