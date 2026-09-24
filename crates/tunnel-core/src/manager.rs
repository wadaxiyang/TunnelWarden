use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use ssh_engine::{HopSpec, SshChain, SshChainError, SshCredential};
use thiserror::Error;
use tokio::{
    sync::{Notify, mpsc, watch},
    task::JoinHandle,
    time::{Instant, interval, timeout, timeout_at},
};
use tokio_util::sync::CancellationToken;
use tunnel_domain::{
    AuthConfig, SecretRef, SshHost, TunnelConfig, TunnelGroup, TunnelId, TunnelMode,
};
use zeroize::Zeroizing;

use crate::{
    ListenerRuntime, LocalForwardSupervisor, RemoteForwardSupervisor, SupervisorMode,
    SupervisorState,
};

const MAX_TUNNELS: usize = 1024;
const COMMAND_CAPACITY: usize = 128;
const STOP_DEADLINE: Duration = Duration::from_secs(3);

/// Blocking OS credential-store access. The manager always invokes this on a
/// Tokio blocking worker and never retains plaintext beyond one SSH attempt.
pub trait CredentialSource: Send + Sync + 'static {
    fn load(&self, reference: &SecretRef) -> Result<Zeroizing<String>, String>;
}

#[derive(Clone, Debug)]
pub struct TunnelView {
    pub state: SupervisorState,
    pub local_addr: Option<SocketAddr>,
}

pub type ManagerSnapshot = HashMap<TunnelId, TunnelView>;

#[derive(Clone, Debug)]
pub enum CoreCommand {
    StartTunnel(TunnelId),
    StopTunnel(TunnelId),
    RestartTunnel(TunnelId),
    RetryTunnel(TunnelId),
    NetworkRecovered,
}

#[derive(Debug, Error)]
pub enum ManagerError {
    #[error("too many tunnels")]
    TooManyTunnels,
    #[error("duplicate tunnel ID: {0}")]
    DuplicateTunnel(String),
}

#[derive(Clone)]
pub struct ManagerHandle {
    commands: mpsc::Sender<CoreCommand>,
    snapshots: watch::Receiver<Arc<ManagerSnapshot>>,
    shutdown: CancellationToken,
}

impl ManagerHandle {
    pub fn try_send(
        &self,
        command: CoreCommand,
    ) -> Result<(), mpsc::error::TrySendError<CoreCommand>> {
        self.commands.try_send(command)
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<ManagerSnapshot>> {
        self.snapshots.clone()
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }
}

struct ActiveTunnel {
    cancellation: CancellationToken,
    task: JoinHandle<Option<String>>,
    state: watch::Receiver<SupervisorState>,
    retry_hint: Arc<Notify>,
}

/// Single writer for desired tunnel state and the bounded UI snapshot. It owns
/// every supervisor task, cancellation token, and completion handle.
pub struct TunnelManager {
    hosts: Vec<SshHost>,
    groups: Vec<TunnelGroup>,
    tunnels: HashMap<TunnelId, TunnelConfig>,
    credentials: Arc<dyn CredentialSource>,
    commands: mpsc::Receiver<CoreCommand>,
    snapshots: watch::Sender<Arc<ManagerSnapshot>>,
    current: ManagerSnapshot,
    active: HashMap<TunnelId, ActiveTunnel>,
    shutdown: CancellationToken,
    auto_start: Vec<TunnelId>,
}

impl TunnelManager {
    pub fn new(
        hosts: Vec<SshHost>,
        groups: Vec<TunnelGroup>,
        tunnels: Vec<TunnelConfig>,
        credentials: Arc<dyn CredentialSource>,
    ) -> Result<(Self, ManagerHandle), ManagerError> {
        if tunnels.len() > MAX_TUNNELS {
            return Err(ManagerError::TooManyTunnels);
        }
        let mut definitions = HashMap::with_capacity(tunnels.len());
        let mut current = HashMap::with_capacity(tunnels.len());
        let mut auto_start = Vec::new();
        for tunnel in tunnels {
            if definitions.contains_key(&tunnel.id) {
                return Err(ManagerError::DuplicateTunnel(tunnel.id.0));
            }
            current.insert(
                tunnel.id.clone(),
                TunnelView {
                    state: SupervisorState::Stopped,
                    local_addr: None,
                },
            );
            if tunnel.auto_start {
                auto_start.push(tunnel.id.clone());
            }
            definitions.insert(tunnel.id.clone(), tunnel);
        }
        let (commands_tx, commands_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (snapshots_tx, snapshots_rx) = watch::channel(Arc::new(current.clone()));
        let shutdown = CancellationToken::new();
        Ok((
            Self {
                hosts,
                groups,
                tunnels: definitions,
                credentials,
                commands: commands_rx,
                snapshots: snapshots_tx,
                current,
                active: HashMap::new(),
                shutdown: shutdown.clone(),
                auto_start,
            },
            ManagerHandle {
                commands: commands_tx,
                snapshots: snapshots_rx,
                shutdown,
            },
        ))
    }

    pub async fn run(mut self) {
        let auto_start = std::mem::take(&mut self.auto_start);
        for id in auto_start {
            if self.shutdown.is_cancelled() {
                break;
            }
            self.start(&id).await;
        }
        let mut poll = interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => break,
                command = self.commands.recv() => match command {
                    Some(CoreCommand::StartTunnel(id)) => self.start(&id).await,
                    Some(CoreCommand::StopTunnel(id)) => self.stop(&id).await,
                    Some(CoreCommand::RestartTunnel(id)) => {
                        self.stop(&id).await;
                        self.start(&id).await;
                    }
                    Some(CoreCommand::RetryTunnel(id)) => {
                        if let Some(active) = self.active.get(&id) {
                            if matches!(&*active.state.borrow(), SupervisorState::Blocked { .. } | SupervisorState::Reconnecting { .. }) {
                                active.retry_hint.notify_one();
                            }
                        } else {
                            self.start(&id).await;
                        }
                    }
                    Some(CoreCommand::NetworkRecovered) => {
                        for active in self.active.values() {
                            if matches!(&*active.state.borrow(), SupervisorState::Reconnecting { .. }) {
                                active.retry_hint.notify_one();
                            }
                        }
                    }
                    None => break,
                },
                _ = poll.tick(), if !self.active.is_empty() => self.refresh().await,
            }
        }
        for active in self.active.values() {
            active.cancellation.cancel();
        }
        let deadline = Instant::now() + STOP_DEADLINE;
        for (_, mut active) in self.active.drain() {
            if timeout_at(deadline, &mut active.task).await.is_err() {
                active.task.abort();
                let _ = active.task.await;
            }
        }
    }

    async fn start(&mut self, id: &TunnelId) {
        if self.active.contains_key(id) {
            return;
        }
        let Some(tunnel) = self.tunnels.get(id).cloned() else {
            return;
        };
        if let Err(error) = tunnel.validate(&self.hosts, &self.groups) {
            self.set_state(
                id,
                SupervisorState::Blocked {
                    reason: error.to_string(),
                },
                None,
            );
            return;
        }
        let hosts = match tunnel
            .jump_chain
            .iter()
            .map(|id| self.hosts.iter().find(|host| host.id == *id).cloned())
            .collect::<Option<Vec<_>>>()
        {
            Some(hosts) => Arc::<[SshHost]>::from(hosts),
            None => {
                self.set_state(
                    id,
                    SupervisorState::Blocked {
                        reason: "Unknown jump host".into(),
                    },
                    None,
                );
                return;
            }
        };
        let cancellation = CancellationToken::new();
        let (task, state, local_addr, retry_hint) = match tunnel.mode {
            TunnelMode::Local | TunnelMode::Dynamic => {
                let address: SocketAddr =
                    match format!("{}:{}", tunnel.local.host, tunnel.local.port).parse() {
                        Ok(address) => address,
                        Err(error) => {
                            self.set_state(
                                id,
                                SupervisorState::Blocked {
                                    reason: format!("Invalid listener address: {error}"),
                                },
                                None,
                            );
                            return;
                        }
                    };
                let mut listener = ListenerRuntime::new();
                if let Err(error) = listener.start_listener(address).await {
                    self.set_state(
                        id,
                        SupervisorState::Blocked {
                            reason: error.to_string(),
                        },
                        None,
                    );
                    return;
                }
                let local_addr = listener.local_addr().ok().flatten();
                let mode = match tunnel.mode {
                    TunnelMode::Local => match tunnel.remote.clone() {
                        Some(remote) => SupervisorMode::Local(remote),
                        None => {
                            self.set_state(
                                id,
                                SupervisorState::Blocked {
                                    reason: "Remote target is missing".into(),
                                },
                                None,
                            );
                            return;
                        }
                    },
                    TunnelMode::Dynamic => SupervisorMode::Dynamic,
                    TunnelMode::Remote => unreachable!(),
                };
                let supervisor = match LocalForwardSupervisor::new(
                    listener,
                    mode,
                    cancellation.clone(),
                    tunnel.reconnect.clone(),
                ) {
                    Ok(supervisor) => supervisor,
                    Err(error) => {
                        self.set_state(
                            id,
                            SupervisorState::Blocked {
                                reason: error.to_string(),
                            },
                            None,
                        );
                        return;
                    }
                };
                let state = supervisor.subscribe();
                let retry_hint = supervisor.retry_handle();
                let credentials = Arc::clone(&self.credentials);
                let remote = None;
                let connect_cancel = cancellation.clone();
                let task = tokio::spawn(async move {
                    supervisor
                        .run(move || {
                            let hosts = Arc::clone(&hosts);
                            let credentials = Arc::clone(&credentials);
                            let cancellation = connect_cancel.clone();
                            let remote = remote.clone();
                            async move {
                                let hops = build_hops(&hosts, credentials).await?;
                                SshChain::connect(hops, remote, &cancellation).await
                            }
                        })
                        .await
                        .err()
                        .map(|error| error.to_string())
                });
                (task, state, local_addr, retry_hint)
            }
            TunnelMode::Remote => {
                let supervisor = RemoteForwardSupervisor::new(
                    tunnel.local.clone(),
                    cancellation.clone(),
                    tunnel.reconnect.clone(),
                );
                let state = supervisor.subscribe();
                let retry_hint = supervisor.retry_handle();
                let credentials = Arc::clone(&self.credentials);
                let remote = tunnel.remote.clone();
                let task = tokio::spawn(async move {
                    supervisor
                        .run(move |session_token| {
                            let hosts = Arc::clone(&hosts);
                            let credentials = Arc::clone(&credentials);
                            let remote = remote.clone();
                            async move {
                                let hops = build_hops(&hosts, credentials).await?;
                                SshChain::connect(hops, remote, &session_token).await
                            }
                        })
                        .await;
                    None
                });
                (task, state, None, retry_hint)
            }
        };
        self.set_state(id, SupervisorState::Connecting { attempt: 1 }, local_addr);
        self.active.insert(
            id.clone(),
            ActiveTunnel {
                cancellation,
                task,
                state,
                retry_hint,
            },
        );
    }

    async fn stop(&mut self, id: &TunnelId) {
        if let Some(mut active) = self.active.remove(id) {
            active.cancellation.cancel();
            if timeout(STOP_DEADLINE, &mut active.task).await.is_err() {
                active.task.abort();
                let _ = active.task.await;
            }
        }
        self.set_state(id, SupervisorState::Stopped, None);
    }

    async fn refresh(&mut self) {
        let mut changed = false;
        let mut finished = Vec::new();
        for (id, active) in &mut self.active {
            if active.state.has_changed().unwrap_or(false) {
                let state = active.state.borrow_and_update().clone();
                if let Some(view) = self.current.get_mut(id) {
                    view.state = state;
                    changed = true;
                }
            }
            if active.task.is_finished() {
                finished.push(id.clone());
            }
        }
        for id in finished {
            if let Some(active) = self.active.remove(&id) {
                let result = active.task.await;
                let reason = match result {
                    Ok(Some(reason)) => reason,
                    Ok(None) => "Tunnel task exited".into(),
                    Err(error) => error.to_string(),
                };
                if let Some(view) = self.current.get_mut(&id) {
                    view.state = SupervisorState::Blocked { reason };
                    view.local_addr = None;
                    changed = true;
                }
            }
        }
        if changed {
            self.publish();
        }
    }

    fn set_state(&mut self, id: &TunnelId, state: SupervisorState, local_addr: Option<SocketAddr>) {
        if let Some(view) = self.current.get_mut(id) {
            *view = TunnelView { state, local_addr };
            self.publish();
        }
    }

    fn publish(&self) {
        self.snapshots.send_replace(Arc::new(self.current.clone()));
    }
}

async fn build_hops(
    hosts: &[SshHost],
    credentials: Arc<dyn CredentialSource>,
) -> Result<Vec<HopSpec>, SshChainError> {
    let known_hosts = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join(".ssh").join("known_hosts"))
        .into_iter()
        .collect::<Vec<_>>();
    let mut hops = Vec::with_capacity(hosts.len());
    for (ix, host) in hosts.iter().enumerate() {
        let credential = match &host.auth {
            AuthConfig::Agent { .. } => SshCredential::Agent,
            AuthConfig::KeyboardInteractive => {
                return Err(SshChainError::Credential {
                    hop: ix + 1,
                    reason: "Keyboard-interactive authentication is not available yet".into(),
                });
            }
            AuthConfig::Password { credential_ref } => {
                let secret =
                    load_secret(Arc::clone(&credentials), credential_ref.clone(), ix + 1).await?;
                SshCredential::Password(secret)
            }
            AuthConfig::PrivateKey { passphrase_ref, .. } => {
                let secret = match passphrase_ref {
                    Some(reference) => Some(
                        load_secret(Arc::clone(&credentials), reference.clone(), ix + 1).await?,
                    ),
                    None => None,
                };
                SshCredential::PrivateKey(secret)
            }
        };
        hops.push(HopSpec {
            host: host.clone(),
            credential,
            known_hosts_paths: known_hosts.clone(),
        });
    }
    Ok(hops)
}

async fn load_secret(
    credentials: Arc<dyn CredentialSource>,
    reference: SecretRef,
    hop: usize,
) -> Result<Zeroizing<String>, SshChainError> {
    tokio::task::spawn_blocking(move || credentials.load(&reference))
        .await
        .map_err(|error| SshChainError::Credential {
            hop,
            reason: error.to_string(),
        })?
        .map_err(|reason| SshChainError::Credential {
            hop,
            reason: reason.chars().take(256).collect(),
        })
}
