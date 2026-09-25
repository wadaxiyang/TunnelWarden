use std::{
    io,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

use russh::{
    Disconnect, client,
    keys::{
        PrivateKeyWithHashAlg,
        agent::{
            AgentIdentity,
            client::{AgentClient, AgentStream},
        },
        decode_secret_key,
    },
};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    net::TcpStream,
    sync::{mpsc, oneshot},
    time::{sleep_until, timeout, timeout_at},
};
use tokio_util::sync::CancellationToken;
use tunnel_domain::{
    AuthConfig, FailureStage, RemoteEndpoint, Retryability, SshHost, TunnelError, TunnelErrorKind,
};
use zeroize::Zeroizing;

use crate::{
    DirectTcpStream,
    cancellable_stream::CancellableStream,
    host_key::{ApprovalPhase, HostKeyApproval, HostKeyError, HostKeyVerifier},
    keyboard_interactive::{
        InteractiveChallenge, InteractiveQuestion, KeyboardInteractiveApproval, MAX_ANSWER_BYTES,
        MAX_PROMPTS_PER_ROUND, MAX_ROUNDS, display_text,
    },
};

const MAX_KNOWN_HOSTS_PATHS: usize = 2;
const MAX_PRIVATE_KEY_BYTES: u64 = 1024 * 1024;
const MAX_FORWARDED_CHANNELS: usize = 64;
const MAX_AGENT_IDENTITIES: usize = 32;
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3);

struct ConnectBudget {
    cancellation: CancellationToken,
    deadline: tokio::time::Instant,
}

struct SessionSetup {
    remote: Option<RemoteEndpoint>,
    private_key: Option<Arc<russh::keys::PrivateKey>>,
    approval: Option<HostKeyApproval>,
    interactive: Option<KeyboardInteractiveApproval>,
}

#[derive(Default)]
pub(crate) struct AuthPrompts {
    pub host_key: Option<HostKeyApproval>,
    pub interactive: Option<KeyboardInteractiveApproval>,
}

pub enum SshCredential {
    Password(Zeroizing<String>),
    PrivateKey(Option<Zeroizing<String>>),
    Agent,
    KeyboardInteractive,
}

#[derive(Debug, Error)]
pub enum SshConnectError {
    #[error("invalid SSH configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("SSH connection cancelled")]
    Cancelled,
    #[error("SSH connection timed out during {0}")]
    Timeout(&'static str),
    #[error("TCP connection failed: {0}")]
    Tcp(#[source] io::Error),
    #[error("SSH handshake failed: {0}")]
    Handshake(#[source] HostKeyError),
    #[error("SSH authentication failed: {0}")]
    Authentication(#[source] russh::Error),
    #[error("SSH credentials were rejected")]
    AuthenticationRejected,
    #[error("keyboard-interactive authentication cancelled")]
    InteractiveCancelled,
    #[error("keyboard-interactive authentication requires an open window")]
    InteractiveUnavailable,
    #[error("keyboard-interactive server challenge is unsupported: {0}")]
    InteractiveLimit(&'static str),
    #[error("private key file is too large")]
    PrivateKeyTooLarge,
    #[error("cannot read private key: {0}")]
    PrivateKeyIo(#[source] io::Error),
    #[error("cannot decode private key: {0}")]
    PrivateKeyDecode(#[source] russh::keys::Error),
    #[error("SSH agent failed: {0}")]
    Agent(#[source] russh::keys::Error),
    #[error("SSH agent authentication failed: {0}")]
    AgentAuthentication(#[source] russh::AgentAuthError),
    #[error("SSH agent has no usable identities")]
    NoAgentIdentities,
    #[error("SSH session operation failed: {0}")]
    Session(#[source] russh::Error),
    #[error("SSH direct-tcpip channel failed: {0}")]
    ChannelOpen(#[source] russh::Error),
    #[error("SSH remote forwarding request failed: {0}")]
    RemoteForward(#[source] russh::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardingFailure {
    NotAllowed,
    ConnectFailed,
    ResourceShortage,
    Timeout,
    SessionUnavailable,
    Other,
}

impl SshConnectError {
    pub fn forwarding_failure(&self) -> ForwardingFailure {
        match self {
            Self::ChannelOpen(russh::Error::ChannelOpenFailure(
                russh::ChannelOpenFailure::AdministrativelyProhibited,
            )) => ForwardingFailure::NotAllowed,
            Self::ChannelOpen(russh::Error::ChannelOpenFailure(
                russh::ChannelOpenFailure::ConnectFailed,
            )) => ForwardingFailure::ConnectFailed,
            Self::ChannelOpen(russh::Error::ChannelOpenFailure(
                russh::ChannelOpenFailure::ResourceShortage,
            )) => ForwardingFailure::ResourceShortage,
            Self::Timeout(_) => ForwardingFailure::Timeout,
            Self::Cancelled | Self::Session(_) => ForwardingFailure::SessionUnavailable,
            _ => ForwardingFailure::Other,
        }
    }

    /// Domain error used by the supervisor and UI. Details are intentionally
    /// bounded and never include the credential value.
    pub fn summary(&self) -> TunnelError {
        let (kind, stage, retryability) = match self {
            Self::InvalidConfig(_) => (
                TunnelErrorKind::InvalidConfig,
                FailureStage::ConfigValidation,
                Retryability::Blocked,
            ),
            Self::Cancelled => (
                TunnelErrorKind::Cancelled,
                FailureStage::Shutdown,
                Retryability::Blocked,
            ),
            Self::Timeout("TCP connect") => (
                TunnelErrorKind::Timeout,
                FailureStage::ConnectTcp,
                Retryability::Transient,
            ),
            Self::Timeout("authentication") | Self::Timeout("authentication interaction") => (
                TunnelErrorKind::Timeout,
                FailureStage::Authentication,
                Retryability::Transient,
            ),
            Self::Timeout("SSH ping") => (
                TunnelErrorKind::Timeout,
                FailureStage::HealthCheck,
                Retryability::Transient,
            ),
            Self::Timeout("SSH shutdown") => (
                TunnelErrorKind::Timeout,
                FailureStage::Shutdown,
                Retryability::Transient,
            ),
            Self::Timeout(_) => (
                TunnelErrorKind::Timeout,
                FailureStage::SshHandshake,
                Retryability::Transient,
            ),
            Self::Tcp(_) => (
                TunnelErrorKind::Network,
                FailureStage::ConnectTcp,
                Retryability::Transient,
            ),
            Self::Handshake(
                HostKeyError::Unknown { .. }
                | HostKeyError::Declined { .. }
                | HostKeyError::ApprovalUnavailable { .. },
            ) => (
                TunnelErrorKind::HostKeyUnknown,
                FailureStage::HostKeyVerification,
                Retryability::Blocked,
            ),
            Self::Handshake(HostKeyError::Changed { .. }) => (
                TunnelErrorKind::HostKeyMismatch,
                FailureStage::HostKeyVerification,
                Retryability::Blocked,
            ),
            Self::Handshake(
                HostKeyError::UnsupportedCertificate { .. }
                | HostKeyError::KnownHostsIo { .. }
                | HostKeyError::KnownHostsTooLarge { .. }
                | HostKeyError::InvalidKnownHosts { .. },
            ) => (
                TunnelErrorKind::InvalidConfig,
                FailureStage::HostKeyVerification,
                Retryability::Blocked,
            ),
            Self::Handshake(HostKeyError::Protocol(_)) => (
                TunnelErrorKind::Network,
                FailureStage::SshHandshake,
                Retryability::Transient,
            ),
            Self::AuthenticationRejected => (
                TunnelErrorKind::Authentication,
                FailureStage::Authentication,
                Retryability::Blocked,
            ),
            Self::InteractiveCancelled
            | Self::InteractiveUnavailable
            | Self::InteractiveLimit(_) => (
                TunnelErrorKind::Authentication,
                FailureStage::Authentication,
                Retryability::Blocked,
            ),
            Self::Authentication(_) => (
                TunnelErrorKind::Network,
                FailureStage::Authentication,
                Retryability::Transient,
            ),
            Self::PrivateKeyTooLarge | Self::PrivateKeyIo(_) | Self::PrivateKeyDecode(_) => (
                TunnelErrorKind::InvalidConfig,
                FailureStage::Authentication,
                Retryability::Blocked,
            ),
            Self::Agent(_) | Self::AgentAuthentication(_) => (
                TunnelErrorKind::Authentication,
                FailureStage::Authentication,
                Retryability::Transient,
            ),
            Self::NoAgentIdentities => (
                TunnelErrorKind::Authentication,
                FailureStage::Authentication,
                Retryability::Blocked,
            ),
            Self::Session(_) => (
                TunnelErrorKind::Network,
                FailureStage::HealthCheck,
                Retryability::Transient,
            ),
            Self::RemoteForward(russh::Error::RequestDenied) => (
                TunnelErrorKind::RemoteForwardRejected,
                FailureStage::RegisterForward,
                Retryability::Blocked,
            ),
            Self::ChannelOpen(_) | Self::RemoteForward(_) => (
                TunnelErrorKind::Network,
                FailureStage::Relay,
                Retryability::Transient,
            ),
        };
        TunnelError {
            kind,
            stage,
            message: self.to_string().chars().take(512).collect(),
            retryability,
            source_chain: Vec::new(),
        }
    }
}

pub struct DirectSshSession {
    handle: Option<client::Handle<HostKeyVerifier>>,
    cancellation: CancellationToken,
    forwarded: Option<(RemoteEndpoint, mpsc::Receiver<DirectTcpStream>)>,
    remote_active: Option<(String, u16)>,
    remote_enabled: Option<Arc<AtomicU32>>,
}

pub struct RemoteForwardRegistration {
    pub port: u16,
    pub channels: mpsc::Receiver<DirectTcpStream>,
}

impl DirectSshSession {
    /// Establish one direct, authenticated SSH session. The caller supplies a
    /// secret loaded from its credential store and retains the parent token.
    /// Unknown/changed keys fail closed under Strict policy.
    pub async fn connect_password(
        host: &SshHost,
        password: Zeroizing<String>,
        known_hosts_paths: &[PathBuf],
        parent_cancellation: &CancellationToken,
    ) -> Result<Self, SshConnectError> {
        Self::connect_authenticated(
            host,
            SshCredential::Password(password),
            known_hosts_paths,
            parent_cancellation,
            None,
            AuthPrompts::default(),
        )
        .await
    }

    /// Authenticates with the private key configured on the host. The optional
    /// passphrase is supplied by the credential store and zeroized on drop.
    pub async fn connect_private_key(
        host: &SshHost,
        passphrase: Option<Zeroizing<String>>,
        known_hosts_paths: &[PathBuf],
        parent_cancellation: &CancellationToken,
    ) -> Result<Self, SshConnectError> {
        Self::connect_authenticated(
            host,
            SshCredential::PrivateKey(passphrase),
            known_hosts_paths,
            parent_cancellation,
            None,
            AuthPrompts::default(),
        )
        .await
    }

    pub async fn connect_agent(
        host: &SshHost,
        known_hosts_paths: &[PathBuf],
        parent_cancellation: &CancellationToken,
    ) -> Result<Self, SshConnectError> {
        Self::connect_authenticated(
            host,
            SshCredential::Agent,
            known_hosts_paths,
            parent_cancellation,
            None,
            AuthPrompts::default(),
        )
        .await
    }

    /// Establishes a session prepared to receive only the specified reverse
    /// forwarding channel. Registration is a separate explicit step.
    pub async fn connect_password_for_remote(
        host: &SshHost,
        password: Zeroizing<String>,
        known_hosts_paths: &[PathBuf],
        parent_cancellation: &CancellationToken,
        remote: RemoteEndpoint,
    ) -> Result<Self, SshConnectError> {
        Self::connect_authenticated(
            host,
            SshCredential::Password(password),
            known_hosts_paths,
            parent_cancellation,
            Some(remote),
            AuthPrompts::default(),
        )
        .await
    }

    pub async fn connect_private_key_for_remote(
        host: &SshHost,
        passphrase: Option<Zeroizing<String>>,
        known_hosts_paths: &[PathBuf],
        parent_cancellation: &CancellationToken,
        remote: RemoteEndpoint,
    ) -> Result<Self, SshConnectError> {
        Self::connect_authenticated(
            host,
            SshCredential::PrivateKey(passphrase),
            known_hosts_paths,
            parent_cancellation,
            Some(remote),
            AuthPrompts::default(),
        )
        .await
    }

    pub async fn connect_agent_for_remote(
        host: &SshHost,
        known_hosts_paths: &[PathBuf],
        parent_cancellation: &CancellationToken,
        remote: RemoteEndpoint,
    ) -> Result<Self, SshConnectError> {
        Self::connect_authenticated(
            host,
            SshCredential::Agent,
            known_hosts_paths,
            parent_cancellation,
            Some(remote),
            AuthPrompts::default(),
        )
        .await
    }

    pub(crate) async fn connect_authenticated(
        host: &SshHost,
        credentials: SshCredential,
        known_hosts_paths: &[PathBuf],
        parent_cancellation: &CancellationToken,
        remote: Option<RemoteEndpoint>,
        prompts: AuthPrompts,
    ) -> Result<Self, SshConnectError> {
        validate(host, &credentials, known_hosts_paths)?;
        let private_key = match (&host.auth, &credentials) {
            (AuthConfig::PrivateKey { key_path, .. }, SshCredential::PrivateKey(passphrase)) => {
                Some(
                    load_private_key(key_path, passphrase.as_ref().map(|value| value.as_str()))
                        .await?,
                )
            }
            _ => None,
        };
        let cancellation = parent_cancellation.child_token();
        let deadline = tokio::time::Instant::now() + host.connect_timeout;
        let stream = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(SshConnectError::Cancelled),
            result = timeout_at(deadline, TcpStream::connect((host.hostname.as_str(), host.port))) => {
                match result {
                    Ok(Ok(stream)) => stream,
                    Ok(Err(source)) => return Err(SshConnectError::Tcp(source)),
                    Err(_) => return Err(SshConnectError::Timeout("TCP connect")),
                }
            }
        };
        configure_first_hop_socket(&stream)?;
        Self::finish_connection(
            host,
            credentials,
            known_hosts_paths,
            ConnectBudget {
                cancellation,
                deadline,
            },
            SessionSetup {
                remote,
                private_key,
                approval: prompts.host_key,
                interactive: prompts.interactive,
            },
            stream,
        )
        .await
    }

    pub(crate) async fn connect_via_channel(
        host: &SshHost,
        credentials: SshCredential,
        known_hosts_paths: &[PathBuf],
        parent_cancellation: &CancellationToken,
        remote: Option<RemoteEndpoint>,
        channel: DirectTcpStream,
        prompts: AuthPrompts,
    ) -> Result<Self, SshConnectError> {
        validate(host, &credentials, known_hosts_paths)?;
        let private_key = match (&host.auth, &credentials) {
            (AuthConfig::PrivateKey { key_path, .. }, SshCredential::PrivateKey(passphrase)) => {
                Some(
                    load_private_key(key_path, passphrase.as_ref().map(|value| value.as_str()))
                        .await?,
                )
            }
            _ => None,
        };
        let cancellation = parent_cancellation.child_token();
        let deadline = tokio::time::Instant::now() + host.connect_timeout;
        Self::finish_connection(
            host,
            credentials,
            known_hosts_paths,
            ConnectBudget {
                cancellation,
                deadline,
            },
            SessionSetup {
                remote,
                private_key,
                approval: prompts.host_key,
                interactive: prompts.interactive,
            },
            channel,
        )
        .await
    }

    async fn finish_connection<S>(
        host: &SshHost,
        credentials: SshCredential,
        known_hosts_paths: &[PathBuf],
        budget: ConnectBudget,
        setup: SessionSetup,
        stream: S,
    ) -> Result<Self, SshConnectError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let ConnectBudget {
            cancellation,
            deadline,
        } = budget;
        let SessionSetup {
            remote,
            private_key,
            approval,
            interactive,
        } = setup;
        let config = Arc::new(client::Config {
            nodelay: true,
            keepalive_interval: if host.keepalive_interval.is_zero() {
                None
            } else {
                Some(host.keepalive_interval)
            },
            keepalive_max: host.keepalive_max as usize,
            ..Default::default()
        });
        let (forwarded_sender, forwarded, remote_enabled) = if let Some(binding) = remote {
            let (sender, receiver) = mpsc::channel(MAX_FORWARDED_CHANNELS);
            let enabled = Arc::new(AtomicU32::new(0));
            (
                Some((binding.clone(), sender, Arc::clone(&enabled))),
                Some((binding, receiver)),
                Some(enabled),
            )
        } else {
            (None, None, None)
        };
        let (approval_phase, mut approval_events) = mpsc::channel(2);
        let verifier = HostKeyVerifier {
            host: host.hostname.clone(),
            port: host.port,
            paths: known_hosts_paths.to_vec(),
            policy: host.host_key_policy,
            approval: approval.clone(),
            approval_phase,
            forwarded: forwarded_sender,
        };
        let transport = CancellableStream::new(stream, cancellation.clone());
        let connect = client::connect_stream(config, transport, verifier);
        tokio::pin!(connect);
        let mut phase_deadline = deadline;
        let mut network_remaining = None;
        let mut approval_events_open = true;
        let handshake = loop {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => break Err(SshConnectError::Cancelled),
                result = &mut connect => break result.map_err(SshConnectError::Handshake),
                phase = approval_events.recv(), if approval_events_open => match phase {
                    Some(ApprovalPhase::Waiting { at }) => {
                        if at >= deadline {
                            break Err(SshConnectError::Timeout("SSH handshake"));
                        }
                        network_remaining = Some(deadline.saturating_duration_since(at));
                        phase_deadline = at + Duration::from_secs(120);
                    }
                    Some(ApprovalPhase::Resolved { at }) => {
                        if let Some(remaining) = network_remaining.take() {
                            phase_deadline = at + remaining;
                        }
                    }
                    None => approval_events_open = false,
                },
                _ = sleep_until(phase_deadline) => {
                    let phase = if network_remaining.is_some() { "host key approval" } else { "SSH handshake" };
                    break Err(SshConnectError::Timeout(phase));
                }
            }
        };
        let mut handle = match handshake {
            Ok(handle) => handle,
            Err(error) => {
                cancellation.cancel();
                return Err(error);
            }
        };

        let authentication = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(SshConnectError::Cancelled),
            result = timeout_at(tokio::time::Instant::now() + if matches!(credentials, SshCredential::KeyboardInteractive) { Duration::from_secs(120) } else { host.connect_timeout }, authenticate(&mut handle, host, credentials, private_key, interactive)) => {
                result.unwrap_or(Err(SshConnectError::Timeout("authentication interaction")))
            },
        };
        if let Err(error) = authentication {
            cancellation.cancel();
            return Err(error);
        }
        Ok(Self {
            handle: Some(handle),
            cancellation,
            forwarded,
            remote_active: None,
            remote_enabled,
        })
    }

    pub fn is_closed(&self) -> bool {
        self.handle.as_ref().is_none_or(client::Handle::is_closed)
    }

    pub async fn request_remote_forward(
        &mut self,
        deadline: Duration,
    ) -> Result<RemoteForwardRegistration, SshConnectError> {
        let (binding, _) = self
            .forwarded
            .as_ref()
            .ok_or(SshConnectError::InvalidConfig(
                "session has no remote forward",
            ))?;
        if binding.host.is_empty() || binding.host.len() > 255 {
            return Err(SshConnectError::InvalidConfig(
                "invalid remote bind address",
            ));
        }
        let handle = self.handle.as_ref().ok_or(SshConnectError::Cancelled)?;
        let assigned = tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => return Err(SshConnectError::Cancelled),
            result = timeout(deadline, handle.tcpip_forward(binding.host.clone(), u32::from(binding.port))) => {
                result.map_err(|_| SshConnectError::Timeout("remote forward request"))?
                    .map_err(SshConnectError::RemoteForward)?
            }
        };
        let port = if binding.port == 0 {
            u16::try_from(assigned)
                .ok()
                .filter(|port| *port != 0)
                .ok_or(SshConnectError::InvalidConfig(
                    "server returned invalid remote port",
                ))?
        } else {
            binding.port
        };
        let (binding, channels) = self.forwarded.take().ok_or(SshConnectError::InvalidConfig(
            "remote forward already registered",
        ))?;
        self.remote_active = Some((binding.host, port));
        if let Some(enabled) = &self.remote_enabled {
            enabled.store(u32::from(port), Ordering::Release);
        }
        Ok(RemoteForwardRegistration { port, channels })
    }

    pub async fn cancel_remote_forward(
        &mut self,
        deadline: Duration,
    ) -> Result<(), SshConnectError> {
        let Some((address, port)) = &self.remote_active else {
            return Ok(());
        };
        if let Some(enabled) = &self.remote_enabled {
            enabled.store(0, Ordering::Release);
        }
        let handle = self.handle.as_ref().ok_or(SshConnectError::Cancelled)?;
        timeout(
            deadline,
            handle.cancel_tcpip_forward(address.clone(), u32::from(*port)),
        )
        .await
        .map_err(|_| SshConnectError::Timeout("cancel remote forward"))?
        .map_err(SshConnectError::RemoteForward)?;
        self.remote_active = None;
        Ok(())
    }

    /// Measures SSH session RTT. Socket/process existence is never used as a
    /// health signal.
    pub async fn ping(&self, deadline: Duration) -> Result<Duration, SshConnectError> {
        let handle = self.handle.as_ref().ok_or(SshConnectError::Cancelled)?;
        let started = Instant::now();
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(SshConnectError::Cancelled),
            result = timeout(deadline, handle.send_ping()) => match result {
                Ok(Ok(())) => Ok(started.elapsed()),
                Ok(Err(source)) => Err(SshConnectError::Session(source)),
                Err(_) => Err(SshConnectError::Timeout("SSH ping")),
            }
        }
    }

    /// Opens one independent SSH forwarding channel. A DOMAIN destination is
    /// passed verbatim to the SSH server, so DNS resolution stays remote.
    pub async fn open_direct_tcpip(
        &self,
        destination_host: &str,
        destination_port: u16,
        originator: SocketAddr,
        deadline: Duration,
    ) -> Result<DirectTcpStream, SshConnectError> {
        if destination_host.is_empty() || destination_host.len() > 255 || destination_port == 0 {
            return Err(SshConnectError::InvalidConfig(
                "invalid forwarding destination",
            ));
        }
        let handle = self.handle.as_ref().ok_or(SshConnectError::Cancelled)?;
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(SshConnectError::Cancelled),
            result = timeout(deadline, handle.channel_open_direct_tcpip(
                destination_host.to_owned(),
                u32::from(destination_port),
                originator.ip().to_string(),
                u32::from(originator.port()),
            )) => match result {
                Ok(Ok(channel)) => Ok(channel.into_stream()),
                Ok(Err(source)) => Err(SshConnectError::ChannelOpen(source)),
                Err(_) => Err(SshConnectError::Timeout("direct-tcpip channel")),
            }
        }
    }

    /// Explicitly ends the session and joins russh's session task with a
    /// bounded deadline. Drop also cancels its transport as a fallback.
    pub async fn disconnect(mut self) -> Result<(), SshConnectError> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        let shutdown = timeout(SHUTDOWN_DEADLINE, async move {
            let sent = handle
                .disconnect(Disconnect::ByApplication, "TunnelWarden stopped", "")
                .await;
            let joined = handle.await;
            (sent, joined)
        })
        .await;
        self.cancellation.cancel();
        let (send_result, joined) =
            shutdown.map_err(|_| SshConnectError::Timeout("SSH shutdown"))?;
        if let Err(source) = send_result {
            return Err(SshConnectError::Session(source));
        }
        match joined {
            Ok(()) => Ok(()),
            Err(HostKeyError::Protocol(russh::Error::Disconnect)) => Ok(()),
            Err(source) => Err(SshConnectError::Handshake(source)),
        }
    }
}

impl Drop for DirectSshSession {
    fn drop(&mut self) {
        if let Some(enabled) = &self.remote_enabled {
            enabled.store(0, Ordering::Release);
        }
        self.cancellation.cancel();
    }
}

async fn authenticate(
    handle: &mut client::Handle<HostKeyVerifier>,
    host: &SshHost,
    credentials: SshCredential,
    private_key: Option<Arc<russh::keys::PrivateKey>>,
    interactive: Option<KeyboardInteractiveApproval>,
) -> Result<(), SshConnectError> {
    if matches!(credentials, SshCredential::Agent) {
        return authenticate_agent(handle, host).await;
    }
    let result = match credentials {
        SshCredential::Password(mut password) => {
            let raw_password = std::mem::take(&mut *password);
            handle
                .authenticate_password(host.username.as_str(), raw_password)
                .await
                .map_err(SshConnectError::Authentication)?
        }
        SshCredential::PrivateKey(_) => {
            let key = private_key.ok_or(SshConnectError::InvalidConfig("missing private key"))?;
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .map_err(SshConnectError::Authentication)?
                .flatten();
            handle
                .authenticate_publickey(
                    host.username.as_str(),
                    PrivateKeyWithHashAlg::new(key, hash),
                )
                .await
                .map_err(SshConnectError::Authentication)?
        }
        SshCredential::Agent => return Err(SshConnectError::InvalidConfig("invalid agent state")),
        SshCredential::KeyboardInteractive => {
            return authenticate_interactive(handle, host, interactive).await;
        }
    };
    if result.success() {
        Ok(())
    } else {
        Err(SshConnectError::AuthenticationRejected)
    }
}

async fn authenticate_interactive(
    handle: &mut client::Handle<HostKeyVerifier>,
    host: &SshHost,
    interactive: Option<KeyboardInteractiveApproval>,
) -> Result<(), SshConnectError> {
    use russh::client::KeyboardInteractiveAuthResponse as Response;
    let interactive = interactive.ok_or(SshConnectError::InteractiveUnavailable)?;
    let mut response = handle
        .authenticate_keyboard_interactive_start(host.username.as_str(), None::<String>)
        .await
        .map_err(SshConnectError::Authentication)?;
    for round in 1..=MAX_ROUNDS {
        let (name, instructions, prompts) = match response {
            Response::Success => return Ok(()),
            Response::Failure { .. } => return Err(SshConnectError::AuthenticationRejected),
            Response::InfoRequest {
                name,
                instructions,
                prompts,
            } => (name, instructions, prompts),
        };
        if prompts.len() > MAX_PROMPTS_PER_ROUND {
            return Err(SshConnectError::InteractiveLimit(
                "more than 8 prompts in one round",
            ));
        }
        let display_bytes = prompts.iter().try_fold(
            name.len().saturating_add(instructions.len()),
            |used, item| used.checked_add(item.prompt.len()),
        );
        if display_bytes.is_none_or(|used| used > crate::keyboard_interactive::MAX_DISPLAY_BYTES) {
            return Err(SshConnectError::InteractiveLimit(
                "challenge display exceeds 4 KiB",
            ));
        }
        let name = display_text(&name).map_err(SshConnectError::InteractiveLimit)?;
        let instructions =
            display_text(&instructions).map_err(SshConnectError::InteractiveLimit)?;
        let questions = prompts
            .into_iter()
            .map(|item| {
                display_text(&item.prompt).map(|text| InteractiveQuestion {
                    text,
                    echo: item.echo,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(SshConnectError::InteractiveLimit)?;
        let expected = questions.len();
        let (reply, answer) = oneshot::channel();
        interactive
            .prompts
            .try_send(interactive.prompt(
                &host.hostname,
                host.port,
                round,
                InteractiveChallenge {
                    name,
                    instructions,
                    questions,
                },
                reply,
            ))
            .map_err(|_| SshConnectError::InteractiveUnavailable)?;
        let mut answers = answer
            .await
            .map_err(|_| SshConnectError::InteractiveCancelled)?
            .ok_or(SshConnectError::InteractiveCancelled)?;
        if answers.len() != expected || answers.iter().any(|value| value.len() > MAX_ANSWER_BYTES) {
            return Err(SshConnectError::InteractiveLimit(
                "answer count or size is invalid",
            ));
        }
        response = handle
            .authenticate_keyboard_interactive_respond(std::mem::take(&mut *answers))
            .await
            .map_err(SshConnectError::Authentication)?;
    }
    match response {
        Response::Success => Ok(()),
        Response::Failure { .. } => Err(SshConnectError::AuthenticationRejected),
        Response::InfoRequest { .. } => Err(SshConnectError::InteractiveLimit(
            "more than 8 challenge rounds",
        )),
    }
}

type DynamicAgent = AgentClient<Box<dyn AgentStream + Send + Unpin>>;

async fn authenticate_agent(
    handle: &mut client::Handle<HostKeyVerifier>,
    host: &SshHost,
) -> Result<(), SshConnectError> {
    let AuthConfig::Agent { socket } = &host.auth else {
        return Err(SshConnectError::InvalidConfig("host auth must be agent"));
    };
    let mut agent = connect_agent_stream(socket.as_deref()).await?;
    let identities = agent
        .request_identities()
        .await
        .map_err(SshConnectError::Agent)?;
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .map_err(SshConnectError::Authentication)?
        .flatten();
    let mut attempted = false;
    for identity in identities.into_iter().take(MAX_AGENT_IDENTITIES) {
        attempted = true;
        let result = match identity {
            AgentIdentity::PublicKey { key, .. } => {
                handle
                    .authenticate_publickey_with(host.username.as_str(), key, hash, &mut agent)
                    .await
            }
            AgentIdentity::Certificate { certificate, .. } => {
                handle
                    .authenticate_certificate_with(
                        host.username.as_str(),
                        certificate,
                        hash,
                        &mut agent,
                    )
                    .await
            }
        }
        .map_err(SshConnectError::AgentAuthentication)?;
        if result.success() {
            return Ok(());
        }
    }
    if attempted {
        Err(SshConnectError::AuthenticationRejected)
    } else {
        Err(SshConnectError::NoAgentIdentities)
    }
}

#[cfg(windows)]
async fn connect_agent_stream(socket: Option<&str>) -> Result<DynamicAgent, SshConnectError> {
    if let Some(path) = socket {
        return AgentClient::connect_named_pipe(path)
            .await
            .map(AgentClient::dynamic)
            .map_err(SshConnectError::Agent);
    }
    match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        Ok(agent) => Ok(agent.dynamic()),
        Err(_) => AgentClient::connect_pageant()
            .await
            .map(AgentClient::dynamic)
            .map_err(SshConnectError::Agent),
    }
}

#[cfg(unix)]
async fn connect_agent_stream(socket: Option<&str>) -> Result<DynamicAgent, SshConnectError> {
    match socket {
        Some(path) => AgentClient::connect_uds(path)
            .await
            .map(AgentClient::dynamic)
            .map_err(SshConnectError::Agent),
        None => AgentClient::connect_env()
            .await
            .map(AgentClient::dynamic)
            .map_err(SshConnectError::Agent),
    }
}

#[cfg(not(any(windows, unix)))]
async fn connect_agent_stream(_socket: Option<&str>) -> Result<DynamicAgent, SshConnectError> {
    Err(SshConnectError::InvalidConfig(
        "SSH agent is unsupported on this platform",
    ))
}

async fn load_private_key(
    path: &PathBuf,
    passphrase: Option<&str>,
) -> Result<Arc<russh::keys::PrivateKey>, SshConnectError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(SshConnectError::PrivateKeyIo)?;
    let mut reader = file.take(MAX_PRIVATE_KEY_BYTES + 1);
    let mut pem = Zeroizing::new(String::new());
    reader
        .read_to_string(&mut pem)
        .await
        .map_err(SshConnectError::PrivateKeyIo)?;
    if pem.len() as u64 > MAX_PRIVATE_KEY_BYTES {
        return Err(SshConnectError::PrivateKeyTooLarge);
    }
    let key = decode_secret_key(&pem, passphrase).map_err(SshConnectError::PrivateKeyDecode)?;
    Ok(Arc::new(key))
}

fn configure_first_hop_socket(stream: &TcpStream) -> Result<(), SshConnectError> {
    stream.set_nodelay(true).map_err(SshConnectError::Tcp)
}

fn validate(
    host: &SshHost,
    credentials: &SshCredential,
    known_hosts_paths: &[PathBuf],
) -> Result<(), SshConnectError> {
    match (&host.auth, credentials) {
        (AuthConfig::Password { .. }, SshCredential::Password(_)) => {}
        (AuthConfig::PrivateKey { .. }, SshCredential::PrivateKey(_)) => {}
        (AuthConfig::Agent { .. }, SshCredential::Agent) => {}
        (AuthConfig::KeyboardInteractive, SshCredential::KeyboardInteractive) => {}
        _ => return Err(SshConnectError::InvalidConfig("SSH auth method mismatch")),
    }
    if host.hostname.trim().is_empty() {
        return Err(SshConnectError::InvalidConfig("hostname is empty"));
    }
    if host.username.trim().is_empty() {
        return Err(SshConnectError::InvalidConfig("username is empty"));
    }
    if host.port == 0 {
        return Err(SshConnectError::InvalidConfig("SSH port is zero"));
    }
    if host.connect_timeout.is_zero() {
        return Err(SshConnectError::InvalidConfig("connect timeout is zero"));
    }
    if known_hosts_paths.len() > MAX_KNOWN_HOSTS_PATHS {
        return Err(SshConnectError::InvalidConfig("too many known_hosts files"));
    }
    Ok(())
}

#[cfg(test)]
mod socket_tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn first_hop_tcp_nodelay_is_set_on_the_socket() {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("local test listener");
        let stream = TcpStream::connect(listener.local_addr().expect("listener address"))
            .await
            .expect("local client");
        let (_accepted, _) = listener.accept().await.expect("accepted client");
        configure_first_hop_socket(&stream).expect("set TCP_NODELAY");
        assert!(stream.nodelay().expect("read TCP_NODELAY"));
    }
}
