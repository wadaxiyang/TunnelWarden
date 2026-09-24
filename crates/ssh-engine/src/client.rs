use std::{
    io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use russh::{Disconnect, client};
use thiserror::Error;
use tokio::{
    net::TcpStream,
    time::{timeout, timeout_at},
};
use tokio_util::sync::CancellationToken;
use tunnel_domain::{
    AuthConfig, FailureStage, Retryability, SshHost, TunnelError, TunnelErrorKind,
};
use zeroize::Zeroizing;

use crate::{
    cancellable_stream::CancellableStream,
    host_key::{HostKeyError, HostKeyVerifier},
};

const MAX_KNOWN_HOSTS_PATHS: usize = 2;
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3);

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
    #[error("SSH password was rejected")]
    AuthenticationRejected,
    #[error("SSH session operation failed: {0}")]
    Session(#[source] russh::Error),
}

impl SshConnectError {
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
            Self::Timeout("authentication") => (
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
            Self::Handshake(HostKeyError::Unknown { .. }) => (
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
            Self::Authentication(_) => (
                TunnelErrorKind::Network,
                FailureStage::Authentication,
                Retryability::Transient,
            ),
            Self::Session(_) => (
                TunnelErrorKind::Network,
                FailureStage::HealthCheck,
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
}

impl DirectSshSession {
    /// Establish one direct, authenticated SSH session. The caller supplies a
    /// secret loaded from its credential store and retains the parent token.
    /// Unknown/changed keys fail closed under Strict policy.
    pub async fn connect_password(
        host: &SshHost,
        mut password: Zeroizing<String>,
        known_hosts_paths: &[PathBuf],
        parent_cancellation: &CancellationToken,
    ) -> Result<Self, SshConnectError> {
        validate(host, known_hosts_paths)?;
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
        let verifier = HostKeyVerifier {
            host: host.hostname.clone(),
            port: host.port,
            paths: known_hosts_paths.to_vec(),
            policy: host.host_key_policy,
        };
        let transport = CancellableStream::new(stream, cancellation.clone());
        let handshake = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(SshConnectError::Cancelled),
            result = timeout_at(deadline, client::connect_stream(config, transport, verifier)) => {
                match result {
                    Ok(Ok(handle)) => Ok(handle),
                    Ok(Err(source)) => Err(SshConnectError::Handshake(source)),
                    Err(_) => Err(SshConnectError::Timeout("SSH handshake")),
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

        let raw_password = std::mem::take(&mut *password);
        let authentication = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(SshConnectError::Cancelled),
            result = timeout_at(deadline, handle.authenticate_password(host.username.as_str(), raw_password)) => {
                match result {
                    Ok(Ok(result)) if result.success() => Ok(()),
                    Ok(Ok(_)) => Err(SshConnectError::AuthenticationRejected),
                    Ok(Err(source)) => Err(SshConnectError::Authentication(source)),
                    Err(_) => Err(SshConnectError::Timeout("authentication")),
                }
            }
        };
        if let Err(error) = authentication {
            cancellation.cancel();
            return Err(error);
        }
        Ok(Self {
            handle: Some(handle),
            cancellation,
        })
    }

    pub fn is_closed(&self) -> bool {
        self.handle.as_ref().is_none_or(client::Handle::is_closed)
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

    /// Explicitly ends the session and joins russh's session task with a
    /// bounded deadline. Drop also cancels its transport as a fallback.
    pub async fn disconnect(mut self) -> Result<(), SshConnectError> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        let send_result = handle
            .disconnect(Disconnect::ByApplication, "TunnelWarden stopped", "")
            .await;
        let joined = timeout(SHUTDOWN_DEADLINE, handle).await;
        self.cancellation.cancel();
        if let Err(source) = send_result {
            return Err(SshConnectError::Session(source));
        }
        match joined {
            Ok(Ok(())) => Ok(()),
            Ok(Err(HostKeyError::Protocol(russh::Error::Disconnect))) => Ok(()),
            Ok(Err(source)) => Err(SshConnectError::Handshake(source)),
            Err(_) => Err(SshConnectError::Timeout("SSH shutdown")),
        }
    }
}

impl Drop for DirectSshSession {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

fn validate(host: &SshHost, known_hosts_paths: &[PathBuf]) -> Result<(), SshConnectError> {
    if !matches!(host.auth, AuthConfig::Password { .. }) {
        return Err(SshConnectError::InvalidConfig("host auth must be password"));
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
