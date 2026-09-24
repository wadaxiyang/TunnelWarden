use std::{io, net::SocketAddr, sync::Arc, time::Duration};

use forwarding::{TrafficCounters, relay_bidirectional};
use ssh_engine::DirectSshSession;
use thiserror::Error;
use tokio::{
    task::JoinSet,
    time::{interval, timeout},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};
use tunnel_domain::RemoteEndpoint;

use crate::{ListenerRuntime, ListenerRuntimeError};

const MAX_ACTIVE_CONNECTIONS: usize = 64;
const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3);

#[derive(Debug, Error)]
pub enum LocalForwardError {
    #[error("local listener is not bound")]
    NoListener,
    #[error("SSH session closed")]
    SessionClosed,
    #[error("local accept failed: {0}")]
    Accept(#[source] io::Error),
    #[error("listener shutdown failed: {0}")]
    Shutdown(#[from] ListenerRuntimeError),
}

/// One active Local forwarding session. The owner drives `run` and then
/// `stop`; the only task tree is `relays`, which is bounded and joined.
pub struct LocalForwardWorker {
    listener: ListenerRuntime,
    session: Option<DirectSshSession>,
    destination: RemoteEndpoint,
    cancellation: CancellationToken,
    relays: JoinSet<io::Result<()>>,
    counters: Arc<TrafficCounters>,
}

impl LocalForwardWorker {
    pub fn new(
        listener: ListenerRuntime,
        session: DirectSshSession,
        destination: RemoteEndpoint,
        cancellation: CancellationToken,
    ) -> Result<Self, LocalForwardError> {
        if listener.listener().is_none() {
            return Err(LocalForwardError::NoListener);
        }
        Ok(Self {
            listener,
            session: Some(session),
            destination,
            cancellation,
            relays: JoinSet::new(),
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
        let mut health_tick = interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                biased;
                _ = self.cancellation.cancelled() => return Ok(()),
                _ = health_tick.tick() => {
                    if session.is_closed() { return Err(LocalForwardError::SessionClosed); }
                }
                Some(result) = self.relays.join_next(), if !self.relays.is_empty() => {
                    match result {
                        Ok(Ok(())) => debug!("local relay completed"),
                        Ok(Err(error)) => warn!(%error, "local relay failed"),
                        Err(error) => warn!(%error, "local relay task failed"),
                    }
                }
                accepted = listener.accept(), if self.relays.len() < MAX_ACTIVE_CONNECTIONS => {
                    let (local, originator) = accepted.map_err(LocalForwardError::Accept)?;
                    if session.is_closed() { return Err(LocalForwardError::SessionClosed); }
                    let channel = session.open_direct_tcpip(
                        &self.destination.host,
                        self.destination.port,
                        originator,
                        CHANNEL_OPEN_TIMEOUT,
                    ).await;
                    match channel {
                        Ok(channel) => {
                            let cancellation = self.cancellation.child_token();
                            let counters = Arc::clone(&self.counters);
                            self.relays.spawn(async move {
                                relay_bidirectional(local, channel, &cancellation, &counters).await
                            });
                        }
                        Err(error) => {
                            warn!(%error, %originator, "direct-tcpip channel rejected");
                            // Dropping `local` closes this connection promptly.
                        }
                    }
                }
            }
        }
    }

    pub async fn stop(mut self) -> Result<(), LocalForwardError> {
        self.cancellation.cancel();
        let mut relays = std::mem::take(&mut self.relays);
        if timeout(SHUTDOWN_DEADLINE, relays.shutdown()).await.is_err() {
            warn!("relay shutdown exceeded deadline; remaining tasks aborted");
        }
        // The root cancellation has already closed the SSH transport. The
        // explicit disconnect joins russh's session task when possible.
        if let Some(session) = self.session.take()
            && let Err(error) = session.disconnect().await
        {
            debug!(%error, "SSH session closed during local forward stop");
        }
        self.listener.stop()?;
        Ok(())
    }
}
