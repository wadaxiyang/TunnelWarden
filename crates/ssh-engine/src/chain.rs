use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    time::Duration,
};

use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tunnel_domain::{RemoteEndpoint, SshHost};

use crate::{
    DirectSshSession, HostKeyApproval, KeyboardInteractiveApproval, SshConnectError, SshCredential,
    client::AuthPrompts,
};

const MAX_HOPS: usize = 16;

pub struct HopSpec {
    pub host: SshHost,
    pub credential: SshCredential,
    pub known_hosts_paths: Vec<PathBuf>,
}

#[derive(Debug, Error)]
pub enum SshChainError {
    #[error("jump chain is empty")]
    Empty,
    #[error("jump chain exceeds 16 hosts")]
    TooManyHops,
    #[error("SSH hop {hop} failed: {source}")]
    Hop {
        hop: usize,
        #[source]
        source: Box<SshConnectError>,
    },
    #[error("credential for SSH hop {hop} unavailable: {reason}")]
    Credential { hop: usize, reason: String },
}

/// Owns every SSH session in order. A later hop's transport is an SSH channel
/// of its predecessor; dropping the chain cancels all transports, and explicit
/// disconnect joins them from the final host back to the first.
pub struct SshChain {
    sessions: Vec<DirectSshSession>,
}

impl SshChain {
    pub fn single(session: DirectSshSession) -> Self {
        Self {
            sessions: vec![session],
        }
    }

    pub async fn connect(
        hops: Vec<HopSpec>,
        remote: Option<RemoteEndpoint>,
        cancellation: &CancellationToken,
    ) -> Result<Self, SshChainError> {
        Self::connect_with_approval(hops, remote, cancellation, None).await
    }

    pub async fn connect_with_approval(
        hops: Vec<HopSpec>,
        remote: Option<RemoteEndpoint>,
        cancellation: &CancellationToken,
        approval: Option<HostKeyApproval>,
    ) -> Result<Self, SshChainError> {
        Self::connect_with_prompts(hops, remote, cancellation, approval, None).await
    }

    pub async fn connect_with_prompts(
        hops: Vec<HopSpec>,
        remote: Option<RemoteEndpoint>,
        cancellation: &CancellationToken,
        approval: Option<HostKeyApproval>,
        interactive: Option<KeyboardInteractiveApproval>,
    ) -> Result<Self, SshChainError> {
        if hops.is_empty() {
            return Err(SshChainError::Empty);
        }
        if hops.len() > MAX_HOPS {
            return Err(SshChainError::TooManyHops);
        }
        let total_hops = hops.len();
        let interactive = interactive.map(|provider| provider.next_attempt());
        let mut sessions = Vec::with_capacity(total_hops);
        let mut remaining = hops.into_iter().peekable();
        let first = remaining.next().ok_or(SshChainError::Empty)?;
        let first_remote = if remaining.peek().is_none() {
            remote.clone()
        } else {
            None
        };
        let session = DirectSshSession::connect_authenticated(
            &first.host,
            first.credential,
            &first.known_hosts_paths,
            cancellation,
            first_remote,
            AuthPrompts {
                host_key: approval.clone(),
                interactive: interactive.as_ref().map(|provider| provider.for_hop(1)),
            },
        )
        .await
        .map_err(|source| SshChainError::Hop {
            hop: 1,
            source: Box::new(source),
        })?;
        sessions.push(session);

        for (index, hop) in remaining.enumerate() {
            let originator = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
            let channel = match sessions.last() {
                Some(previous) => {
                    previous
                        .open_direct_tcpip(
                            &hop.host.hostname,
                            hop.host.port,
                            originator,
                            hop.host.connect_timeout,
                        )
                        .await
                }
                None => unreachable!("first hop was inserted"),
            };
            let channel = match channel {
                Ok(channel) => channel,
                Err(source) => {
                    disconnect_all(&mut sessions).await;
                    return Err(SshChainError::Hop {
                        hop: index + 2,
                        source: Box::new(source),
                    });
                }
            };
            let final_remote = if index + 2 == total_hops {
                remote.clone()
            } else {
                None
            };
            let next = DirectSshSession::connect_via_channel(
                &hop.host,
                hop.credential,
                &hop.known_hosts_paths,
                cancellation,
                final_remote,
                channel,
                AuthPrompts {
                    host_key: approval.clone(),
                    interactive: interactive
                        .as_ref()
                        .map(|provider| provider.for_hop(index + 2)),
                },
            )
            .await;
            match next {
                Ok(session) => sessions.push(session),
                Err(source) => {
                    disconnect_all(&mut sessions).await;
                    return Err(SshChainError::Hop {
                        hop: index + 2,
                        source: Box::new(source),
                    });
                }
            }
        }
        Ok(Self { sessions })
    }

    pub fn is_closed(&self) -> bool {
        self.sessions.iter().any(DirectSshSession::is_closed)
    }

    pub fn final_session(&self) -> &DirectSshSession {
        self.sessions.last().expect("nonempty chain invariant")
    }

    pub fn final_session_mut(&mut self) -> &mut DirectSshSession {
        self.sessions.last_mut().expect("nonempty chain invariant")
    }

    pub async fn ping_all(&self, deadline: Duration) -> Result<Vec<Duration>, SshChainError> {
        let mut rtts = Vec::with_capacity(self.sessions.len());
        for (index, session) in self.sessions.iter().enumerate() {
            let rtt = session
                .ping(deadline)
                .await
                .map_err(|source| SshChainError::Hop {
                    hop: index + 1,
                    source: Box::new(source),
                })?;
            rtts.push(rtt);
        }
        Ok(rtts)
    }

    pub async fn disconnect(mut self) {
        disconnect_all(&mut self.sessions).await;
    }
}

async fn disconnect_all(sessions: &mut Vec<DirectSshSession>) {
    while let Some(session) = sessions.pop() {
        let _ = session.disconnect().await;
    }
}
