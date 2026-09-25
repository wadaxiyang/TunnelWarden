use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use config_store::{
    AppSettings, ConfigDocument, ConfigStore, DomainConfig, SaveOutcome, SecretStore,
    SshImportPreview, preview_ssh_config, set_run_at_login,
};
use ssh_engine::{
    HopSpec, HostKeyApproval, HostKeyDecision, HostKeyPrompt, InteractiveQuestion,
    KeyboardInteractiveApproval, KeyboardInteractivePrompt, MAX_ANSWER_BYTES, SshChain,
    SshChainError, SshCredential,
};
use thiserror::Error;
use tokio::{
    sync::{Mutex, Notify, mpsc, oneshot, watch},
    task::{JoinHandle, JoinSet},
    time::{Instant, interval, timeout_at},
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
const MAX_INTERACTIVE_PROMPTS: usize = 16;
const MAX_FILE_JOBS: usize = 4;
const MAX_LOG_EVENTS: usize = 500;
const MAX_LOG_TEXT_CHARS: usize = 256;

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
    pub message: Arc<str>,
}

pub type LogSnapshot = VecDeque<Arc<LogEvent>>;

#[derive(Clone, Debug)]
pub struct HostKeyPromptView {
    pub id: u64,
    pub tunnel_id: String,
    pub host_id: String,
    pub generation: u64,
    pub attempt: u64,
    pub hop: usize,
    pub expires_at: Instant,
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug)]
pub struct InteractivePromptView {
    pub id: u64,
    pub tunnel_id: String,
    pub generation: u64,
    pub attempt: u64,
    pub hop: usize,
    pub host: String,
    pub port: u16,
    pub round: usize,
    pub name: String,
    pub instructions: String,
    pub questions: Vec<InteractiveQuestion>,
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
        stop_running_on_commit: bool,
        reply: oneshot::Sender<Result<SaveReceipt, String>>,
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
        generation: u64,
        decision: HostKeyDecision,
    },
    ResolveInteractive {
        id: u64,
        generation: u64,
        answers: Option<Zeroizing<Vec<String>>>,
    },
    PromptWindowOpened,
    PromptWindowClosed,
}

pub struct SecretUpdate {
    /// A new keyring reference. Existing secrets remain available on failure.
    pub reference: SecretRef,
    pub value: Zeroizing<String>,
}

pub struct SaveReceipt {
    pub canonical_config: DomainConfig,
    pub warnings: Vec<String>,
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
    interactive: watch::Receiver<Arc<Vec<InteractivePromptView>>>,
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
    pub fn subscribe_interactive(&self) -> watch::Receiver<Arc<Vec<InteractivePromptView>>> {
        self.interactive.clone()
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

struct PendingInteractive {
    view: InteractivePromptView,
    reply: oneshot::Sender<Option<Zeroizing<Vec<String>>>>,
}

struct PendingSave {
    config: DomainConfig,
    stop_running_on_commit: bool,
    reply: oneshot::Sender<Result<SaveReceipt, String>>,
}

enum FileJobResult {
    Imported(Result<DomainConfig, String>),
    SshPreview(Result<SshImportPreview, String>),
    Exported(Result<(), String>),
}

enum FileJobReply {
    Imported(oneshot::Sender<Result<DomainConfig, String>>),
    SshPreview(oneshot::Sender<Result<SshImportPreview, String>>),
    Exported(oneshot::Sender<Result<(), String>>),
}

impl FileJobReply {
    fn finish(self, result: FileJobResult) {
        match (self, result) {
            (Self::Imported(reply), FileJobResult::Imported(result)) => {
                let _ = reply.send(result);
            }
            (Self::SshPreview(reply), FileJobResult::SshPreview(result)) => {
                let _ = reply.send(result);
            }
            (Self::Exported(reply), FileJobResult::Exported(result)) => {
                let _ = reply.send(result);
            }
            _ => unreachable!("file job result must match its owner"),
        }
    }

    fn fail(self, reason: &str) {
        match self {
            Self::Imported(reply) => {
                let _ = reply.send(Err(reason.to_owned()));
            }
            Self::SshPreview(reply) => {
                let _ = reply.send(Err(reason.to_owned()));
            }
            Self::Exported(reply) => {
                let _ = reply.send(Err(reason.to_owned()));
            }
        }
    }
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
    save_tasks: JoinSet<Result<SaveOutcome, String>>,
    pending_save: Option<PendingSave>,
    file_tasks: JoinSet<FileJobResult>,
    pending_file_jobs: HashMap<tokio::task::Id, FileJobReply>,
    shutdown: CancellationToken,
    auto_start: Vec<TunnelId>,
    host_key_sender: mpsc::Sender<HostKeyPrompt>,
    host_key_requests: mpsc::Receiver<HostKeyPrompt>,
    host_key_views: watch::Sender<Arc<Vec<HostKeyPromptView>>>,
    interactive_sender: mpsc::Sender<KeyboardInteractivePrompt>,
    interactive_requests: mpsc::Receiver<KeyboardInteractivePrompt>,
    interactive_views: watch::Sender<Arc<Vec<InteractivePromptView>>>,
    config_views: watch::Sender<Arc<DomainConfig>>,
    pending_host_keys: Vec<PendingHostKey>,
    pending_interactive: Vec<PendingInteractive>,
    next_host_key_id: u64,
    next_interactive_id: u64,
    next_generation: u64,
    prompt_windows: usize,
    prompt_window_closed: bool,
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
        let (interactive_sender, interactive_requests) = mpsc::channel(MAX_INTERACTIVE_PROMPTS);
        let (interactive_views, interactive_rx) = watch::channel(Arc::new(Vec::new()));
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
                save_tasks: JoinSet::new(),
                pending_save: None,
                file_tasks: JoinSet::new(),
                pending_file_jobs: HashMap::new(),
                shutdown: shutdown.clone(),
                auto_start,
                host_key_sender,
                host_key_requests,
                host_key_views,
                interactive_sender,
                interactive_requests,
                interactive_views,
                config_views,
                pending_host_keys: Vec::new(),
                pending_interactive: Vec::new(),
                next_host_key_id: 1,
                next_interactive_id: 1,
                next_generation: 1,
                prompt_windows: 0,
                prompt_window_closed: false,
                app_known_hosts_path: None,
                host_key_save_lock: Arc::new(Mutex::new(())),
            },
            ManagerHandle {
                commands: commands_tx,
                snapshots: snapshots_rx,
                logs: log_rx,
                host_keys: host_key_rx,
                interactive: interactive_rx,
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
                completed = self.file_tasks.join_next_with_id(), if !self.pending_file_jobs.is_empty() => {
                    match completed {
                        Some(Ok((id, result))) => {
                            if let Some(reply) = self.pending_file_jobs.remove(&id) {
                                reply.finish(result);
                            }
                        }
                        Some(Err(error)) => {
                            if let Some(reply) = self.pending_file_jobs.remove(&error.id()) {
                                reply.fail(&format!("File operation failed: {error}"));
                            }
                        }
                        None => {
                            for (_, reply) in self.pending_file_jobs.drain() {
                                reply.fail("File operation ended without a result");
                            }
                        }
                    }
                },
                completed = self.save_tasks.join_next(), if self.pending_save.is_some() => {
                    if let Some(pending) = self.pending_save.take() {
                        match completed {
                            Some(Ok(Ok(outcome))) => {
                                self.replace_config(pending.config.clone(), pending.stop_running_on_commit).await;
                                let _ = pending.reply.send(Ok(SaveReceipt {
                                    canonical_config: pending.config,
                                    warnings: outcome.warnings,
                                }));
                            }
                            Some(Ok(Err(error))) => { let _ = pending.reply.send(Err(error)); }
                            Some(Err(error)) => {
                                let _ = pending.reply.send(Err(format!(
                                    "Configuration commit status is unknown; reload the file before retrying: {error}"
                                )));
                            }
                            None => { let _ = pending.reply.send(Err("Configuration worker ended without a result".into())); }
                        }
                    }
                },
                prompt = self.host_key_requests.recv() => {
                    if let Some(prompt) = prompt { self.queue_host_key(prompt); }
                },
                prompt = self.interactive_requests.recv() => {
                    if let Some(prompt) = prompt { self.queue_interactive(prompt); }
                },
                _ = poll.tick(), if !self.active.is_empty() => self.refresh().await,
                command = self.commands.recv() => match command {
                    Some(CoreCommand::StartAll) => {
                        let ids: Vec<_> = self.tunnels.keys().cloned().collect();
                        for id in ids { self.start(&id).await; }
                    }
                    Some(CoreCommand::StopAll) => {
                        let ids: Vec<_> = self.tunnels.keys().cloned().collect();
                        self.stop_many(ids).await;
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
                        self.stop_many(ids).await;
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
                    Some(CoreCommand::SaveConfig { directory, config, secret, stop_running_on_commit, reply }) => {
                        if self.pending_save.is_some() {
                            let _ = reply.send(Err("Another configuration save is in progress".into()));
                            continue;
                        }
                        match ConfigDocument::try_from(config).and_then(ConfigDocument::into_domain) {
                            Ok(config) => {
                                let old_run_at_login = self.config_views.borrow().app.run_at_startup;
                                let to_write = config.clone();
                                self.save_tasks.spawn_blocking(move || {
                                    Self::persist_config_blocking(directory, to_write, secret, old_run_at_login)
                                });
                                self.pending_save = Some(PendingSave { config, stop_running_on_commit, reply });
                            }
                            Err(error) => { let _ = reply.send(Err(error.to_string())); }
                        }
                    }
                    Some(CoreCommand::ImportConfig { path, reply }) => {
                        if self.pending_file_jobs.len() >= MAX_FILE_JOBS {
                            let _ = reply.send(Err("Too many file operations are in progress".into()));
                        } else {
                            let task = self.file_tasks.spawn_blocking(move || FileJobResult::Imported(
                                ConfigStore::import_file(&path).map_err(|error| error.to_string())
                            ));
                            self.pending_file_jobs.insert(task.id(), FileJobReply::Imported(reply));
                        }
                    }
                    Some(CoreCommand::PreviewSshConfig { path, reply }) => {
                        if self.pending_file_jobs.len() >= MAX_FILE_JOBS {
                            let _ = reply.send(Err("Too many file operations are in progress".into()));
                        } else {
                            let task = self.file_tasks.spawn_blocking(move || FileJobResult::SshPreview(
                                preview_ssh_config(&path).map_err(|error| error.to_string())
                            ));
                            self.pending_file_jobs.insert(task.id(), FileJobReply::SshPreview(reply));
                        }
                    }
                    Some(CoreCommand::ExportConfig { path, config, reply }) => {
                        if self.pending_file_jobs.len() >= MAX_FILE_JOBS {
                            let _ = reply.send(Err("Too many file operations are in progress".into()));
                        } else {
                            let task = self.file_tasks.spawn_blocking(move || FileJobResult::Exported(
                                ConfigStore::export_file(&path, config).map_err(|error| error.to_string())
                            ));
                            self.pending_file_jobs.insert(task.id(), FileJobReply::Exported(reply));
                        }
                    }
                    Some(CoreCommand::ResolveHostKey { id, generation, decision }) => {
                        if let Some(index) = self.pending_host_keys.iter().position(|prompt| prompt.view.id == id && prompt.view.generation == generation) {
                            let pending = self.pending_host_keys.remove(index);
                            if !pending.reply.is_closed() && pending.view.expires_at > Instant::now() {
                                let _ = pending.reply.send(decision);
                            }
                            self.publish_host_keys();
                        }
                    }
                    Some(CoreCommand::ResolveInteractive { id, generation, answers }) => {
                        if let Some(index) = self.pending_interactive.iter().position(|prompt| prompt.view.id == id && prompt.view.generation == generation) {
                            let pending = self.pending_interactive.remove(index);
                            if !pending.reply.is_closed() && answers.as_ref().is_none_or(|values| values.len() == pending.view.questions.len() && values.iter().all(|value| value.len() <= MAX_ANSWER_BYTES)) {
                                let _ = pending.reply.send(answers);
                            }
                            self.publish_interactive();
                        }
                    }
                    Some(CoreCommand::PromptWindowOpened) => {
                        self.prompt_windows = self.prompt_windows.saturating_add(1);
                        self.prompt_window_closed = false;
                    }
                    Some(CoreCommand::PromptWindowClosed) => {
                        self.prompt_windows = self.prompt_windows.saturating_sub(1);
                        if self.prompt_windows == 0 {
                            self.prompt_window_closed = true;
                            self.pending_host_keys.clear();
                            self.publish_host_keys();
                            self.pending_interactive.clear();
                            self.publish_interactive();
                        }
                    }
                    None => break,
                },
            }
        }
        for active in self.active.values() {
            active.cancellation.cancel();
        }
        self.pending_host_keys.clear();
        self.pending_interactive.clear();
        let deadline = Instant::now() + STOP_DEADLINE;
        for (_, mut active) in self.active.drain() {
            if timeout_at(deadline, &mut active.task).await.is_err() {
                active.task.abort();
                let _ = active.task.await;
            }
        }
        if let Some(pending) = self.pending_save.take() {
            let result = timeout_at(deadline, self.save_tasks.join_next()).await;
            let message = match result {
                Ok(Some(Ok(Ok(_)))) => {
                    "Configuration was committed while the runtime was closing; reload it on next start"
                }
                Ok(Some(Ok(Err(_)))) => "Configuration save failed while the runtime was closing",
                _ => {
                    "Configuration commit status is unknown during shutdown; inspect the file on next start"
                }
            };
            let _ = pending.reply.send(Err(message.into()));
        }
        for (_, reply) in self.pending_file_jobs.drain() {
            reply.fail("File operation outcome is unknown during shutdown; inspect the destination before retrying");
        }
        self.file_tasks.abort_all();
        while !self.file_tasks.is_empty() {
            if timeout_at(deadline, self.file_tasks.join_next())
                .await
                .is_err()
            {
                break;
            }
        }
    }

    fn queue_host_key(&mut self, prompt: HostKeyPrompt) {
        if self.prompt_window_closed
            || prompt.reply.is_closed()
            || prompt.expires_at <= Instant::now()
            || self.pending_host_keys.len() >= MAX_HOST_KEY_PROMPTS
        {
            let _ = prompt.reply.send(HostKeyDecision::Cancel);
            return;
        }
        let view = HostKeyPromptView {
            id: self.next_host_key_id,
            tunnel_id: prompt.tunnel_id,
            host_id: prompt.host_id,
            generation: prompt.generation,
            attempt: prompt.attempt,
            hop: prompt.hop,
            expires_at: prompt.expires_at,
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

    fn queue_interactive(&mut self, prompt: KeyboardInteractivePrompt) {
        if self.prompt_window_closed
            || prompt.reply.is_closed()
            || self.pending_interactive.len() >= MAX_INTERACTIVE_PROMPTS
        {
            let _ = prompt.reply.send(None);
            return;
        }
        let view = InteractivePromptView {
            id: self.next_interactive_id,
            tunnel_id: prompt.tunnel_id,
            generation: prompt.generation,
            attempt: prompt.attempt,
            hop: prompt.hop,
            host: prompt.host,
            port: prompt.port,
            round: prompt.round,
            name: prompt.name,
            instructions: prompt.instructions,
            questions: prompt.questions,
        };
        self.next_interactive_id = self.next_interactive_id.wrapping_add(1).max(1);
        self.pending_interactive.push(PendingInteractive {
            view,
            reply: prompt.reply,
        });
        self.publish_interactive();
    }

    fn publish_interactive(&self) {
        self.interactive_views.send_replace(Arc::new(
            self.pending_interactive
                .iter()
                .map(|pending| pending.view.clone())
                .collect(),
        ));
    }

    fn persist_config_blocking(
        directory: PathBuf,
        config: DomainConfig,
        secret: Option<SecretUpdate>,
        old_run_at_login: bool,
    ) -> Result<SaveOutcome, String> {
        let run_at_login = config.app.run_at_startup;
        let document = ConfigDocument::try_from(config).map_err(|error| error.to_string())?;
        if let Some(secret) = &secret {
            SecretStore::save(&secret.reference, &secret.value)
                .map_err(|error| error.to_string())?;
        }
        match ConfigStore::new(directory).save(document) {
            Ok(mut outcome) => {
                if run_at_login != old_run_at_login
                    && let Err(error) = set_run_at_login(run_at_login)
                {
                    outcome.warnings.push(format!(
                        "Configuration saved, but login startup was not applied: {error}"
                    ));
                }
                Ok(outcome)
            }
            Err(error) => {
                if let Some(secret) = &secret {
                    let _ = SecretStore::delete(&secret.reference);
                }
                Err(error.to_string())
            }
        }
    }

    async fn replace_config(&mut self, config: DomainConfig, stop_running: bool) {
        if stop_running {
            self.stop_many(self.active.keys().cloned().collect()).await;
            self.desired_running.clear();
        }
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
        self.stop_many(needs_restart).await;
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
        if let Err(error) = tunnel
            .validate(&self.hosts, &self.groups)
            .and_then(|()| tunnel.validate_exposure())
        {
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
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let approval = app_path.as_ref().map(|path| {
            HostKeyApproval::new(
                self.host_key_sender.clone(),
                path.clone(),
                Arc::clone(&self.host_key_save_lock),
                id.0.clone(),
                generation,
            )
        });
        let interactive =
            KeyboardInteractiveApproval::new(self.interactive_sender.clone(), id.0.clone())
                .with_generation(generation);
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
                let interactive = interactive.clone();
                let task = tokio::spawn(async move {
                    supervisor
                        .run(move || {
                            let hosts = Arc::clone(&hosts);
                            let credentials = Arc::clone(&credentials);
                            let cancellation = connect_cancel.clone();
                            let remote = remote.clone();
                            let known_path = known_path.clone();
                            let approval = approval.clone();
                            let interactive = interactive.clone();
                            async move {
                                let hops =
                                    build_hops(&hosts, credentials, known_path.as_ref()).await?;
                                SshChain::connect_with_prompts(
                                    hops,
                                    remote,
                                    &cancellation,
                                    approval,
                                    Some(interactive),
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
                let interactive = interactive.clone();
                let task = tokio::spawn(async move {
                    supervisor
                        .run(move |session_token| {
                            let hosts = Arc::clone(&hosts);
                            let credentials = Arc::clone(&credentials);
                            let remote = remote.clone();
                            let known_path = known_path.clone();
                            let approval = approval.clone();
                            let interactive = interactive.clone();
                            async move {
                                let hops =
                                    build_hops(&hosts, credentials, known_path.as_ref()).await?;
                                SshChain::connect_with_prompts(
                                    hops,
                                    remote,
                                    &session_token,
                                    approval,
                                    Some(interactive),
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
        self.stop_many(vec![id.clone()]).await;
    }

    async fn stop_many(&mut self, ids: Vec<TunnelId>) {
        let deadline = Instant::now() + STOP_DEADLINE;
        let mut tasks = Vec::with_capacity(ids.len());
        for id in ids {
            self.desired_running.remove(&id);
            let previous_host_keys = self.pending_host_keys.len();
            self.pending_host_keys
                .retain(|pending| pending.view.tunnel_id != id.0);
            if self.pending_host_keys.len() != previous_host_keys {
                self.publish_host_keys();
            }
            let previous = self.pending_interactive.len();
            self.pending_interactive
                .retain(|pending| pending.view.tunnel_id != id.0);
            if self.pending_interactive.len() != previous {
                self.publish_interactive();
            }
            if let Some(active) = self.active.remove(&id) {
                active.cancellation.cancel();
                tasks.push((id, active.task));
            } else {
                self.set_state(&id, SupervisorState::Stopped, None);
            }
        }
        for (id, mut task) in tasks {
            if timeout_at(deadline, &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
            self.set_state(&id, SupervisorState::Stopped, None);
        }
    }

    async fn refresh(&mut self) {
        let previous_prompts = self.pending_host_keys.len();
        self.pending_host_keys
            .retain(|prompt| !prompt.reply.is_closed() && prompt.view.expires_at > Instant::now());
        if self.pending_host_keys.len() != previous_prompts {
            self.publish_host_keys();
        }
        let previous_interactive = self.pending_interactive.len();
        self.pending_interactive
            .retain(|prompt| !prompt.reply.is_closed());
        if self.pending_interactive.len() != previous_interactive {
            self.publish_interactive();
        }
        let mut changed = false;
        let mut transitions = Vec::new();
        let mut finished = Vec::new();
        for (id, active) in &mut self.active {
            if active.state.has_changed().unwrap_or(false) {
                let state = active.state.borrow_and_update().clone();
                if let Some(view) = self.current.get_mut(id) {
                    if loggable_state_changed(&view.state, &state) {
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
            let changed = loggable_state_changed(&view.state, &state);
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
        let (level, stage, message) = match state {
            SupervisorState::Stopped => (LogLevel::Info, "Lifecycle", "Stopped".to_owned()),
            SupervisorState::Connecting { attempt } => (
                LogLevel::Info,
                "SSH",
                format!("Connecting, attempt {attempt}"),
            ),
            SupervisorState::Healthy { remote_port, .. } => (
                LogLevel::Info,
                "Health",
                match remote_port {
                    Some(port) => format!("SSH session healthy; remote port {port}"),
                    None => "SSH session healthy".to_owned(),
                },
            ),
            SupervisorState::Degraded {
                failed_pings,
                reason,
            } => (
                LogLevel::Warning,
                "Health",
                format!("Health degraded after {failed_pings} failed pings: {reason}"),
            ),
            SupervisorState::Reconnecting {
                attempt,
                delay,
                reason,
            } => (
                LogLevel::Warning,
                "Retry",
                format!(
                    "Reconnect attempt {attempt} in {} ms: {reason}",
                    delay.as_millis()
                ),
            ),
            SupervisorState::Blocked { reason } => (
                LogLevel::Error,
                "Lifecycle",
                format!("Tunnel blocked: {reason}"),
            ),
        };
        self.push_log(level, LogSource::Tunnel, id.0.clone(), stage, message);
    }

    fn push_log(
        &mut self,
        level: LogLevel,
        source: LogSource,
        subject: String,
        stage: &'static str,
        message: impl AsRef<str>,
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
            subject: bounded_log_text(&subject, MAX_LOG_TEXT_CHARS),
            stage,
            message: Arc::from(bounded_log_text(message.as_ref(), MAX_LOG_TEXT_CHARS)),
        }));
        self.log_views.send_replace(Arc::new(self.logs.clone()));
    }

    fn publish(&self) {
        self.snapshots.send_replace(Arc::new(self.current.clone()));
    }
}

fn loggable_state_changed(old: &SupervisorState, new: &SupervisorState) -> bool {
    match (old, new) {
        (SupervisorState::Stopped, SupervisorState::Stopped)
        | (SupervisorState::Healthy { .. }, SupervisorState::Healthy { .. }) => false,
        (
            SupervisorState::Connecting { attempt: old },
            SupervisorState::Connecting { attempt: new },
        ) => old != new,
        (
            SupervisorState::Reconnecting {
                attempt: old_attempt,
                delay: old_delay,
                reason: old_reason,
            },
            SupervisorState::Reconnecting {
                attempt: new_attempt,
                delay: new_delay,
                reason: new_reason,
            },
        ) => old_attempt != new_attempt || old_delay != new_delay || old_reason != new_reason,
        (
            SupervisorState::Degraded {
                failed_pings: old_pings,
                reason: old_reason,
            },
            SupervisorState::Degraded {
                failed_pings: new_pings,
                reason: new_reason,
            },
        ) => old_pings != new_pings || old_reason != new_reason,
        (SupervisorState::Blocked { reason: old }, SupervisorState::Blocked { reason: new }) => {
            old != new
        }
        _ => true,
    }
}

fn bounded_log_text(input: &str, limit: usize) -> String {
    let mut chars = input.chars();
    let mut output: String = chars
        .by_ref()
        .take(limit)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if chars.next().is_some() {
        output.push('…');
    }
    output
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
            AuthConfig::KeyboardInteractive => SshCredential::KeyboardInteractive,
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
        && old.exposure_approved == new.exposure_approved
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
    use std::sync::atomic::{AtomicUsize, Ordering};
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
            exposure_approved: false,
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
            .replace_config(
                DomainConfig {
                    app: AppSettings::default(),
                    hosts: vec![test_host()],
                    groups: Vec::new(),
                    tunnels: vec![edited],
                },
                false,
            )
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

    #[tokio::test]
    async fn non_loopback_socks_does_not_bind_without_approval() {
        let reserve = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
        let port = reserve.local_addr().expect("reserved address").port();
        drop(reserve);
        let tunnel = test_tunnel("0.0.0.0", port);
        let (mut manager, _) = TunnelManager::new(
            vec![test_host()],
            Vec::new(),
            vec![tunnel.clone()],
            Arc::new(EmptyCredentials),
        )
        .expect("manager");
        manager.start(&tunnel.id).await;
        assert!(!manager.active.contains_key(&tunnel.id));
        assert!(matches!(
            manager.current[&tunnel.id].state,
            SupervisorState::Blocked { .. }
        ));
        std::net::TcpListener::bind(("0.0.0.0", port)).expect("unapproved listener not bound");
        let mut approved = tunnel.clone();
        approved.exposure_approved = true;
        manager.tunnels.insert(tunnel.id.clone(), approved);
        manager.start(&tunnel.id).await;
        assert!(manager.active.contains_key(&tunnel.id));
        manager
            .replace_config(
                DomainConfig {
                    app: AppSettings::default(),
                    hosts: vec![test_host()],
                    groups: Vec::new(),
                    tunnels: vec![tunnel.clone()],
                },
                false,
            )
            .await;
        assert!(!manager.active.contains_key(&tunnel.id));
        assert!(matches!(
            manager.current[&tunnel.id].state,
            SupervisorState::Blocked { .. }
        ));
        std::net::TcpListener::bind(("0.0.0.0", port)).expect("revoked listener released");
        manager.stop(&tunnel.id).await;
    }

    #[tokio::test]
    async fn imported_config_stops_matching_running_tunnel() {
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
        let mut imported = tunnel.clone();
        imported.auto_start = false;
        manager
            .replace_config(
                DomainConfig {
                    app: AppSettings::default(),
                    hosts: vec![test_host()],
                    groups: Vec::new(),
                    tunnels: vec![imported],
                },
                true,
            )
            .await;
        assert!(!manager.active.contains_key(&tunnel.id));
        assert!(!manager.desired_running.contains(&tunnel.id));
        std::net::TcpListener::bind(("127.0.0.1", port)).expect("import released listener");
    }

    #[tokio::test]
    async fn slow_file_job_does_not_block_stop_command() {
        let reserve = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
        let port = reserve.local_addr().expect("reserved address").port();
        drop(reserve);
        let tunnel = test_tunnel("127.0.0.1", port);
        let (mut manager, handle) = TunnelManager::new(
            vec![test_host()],
            Vec::new(),
            vec![tunnel.clone()],
            Arc::new(EmptyCredentials),
        )
        .expect("manager");
        manager.start(&tunnel.id).await;
        let (reply, _answer) = oneshot::channel();
        let task = manager.file_tasks.spawn_blocking(|| {
            std::thread::sleep(Duration::from_millis(400));
            FileJobResult::Imported(Err("simulated slow file read".into()))
        });
        manager
            .pending_file_jobs
            .insert(task.id(), FileJobReply::Imported(reply));
        let runner = tokio::spawn(manager.run());
        handle
            .try_send(CoreCommand::StopTunnel(tunnel.id.clone()))
            .expect("send stop");
        tokio::time::timeout(Duration::from_millis(250), async {
            loop {
                if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("stop released listener while file job was pending");
        handle.shutdown();
        runner.await.expect("manager stopped");
    }

    #[tokio::test]
    async fn blocked_retry_retries_and_stop_releases_listener() {
        struct CountingCredentials(Arc<AtomicUsize>);
        impl CredentialSource for CountingCredentials {
            fn load(&self, _: &SecretRef) -> Result<Zeroizing<String>, String> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Err("credential unavailable".into())
            }
        }

        let reserve = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
        let port = reserve.local_addr().expect("reserved address").port();
        drop(reserve);
        let tunnel = test_tunnel("127.0.0.1", port);
        let mut host = test_host();
        host.auth = AuthConfig::Password {
            credential_ref: SecretRef("missing".into()),
        };
        let loads = Arc::new(AtomicUsize::new(0));
        let (manager, handle) = TunnelManager::new(
            vec![host],
            Vec::new(),
            vec![tunnel.clone()],
            Arc::new(CountingCredentials(Arc::clone(&loads))),
        )
        .expect("manager");
        let task = tokio::spawn(manager.run());
        let mut snapshots = handle.subscribe();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                snapshots.changed().await.expect("snapshot update");
                if matches!(
                    snapshots.borrow_and_update()[&tunnel.id].state,
                    SupervisorState::Blocked { .. }
                ) {
                    break;
                }
            }
        })
        .await
        .expect("blocked state");
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_err());
        handle
            .try_send(CoreCommand::RetryTunnel(tunnel.id.clone()))
            .expect("retry command");
        tokio::time::timeout(Duration::from_secs(3), async {
            while loads.load(Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("second credential load");
        handle
            .try_send(CoreCommand::StopTunnel(tunnel.id.clone()))
            .expect("stop command");
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                snapshots.changed().await.expect("snapshot update");
                if matches!(
                    snapshots.borrow_and_update()[&tunnel.id].state,
                    SupervisorState::Stopped
                ) {
                    break;
                }
            }
        })
        .await
        .expect("stopped state");
        std::net::TcpListener::bind(("127.0.0.1", port)).expect("listener released");
        handle.shutdown();
        task.await.expect("manager shutdown");
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
    fn repeated_blocked_states_keep_distinct_bounded_reasons() {
        let tunnel = test_tunnel("127.0.0.1", 1080);
        let (mut manager, handle) = TunnelManager::new(
            vec![test_host()],
            Vec::new(),
            vec![tunnel.clone()],
            Arc::new(EmptyCredentials),
        )
        .expect("manager");
        manager.set_state(
            &tunnel.id,
            SupervisorState::Blocked {
                reason: "host key rejected".into(),
            },
            None,
        );
        manager.set_state(
            &tunnel.id,
            SupervisorState::Blocked {
                reason: format!("authentication\n{}", "x".repeat(500)),
            },
            None,
        );
        let logs = handle.subscribe_logs();
        let snapshot = logs.borrow();
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot[0].message.contains("host key rejected"));
        assert!(snapshot[1].message.contains("authentication "));
        assert!(!snapshot[1].message.contains('\n'));
        assert!(snapshot[1].message.chars().count() <= MAX_LOG_TEXT_CHARS + 1);
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
            exposure_approved: false,
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
                    stop_running_on_commit: false,
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
                stop_running_on_commit: false,
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
    async fn committed_save_warning_still_updates_manager_config() {
        let directory = TempDir::new().expect("temporary directory");
        let store = ConfigStore::new(directory.path().to_path_buf());
        store.save(ConfigDocument::default()).expect("initial save");
        let backups = directory.path().join("backups");
        std::fs::create_dir_all(&backups).expect("backup directory");
        std::fs::create_dir(backups.join("config-0000000000000-0.toml"))
            .expect("unremovable old backup");
        for index in 1..=4 {
            std::fs::write(
                backups.join(format!("config-000000000000{index}-0.toml")),
                "old backup",
            )
            .expect("backup fixture");
        }
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
            minimize_to_tray: false,
            ..AppSettings::default()
        };
        handle
            .try_send(CoreCommand::SaveConfig {
                directory: directory.path().to_path_buf(),
                config: DomainConfig {
                    app,
                    hosts: Vec::new(),
                    groups: Vec::new(),
                    tunnels: Vec::new(),
                },
                secret: None,
                stop_running_on_commit: false,
                reply,
            })
            .expect("queue save");
        let receipt = answer.await.expect("save reply").expect("committed save");
        assert!(!receipt.warnings.is_empty());
        assert!(!receipt.canonical_config.app.minimize_to_tray);
        assert!(!handle.subscribe_config().borrow().app.minimize_to_tray);
        assert!(!store.load().expect("committed file").app.minimize_to_tray);
        handle.shutdown();
        task.await.expect("manager shutdown");
    }

    #[tokio::test]
    async fn slow_config_write_does_not_block_stop_command() {
        let directory = TempDir::new().expect("temporary directory");
        ConfigStore::new(directory.path().to_path_buf())
            .save(ConfigDocument::default())
            .expect("initial config");
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.path().join("config.lock"))
            .expect("config lock file");
        lock.lock().expect("hold config write lock");
        let reserve = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve port");
        let port = reserve.local_addr().expect("reserved address").port();
        drop(reserve);
        let tunnel = test_tunnel("127.0.0.1", port);
        let (manager, handle) = TunnelManager::new(
            vec![test_host()],
            Vec::new(),
            vec![tunnel.clone()],
            Arc::new(EmptyCredentials),
        )
        .expect("manager");
        let task = tokio::spawn(manager.run());
        let mut snapshots = handle.subscribe();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                snapshots.changed().await.expect("snapshot update");
                if snapshots.borrow_and_update()[&tunnel.id]
                    .local_addr
                    .is_some()
                {
                    break;
                }
            }
        })
        .await
        .expect("listener bound");
        let (reply, mut answer) = oneshot::channel();
        handle
            .try_send(CoreCommand::SaveConfig {
                directory: directory.path().to_path_buf(),
                config: DomainConfig {
                    app: AppSettings::default(),
                    hosts: vec![test_host()],
                    groups: Vec::new(),
                    tunnels: vec![tunnel.clone()],
                },
                secret: None,
                stop_running_on_commit: false,
                reply,
            })
            .expect("queue save");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut answer)
                .await
                .is_err()
        );
        handle
            .try_send(CoreCommand::StopTunnel(tunnel.id.clone()))
            .expect("queue stop");
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                snapshots.changed().await.expect("snapshot update");
                if matches!(
                    snapshots.borrow_and_update()[&tunnel.id].state,
                    SupervisorState::Stopped
                ) {
                    break;
                }
            }
        })
        .await
        .expect("stop while save waits for lock");
        std::net::TcpListener::bind(("127.0.0.1", port)).expect("listener released");
        drop(lock);
        answer.await.expect("save reply").expect("save completes");
        handle.shutdown();
        task.await.expect("manager shutdown");
    }

    #[tokio::test]
    async fn stop_all_cancels_every_owner_before_waiting_on_cleanup() {
        let tunnels: Vec<_> = (0..3)
            .map(|index| {
                let mut tunnel = test_tunnel("127.0.0.1", 10000 + index);
                tunnel.id = TunnelId(format!("tunnel-{index}"));
                tunnel
            })
            .collect();
        let ids: Vec<_> = tunnels.iter().map(|tunnel| tunnel.id.clone()).collect();
        let (mut manager, _) = TunnelManager::new(
            vec![test_host()],
            Vec::new(),
            tunnels,
            Arc::new(EmptyCredentials),
        )
        .expect("manager");
        let mut cancellations = Vec::new();
        for id in &ids {
            let cancellation = CancellationToken::new();
            let task_cancel = cancellation.clone();
            let task = tokio::spawn(async move {
                task_cancel.cancelled().await;
                tokio::time::sleep(Duration::from_millis(500)).await;
                None
            });
            let (_state_tx, state) = watch::channel(SupervisorState::Connecting { attempt: 1 });
            manager.active.insert(
                id.clone(),
                ActiveTunnel {
                    cancellation: cancellation.clone(),
                    task,
                    state,
                    retry_hint: Arc::new(Notify::new()),
                },
            );
            cancellations.push(cancellation);
        }
        let stop_task = tokio::spawn(async move { manager.stop_many(ids).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(cancellations.iter().all(CancellationToken::is_cancelled));
        tokio::time::timeout(Duration::from_secs(2), stop_task)
            .await
            .expect("shared cleanup wait")
            .expect("stop task");
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
                    tunnel_id: "test-tunnel".into(),
                    host_id: "test-host".into(),
                    generation: 5,
                    attempt: 1,
                    hop: 1,
                    expires_at: Instant::now() + Duration::from_secs(120),
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
                    generation: 5,
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

    #[tokio::test]
    async fn host_key_stale_generation_expiry_and_window_close_cannot_approve() {
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
        handle
            .try_send(CoreCommand::PromptWindowOpened)
            .expect("open window");
        let (expired_reply, expired_answer) = oneshot::channel();
        sender
            .try_send(HostKeyPrompt {
                tunnel_id: "tunnel".into(),
                host_id: "host".into(),
                generation: 1,
                attempt: 1,
                hop: 1,
                expires_at: Instant::now() - Duration::from_millis(1),
                host: "example.test".into(),
                port: 22,
                algorithm: "ssh-ed25519".into(),
                fingerprint: "SHA256:old".into(),
                reply: expired_reply,
            })
            .expect("expired prompt queued");
        assert_eq!(
            expired_answer.await.expect("expired decision"),
            HostKeyDecision::Cancel
        );
        let (first_reply, mut first_answer) = oneshot::channel();
        let (second_reply, second_answer) = oneshot::channel();
        for (generation, fingerprint, reply) in [
            (2, "SHA256:first", first_reply),
            (3, "SHA256:second", second_reply),
        ] {
            sender
                .try_send(HostKeyPrompt {
                    tunnel_id: "tunnel".into(),
                    host_id: "host".into(),
                    generation,
                    attempt: 1,
                    hop: 1,
                    expires_at: Instant::now() + Duration::from_secs(120),
                    host: "example.test".into(),
                    port: 22,
                    algorithm: "ssh-ed25519".into(),
                    fingerprint: fingerprint.into(),
                    reply,
                })
                .expect("prompt queued");
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                views.changed().await.expect("prompt state");
                if views.borrow_and_update().len() == 2 {
                    break;
                }
            }
        })
        .await
        .expect("both prompts visible");
        let id = views.borrow()[0].id;
        handle
            .try_send(CoreCommand::ResolveHostKey {
                id,
                generation: 3,
                decision: HostKeyDecision::TrustOnce,
            })
            .expect("stale command");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut first_answer)
                .await
                .is_err()
        );
        handle
            .try_send(CoreCommand::ResolveHostKey {
                id,
                generation: 2,
                decision: HostKeyDecision::TrustOnce,
            })
            .expect("current command");
        assert_eq!(
            first_answer.await.expect("first decision"),
            HostKeyDecision::TrustOnce
        );
        handle
            .try_send(CoreCommand::PromptWindowClosed)
            .expect("close window");
        assert!(second_answer.await.is_err());
        handle.shutdown();
        task.await.expect("manager shutdown");
    }
}
