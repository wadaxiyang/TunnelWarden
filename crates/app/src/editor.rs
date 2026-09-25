use std::{net::IpAddr, path::PathBuf, time::Duration};

use gpui_kit::component::input::InputState;
use gpui_kit::{AppContext as _, Context, Entity, Window};
use tunnel_core::SecretUpdate;
use tunnel_domain::{
    AuthConfig, GroupId, HostId, HostKeyPolicy, LocalEndpoint, RemoteEndpoint, RetryPolicy,
    SecretRef, SshHost, TunnelConfig, TunnelId, TunnelMode,
};
use zeroize::Zeroizing;

pub struct HostEditor {
    pub id: HostId,
    pub name: Entity<InputState>,
    pub hostname: Entity<InputState>,
    pub port: Entity<InputState>,
    pub username: Entity<InputState>,
    pub identity_file: Entity<InputState>,
    pub agent_socket: Entity<InputState>,
    pub password: Entity<InputState>,
    pub passphrase: Entity<InputState>,
    pub notes: Entity<InputState>,
    pub connect_timeout_ms: Entity<InputState>,
    pub keepalive_interval_ms: Entity<InputState>,
    pub keepalive_max: Entity<InputState>,
    pub auth: AuthConfig,
    pub original_auth: AuthConfig,
    pub policy: HostKeyPolicy,
}

impl HostEditor {
    pub fn new(
        id: HostId,
        existing: Option<&SshHost>,
        window: &mut Window,
        cx: &mut Context<crate::workspace::Workspace>,
    ) -> Self {
        let text =
            |value: &str, window: &mut Window, cx: &mut Context<crate::workspace::Workspace>| {
                cx.new(|cx| InputState::new(window, cx).default_value(value))
            };
        Self {
            id,
            name: text(existing.map_or("", |host| &host.name), window, cx),
            hostname: text(existing.map_or("", |host| &host.hostname), window, cx),
            port: text(
                &existing.map_or(22, |host| host.port).to_string(),
                window,
                cx,
            ),
            username: text(existing.map_or("", |host| &host.username), window, cx),
            identity_file: text(
                existing
                    .and_then(|host| match &host.auth {
                        AuthConfig::PrivateKey { key_path, .. } => key_path.to_str(),
                        _ => None,
                    })
                    .unwrap_or(""),
                window,
                cx,
            ),
            agent_socket: text(
                existing
                    .and_then(|host| match &host.auth {
                        AuthConfig::Agent { socket } => socket.as_deref(),
                        _ => None,
                    })
                    .unwrap_or(""),
                window,
                cx,
            ),
            password: text("", window, cx),
            passphrase: text("", window, cx),
            notes: text(existing.map_or("", |host| &host.notes), window, cx),
            connect_timeout_ms: text(
                &existing
                    .map_or(Duration::from_secs(8), |host| host.connect_timeout)
                    .as_millis()
                    .to_string(),
                window,
                cx,
            ),
            keepalive_interval_ms: text(
                &existing
                    .map_or(Duration::from_secs(10), |host| host.keepalive_interval)
                    .as_millis()
                    .to_string(),
                window,
                cx,
            ),
            keepalive_max: text(
                &existing.map_or(3, |host| host.keepalive_max).to_string(),
                window,
                cx,
            ),
            auth: existing.map_or(AuthConfig::Agent { socket: None }, |host| host.auth.clone()),
            original_auth: existing
                .map_or(AuthConfig::Agent { socket: None }, |host| host.auth.clone()),
            policy: existing.map_or(HostKeyPolicy::Strict, |host| host.host_key_policy),
        }
    }

    pub fn collect(
        &self,
        cx: &Context<crate::workspace::Workspace>,
    ) -> Result<(SshHost, Option<SecretUpdate>), String> {
        let port = self
            .port
            .read(cx)
            .value()
            .parse::<u16>()
            .map_err(|_| "SSH port must be between 1 and 65535")?;
        let mut secret_update = None;
        let auth = match &self.auth {
            AuthConfig::Password { credential_ref } => {
                let password = self.password.read(cx).value().to_string();
                if password.is_empty() && credential_ref.0 == "pending" {
                    return Err("Enter a password before saving".into());
                }
                let reference = if password.is_empty() {
                    credential_ref.clone()
                } else {
                    let reference = new_secret_ref();
                    secret_update = Some(SecretUpdate {
                        reference: reference.clone(),
                        value: Zeroizing::new(password),
                    });
                    reference
                };
                AuthConfig::Password {
                    credential_ref: reference,
                }
            }
            AuthConfig::PrivateKey { passphrase_ref, .. } => AuthConfig::PrivateKey {
                key_path: {
                    let path = self.identity_file.read(cx).value().to_string();
                    if path.trim().is_empty() {
                        return Err("Choose a private key file".into());
                    }
                    PathBuf::from(path)
                },
                passphrase_ref: {
                    let passphrase = self.passphrase.read(cx).value().to_string();
                    if passphrase.is_empty() {
                        passphrase_ref.clone()
                    } else {
                        let reference = new_secret_ref();
                        secret_update = Some(SecretUpdate {
                            reference: reference.clone(),
                            value: Zeroizing::new(passphrase),
                        });
                        Some(reference)
                    }
                },
            },
            AuthConfig::Agent { .. } => {
                let socket = self
                    .agent_socket
                    .read(cx)
                    .value()
                    .to_string()
                    .trim()
                    .to_owned();
                AuthConfig::Agent {
                    socket: if socket.is_empty() {
                        None
                    } else {
                        Some(socket)
                    },
                }
            }
            other => other.clone(),
        };
        let connect_timeout_ms = self
            .connect_timeout_ms
            .read(cx)
            .value()
            .parse::<u64>()
            .map_err(|_| "Connect timeout must be a number of milliseconds")?;
        let keepalive_interval_ms = self
            .keepalive_interval_ms
            .read(cx)
            .value()
            .parse::<u64>()
            .map_err(|_| "Keepalive interval must be a number of milliseconds")?;
        let keepalive_max = self
            .keepalive_max
            .read(cx)
            .value()
            .parse::<u32>()
            .map_err(|_| "Keepalive max must be a number")?;
        Ok((
            SshHost {
                id: self.id.clone(),
                name: self.name.read(cx).value().to_string().trim().to_owned(),
                hostname: self.hostname.read(cx).value().to_string().trim().to_owned(),
                port,
                username: self.username.read(cx).value().to_string().trim().to_owned(),
                auth,
                host_key_policy: self.policy,
                connect_timeout: Duration::from_millis(connect_timeout_ms),
                keepalive_interval: Duration::from_millis(keepalive_interval_ms),
                keepalive_max,
                notes: self.notes.read(cx).value().to_string(),
            },
            secret_update,
        ))
    }
}

fn new_secret_ref() -> SecretRef {
    SecretRef(format!("credential-{:032x}", rand::random::<u128>()))
}

pub struct TunnelEditor {
    pub id: TunnelId,
    pub name: Entity<InputState>,
    pub local_host: Entity<InputState>,
    pub local_port: Entity<InputState>,
    pub remote_host: Entity<InputState>,
    pub remote_port: Entity<InputState>,
    pub description: Entity<InputState>,
    pub mode: TunnelMode,
    pub jump_chain: Vec<HostId>,
    pub auto_start: bool,
    pub exposure_approved_for: Option<(TunnelMode, IpAddr)>,
    pub reconnect: RetryPolicy,
    pub group_id: Option<GroupId>,
}

impl TunnelEditor {
    pub fn new(
        id: TunnelId,
        existing: Option<&TunnelConfig>,
        window: &mut Window,
        cx: &mut Context<crate::workspace::Workspace>,
    ) -> Self {
        let text =
            |value: &str, window: &mut Window, cx: &mut Context<crate::workspace::Workspace>| {
                cx.new(|cx| InputState::new(window, cx).default_value(value))
            };
        Self {
            id,
            name: text(existing.map_or("", |tunnel| &tunnel.name), window, cx),
            local_host: text(
                existing.map_or("127.0.0.1", |tunnel| &tunnel.local.host),
                window,
                cx,
            ),
            local_port: text(
                &existing
                    .map(|tunnel| tunnel.local.port.to_string())
                    .unwrap_or_default(),
                window,
                cx,
            ),
            remote_host: text(
                existing
                    .and_then(|tunnel| tunnel.remote.as_ref())
                    .map_or("", |remote| &remote.host),
                window,
                cx,
            ),
            remote_port: text(
                &existing
                    .and_then(|tunnel| tunnel.remote.as_ref())
                    .map_or(String::new(), |remote| remote.port.to_string()),
                window,
                cx,
            ),
            description: text(
                existing.map_or("", |tunnel| &tunnel.description),
                window,
                cx,
            ),
            mode: existing.map_or(TunnelMode::Dynamic, |tunnel| tunnel.mode),
            jump_chain: existing.map_or_else(Vec::new, |tunnel| tunnel.jump_chain.clone()),
            auto_start: existing.is_some_and(|tunnel| tunnel.auto_start),
            exposure_approved_for: existing.and_then(|tunnel| {
                if !tunnel.exposure_approved {
                    return None;
                }
                let host = match tunnel.mode {
                    TunnelMode::Local | TunnelMode::Dynamic => &tunnel.local.host,
                    TunnelMode::Remote => &tunnel.remote.as_ref()?.host,
                };
                host.parse().ok().map(|address| (tunnel.mode, address))
            }),
            reconnect: existing
                .map_or_else(RetryPolicy::default, |tunnel| tunnel.reconnect.clone()),
            group_id: existing.and_then(|tunnel| tunnel.group_id.clone()),
        }
    }

    pub fn collect(
        &self,
        cx: &Context<crate::workspace::Workspace>,
    ) -> Result<TunnelConfig, String> {
        let local_port = self
            .local_port
            .read(cx)
            .value()
            .parse::<u16>()
            .map_err(|_| "Enter a listen port between 1 and 65535")?;
        let remote = if self.mode == TunnelMode::Dynamic {
            None
        } else {
            Some(RemoteEndpoint {
                host: self
                    .remote_host
                    .read(cx)
                    .value()
                    .to_string()
                    .trim()
                    .to_owned(),
                port: self
                    .remote_port
                    .read(cx)
                    .value()
                    .parse::<u16>()
                    .map_err(|_| "Destination port must be between 1 and 65535")?,
            })
        };
        let exposure_approved = self
            .exposure_scope(cx)
            .is_some_and(|scope| self.exposure_approved_for == Some(scope));
        Ok(TunnelConfig {
            id: self.id.clone(),
            name: self.name.read(cx).value().to_string().trim().to_owned(),
            group_id: self.group_id.clone(),
            mode: self.mode,
            jump_chain: self.jump_chain.clone(),
            local: LocalEndpoint {
                host: self
                    .local_host
                    .read(cx)
                    .value()
                    .to_string()
                    .trim()
                    .to_owned(),
                port: local_port,
            },
            remote,
            auto_start: self.auto_start,
            exposure_approved,
            reconnect: self.reconnect.clone(),
            description: self.description.read(cx).value().to_string(),
        })
    }

    pub fn exposure_scope(
        &self,
        cx: &Context<crate::workspace::Workspace>,
    ) -> Option<(TunnelMode, IpAddr)> {
        let state = match self.mode {
            TunnelMode::Local | TunnelMode::Dynamic => &self.local_host,
            TunnelMode::Remote => &self.remote_host,
        };
        state
            .read(cx)
            .value()
            .parse::<IpAddr>()
            .ok()
            .filter(|address| !address.is_loopback())
            .map(|address| (self.mode, address))
    }
}

pub enum Editor {
    Host(Box<HostEditor>),
    Tunnel(Box<TunnelEditor>),
}
