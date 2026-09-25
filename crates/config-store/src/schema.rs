use std::{collections::HashSet, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tunnel_domain::{
    AuthConfig, GroupId, HostId, HostKeyPolicy, LocalEndpoint, RemoteEndpoint, RetryPolicy,
    SecretRef, SshHost, TunnelConfig, TunnelGroup, TunnelId, TunnelMode,
};

pub const SCHEMA_VERSION: u32 = 1;
const MAX_HOSTS: usize = 512;
const MAX_GROUPS: usize = 128;
const MAX_TUNNELS: usize = 1024;
const MAX_JUMP_HOPS: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppSettings {
    pub theme: ThemePreference,
    pub run_at_startup: bool,
    pub minimize_to_tray: bool,
    pub traffic_monitor: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            run_at_startup: false,
            minimize_to_tray: true,
            traffic_monitor: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthType {
    Password,
    PrivateKey,
    Agent,
    KeyboardInteractive,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRecord {
    pub id: String,
    pub name: String,
    pub hostname: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_socket: Option<String>,
    #[serde(default)]
    pub host_key_policy: HostKeyPolicy,
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,
    #[serde(default = "default_keepalive_interval_ms")]
    pub keepalive_interval_ms: u64,
    #[serde(default = "default_keepalive_max")]
    pub keepalive_max: u32,
    #[serde(default)]
    pub notes: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupRecord {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub sort_order: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelRecord {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    pub mode: TunnelMode,
    pub jump_chain: Vec<String>,
    pub local_host: String,
    pub local_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_port: Option<u16>,
    #[serde(default)]
    pub auto_start: bool,
    #[serde(default)]
    pub exposure_approved: bool,
    #[serde(default = "default_reconnect_base_ms")]
    pub reconnect_base_ms: u64,
    #[serde(default = "default_reconnect_max_ms")]
    pub reconnect_max_ms: u64,
    #[serde(default = "default_reconnect_reset_ms")]
    pub reconnect_reset_ms: u64,
    #[serde(default = "default_reconnect_jitter_percent")]
    pub reconnect_jitter_percent: u8,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigDocument {
    pub schema_version: u32,
    #[serde(default)]
    pub app: AppSettings,
    #[serde(default)]
    pub hosts: Vec<HostRecord>,
    #[serde(default)]
    pub groups: Vec<GroupRecord>,
    #[serde(default)]
    pub tunnels: Vec<TunnelRecord>,
}

impl Default for ConfigDocument {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            app: AppSettings::default(),
            hosts: Vec::new(),
            groups: Vec::new(),
            tunnels: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct DomainConfig {
    pub app: AppSettings,
    pub hosts: Vec<SshHost>,
    pub groups: Vec<TunnelGroup>,
    pub tunnels: Vec<TunnelConfig>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigValidationError {
    #[error("unsupported config schema version {0}")]
    UnsupportedVersion(u32),
    #[error("too many {0} entries")]
    TooManyEntries(&'static str),
    #[error("duplicate {kind} ID {id}")]
    DuplicateId { kind: &'static str, id: String },
    #[error("invalid host {id}: {reason}")]
    Host { id: String, reason: &'static str },
    #[error("invalid tunnel {id}: {reason}")]
    Tunnel { id: String, reason: String },
    #[error("duration in configuration exceeds supported millisecond range")]
    DurationTooLarge,
}

impl ConfigDocument {
    /// Consumes the versioned file model and builds the runtime domain model.
    /// All counts are capped before allocating conversion collections.
    pub fn into_domain(self) -> Result<DomainConfig, ConfigValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ConfigValidationError::UnsupportedVersion(
                self.schema_version,
            ));
        }
        if self.hosts.len() > MAX_HOSTS {
            return Err(ConfigValidationError::TooManyEntries("hosts"));
        }
        if self.groups.len() > MAX_GROUPS {
            return Err(ConfigValidationError::TooManyEntries("groups"));
        }
        if self.tunnels.len() > MAX_TUNNELS {
            return Err(ConfigValidationError::TooManyEntries("tunnels"));
        }
        let mut host_ids = HashSet::with_capacity(self.hosts.len());
        let mut hosts = Vec::with_capacity(self.hosts.len());
        for record in self.hosts {
            if !host_ids.insert(record.id.clone()) {
                return Err(ConfigValidationError::DuplicateId {
                    kind: "host",
                    id: record.id,
                });
            }
            hosts.push(record.into_domain()?);
        }
        let mut group_ids = HashSet::with_capacity(self.groups.len());
        let mut groups = Vec::with_capacity(self.groups.len());
        for record in self.groups {
            if !group_ids.insert(record.id.clone()) {
                return Err(ConfigValidationError::DuplicateId {
                    kind: "group",
                    id: record.id,
                });
            }
            groups.push(TunnelGroup {
                id: GroupId(record.id),
                name: record.name,
                sort_order: record.sort_order,
            });
        }
        let mut tunnel_ids = HashSet::with_capacity(self.tunnels.len());
        let mut tunnels = Vec::with_capacity(self.tunnels.len());
        for record in self.tunnels {
            if !tunnel_ids.insert(record.id.clone()) {
                return Err(ConfigValidationError::DuplicateId {
                    kind: "tunnel",
                    id: record.id,
                });
            }
            let tunnel = record.into_domain()?;
            tunnel
                .validate(&hosts, &groups)
                .map_err(|error| ConfigValidationError::Tunnel {
                    id: tunnel.id.0.clone(),
                    reason: error.to_string(),
                })?;
            tunnels.push(tunnel);
        }
        Ok(DomainConfig {
            app: self.app,
            hosts,
            groups,
            tunnels,
        })
    }
}

impl HostRecord {
    fn into_domain(self) -> Result<SshHost, ConfigValidationError> {
        let bad = |reason| ConfigValidationError::Host {
            id: self.id.clone(),
            reason,
        };
        if self.id.trim().is_empty()
            || self.name.trim().is_empty()
            || self.hostname.trim().is_empty()
            || self.username.trim().is_empty()
            || self.port == 0
            || self.connect_timeout_ms == 0
            || self.keepalive_max == 0
        {
            return Err(bad("required field is empty or zero"));
        }
        let auth = match self.auth_type {
            AuthType::Password => AuthConfig::Password {
                credential_ref: SecretRef(
                    self.credential_ref
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| bad("password credential_ref is missing"))?,
                ),
            },
            AuthType::PrivateKey => AuthConfig::PrivateKey {
                key_path: expand_home(
                    self.identity_file
                        .ok_or_else(|| bad("identity_file is missing"))?,
                )
                .ok_or_else(|| bad("home directory is unavailable"))?,
                passphrase_ref: self.passphrase_ref.map(SecretRef),
            },
            AuthType::Agent => AuthConfig::Agent {
                socket: self.agent_socket,
            },
            AuthType::KeyboardInteractive => AuthConfig::KeyboardInteractive,
        };
        Ok(SshHost {
            id: HostId(self.id),
            name: self.name,
            hostname: self.hostname,
            port: self.port,
            username: self.username,
            auth,
            host_key_policy: self.host_key_policy,
            connect_timeout: Duration::from_millis(self.connect_timeout_ms),
            keepalive_interval: Duration::from_millis(self.keepalive_interval_ms),
            keepalive_max: self.keepalive_max,
            notes: self.notes,
        })
    }
}

impl TunnelRecord {
    fn into_domain(self) -> Result<TunnelConfig, ConfigValidationError> {
        if self.jump_chain.len() > MAX_JUMP_HOPS {
            return Err(ConfigValidationError::Tunnel {
                id: self.id,
                reason: "jump chain exceeds 16 hosts".into(),
            });
        }
        let remote = match (self.remote_host, self.remote_port) {
            (Some(host), Some(port)) => Some(RemoteEndpoint { host, port }),
            (None, None) => None,
            _ => {
                return Err(ConfigValidationError::Tunnel {
                    id: self.id,
                    reason: "remote_host and remote_port must be set together".into(),
                });
            }
        };
        Ok(TunnelConfig {
            id: TunnelId(self.id),
            name: self.name,
            group_id: self.group_id.map(GroupId),
            mode: self.mode,
            jump_chain: self.jump_chain.into_iter().map(HostId).collect(),
            local: LocalEndpoint {
                host: self.local_host,
                port: self.local_port,
            },
            remote,
            auto_start: self.auto_start,
            exposure_approved: self.exposure_approved,
            reconnect: RetryPolicy {
                base_delay: Duration::from_millis(self.reconnect_base_ms),
                max_delay: Duration::from_millis(self.reconnect_max_ms),
                reset_after_healthy: Duration::from_millis(self.reconnect_reset_ms),
                jitter_percent: self.reconnect_jitter_percent,
            },
            description: self.description,
        })
    }
}

const fn default_ssh_port() -> u16 {
    22
}
const fn default_connect_timeout_ms() -> u64 {
    8000
}
const fn default_keepalive_interval_ms() -> u64 {
    10000
}
const fn default_keepalive_max() -> u32 {
    3
}
const fn default_reconnect_base_ms() -> u64 {
    1000
}
const fn default_reconnect_max_ms() -> u64 {
    30000
}
const fn default_reconnect_reset_ms() -> u64 {
    60000
}
const fn default_reconnect_jitter_percent() -> u8 {
    20
}

fn expand_home(path: PathBuf) -> Option<PathBuf> {
    let Ok(remainder) = path.strip_prefix("~") else {
        return Some(path);
    };
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE")
    } else {
        std::env::var_os("HOME")
    }?;
    Some(PathBuf::from(home).join(remainder))
}

impl TryFrom<DomainConfig> for ConfigDocument {
    type Error = ConfigValidationError;

    fn try_from(value: DomainConfig) -> Result<Self, Self::Error> {
        let hosts = value
            .hosts
            .into_iter()
            .map(HostRecord::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        let groups = value
            .groups
            .into_iter()
            .map(|group| GroupRecord {
                id: group.id.0,
                name: group.name,
                sort_order: group.sort_order,
            })
            .collect();
        let tunnels = value
            .tunnels
            .into_iter()
            .map(TunnelRecord::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            app: value.app,
            hosts,
            groups,
            tunnels,
        })
    }
}

impl TryFrom<SshHost> for HostRecord {
    type Error = ConfigValidationError;

    fn try_from(host: SshHost) -> Result<Self, Self::Error> {
        let (auth_type, credential_ref, identity_file, passphrase_ref, agent_socket) =
            match host.auth {
                AuthConfig::Password { credential_ref } => {
                    (AuthType::Password, Some(credential_ref.0), None, None, None)
                }
                AuthConfig::PrivateKey {
                    key_path,
                    passphrase_ref,
                } => (
                    AuthType::PrivateKey,
                    None,
                    Some(key_path),
                    passphrase_ref.map(|value| value.0),
                    None,
                ),
                AuthConfig::Agent { socket } => (AuthType::Agent, None, None, None, socket),
                AuthConfig::KeyboardInteractive => {
                    (AuthType::KeyboardInteractive, None, None, None, None)
                }
            };
        Ok(Self {
            id: host.id.0,
            name: host.name,
            hostname: host.hostname,
            port: host.port,
            username: host.username,
            auth_type,
            credential_ref,
            identity_file,
            passphrase_ref,
            agent_socket,
            host_key_policy: host.host_key_policy,
            connect_timeout_ms: milliseconds(host.connect_timeout)?,
            keepalive_interval_ms: milliseconds(host.keepalive_interval)?,
            keepalive_max: host.keepalive_max,
            notes: host.notes,
        })
    }
}

impl TryFrom<TunnelConfig> for TunnelRecord {
    type Error = ConfigValidationError;

    fn try_from(tunnel: TunnelConfig) -> Result<Self, Self::Error> {
        let (remote_host, remote_port) = match tunnel.remote {
            Some(remote) => (Some(remote.host), Some(remote.port)),
            None => (None, None),
        };
        Ok(Self {
            id: tunnel.id.0,
            name: tunnel.name,
            group_id: tunnel.group_id.map(|value| value.0),
            mode: tunnel.mode,
            jump_chain: tunnel.jump_chain.into_iter().map(|value| value.0).collect(),
            local_host: tunnel.local.host,
            local_port: tunnel.local.port,
            remote_host,
            remote_port,
            auto_start: tunnel.auto_start,
            exposure_approved: tunnel.exposure_approved,
            reconnect_base_ms: milliseconds(tunnel.reconnect.base_delay)?,
            reconnect_max_ms: milliseconds(tunnel.reconnect.max_delay)?,
            reconnect_reset_ms: milliseconds(tunnel.reconnect.reset_after_healthy)?,
            reconnect_jitter_percent: tunnel.reconnect.jitter_percent,
            description: tunnel.description,
        })
    }
}

fn milliseconds(duration: Duration) -> Result<u64, ConfigValidationError> {
    u64::try_from(duration.as_millis()).map_err(|_| ConfigValidationError::DurationTooLarge)
}
