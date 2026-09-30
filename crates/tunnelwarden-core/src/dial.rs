// Copyright (C) 2026 Wilson Glasser and Oryxis contributors
// SPDX-License-Identifier: AGPL-3.0-or-later
// Modified 2026-09-30: scoped cancellable tests and in-memory key unlock.
// Modified 2026-09-29: TunnelWarden configuration/keyring and prompt bridge.
// Source: crates/oryxis-app/src/connect_methods.rs (make_jump_resolver,
// expand_jump_chain, make_host_key_check), dispatch_port_forwards.rs (dial).
// Oryxis 16ad2af83b424780ff60b6420a6c20a3b30c7143.
use crate::{
    credentials::SecretStore,
    dto::{HostKeyPrompt, KeyPassphrasePrompt},
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
use zeroize::Zeroizing;

struct Pending {
    scope: Uuid,
    prompt: HostKeyPrompt,
    response: oneshot::Sender<bool>,
}
struct PendingKey {
    scope: Uuid,
    prompt: KeyPassphrasePrompt,
    response: oneshot::Sender<Option<Zeroizing<String>>>,
}
struct CachedKey {
    source: Zeroizing<String>,
    plain: Zeroizing<String>,
}
#[derive(Default)]
struct PromptState {
    pending: HashMap<Uuid, Pending>,
    keys: HashMap<Uuid, PendingKey>,
    scopes: HashMap<Uuid, watch::Sender<bool>>,
    cache: HashMap<Uuid, CachedKey>,
}
pub struct PromptBroker {
    store: Arc<ConfigStore>,
    pending: Mutex<PromptState>,
    updates: watch::Sender<Vec<HostKeyPrompt>>,
    key_updates: watch::Sender<Vec<KeyPassphrasePrompt>>,
}
impl PromptBroker {
    pub fn new(store: Arc<ConfigStore>) -> Self {
        let (updates, _) = watch::channel(vec![]);
        let (key_updates, _) = watch::channel(vec![]);
        Self {
            store,
            pending: Mutex::new(PromptState::default()),
            updates,
            key_updates,
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
        let mut keys: Vec<_> = state.keys.values().map(|p| p.prompt.clone()).collect();
        keys.sort_by_key(|p| p.id);
        self.key_updates.send_replace(keys);
    }
    fn ask(&self, scope: Uuid, host_id: Uuid, q: HostKeyQuery, response: oneshot::Sender<bool>) {
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
        if !pending.scopes.contains_key(&scope) {
            let _ = response.send(false);
            return;
        }
        pending.pending.insert(
            prompt.id,
            Pending {
                scope,
                prompt,
                response,
            },
        );
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
    pub fn cancel_for_host(&self, scope: Uuid) {
        let mut state = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(tx) = state.scopes.remove(&scope) {
            tx.send_replace(true);
        }
        state.pending.retain(|_, p| p.scope != scope);
        state.keys.retain(|_, p| p.scope != scope);
        self.publish(&state);
    }
    pub fn cancel_interactive(&self) {
        let scopes: HashSet<_> = {
            let state = self.pending.lock().unwrap_or_else(|p| p.into_inner());
            state
                .pending
                .values()
                .map(|p| p.scope)
                .chain(state.keys.values().map(|p| p.scope))
                .collect()
        };
        for scope in scopes {
            self.cancel_for_host(scope);
        }
    }
    fn begin(&self, scope: Uuid) -> watch::Receiver<bool> {
        let mut state = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let (tx, rx) = watch::channel(false);
        state.scopes.insert(scope, tx);
        rx
    }
    pub fn key_subscribe(&self) -> watch::Receiver<Vec<KeyPassphrasePrompt>> {
        self.key_updates.subscribe()
    }
    pub fn key_snapshot(&self) -> Vec<KeyPassphrasePrompt> {
        self.key_updates.borrow().clone()
    }
    pub fn respond_key(&self, id: Uuid, passphrase: Option<String>) -> Result<(), String> {
        let passphrase = passphrase.map(Zeroizing::new);
        if passphrase.as_ref().is_some_and(|p| p.is_empty()) {
            return Err("Enter a passphrase or cancel".into());
        }
        let mut state = self.pending.lock().map_err(|_| "Prompt lock poisoned")?;
        let question = state
            .keys
            .remove(&id)
            .ok_or("Private-key question expired")?;
        let _ = question.response.send(passphrase);
        self.publish(&state);
        Ok(())
    }
    pub fn forget_key(&self, host: Uuid) {
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cache
            .remove(&host);
    }
    async fn unlock(
        &self,
        scope: Uuid,
        conn: &Connection,
        raw: String,
        unattended: bool,
    ) -> Result<String, String> {
        let raw = Zeroizing::new(raw);
        {
            let state = self.pending.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(key) = state.cache.get(&conn.id).filter(|key| key.source == raw) {
                return Ok(key.plain.to_string());
            }
        }
        let mut passphrase: Option<Zeroizing<String>> = None;
        loop {
            let input = raw.clone();
            let pass = passphrase.take();
            // Exactly the pinned Oryxis import API used by dispatch_keys/import.rs.
            // No vault is opened and no decrypted key or passphrase is persisted.
            let result = tokio::task::spawn_blocking(move || {
                oryxis_vault::import_key(
                    "TunnelWarden session",
                    &input,
                    pass.as_ref().map(|p| p.as_str()),
                )
            })
            .await
            .map_err(|e| e.to_string())?;
            let error = match result {
                Ok(key) => {
                    let plain = key.private_pem;
                    let mut state = self.pending.lock().unwrap_or_else(|p| p.into_inner());
                    if !state.scopes.contains_key(&scope) {
                        return Err("Connection cancelled".into());
                    }
                    state.cache.insert(
                        conn.id,
                        CachedKey {
                            source: raw,
                            plain: Zeroizing::new(plain.clone()),
                        },
                    );
                    return Ok(plain);
                }
                Err(oryxis_vault::VaultError::KeyNeedsPassphrase) => None,
                Err(oryxis_vault::VaultError::WrongKeyPassphrase) => {
                    Some("Incorrect passphrase. Try again or cancel.".to_string())
                }
                Err(e) => return Err(format!("Private key for {}: {e}", conn.label)),
            };
            if unattended {
                return Err(format!(
                    "Encrypted private key for {} is locked. Test this SSH host or start manually to unlock it for this app session.",
                    conn.label
                ));
            }
            let (tx, rx) = oneshot::channel();
            {
                let mut state = self.pending.lock().unwrap_or_else(|p| p.into_inner());
                if !state.scopes.contains_key(&scope) {
                    return Err("Connection cancelled".into());
                }
                let prompt = KeyPassphrasePrompt {
                    id: Uuid::new_v4(),
                    host_id: conn.id,
                    host_name: conn.label.clone(),
                    identity_file: conn.identity_file.clone().unwrap_or_default(),
                    error,
                };
                state.keys.insert(
                    prompt.id,
                    PendingKey {
                        scope,
                        prompt,
                        response: tx,
                    },
                );
                self.publish(&state);
            }
            passphrase = Some(
                rx.await
                    .ok()
                    .flatten()
                    .ok_or("Private-key unlock cancelled")?,
            );
        }
    }
}

pub struct ApplicationDialer {
    pub store: Arc<ConfigStore>,
    pub secrets: Arc<dyn SecretStore>,
    pub prompts: Arc<PromptBroker>,
}
struct PromptGuard {
    task: Option<tokio::task::JoinHandle<()>>,
    broker: Arc<PromptBroker>,
    host: Uuid,
}
impl Drop for PromptGuard {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
        self.broker.cancel_for_host(self.host);
    }
}

impl ForwardDialer for ApplicationDialer {
    fn connect(&self, host_id: Uuid, unattended: bool) -> DialFuture {
        self.connect_scoped(host_id, unattended, host_id)
    }
    fn cancel_prompts(&self, host_id: Uuid) {
        self.prompts.cancel_for_host(host_id);
    }
}
impl ApplicationDialer {
    pub fn connect_scoped(&self, host_id: Uuid, unattended: bool, scope: Uuid) -> DialFuture {
        let config = self.store.snapshot();
        let secrets = self.secrets.clone();
        let prompts = self.prompts.clone();
        let mut cancelled = prompts.begin(scope);
        let mut guard = PromptGuard {
            task: None,
            broker: prompts.clone(),
            host: scope,
        };
        Box::pin(async move {
            let operation = async {
                let (mut conn, password, key, resolver) =
                    prepare(&config, secrets, host_id, &prompts, scope, unattended).await?;
                let mut engine = SshEngine::new()
                    .with_strict_host_key(true)
                    .with_host_key_check(host_key_check(prompts.store.snapshot().known_hosts))
                    .with_keepalive(
                        conn.keepalive_interval
                            .filter(|s| *s > 0)
                            .map(|s| Duration::from_secs(s.into())),
                    );
                if !unattended {
                    let (tx, mut rx) = mpsc::channel::<(HostKeyQuery, oneshot::Sender<bool>)>(8);
                    engine = engine.with_host_key_ask(tx);
                    let broker = prompts.clone();
                    guard.task = Some(tokio::spawn(async move {
                        while let Some((query, response)) = rx.recv().await {
                            broker.ask(scope, host_id, query, response);
                        }
                    }));
                }
                conn.proxy = None;
                engine.connect_forward_conn(&conn, password.as_deref(), key.as_deref().map(KeyMaterial::plain), resolver.as_ref()).await
                    .map_err(|e| format!("{e}. For first connection or a changed host key, start manually and verify the fingerprint."))
            };
            tokio::select! {
                biased;
                _ = cancelled.changed() => Err("Connection cancelled".into()),
                result = tokio::time::timeout(Duration::from_secs(180), operation) => result.map_err(|_| "Connection timed out (including key unlock and host-key confirmation)".to_string())?,
            }
        })
    }
}

type Prepared = (
    Connection,
    Option<String>,
    Option<String>,
    Option<ConnectionResolver>,
);
async fn prepare(
    config: &Config,
    secrets: Arc<dyn SecretStore>,
    id: Uuid,
    prompts: &PromptBroker,
    scope: Uuid,
    unattended: bool,
) -> Result<Prepared, String> {
    let mut conn = config
        .hosts
        .iter()
        .find(|c| c.id == id)
        .cloned()
        .ok_or("SSH host not found")?;
    let (password, key) =
        resolve_credentials(&conn, secrets.clone(), prompts, scope, unattended).await?;
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
            let (pw, pk) =
                resolve_credentials(hop, secrets.clone(), prompts, scope, unattended).await?;
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

async fn resolve_credentials(
    conn: &Connection,
    secrets: Arc<dyn SecretStore>,
    prompts: &PromptBroker,
    scope: Uuid,
    unattended: bool,
) -> Result<(Option<String>, Option<String>), String> {
    let owned = conn.clone();
    let (password, key) =
        tokio::task::spawn_blocking(move || read_credentials(&owned, secrets.as_ref()))
            .await
            .map_err(|e| e.to_string())??;
    let key = match key {
        Some(raw) => Some(prompts.unlock(scope, conn, raw, unattended).await?),
        None => None,
    };
    Ok((password, key))
}
fn read_credentials(
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
