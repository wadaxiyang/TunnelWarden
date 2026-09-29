// Copyright (C) 2026 Wilson Glasser and Oryxis contributors
// SPDX-License-Identifier: AGPL-3.0-or-later
// Modified 2026-09-29: TunnelWarden configuration/keyring and prompt bridge.
// Source: crates/oryxis-app/src/connect_methods.rs (make_jump_resolver,
// expand_jump_chain, make_host_key_check), dispatch_port_forwards.rs (dial).
// Oryxis 16ad2af83b424780ff60b6420a6c20a3b30c7143.
use crate::{
    credentials::SecretStore,
    dto::HostKeyPrompt,
    runtime::{DialFuture, ForwardDialer},
    storage::{Config, ConfigStore},
};
use oryxis_core::models::{
    connection::{AuthMethod, Connection},
    known_host::KnownHost,
};
use oryxis_ssh::{
    ConnectionResolver, HostKeyCheckCallback, HostKeyQuery, HostKeyStatus, KeyMaterial, SshEngine,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;

struct Pending {
    prompt: HostKeyPrompt,
    response: oneshot::Sender<bool>,
}
#[derive(Default)]
struct PromptState {
    pending: HashMap<Uuid, Pending>,
    cancelled: HashSet<Uuid>,
}
pub struct PromptBroker {
    store: Arc<ConfigStore>,
    pending: Mutex<PromptState>,
    updates: watch::Sender<Vec<HostKeyPrompt>>,
}
impl PromptBroker {
    pub fn new(store: Arc<ConfigStore>) -> Self {
        let (updates, _) = watch::channel(vec![]);
        Self {
            store,
            pending: Mutex::new(PromptState::default()),
            updates,
        }
    }
    pub fn subscribe(&self) -> watch::Receiver<Vec<HostKeyPrompt>> {
        self.updates.subscribe()
    }
    pub fn snapshot(&self) -> Vec<HostKeyPrompt> {
        self.updates.borrow().clone()
    }
    fn publish(&self, state: &PromptState) {
        let mut list: Vec<_> = state.pending.values().map(|p| p.prompt.clone()).collect();
        list.sort_by_key(|p| p.id);
        self.updates.send_replace(list);
    }
    fn ask(&self, host_id: Uuid, q: HostKeyQuery, response: oneshot::Sender<bool>) {
        let prompt = HostKeyPrompt {
            id: Uuid::new_v4(),
            host_id,
            hostname: q.hostname,
            port: q.port,
            key_type: q.key_type,
            fingerprint: q.fingerprint,
            old_fingerprint: match q.status {
                HostKeyStatus::Changed { old_fingerprint } => Some(old_fingerprint),
                _ => None,
            },
        };
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        if pending.cancelled.contains(&host_id) {
            let _ = response.send(false);
            return;
        }
        pending
            .pending
            .insert(prompt.id, Pending { prompt, response });
        self.publish(&pending);
    }
    pub fn respond(&self, id: Uuid, trust: bool) -> Result<(), String> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "Host-key prompt lock poisoned")?;
        let p = pending
            .pending
            .get(&id)
            .ok_or("Host-key question expired")?;
        if trust {
            let q = &p.prompt;
            self.store.update(|c| {
                c.known_hosts.retain(|k| {
                    !(k.hostname == q.hostname && k.port == q.port && k.key_type == q.key_type)
                });
                c.known_hosts.push(KnownHost::new(
                    &q.hostname,
                    q.port,
                    &q.key_type,
                    &q.fingerprint,
                ));
                Ok(())
            })?;
        }
        if let Some(p) = pending.pending.remove(&id) {
            let _ = p.response.send(trust);
        }
        self.publish(&pending);
        Ok(())
    }
    pub fn cancel_for_host(&self, host: Uuid) {
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        pending.cancelled.insert(host);
        pending.pending.retain(|_, p| p.prompt.host_id != host);
        self.publish(&pending);
    }
    fn begin(&self, host: Uuid) {
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cancelled
            .remove(&host);
    }
}

pub struct ApplicationDialer {
    pub store: Arc<ConfigStore>,
    pub secrets: Arc<dyn SecretStore>,
    pub prompts: Arc<PromptBroker>,
}
struct PromptGuard {
    task: tokio::task::JoinHandle<()>,
    broker: Arc<PromptBroker>,
    host: Uuid,
}
impl Drop for PromptGuard {
    fn drop(&mut self) {
        self.task.abort();
        self.broker.cancel_for_host(self.host);
    }
}

impl ForwardDialer for ApplicationDialer {
    fn connect(&self, host_id: Uuid, unattended: bool) -> DialFuture {
        let config = self.store.snapshot();
        let secrets = self.secrets.clone();
        let prompts = self.prompts.clone();
        prompts.begin(host_id);
        Box::pin(async move {
            let (mut conn, password, key, resolver) =
                tokio::task::spawn_blocking(move || prepare(&config, secrets.as_ref(), host_id))
                    .await
                    .map_err(|e| e.to_string())??;
            let mut engine = SshEngine::new()
                .with_strict_host_key(true)
                .with_host_key_check(host_key_check(prompts.store.snapshot().known_hosts))
                .with_keepalive(
                    conn.keepalive_interval
                        .filter(|s| *s > 0)
                        .map(|s| Duration::from_secs(s.into())),
                );
            let _bridge = if unattended {
                None
            } else {
                let (tx, mut rx) = mpsc::channel::<(HostKeyQuery, oneshot::Sender<bool>)>(8);
                engine = engine.with_host_key_ask(tx);
                let broker = prompts.clone();
                Some(PromptGuard {
                    task: tokio::spawn(async move {
                        while let Some((query, response)) = rx.recv().await {
                            broker.ask(host_id, query, response);
                        }
                    }),
                    broker: prompts,
                    host: host_id,
                })
            };
            // No group/proxy settings are exposed by TunnelWarden; the copied
            // resolver supplies each hop's own username, port and credentials.
            conn.proxy = None;
            tokio::time::timeout(Duration::from_secs(180), engine.connect_forward_conn(&conn, password.as_deref(), key.as_deref().map(KeyMaterial::plain), resolver.as_ref()))
                .await.map_err(|_| "Connection timed out (including host-key confirmation)".to_string())?
                .map_err(|e| format!("{e}. For first connection or a changed host key, start manually and verify the fingerprint."))
        })
    }
    fn cancel_prompts(&self, host_id: Uuid) {
        self.prompts.cancel_for_host(host_id);
    }
}

type Prepared = (
    Connection,
    Option<String>,
    Option<String>,
    Option<ConnectionResolver>,
);
fn prepare(config: &Config, secrets: &dyn SecretStore, id: Uuid) -> Result<Prepared, String> {
    let mut conn = config
        .hosts
        .iter()
        .find(|c| c.id == id)
        .cloned()
        .ok_or("SSH host not found")?;
    let (password, key) = resolve_credentials(&conn, secrets)?;
    // Expand nested hop routes on the working copy only, using the public API.
    conn.jump_chain =
        oryxis_core::jump_route::expanded_jump_chain(conn.id, &conn.jump_chain, &config.hosts);
    let resolver = if conn.jump_chain.is_empty() {
        None
    } else {
        let mut connections = Vec::with_capacity(conn.jump_chain.len());
        let mut passwords = HashMap::new();
        let mut keys = HashMap::new();
        for jid in &conn.jump_chain {
            let Some(hop) = config.hosts.iter().find(|c| c.id == *jid) else {
                continue;
            };
            let (pw, pk) = resolve_credentials(hop, secrets)?;
            if let Some(pw) = pw {
                passwords.insert(*jid, pw);
            }
            if let Some(pk) = pk {
                keys.insert(*jid, pk);
            }
            connections.push(hop.clone());
        }
        Some(ConnectionResolver {
            connections,
            passwords,
            private_keys: keys,
            certificates: HashMap::new(),
            proxies: HashMap::new(),
            totp_secrets: HashMap::new(),
        })
    };
    Ok((conn, password, key, resolver))
}

fn resolve_credentials(
    conn: &Connection,
    secrets: &dyn SecretStore,
) -> Result<(Option<String>, Option<String>), String> {
    let password = if conn.auth_method == AuthMethod::Agent {
        None
    } else {
        secrets.get(conn.id)?
    };
    let key = if matches!(conn.auth_method, AuthMethod::Key | AuthMethod::Auto) {
        conn.identity_file
            .as_ref()
            .map(|path| {
                let path = if let Some(rest) =
                    path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"))
                {
                    dirs::home_dir()
                        .ok_or("Home directory unavailable".to_string())?
                        .join(rest)
                } else {
                    PathBuf::from(path)
                };
                std::fs::read_to_string(&path)
                    .map_err(|e| format!("Cannot read private key {}: {e}", path.display()))
            })
            .transpose()?
    } else {
        None
    };
    Ok((password, key))
}

pub fn host_key_check(snapshot: Vec<KnownHost>) -> HostKeyCheckCallback {
    Arc::new(move |host, port, key_type, fingerprint| {
        // Per (host, port, key_type): another algorithm is Unknown, not Changed.
        if let Some(existing) = snapshot
            .iter()
            .find(|h| h.hostname == host && h.port == port && h.key_type == key_type)
        {
            if existing.fingerprint != fingerprint {
                return HostKeyStatus::Changed {
                    old_fingerprint: existing.fingerprint.clone(),
                };
            }
            return HostKeyStatus::Known;
        }
        HostKeyStatus::Unknown
    })
}
