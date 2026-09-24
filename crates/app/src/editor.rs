use std::{path::PathBuf, time::Duration};

use gpui_kit::component::input::InputState;
use gpui_kit::{AppContext as _, Context, Entity, Window};
use tunnel_domain::{
    AuthConfig, GroupId, HostId, HostKeyPolicy, LocalEndpoint, RemoteEndpoint, RetryPolicy,
    SshHost, TunnelConfig, TunnelId, TunnelMode,
};

pub struct HostEditor {
    pub id: HostId,
    pub name: Entity<InputState>,
    pub hostname: Entity<InputState>,
    pub port: Entity<InputState>,
    pub username: Entity<InputState>,
    pub identity_file: Entity<InputState>,
    pub notes: Entity<InputState>,
    pub auth: AuthConfig,
    pub policy: HostKeyPolicy,
    pub connect_timeout: Duration,
    pub keepalive_interval: Duration,
    pub keepalive_max: u32,
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
            notes: text(existing.map_or("", |host| &host.notes), window, cx),
            auth: existing.map_or(AuthConfig::Agent { socket: None }, |host| host.auth.clone()),
            policy: existing.map_or(HostKeyPolicy::Strict, |host| host.host_key_policy),
            connect_timeout: existing.map_or(Duration::from_secs(8), |host| host.connect_timeout),
            keepalive_interval: existing
                .map_or(Duration::from_secs(10), |host| host.keepalive_interval),
            keepalive_max: existing.map_or(3, |host| host.keepalive_max),
        }
    }

    pub fn collect(&self, cx: &Context<crate::workspace::Workspace>) -> Result<SshHost, String> {
        let port = self
            .port
            .read(cx)
            .value()
            .parse::<u16>()
            .map_err(|_| "SSH port must be between 1 and 65535")?;
        let auth = match &self.auth {
            AuthConfig::PrivateKey { passphrase_ref, .. } => AuthConfig::PrivateKey {
                key_path: PathBuf::from(self.identity_file.read(cx).value().to_string()),
                passphrase_ref: passphrase_ref.clone(),
            },
            other => other.clone(),
        };
        Ok(SshHost {
            id: self.id.clone(),
            name: self.name.read(cx).value().to_string().trim().to_owned(),
            hostname: self.hostname.read(cx).value().to_string().trim().to_owned(),
            port,
            username: self.username.read(cx).value().to_string().trim().to_owned(),
            auth,
            host_key_policy: self.policy,
            connect_timeout: self.connect_timeout,
            keepalive_interval: self.keepalive_interval,
            keepalive_max: self.keepalive_max,
            notes: self.notes.read(cx).value().to_string(),
        })
    }
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
                    .map_or(1080, |tunnel| tunnel.local.port)
                    .to_string(),
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
            .map_err(|_| "Local port must be between 0 and 65535")?;
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
            reconnect: self.reconnect.clone(),
            description: self.description.read(cx).value().to_string(),
        })
    }
}

pub enum Editor {
    Host(HostEditor),
    Tunnel(TunnelEditor),
}
