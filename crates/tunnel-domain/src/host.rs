use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HostId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretRef(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthConfig {
    Password {
        credential_ref: SecretRef,
    },
    PrivateKey {
        key_path: PathBuf,
        passphrase_ref: Option<SecretRef>,
    },
    Agent {
        socket: Option<String>,
    },
    KeyboardInteractive,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostKeyPolicy {
    #[default]
    Strict,
    Bypass,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshHost {
    pub id: HostId,
    pub name: String,
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthConfig,
    pub host_key_policy: HostKeyPolicy,
    pub connect_timeout: Duration,
    pub keepalive_interval: Duration,
    pub keepalive_max: u32,
    pub notes: String,
}
