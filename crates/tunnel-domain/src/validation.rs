use std::net::IpAddr;

use thiserror::Error;

use crate::{SshHost, TunnelConfig, TunnelGroup, TunnelMode};

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TunnelValidationError {
    #[error("tunnel ID is empty")]
    EmptyId,
    #[error("tunnel name is empty")]
    EmptyName,
    #[error("jump chain must contain a final SSH host")]
    EmptyJumpChain,
    #[error("jump chain repeats host {0}")]
    RepeatedHop(String),
    #[error("jump chain refers to unknown host {0}")]
    UnknownHost(String),
    #[error("host ID {0} is ambiguous")]
    AmbiguousHost(String),
    #[error("tunnel refers to unknown group {0}")]
    UnknownGroup(String),
    #[error("group ID {0} is ambiguous")]
    AmbiguousGroup(String),
    #[error("local listener must use an IP address")]
    InvalidBindAddress,
    #[error("local endpoint is invalid")]
    InvalidLocalEndpoint,
    #[error("remote endpoint is missing or invalid")]
    InvalidRemoteEndpoint,
    #[error("Dynamic forwarding must not have a remote endpoint")]
    UnexpectedRemoteEndpoint,
    #[error("reconnect policy is invalid")]
    InvalidReconnectPolicy,
    #[error("non-loopback listener requires explicit exposure approval")]
    ExposureApprovalRequired,
}

impl TunnelConfig {
    /// Checks consent immediately before starting, after structural validation.
    /// Remote forwarding describes the requested server bind address; server
    /// policy may choose a different effective address.
    pub fn validate_exposure(&self) -> Result<(), TunnelValidationError> {
        let requested_host = match self.mode {
            TunnelMode::Local | TunnelMode::Dynamic => Some(self.local.host.as_str()),
            TunnelMode::Remote => self.remote.as_ref().map(|remote| remote.host.as_str()),
        };
        if requested_host
            .and_then(|host| host.parse::<IpAddr>().ok())
            .is_some_and(|address| !address.is_loopback())
            && !self.exposure_approved
        {
            return Err(TunnelValidationError::ExposureApprovalRequired);
        }
        Ok(())
    }

    /// Validates only settings needed to start one tunnel. It does not load
    /// credentials or make network calls. Runtime validation still checks port
    /// availability and SSH host-key trust.
    pub fn validate(
        &self,
        hosts: &[SshHost],
        groups: &[TunnelGroup],
    ) -> Result<(), TunnelValidationError> {
        if self.id.0.trim().is_empty() {
            return Err(TunnelValidationError::EmptyId);
        }
        if self.name.trim().is_empty() {
            return Err(TunnelValidationError::EmptyName);
        }
        if self.jump_chain.is_empty() {
            return Err(TunnelValidationError::EmptyJumpChain);
        }
        for (index, hop) in self.jump_chain.iter().enumerate() {
            if self.jump_chain[..index].contains(hop) {
                return Err(TunnelValidationError::RepeatedHop(hop.0.clone()));
            }
            match hosts.iter().filter(|host| host.id == *hop).take(2).count() {
                0 => return Err(TunnelValidationError::UnknownHost(hop.0.clone())),
                1 => {}
                _ => return Err(TunnelValidationError::AmbiguousHost(hop.0.clone())),
            }
        }
        if let Some(group_id) = &self.group_id {
            match groups
                .iter()
                .filter(|group| group.id == *group_id)
                .take(2)
                .count()
            {
                0 => return Err(TunnelValidationError::UnknownGroup(group_id.0.clone())),
                1 => {}
                _ => return Err(TunnelValidationError::AmbiguousGroup(group_id.0.clone())),
            }
        }
        if self.reconnect.base_delay.is_zero()
            || self.reconnect.max_delay < self.reconnect.base_delay
            || self.reconnect.reset_after_healthy.is_zero()
            || self.reconnect.jitter_percent > 100
        {
            return Err(TunnelValidationError::InvalidReconnectPolicy);
        }
        if self.local.host.trim().is_empty() || self.local.host.len() > 255 || self.local.port == 0
        {
            return Err(TunnelValidationError::InvalidLocalEndpoint);
        }
        match self.mode {
            TunnelMode::Local | TunnelMode::Dynamic => {
                if self.local.host.parse::<IpAddr>().is_err() {
                    return Err(TunnelValidationError::InvalidBindAddress);
                }
            }
            TunnelMode::Remote => {}
        }
        match (&self.mode, &self.remote) {
            (TunnelMode::Dynamic, None) => Ok(()),
            (TunnelMode::Dynamic, Some(_)) => Err(TunnelValidationError::UnexpectedRemoteEndpoint),
            (TunnelMode::Local | TunnelMode::Remote, Some(remote))
                if !remote.host.trim().is_empty()
                    && remote.host.len() <= 255
                    && (remote.port != 0 || self.mode == TunnelMode::Remote) =>
            {
                if self.mode == TunnelMode::Remote && remote.host.parse::<IpAddr>().is_err() {
                    return Err(TunnelValidationError::InvalidBindAddress);
                }
                Ok(())
            }
            _ => Err(TunnelValidationError::InvalidRemoteEndpoint),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthConfig, HostId, HostKeyPolicy, LocalEndpoint, RemoteEndpoint, RetryPolicy, SecretRef,
        TunnelId,
    };
    use std::time::Duration;

    fn host() -> SshHost {
        SshHost {
            id: HostId("target".into()),
            name: "target".into(),
            hostname: "target.example".into(),
            port: 22,
            username: "user".into(),
            auth: AuthConfig::Password {
                credential_ref: SecretRef("test".into()),
            },
            host_key_policy: HostKeyPolicy::Strict,
            connect_timeout: Duration::from_secs(5),
            keepalive_interval: Duration::from_secs(10),
            keepalive_max: 3,
            inactivity_timeout: None,
            notes: String::new(),
        }
    }

    fn tunnel(mode: TunnelMode) -> TunnelConfig {
        TunnelConfig {
            id: TunnelId("test".into()),
            name: "test".into(),
            group_id: None,
            mode,
            jump_chain: vec![HostId("target".into())],
            local: LocalEndpoint {
                host: "127.0.0.1".into(),
                port: 1080,
            },
            remote: None,
            auto_start: false,
            exposure_approved: false,
            reconnect: RetryPolicy::default(),
            description: String::new(),
        }
    }

    #[test]
    fn mode_endpoints_are_checked_before_start() {
        let host = host();
        let mut local = tunnel(TunnelMode::Local);
        assert_eq!(
            local.validate(std::slice::from_ref(&host), &[]),
            Err(TunnelValidationError::InvalidRemoteEndpoint)
        );
        local.remote = Some(RemoteEndpoint {
            host: "database.internal".into(),
            port: 5432,
        });
        assert!(local.validate(std::slice::from_ref(&host), &[]).is_ok());
        let mut dynamic = tunnel(TunnelMode::Dynamic);
        dynamic.remote = local.remote;
        assert_eq!(
            dynamic.validate(&[host], &[]),
            Err(TunnelValidationError::UnexpectedRemoteEndpoint)
        );
    }

    #[test]
    fn jump_chain_must_resolve_unambiguously() {
        let host = host();
        let mut config = tunnel(TunnelMode::Dynamic);
        config.jump_chain.push(HostId("target".into()));
        assert_eq!(
            config.validate(std::slice::from_ref(&host), &[]),
            Err(TunnelValidationError::RepeatedHop("target".into()))
        );
        config.jump_chain.pop();
        assert_eq!(
            config.validate(&[host.clone(), host], &[]),
            Err(TunnelValidationError::AmbiguousHost("target".into()))
        );
    }

    #[test]
    fn remote_mode_allows_server_assigned_port_and_local_hostname() {
        let mut config = tunnel(TunnelMode::Remote);
        config.local.host = "web.internal".into();
        config.local.port = 3000;
        config.remote = Some(RemoteEndpoint {
            host: "127.0.0.1".into(),
            port: 0,
        });
        assert!(config.validate(&[host()], &[]).is_ok());
    }

    #[test]
    fn non_loopback_listeners_require_explicit_consent() {
        for (address, exposed) in [
            ("127.0.0.1", false),
            ("127.0.0.2", false),
            ("192.168.1.10", true),
            ("0.0.0.0", true),
            ("::1", false),
            ("::", true),
            ("2001:db8::1", true),
        ] {
            let mut config = tunnel(TunnelMode::Dynamic);
            config.local.host = address.into();
            assert_eq!(
                config.validate_exposure().is_err(),
                exposed,
                "address {address}"
            );
            config.exposure_approved = true;
            assert!(config.validate_exposure().is_ok());
        }
        let mut remote = tunnel(TunnelMode::Remote);
        remote.remote = Some(RemoteEndpoint {
            host: "::".into(),
            port: 0,
        });
        assert_eq!(
            remote.validate_exposure(),
            Err(TunnelValidationError::ExposureApprovalRequired)
        );
    }
}
