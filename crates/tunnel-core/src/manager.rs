use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use config_store::{
    AppSettings, ConfigDocument, ConfigStore, DomainConfig, SecretStore, SshImportPreview,
    preview_ssh_config, set_run_at_login,
};
use ssh_engine::{
    HopSpec, HostKeyApproval, HostKeyDecision, HostKeyPrompt, SshChain, SshChainError,
    SshCredential,
};
use thiserror::Error;
use tokio::{
    sync::{Mutex, Notify, mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, interval, timeout, timeout_at},
};
use tokio_util::sync::CancellationToken;
use tunnel_domain::{
    AuthConfig, GroupId, SecretRef, SshHost, TunnelConfig, TunnelGroup, TunnelId, TunnelMode,
};
use zeroize::Zeroizing;

use crate::{
    ListenerRuntime, LocalForwardSupervisor, RemoteForwardSupervisor, SupervisorMode,
    SupervisorState,
};

const MAX_TUNNELS: usize = 1024;
const COMMAND_CAPACITY: usize = 128;
const STOP_DEADLINE: Duration = Duration::from_secs(3);
const MAX_HOST_KEY_PROMPTS: usize = 16;
const MAX_LOG_EVENTS: usize = 500;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogSource {
    Tunnel,
    Host,
}

#[derive(Debug)]
pub struct LogEvent {
    pub timestamp_unix: u64,
    pub level: LogLevel,
    pub source: LogSource,
    pub subject: String,
    pub stage: &'static str,
    pub message: &'static str,
}

pub type LogSnapshot = VecDeque<Arc<LogEvent>>;

#[derive(Clone, Debug)]
pub struct HostKeyPromptView {
    pub id: u64,
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
}

pub enum CoreCommand {
    StartAll,
    StopAll,
    StartGroup(GroupId),
    StopGroup(GroupId),
    StartTunnel(TunnelId),
    StopTunnel(TunnelId),
    RestartTunnel(TunnelId),
    RetryTunnel(TunnelId),
    NetworkRecovered,
    SaveConfig {
        directory: PathBuf,
        config: DomainConfig,
        secret: Option<SecretUpdate>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    ImportConfig {
        path: PathBuf,
        reply: oneshot::Sender<Result<DomainConfig, String>>,
    },
    PreviewSshConfig {
        path: PathBuf,
        reply: oneshot::Sender<Result<SshImportPreview, String>>,
    },
    ExportConfig {
        path: PathBuf,
        config: DomainConfig,
        reply: oneshot::Sender<Result<(), String>>,
    },
    ResolveHostKey {
        id: u64,
        decision: HostKeyDecision,
    },
}

pub struct SecretUpdate {
    /// A new keyring reference. Existing secrets remain available on failure.
    pub reference: SecretRef,
    pub value: Zeroizing<String>,
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
    host_keys: watch::Receiver<Arc<Vec<HostKeyPromptView>>>,
    config: watch::Receiver<Arc<DomainConfig>>,
    logs: watch::Receiver<Arc<LogSnapshot>>,
    shutdown: CancellationToken,
}

impl ManagerHandle {
    pub fn try_send(&self, command: CoreCommand) -> Result<(), &'static str> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => "Tunnel manager is busy",
                mpsc::error::TrySendError::Closed(_) => "Tunnel manager has stopped",
            })
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<ManagerSnapshot>> {
        self.snapshots.clone()
    }

    pub fn subscribe_host_keys(&self) -> watch::Receiver<Arc<Vec<HostKeyPromptView>>> {
        self.host_keys.clone()
    }
    pub fn subscribe_config(&self) -> watch::Receiver<Arc<DomainConfig>> {
        self.config.clone()
    }

    pub fn subscribe_logs(&self) -> watch::Receiver<Arc<LogSnapshot>> {
        self.logs.clone()
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

struct PendingHostKey {
    view: HostKeyPromptView,
    reply: oneshot::Sender<HostKeyDecision>,
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
    logs: LogSnapshot,
    log_views: watch::Sender<Arc<LogSnapshot>>,
    active: HashMap<TunnelId, ActiveTunnel>,
    desired_running: HashSet<TunnelId>,
    shutdown: CancellationToken,
    auto_start: Vec<TunnelId>,
    host_key_sender: mpsc::Sender<HostKeyPrompt>,
    host_key_requests: mpsc::Receiver<HostKeyPrompt>,
    host_key_views: watch::Sender<Arc<Vec<HostKeyPromptView>>>,
    config_views: watch::Sender<Arc<DomainConfig>>,
    pending_host_keys: Vec<PendingHostKey>,
    next_host_key_id: u64,
    app_known_hosts_path: Option<PathBuf>,
    host_key_save_lock: Arc<Mutex<()>>,
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
        let (log_views, log_rx) = watch::channel(Arc::new(LogSnapshot::new()));
        let (host_key_sender, host_key_requests) = mpsc::channel(MAX_HOST_KEY_PROMPTS);
        let (host_key_views, host_key_rx) = watch::channel(Arc::new(Vec::new()));
        let (config_views, config_rx) = watch::channel(Arc::new(DomainConfig {
            app: AppSettings::default(),
            hosts: hosts.clone(),
            groups: groups.clone(),
            tunnels: definitions.values().cloned().collect(),
        }));
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
                logs: LogSnapshot::new(),
                log_views,
                active: HashMap::new(),
                desired_running: HashSet::new(),
                shutdown: shutdown.clone(),
                auto_start,
                host_key_sender,
                host_key_requests,
                host_key_views,
                config_views,
                pending_host_keys: Vec::new(),
                next_host_key_id: 1,
                app_known_hosts_path: None,
                host_key_save_lock: Arc::new(Mutex::new(())),
            },
            ManagerHandle {
                commands: commands_tx,
                snapshots: snapshots_rx,
                logs: log_rx,
                host_keys: host_key_rx,
                config: config_rx,
                shutdown,
            },
        ))
    }

    pub fn with_app_known_hosts(mut self, path: PathBuf) -> Self {
        self.app_known_hosts_path = Some(path);
        self
    }

    pub fn with_app_settings(self, app: AppSettings) -> Self {
        let mut config = (*self.config_views.borrow()).as_ref().clone();
        config.app = app;
        self.config_views.send_replace(Arc::new(config));
        self
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
                    Some(CoreCommand::StartAll) => {
                        let ids: Vec<_> = self.tunnels.keys().cloned().collect();
                        for id in ids { self.start(&id).await; }
                    }
                    Some(CoreCommand::StopAll) => {
                        let ids: Vec<_> = self.active.keys().cloned().collect();
                        for id in ids { self.stop(&id).await; }
                    }
                    Some(CoreCommand::StartGroup(group)) => {
                        let ids: Vec<_> = self.tunnels.iter()
                            .filter(|(_, tunnel)| tunnel.group_id.as_ref() == Some(&group))
                            .map(|(id, _)| id.clone()).collect();
                        for id in ids { self.start(&id).await; }
                    }
                    Some(CoreCommand::StopGroup(group)) => {
                        let ids: Vec<_> = self.tunnels.iter()
                            .filter(|(_, tunnel)| tunnel.group_id.as_ref() == Some(&group))
                            .map(|(id, _)| id.clone()).collect();
                        for id in ids { self.stop(&id).await; }
                    }
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
                    Some(CoreCommand::SaveConfig { directory, config, secret, reply }) => {
                        let old_run_at_login = self.config_views.borrow().app.run_at_startup;
                        let canonical = ConfigDocument::try_from(config)
                            .and_then(ConfigDocument::into_domain)
                            .map_err(|error| error.to_string());
                        let result = match canonical {
                            Ok(config) => {
                                let result = Self::persist_config(directory, config.clone(), secret, old_run_at_login).await;
                                if result.is_ok() {
                                    self.replace_config(config).await;
                                }
                                result
                            }
                            Err(error) => Err(error),
                        };
                        let _ = reply.send(result);
                    }
                    Some(CoreCommand::ImportConfig { path, reply }) => {
                        let result = tokio::task::spawn_blocking(move || ConfigStore::import_file(&path).map_err(|error| error.to_string()))
                            .await.map_err(|error| error.to_string()).and_then(|result| result);
                        let _ = reply.send(result);
                    }
                    Some(CoreCommand::PreviewSshConfig { path, reply }) => {
                        let result = tokio::task::spawn_blocking(move || preview_ssh_config(&path).map_err(|error| error.to_string()))
                            .await.map_err(|error| error.to_string()).and_then(|result| result);
                        let _ = reply.send(result);
                    }
                    Some(CoreCommand::ExportConfig { path, config, reply }) => {
                        let result = tokio::task::spawn_blocking(move || ConfigStore::export_file(&path, config).map_err(|error| error.to_string()))
                            .await.map_err(|error| error.to_string()).and_then(|result| result);
                        let _ = reply.send(result);
                    }
                    Some(CoreCommand::ResolveHostKey { id, decision }) => {
                        if let Some(index) = self.pending_host_keys.iter().position(|prompt| prompt.view.id == id) {
                            let pending = self.pending_host_keys.remove(index);
                            let _ = pending.reply.send(decision);
                            self.publish_host_keys();
                        }
                    }
                    None => break,
                },
                prompt = self.host_key_requests.recv() => {
                    if let Some(prompt) = prompt { self.queue_host_key(prompt); }
                },
                _ = poll.tick(), if !self.active.is_empty() => self.refresh().await,
            }
        }
        for active in self.active.values() {
            active.cancellation.cancel();
        }
        self.pending_host_keys.clear();
        let deadline = Instant::now() + STOP_DEADLINE;
        for (_, mut active) in self.active.drain() {
            if timeout_at(deadline, &mut active.task).await.is_err() {
                active.task.abort();
                let _ = active.task.await;
            }
        }
    }

    fn queue_host_key(&mut self, prompt: HostKeyPrompt) {
        if self.pending_host_keys.len() >= MAX_HOST_KEY_PROMPTS {
            let _ = prompt.reply.send(HostKeyDecision::Cancel);
            return;
        }
        let view = HostKeyPromptView {
            id: self.next_host_key_id,
            host: prompt.host,
            port: prompt.port,
            algorithm: prompt.algorithm,
            fingerprint: prompt.fingerprint,
        };
        self.next_host_key_id = self.next_host_key_id.wrapping_add(1).max(1);
        self.pending_host_keys.push(PendingHostKey {
            view,
            reply: prompt.reply,
        });
        if let Some(pending) = self.pending_host_keys.last() {
            let host = pending.view.host.clone();
            self.push_log(
                LogLevel::Warning,
                LogSource::Host,
                host,
                "Host key",
                "Confirmation required",
            );
        }
        self.publish_host_keys();
    }

    fn publish_host_keys(&self) {
        self.host_key_views.send_replace(Arc::new(
            self.pending_host_keys
                .iter()
                .map(|prompt| prompt.view.clone())
                .collect(),
        ));
    }

    async fn persist_config(
        directory: PathBuf,
        config: DomainConfig,
        secret: Option<SecretUpdate>,
        old_run_at_login: bool,
    ) -> Result<(), String> {
        tokio::task::spawn_blocking(move || {
            let run_at_login = config.app.run_at_startup;
            let document = ConfigDocument::try_from(config).map_err(|error| error.to_string())?;
            if let Some(secret) = &secret {
                SecretStore::save(&secret.reference, &secret.value)
                    .map_err(|error| error.to_string())?;
            }
            if run_at_login != old_run_at_login
                && let Err(error) = set_run_at_login(run_at_login)
            {
                if let Some(secret) = &secret {
                    let _ = SecretStore::delete(&secret.reference);
                }
                return Err(error.to_string());
            }
            match ConfigStore::new(directory).save(document) {
                Ok(()) => Ok(()),
                Err(error) => {
                    if let Some(secret) = &secret {
                        let _ = SecretStore::delete(&secret.reference);
                    }
                    if run_at_login != old_run_at_login
                        && let Err(rollback) = set_run_at_login(old_run_at_login)
                    {
                        return Err(format!(
                            "{error}; could not restore login startup setting: {rollback}"
                        ));
                    }
                    Err(error.to_string())
                }
            }
        })
        .await
        .map_err(|error| error.to_string())?
    }

    async fn replace_config(&mut self, config: DomainConfig) {
        self.config_views.send_replace(Arc::new(config.clone()));
        let old_running = self.desired_running.clone();
        let updated: HashMap<TunnelId, TunnelConfig> = config
            .tunnels
            .into_iter()
            .map(|tunnel| (tunnel.id.clone(), tunnel))
            .collect();
        let needs_restart: Vec<TunnelId> = old_running
            .iter()
            .filter(|id| {
                let Some(old) = self.tunnels.get(*id) else {
                    return true;
                };
                let Some(new) = updated.get(*id) else {
                    return true;
                };
                !same_tunnel_runtime(old, new)
                    || old.jump_chain.iter().any(|hop| {
                        !same_host_connection(
                            self.hosts.iter().find(|host| &host.id == hop),
                            config.hosts.iter().find(|host| &host.id == hop),
                        )
                    })
            })
            .cloned()
            .collect();
        for id in &needs_restart {
            self.stop(id).await;
        }
        self.hosts = config.hosts;
        self.groups = config.groups;
        self.tunnels = updated;
        self.desired_running
            .retain(|id| self.tunnels.contains_key(id));
        self.current.retain(|id, _| self.tunnels.contains_key(id));
        for id in self.tunnels.keys() {
            self.current.entry(id.clone()).or_insert(TunnelView {
                state: SupervisorState::Stopped,
                local_addr: None,
            });
        }
        self.publish();
        let to_start: Vec<TunnelId> = self
            .tunnels
            .values()
            .filter(|tunnel| old_running.contains(&tunnel.id))
            .map(|tunnel| tunnel.id.clone())
            .collect();
        for id in to_start {
            self.start(&id).await;
        }
    }

    async fn start(&mut self, id: &TunnelId) {
        if !self.tunnels.contains_key(id) {
            return;
        }
        self.desired_running.insert(id.clone());
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
        let app_path = self.app_known_hosts_path.clone();
        let approval = app_path.as_ref().map(|path| HostKeyApproval {
            prompts: self.host_key_sender.clone(),
            save_path: path.clone(),
            save_lock: Arc::clone(&self.host_key_save_lock),
        });
        let (task, state, local_addr, retry_hint) = match tunnel.mode {
            TunnelMode::Local | TunnelMode::Dynamic => {
                let address = match tunnel.local.host.parse::<IpAddr>() {
                    Ok(ip) => SocketAddr::new(ip, tunnel.local.port),
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
                let known_path = app_path.clone();
                let approval = approval.clone();
                let task = tokio::spawn(async move {
                    supervisor
                        .run(move || {
                            let hosts = Arc::clone(&hosts);
                            let credentials = Arc::clone(&credentials);
                            let cancellation = connect_cancel.clone();
                            let remote = remote.clone();
                            let known_path = known_path.clone();
                            let approval = approval.clone();
                            async move {
                                let hops =
                                    build_hops(&hosts, credentials, known_path.as_ref()).await?;
                                SshChain::connect_with_approval(
                                    hops,
                                    remote,
                                    &cancellation,
                                    approval,
                                )
                                .await
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
                let known_path = app_path.clone();
                let approval = approval.clone();
                let task = tokio::spawn(async move {
                    supervisor
                        .run(move |session_token| {
                            let hosts = Arc::clone(&hosts);
                            let credentials = Arc::clone(&credentials);
                            let remote = remote.clone();
                            let known_path = known_path.clone();
                            let approval = approval.clone();
                            async move {
                                let hops =
                                    build_hops(&hosts, credentials, known_path.as_ref()).await?;
                                SshChain::connect_with_approval(
                                    hops,
                                    remote,
                                    &session_token,
                                    approval,
                                )
                                .await
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
        self.desired_running.remove(id);
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
        let previous_prompts = self.pending_host_keys.len();
        self.pending_host_keys
            .retain(|prompt| !prompt.reply.is_closed());
        if self.pending_host_keys.len() != previous_prompts {
            self.publish_host_keys();
        }
        let mut changed = false;
        let mut transitions = Vec::new();
        let mut finished = Vec::new();
        for (id, active) in &mut self.active {
            if active.state.has_changed().unwrap_or(false) {
                let state = active.state.borrow_and_update().clone();
                if let Some(view) = self.current.get_mut(id) {
                    if std::mem::discriminant(&view.state) != std::mem::discriminant(&state) {
                        transitions.push((id.clone(), state.clone()));
                    }
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
                    transitions.push((id.clone(), view.state.clone()));
                    changed = true;
                }
            }
        }
        for (id, state) in transitions {
            self.log_state(&id, &state);
        }
        if changed {
            self.publish();
        }
    }

    fn set_state(&mut self, id: &TunnelId, state: SupervisorState, local_addr: Option<SocketAddr>) {
        if let Some(view) = self.current.get_mut(id) {
            let changed = std::mem::discriminant(&view.state) != std::mem::discriminant(&state);
            *view = TunnelView { state, local_addr };
            if changed {
                let state = self.current.get(id).map(|view| view.state.clone());
                if let Some(state) = state {
                    self.log_state(id, &state);
                }
            }
            self.publish();
        }
    }

    fn log_state(&mut self, id: &TunnelId, state: &SupervisorState) {
        let (level, message) = match state {
            SupervisorState::Stopped => (LogLevel::Info, "Stopped"),
            SupervisorState::Connecting { .. } => (LogLevel::Info, "Connecting to SSH host"),
            SupervisorState::Healthy { .. } => (LogLevel::Info, "SSH connection healthy"),
            SupervisorState::Degraded { .. } => (LogLevel::Warning, "SSH connection degraded"),
            SupervisorState::Reconnecting { .. } => (LogLevel::Warning, "Reconnecting"),
            SupervisorState::Blocked { .. } => (
                LogLevel::Error,
                "Tunnel blocked; inspect status for details",
            ),
        };
        self.push_log(level, LogSource::Tunnel, id.0.clone(), "Lifecycle", message);
    }

    fn push_log(
        &mut self,
        level: LogLevel,
        source: LogSource,
        subject: String,
        stage: &'static str,
        message: &'static str,
    ) {
        if self.logs.len() == MAX_LOG_EVENTS {
            self.logs.pop_front();
        }
        self.logs.push_back(Arc::new(LogEvent {
            timestamp_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            level,
            source,
            subject,
            stage,
            message,
        }));
        self.log_views.send_replace(Arc::new(self.logs.clone()));
    }

    fn publish(&self) {
        self.snapshots.send_replace(Arc::new(self.current.clone()));
    }
}

async fn build_hops(
    hosts: &[SshHost],
    credentials: Arc<dyn CredentialSource>,
    app_known_hosts_path: Option<&PathBuf>,
) -> Result<Vec<HopSpec>, SshChainError> {
    let mut known_hosts = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join(".ssh").join("known_hosts"))
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(path) = app_known_hosts_path {
        known_hosts.push(path.clone());
    }
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

fn same_tunnel_runtime(old: &TunnelConfig, new: &TunnelConfig) -> bool {
    old.mode == new.mode
        && old.jump_chain == new.jump_chain
        && old.local == new.local
        && old.remote == new.remote
        && old.reconnect == new.reconnect
}

fn same_host_connection(old: Option<&SshHost>, new: Option<&SshHost>) -> bool {
    match (old, new) {
        (Some(old), Some(new)) => {
            old.hostname == new.hostname
                && old.port == new.port
                && old.username == new.username
                && old.auth == new.auth
                && old.host_key_policy == new.host_key_policy
                && old.connect_timeout == new.connect_timeout
                && old.keepalive_interval == new.keepalive_interval
                && old.keepalive_max == new.keepalive_max
        }
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;
    use config_store::AppSettings;
    use tempfile::TempDir;
    use tunnel_domain::{HostId, HostKeyPolicy, LocalEndpoint, RetryPolicy};

    struct EmptyCredentials;
    impl CredentialSource for EmptyCredentials {
        fn load(&self, _: &SecretRef) -> Result<Zeroizing<String>, String> {
            Err("no credentials configured".into())
        }
    }

    fn test_host() -> SshHost {
        SshHost {
            id: HostId("host".into()),
            name: "host".into(),
            hostname: "127.0.0.1".into(),
            port: 9,
            username: "user".into(),
            auth: AuthConfig::Agent { socket: None },
            host_key_policy: HostKeyPolicy::Strict,
            connect_timeout: Duration::from_secs(1),
            keepalive_interval: Duration::from_secs(10),
            keepalive_max: 3,
            notes: String::new(),
        }
    }

    fn test_tunnel(bind_host: &str, port: u16) -> TunnelConfig {
        TunnelConfig {
            id: TunnelId("test".into()),
            name: "test".into(),
            group_id: None,
            mode: TunnelMode::Dynamic,
            jump_chain: vec![HostId("host".into())],
            local: LocalEndpoint {
                host: bind_host.into(),
                port,
            },
            remote: None,
            auto_start: true,
            reconnect: RetryPolicy::default(),
            description: String::new(),
        }
    }

    #[tokio::test]
    async fn manual_stop_survives_unrelated_config_save() {
        let reserve = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
        let port = reserve.local_addr().expect("reserved address").port();
        drop(reserve);
        let tunnel = test_tunnel("127.0.0.1", port);
        let (mut manager, _) = TunnelManager::new(
            vec![test_host()],
            Vec::new(),
            vec![tunnel.clone()],
            Arc::new(EmptyCredentials),
        )
        .expect("manager");
        manager.start(&tunnel.id).await;
        assert!(manager.active.contains_key(&tunnel.id));
        manager.stop(&tunnel.id).await;
        let mut edited = tunnel.clone();
        edited.description = "unrelated note".into();
        manager
            .replace_config(DomainConfig {
                app: AppSettings::default(),
                hosts: vec![test_host()],
                groups: Vec::new(),
                tunnels: vec![edited],
            })
            .await;
        assert!(!manager.desired_running.contains(&tunnel.id));
        assert!(!manager.active.contains_key(&tunnel.id));
        std::net::TcpListener::bind(("127.0.0.1", port)).expect("listener released");
    }

    #[tokio::test]
    async fn ipv6_loopback_listener_uses_structured_address() {
        let reserve = match std::net::TcpListener::bind("[::1]:0") {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("IPv6 loopback unavailable; skipping: {error}");
                return;
            }
        };
        let port = reserve.local_addr().expect("reserved address").port();
        drop(reserve);
        let tunnel = test_tunnel("::1", port);
        let (mut manager, _) = TunnelManager::new(
            vec![test_host()],
            Vec::new(),
            vec![tunnel.clone()],
            Arc::new(EmptyCredentials),
        )
        .expect("manager");
        manager.start(&tunnel.id).await;
        assert_eq!(
            manager.current[&tunnel.id].local_addr,
            Some(SocketAddr::new(
                IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
                port
            ))
        );
        manager.stop(&tunnel.id).await;
        std::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, port))
            .expect("IPv6 listener released");
    }

    #[test]
    fn runtime_event_history_evicts_old_entries() {
        let (mut manager, handle) = TunnelManager::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Arc::new(EmptyCredentials),
        )
        .expect("manager");
        for index in 0..=MAX_LOG_EVENTS {
            manager.push_log(
                LogLevel::Info,
                LogSource::Tunnel,
                index.to_string(),
                "Lifecycle",
                "Connected",
            );
        }
        let logs = handle.subscribe_logs();
        let snapshot = logs.borrow();
        assert_eq!(snapshot.len(), MAX_LOG_EVENTS);
        assert_eq!(snapshot.front().expect("oldest event").subject, "1");
        assert_eq!(
            snapshot.back().expect("newest event").subject,
            MAX_LOG_EVENTS.to_string()
        );
    }

    #[test]
    fn group_and_label_edits_preserve_a_running_tunnel() {
        let old = TunnelConfig {
            id: TunnelId("one".into()),
            name: "one".into(),
            group_id: None,
            mode: TunnelMode::Dynamic,
            jump_chain: vec![],
            local: LocalEndpoint {
                host: "127.0.0.1".into(),
                port: 1080,
            },
            remote: None,
            auto_start: false,
            reconnect: RetryPolicy::default(),
            description: String::new(),
        };
        let mut edited = old.clone();
        edited.name = "renamed".into();
        edited.group_id = Some(GroupId("other".into()));
        edited.auto_start = true;
        edited.description = "notes".into();
        assert!(same_tunnel_runtime(&old, &edited));
        edited.local.port = 1081;
        assert!(!same_tunnel_runtime(&old, &edited));
    }

    #[tokio::test]
    async fn saved_configuration_is_reloaded_by_manager() {
        let directory = TempDir::new().expect("temporary directory");
        let (manager, handle) = TunnelManager::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Arc::new(EmptyCredentials),
        )
        .expect("manager");
        let task = tokio::spawn(manager.run());
        let (reply, answer) = oneshot::channel();
        let app = AppSettings {
            minimize_to_tray: true,
            ..AppSettings::default()
        };
        assert!(
            handle
                .try_send(CoreCommand::SaveConfig {
                    directory: directory.path().to_path_buf(),
                    config: DomainConfig {
                        app,
                        hosts: Vec::new(),
                        groups: Vec::new(),
                        tunnels: Vec::new()
                    },
                    secret: None,
                    reply,
                })
                .is_ok()
        );
        assert!(answer.await.expect("save result").is_ok());
        assert!(
            ConfigStore::new(directory.path().to_path_buf())
                .load()
                .expect("load saved config")
                .app
                .minimize_to_tray
        );
        handle.shutdown();
        task.await.expect("manager shutdown");
    }

    #[tokio::test]
    async fn save_uses_the_same_canonical_key_path_as_reload() {
        let directory = TempDir::new().expect("temporary directory");
        let (manager, handle) = TunnelManager::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Arc::new(EmptyCredentials),
        )
        .expect("manager");
        let task = tokio::spawn(manager.run());
        let mut host = test_host();
        host.auth = AuthConfig::PrivateKey {
            key_path: PathBuf::from("~/.ssh/id_tunnelwarden_test"),
            passphrase_ref: None,
        };
        let (reply, answer) = oneshot::channel();
        handle
            .try_send(CoreCommand::SaveConfig {
                directory: directory.path().to_path_buf(),
                config: DomainConfig {
                    app: AppSettings::default(),
                    hosts: vec![host],
                    groups: Vec::new(),
                    tunnels: Vec::new(),
                },
                secret: None,
                reply,
            })
            .expect("queue save");
        answer.await.expect("save reply").expect("save succeeds");
        let in_memory = handle.subscribe_config();
        let reloaded = ConfigStore::new(directory.path().to_path_buf())
            .load()
            .expect("reload config");
        fn key_path(host: &SshHost) -> &std::path::Path {
            match &host.auth {
                AuthConfig::PrivateKey { key_path, .. } => key_path,
                _ => panic!("expected private key"),
            }
        }
        assert_eq!(
            key_path(&in_memory.borrow().hosts[0]),
            key_path(&reloaded.hosts[0])
        );
        assert!(!key_path(&reloaded.hosts[0]).starts_with("~"));
        handle.shutdown();
        task.await.expect("manager shutdown");
    }

    #[tokio::test]
    async fn host_key_prompt_reaches_ui_and_resolution_returns_to_handshake() {
        let (manager, handle) = TunnelManager::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Arc::new(EmptyCredentials),
        )
        .expect("manager");
        let sender = manager.host_key_sender.clone();
        let task = tokio::spawn(manager.run());
        let mut views = handle.subscribe_host_keys();
        let (reply, decision) = oneshot::channel();
        assert!(
            sender
                .try_send(HostKeyPrompt {
                    host: "example.test".into(),
                    port: 22,
                    algorithm: "ssh-ed25519".into(),
                    fingerprint: "SHA256:test".into(),
                    reply,
                })
                .is_ok()
        );
        views.changed().await.expect("prompt published");
        let id = views.borrow_and_update()[0].id;
        assert!(
            handle
                .try_send(CoreCommand::ResolveHostKey {
                    id,
                    decision: HostKeyDecision::TrustOnce
                })
                .is_ok()
        );
        assert_eq!(
            decision.await.expect("handshake decision"),
            HostKeyDecision::TrustOnce
        );
        views.changed().await.expect("prompt removed");
        assert!(views.borrow_and_update().is_empty());
        handle.shutdown();
        task.await.expect("manager shutdown");
    }
}
