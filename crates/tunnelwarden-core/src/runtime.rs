// Copyright (C) 2026 Wilson Glasser and Oryxis contributors
// SPDX-License-Identifier: AGPL-3.0-or-later
// Modified 2026-09-29 for TunnelWarden: replace iced tasks with Tokio, vault
// access with a dial adapter and form errors with per-rule DTOs. Await stop IPC.
// Source: crates/oryxis-app/src/dispatch_port_forwards.rs, app.rs,
// boot/mod.rs, subscription.rs, messages/port_forward.rs
// Oryxis 16ad2af83b424780ff60b6420a6c20a3b30c7143.
//! The Oryxis standalone registry, shared host connections and retry ladder.

use crate::dto::{RuntimeSnapshot, RuntimeTunnel, TunnelStatus};
use oryxis_core::models::port_forward_rule::PortForwardRule;
use oryxis_ssh::{ForwardConn, ForwardSession};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
    time::Instant,
};
use uuid::Uuid;

pub type DialFuture = Pin<Box<dyn Future<Output = Result<ForwardConn, String>> + Send>>;
/// Application credential/host-key adapter. SSH remains entirely in Oryxis.
pub trait ForwardDialer: Send + Sync + 'static {
    fn connect(&self, host_id: Uuid, unattended: bool) -> DialFuture;
    fn cancel_prompts(&self, _host_id: Uuid) {}
}

type Reply = oneshot::Sender<Result<(), String>>;
enum Command {
    Start(Uuid, Reply),
    Stop(Uuid, Reply),
    Configure(Vec<PortForwardRule>, bool, Reply),
    AutoStart(Reply),
    Shutdown(Reply),
    EnsureHostIdle(Uuid, Reply),
}
enum Message {
    ConnReady(Uuid, Result<ForwardConn, String>),
    Started(Uuid, Result<Arc<ForwardSession>, String>),
    Cancelled(Uuid),
    AgentCensus(Vec<String>),
}

// Mechanical iced::Task replacement. It makes no lifecycle/retry decisions.
struct Task<T>(Vec<Pin<Box<dyn Future<Output = T> + Send>>>);
impl<T: Send + 'static> Task<T> {
    fn none() -> Self {
        Self(Vec::new())
    }
    fn done(value: T) -> Self {
        Self(vec![Box::pin(async move { value })])
    }
    fn perform<F, V, M>(future: F, map: M) -> Self
    where
        F: Future<Output = V> + Send + 'static,
        M: FnOnce(V) -> T + Send + 'static,
    {
        Self(vec![Box::pin(async move { map(future.await) })])
    }
    fn batch(tasks: impl IntoIterator<Item = Self>) -> Self {
        Self(tasks.into_iter().flat_map(|t| t.0).collect())
    }
}

#[derive(Clone)]
pub struct TunnelRuntimeManager {
    tx: mpsc::UnboundedSender<Command>,
    snapshot: watch::Receiver<RuntimeSnapshot>,
}
impl TunnelRuntimeManager {
    pub fn new(
        dialer: Arc<dyn ForwardDialer>,
        rules: Vec<PortForwardRule>,
        auto_reconnect: bool,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let state = RuntimeState::new(dialer, rules, auto_reconnect);
        let (updates, snapshot) = watch::channel(state.snapshot());
        tokio::spawn(state.run(rx, updates));
        Self { tx, snapshot }
    }
    pub fn snapshot(&self) -> RuntimeSnapshot {
        self.snapshot.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<RuntimeSnapshot> {
        self.snapshot.clone()
    }
    async fn request(&self, make: impl FnOnce(Reply) -> Command) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(make(tx))
            .map_err(|_| "Runtime closed".to_string())?;
        rx.await.map_err(|_| "Runtime closed".to_string())?
    }
    pub async fn start(&self, id: Uuid) -> Result<(), String> {
        self.request(|r| Command::Start(id, r)).await
    }
    /// Returns after active/attaching sessions have explicitly cancelled.
    pub async fn stop(&self, id: Uuid) -> Result<(), String> {
        self.request(|r| Command::Stop(id, r)).await
    }
    pub async fn restart(&self, id: Uuid) -> Result<(), String> {
        self.stop(id).await?;
        self.start(id).await
    }
    pub async fn configure(
        &self,
        rules: Vec<PortForwardRule>,
        auto_reconnect: bool,
    ) -> Result<(), String> {
        self.request(|r| Command::Configure(rules, auto_reconnect, r))
            .await
    }
    pub async fn auto_start(&self) -> Result<(), String> {
        self.request(Command::AutoStart).await
    }
    pub async fn ensure_host_idle(&self, host: Uuid) -> Result<(), String> {
        self.request(|reply| Command::EnsureHostIdle(host, reply))
            .await
    }
    pub async fn shutdown(&self) -> Result<(), String> {
        for rule in self.snapshot().tunnels {
            self.stop(rule.id).await?;
        }
        self.request(Command::Shutdown).await
    }
}

struct RuntimeState {
    dialer: Arc<dyn ForwardDialer>,
    port_forward_rules: Vec<PortForwardRule>,
    active_forwards: HashMap<Uuid, Arc<ForwardSession>>,
    forward_conns: HashMap<Uuid, PfHostConn>,
    port_forward_starting: HashSet<Uuid>,
    port_forward_retry: HashMap<Uuid, PfRetry>,
    port_forward_agent_watch: Option<PfAgentWatch>,
    auto_reconnect: bool,
    errors: HashMap<Uuid, String>,
    // IPC bookkeeping only: Stop waits for an in-flight attach to cancel
    // before allowing a new Start. No generations or alternate supervisor.
    attaching: HashSet<Uuid>,
    stop_waiters: HashMap<Uuid, Vec<Reply>>,
}
impl RuntimeState {
    fn new(
        dialer: Arc<dyn ForwardDialer>,
        port_forward_rules: Vec<PortForwardRule>,
        auto_reconnect: bool,
    ) -> Self {
        Self {
            dialer,
            port_forward_rules,
            auto_reconnect,
            active_forwards: HashMap::new(),
            forward_conns: HashMap::new(),
            port_forward_starting: HashSet::new(),
            port_forward_retry: HashMap::new(),
            port_forward_agent_watch: None,
            errors: HashMap::new(),
            attaching: HashSet::new(),
            stop_waiters: HashMap::new(),
        }
    }
    fn snapshot(&self) -> RuntimeSnapshot {
        let now = Instant::now();
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        RuntimeSnapshot {
            tunnels: self
                .port_forward_rules
                .iter()
                .map(|r| {
                    let status = if self.active_forwards.contains_key(&r.id) {
                        TunnelStatus::Running
                    } else if self.port_forward_starting.contains(&r.id) {
                        TunnelStatus::Connecting
                    } else if self.port_forward_retry.contains_key(&r.id) {
                        TunnelStatus::Retrying
                    } else if self.errors.contains_key(&r.id) {
                        TunnelStatus::Error
                    } else {
                        TunnelStatus::Stopped
                    };
                    RuntimeTunnel {
                        id: r.id,
                        status,
                        last_error: self.errors.get(&r.id).cloned(),
                        next_retry_at: self.port_forward_retry.get(&r.id).map(|p| {
                            epoch.saturating_add(
                                p.next_at.saturating_duration_since(now).as_millis() as u64,
                            )
                        }),
                    }
                })
                .collect(),
        }
    }
    async fn run(
        mut self,
        mut rx: mpsc::UnboundedReceiver<Command>,
        updates: watch::Sender<RuntimeSnapshot>,
    ) {
        let mut tasks = JoinSet::new();
        // Oryxis subscription.rs: 5s liveness / 15s retry. Empty registries
        // make the callbacks no-ops; the agent census only runs during retry.
        let mut liveness = tokio::time::interval(Duration::from_secs(5));
        let mut retry = tokio::time::interval(Duration::from_secs(15));
        loop {
            let task = tokio::select! {
                cmd = rx.recv() => match cmd {
                    Some(Command::EnsureHostIdle(id, reply)) => {
                        let result = if self.forward_conns.contains_key(&id) { Err("SSH host still has a live or pending connection; stop its tunnels and wait for the connection to finish".into()) } else { Ok(()) };
                        let _ = reply.send(result); Task::none()
                    }
                    Some(Command::Start(id, reply)) => {
                        if !self.port_forward_rules.iter().any(|r| r.id == id) {
                            let _ = reply.send(Err("Tunnel not found".into())); Task::none()
                        } else if self.stop_waiters.contains_key(&id) {
                            let _ = reply.send(Err("Tunnel is stopping; wait for Stop to complete".into())); Task::none()
                        } else {
                            self.errors.remove(&id);
                            let task = self.start_port_forward(id, false);
                            let _ = reply.send(Ok(())); task
                        }
                    }
                    Some(Command::Stop(id, reply)) => self.stop_port_forward(id, reply),
                    Some(Command::Configure(rules, auto_reconnect, reply)) => {
                        let busy_change = self.port_forward_rules.iter().any(|old| {
                            (self.active_forwards.contains_key(&old.id) || self.port_forward_starting.contains(&old.id) || self.stop_waiters.contains_key(&old.id))
                            && !rules.iter().any(|new| new.id == old.id && serde_json::to_value(old).ok() == serde_json::to_value(new).ok())
                        });
                        if busy_change { let _ = reply.send(Err("Stop the tunnel before editing or deleting it".into())); }
                        else {
                            self.port_forward_rules = rules; self.auto_reconnect = auto_reconnect;
                            self.errors.retain(|id, _| self.port_forward_rules.iter().any(|r| r.id == *id));
                            self.port_forward_retry.retain(|id, _| self.port_forward_rules.iter().any(|r| r.id == *id && pf_retry_still_wanted(r.auto_start, auto_reconnect)));
                            let _ = reply.send(Ok(()));
                        }
                        Task::none()
                    }
                    Some(Command::AutoStart(reply)) => {
                        let t = Task::batch(self.auto_start_port_forwards()); let _ = reply.send(Ok(())); t
                    }
                    Some(Command::Shutdown(reply)) => {
                        for session in self.active_forwards.values() { session.cancel().await; }
                        tasks.abort_all(); self.forward_conns.clear();
                        let _ = reply.send(Ok(())); break;
                    }
                    None => { for s in self.active_forwards.values() { s.cancel().await; } break; }
                },
                completed = tasks.join_next(), if !tasks.is_empty() => match completed {
                    Some(Ok(msg)) => self.handle_message(msg),
                    Some(Err(e)) => { tracing::error!("Runtime task failed: {e}"); Task::none() }
                    None => Task::none(),
                },
                _ = liveness.tick() => { self.handle_liveness_tick(); Task::none() },
                _ = retry.tick() => self.handle_port_forward_retry_tick(),
            };
            for future in task.0 {
                tasks.spawn(future);
            }
            let snapshot = self.snapshot();
            updates.send_if_modified(|old| {
                if *old == snapshot {
                    false
                } else {
                    *old = snapshot;
                    true
                }
            });
        }
    }
    fn stop_port_forward(&mut self, id: Uuid, reply: Reply) -> Task<Message> {
        self.port_forward_starting.remove(&id);
        if let Some(rule) = self.port_forward_rules.iter().find(|r| r.id == id) {
            let host_id = rule.host_id;
            if !self
                .port_forward_rules
                .iter()
                .any(|r| r.host_id == host_id && self.port_forward_starting.contains(&r.id))
            {
                self.dialer.cancel_prompts(host_id);
            }
        }
        // A manual stop must never resurrect on a later retry tick.
        self.port_forward_retry.remove(&id);
        self.errors.remove(&id);
        if let Some(waiters) = self.stop_waiters.get_mut(&id) {
            waiters.push(reply);
            return Task::none();
        }
        if let Some(session) = self.active_forwards.remove(&id) {
            self.stop_waiters.insert(id, vec![reply]);
            self.pf_gc_host_conns();
            return Task::perform(async move { session.cancel().await }, move |_| {
                Message::Cancelled(id)
            });
        }
        if self.attaching.contains(&id) {
            self.stop_waiters.insert(id, vec![reply]);
        } else {
            let _ = reply.send(Ok(()));
        }
        self.pf_gc_host_conns();
        Task::none()
    }
    fn finish_stop(&mut self, id: Uuid) {
        if let Some(waiters) = self.stop_waiters.remove(&id) {
            for r in waiters {
                let _ = r.send(Ok(()));
            }
        }
    }
    fn handle_message(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::ConnReady(host, res) => self.handle_port_forward_conn_ready(host, res),
            Message::AgentCensus(agents) => self.handle_port_forward_agent_census(agents),
            Message::Cancelled(id) => {
                self.finish_stop(id);
                Task::none()
            }
            Message::Started(id, res) => {
                self.attaching.remove(&id);
                // remove=false means Stop withdrew this in-flight intent.
                let was_starting = self.port_forward_starting.remove(&id);
                match res {
                    Ok(session) => {
                        if was_starting && self.port_forward_rules.iter().any(|r| r.id == id) {
                            self.active_forwards.insert(id, session);
                            self.port_forward_retry.remove(&id);
                            self.errors.remove(&id);
                        } else {
                            // A stopped/deleted -R must release its server bind,
                            // even when siblings keep the shared connection up.
                            self.pf_gc_host_conns();
                            return Task::perform(
                                async move { session.cancel().await },
                                move |_| Message::Cancelled(id),
                            );
                        }
                    }
                    Err(e) => {
                        if was_starting {
                            self.pf_mark_retry_pending(id, PfRetryCause::StartFailed);
                            self.errors.insert(id, e);
                        }
                        self.finish_stop(id);
                        self.pf_gc_host_conns();
                    }
                }
                Task::none()
            }
        }
    }
    fn handle_liveness_tick(&mut self) {
        // Drop forwards whose underlying connection died; never lie "running".
        let dead: Vec<Uuid> = self
            .active_forwards
            .iter()
            .filter(|(_, s)| !s.is_alive())
            .map(|(id, _)| *id)
            .collect();
        for id in dead {
            self.active_forwards.remove(&id);
            self.pf_mark_retry_pending(id, PfRetryCause::Dropped);
            self.errors.insert(
                id,
                "SSH connection closed; check the server and network".into(),
            );
        }
        self.pf_gc_host_conns();
    }
    fn start_port_forward(&mut self, id: Uuid, boot_auto_start: bool) -> Task<Message> {
        if self.active_forwards.contains_key(&id)
            || self.port_forward_starting.contains(&id)
            || self.stop_waiters.contains_key(&id)
        {
            return Task::none();
        }
        let Some(rule) = self.port_forward_rules.iter().find(|r| r.id == id) else {
            return Task::none();
        };
        let host_id = rule.host_id;
        match self.forward_conns.get_mut(&host_id) {
            Some(PfHostConn::Up(fconn)) if fconn.is_alive() => {
                let fconn = fconn.clone();
                self.port_forward_starting.insert(id);
                return self.attach_port_forward_task(fconn, id);
            }
            Some(PfHostConn::Up(_)) => {
                self.forward_conns.remove(&host_id);
            }
            Some(PfHostConn::Connecting { pending }) => {
                if !pending.contains(&id) {
                    pending.push(id);
                }
                self.port_forward_starting.insert(id);
                return Task::none();
            }
            None => {}
        }
        self.port_forward_starting.insert(id);
        self.forward_conns
            .insert(host_id, PfHostConn::Connecting { pending: vec![id] });
        Task::perform(self.dialer.connect(host_id, boot_auto_start), move |r| {
            Message::ConnReady(host_id, r)
        })
    }
    /// Task that attaches a rule onto its host's live shared connection:
    /// binds the listener / requests the server-side bind and reports back
    /// as `PortForwardStarted`, exactly like a dedicated dial used to. A
    /// rule deleted while queued has already left `port_forward_starting`,
    /// so the silent no-op is correct.
    fn attach_port_forward_task(&mut self, fconn: ForwardConn, id: Uuid) -> Task<Message> {
        let Some(rule) = self.port_forward_rules.iter().find(|r| r.id == id).cloned() else {
            return Task::none();
        };
        self.attaching.insert(id);
        Task::perform(
            async move {
                fconn
                    .attach(&rule)
                    .await
                    .map(Arc::new)
                    .map_err(|e| e.to_string())
            },
            move |res| Message::Started(id, res),
        )
    }

    /// The shared dial for `host_id` settled: on success, store the live
    /// connection and attach every rule that queued while it was in
    /// flight; on failure, fan the one error out to each queued rule so
    /// all of them get the exact `PortForwardStarted(Err)` bookkeeping
    /// (error surface, retry ladder) a dedicated dial would have given.
    fn handle_port_forward_conn_ready(
        &mut self,
        host_id: Uuid,
        res: Result<ForwardConn, String>,
    ) -> Task<Message> {
        let pending = match self.forward_conns.remove(&host_id) {
            Some(PfHostConn::Connecting { pending }) => pending,
            // One dial per host at a time, so this can only be a stray
            // late completion; keep whatever state superseded it.
            Some(up @ PfHostConn::Up(_)) => {
                self.forward_conns.insert(host_id, up);
                Vec::new()
            }
            None => Vec::new(),
        };
        match res {
            Ok(fconn) => {
                // Only rules still wanted: a stop or delete during the dial
                // pulls the id from `port_forward_starting`, which is the
                // record of intent here.
                let due: Vec<Uuid> = pending
                    .into_iter()
                    .filter(|rid| self.port_forward_starting.contains(rid))
                    .collect();
                if due.is_empty() {
                    // Everyone gave up while dialing: let the fresh
                    // connection drop and close.
                    return Task::none();
                }
                let mut tasks: Vec<Task<Message>> = due
                    .iter()
                    .map(|rid| self.attach_port_forward_task(fconn.clone(), *rid))
                    .collect();
                self.forward_conns.insert(host_id, PfHostConn::Up(fconn));
                // This dial proves the route is back: pull every other
                // pending forward off its backoff too (issue #144). The
                // rules attaching above are still in
                // `port_forward_starting`, so the kick skips them.
                tasks.push(self.pf_kick_pending_retries());
                Task::batch(tasks)
            }
            Err(e) => {
                // Same still-wanted filter as the Ok arm: a rule stopped
                // mid-dial already left `port_forward_starting`, and
                // handing it a failure now would re-arm the retry ladder
                // the stop just cleared.
                Task::batch(
                    pending
                        .into_iter()
                        .filter(|rid| self.port_forward_starting.contains(rid))
                        .map(|rid| Task::done(Message::Started(rid, Err(e.clone())))),
                )
            }
        }
    }

    /// Drop the shared connection of any host that no longer has a live
    /// or starting forward: the last toggle-off must close the SSH
    /// connection, not leave it idling. `Connecting` entries are exempt,
    /// their `PortForwardConnReady` is still in flight and settles them.
    fn pf_gc_host_conns(&mut self) {
        let rules = &self.port_forward_rules;
        let active = &self.active_forwards;
        let starting = &self.port_forward_starting;
        self.forward_conns.retain(|host_id, state| match state {
            PfHostConn::Connecting { .. } => true,
            PfHostConn::Up(_) => {
                let on_this_host =
                    |rid: &Uuid| rules.iter().any(|r| r.id == *rid && r.host_id == *host_id);
                active.keys().any(&on_this_host) || starting.iter().any(on_this_host)
            }
        });
    }

    /// Start every rule marked `auto_start`. Called once after the vault is
    /// unlocked (boot or `VaultUnlock`). Returns the connect tasks to batch
    /// into the caller's task list.
    pub(crate) fn auto_start_port_forwards(&mut self) -> Vec<Task<Message>> {
        let ids: Vec<Uuid> = self
            .port_forward_rules
            .iter()
            .filter(|r| r.auto_start)
            .map(|r| r.id)
            .collect();
        ids.into_iter()
            .map(|id| self.start_port_forward(id, true))
            .collect()
    }

    /// Mark a failed/dropped rule as pending and schedule its first
    /// re-attempt. No-op for a rule that nothing opted into self-healing
    /// (per-cause gates below) or that already has a pending retry
    /// (`or_insert` so a repeated failure never resets a backoff that's
    /// already climbing).
    fn pf_mark_retry_pending(&mut self, id: Uuid, cause: PfRetryCause) {
        let is_auto = self
            .port_forward_rules
            .iter()
            .any(|r| r.id == id && r.auto_start);
        let allowed = match cause {
            // A forward that was UP and fell over is the same event as a
            // host disconnecting, so it answers to the same setting the
            // user already set for that ("Auto-reconnect on disconnect",
            // on by default). Gating this on `auto_start` was wrong:
            // auto_start says "bring it up at launch", which is a
            // different question from "keep it up", and it left a manually
            // started forward silently dead after a network blip while
            // every terminal tab climbed back on its own.
            PfRetryCause::Dropped => self.auto_reconnect || is_auto,
            // A start the user just watched fail shows its error instead
            // of looping behind their back. Only an auto_start rule, whose
            // attempt nobody is watching, retries from here.
            PfRetryCause::StartFailed => is_auto,
        };
        if !allowed {
            return;
        }
        self.port_forward_retry
            .entry(id)
            .or_insert_with(|| PfRetry {
                next_at: Instant::now() + pf_retry_backoff(0),
                attempts: 0,
            });
    }

    /// One beat of the self-healing loop: re-attempt whatever is due, and
    /// ask the agents what they hold so an arriving key can shortcut the
    /// backoff. Driven by the `PortForwardRetryTick` subscription, which
    /// only mounts while `port_forward_retry` is non-empty and the vault
    /// is unlocked, so neither half ever runs on an idle app.
    fn handle_port_forward_retry_tick(&mut self) -> Task<Message> {
        if self.port_forward_retry.is_empty() {
            // Nothing pending: drop the baseline so the next retry run
            // compares against a fresh reading instead of one taken an
            // arbitrarily long time ago (agents come and go meanwhile).
            self.port_forward_agent_watch = None;
            return Task::none();
        }
        // Ask the agents what they are holding, off the update loop (a
        // dial + LIST per endpoint); the answer comes back as
        // `PortForwardAgentCensus` and can only make rules due EARLIER,
        // so the backoff pass below never waits on it.
        Task::batch([
            Task::perform(oryxis_ssh::agent_key_census(), |census| {
                Message::AgentCensus(census)
            }),
            self.pf_issue_due_retries(Instant::now()),
        ])
    }

    /// Fold a fresh agent census into the watch and, when the keys moved,
    /// re-attempt every pending rule at once.
    fn handle_port_forward_agent_census(&mut self, agents: Vec<String>) -> Task<Message> {
        if self.port_forward_retry.is_empty() {
            self.port_forward_agent_watch = None;
            return Task::none();
        }
        let watch = PfAgentWatch {
            agents,
            added_keys: 0, // TunnelWarden does not run its own SSH agent server.
        };
        let changed = pf_agent_changed(self.port_forward_agent_watch.as_ref(), &watch);
        self.port_forward_agent_watch = Some(watch);
        if !changed {
            return Task::none();
        }
        // A key just arrived (or left): the most likely cause of the
        // failure is gone, so nothing waits out a backoff that has
        // already climbed to the 120 s ceiling (issue #101). Every
        // pending rule is kicked, not just the agent-authenticated ones:
        // the reading only moves on a real agent event, and one extra
        // dial is cheap next to leaving a forward down for two minutes.
        tracing::info!("ssh-agent keys changed, re-attempting pending port forwards");
        self.pf_reissue_pending_now()
    }

    /// Pull every pending rule off its backoff and re-attempt it now.
    /// Shared by the two "conditions changed" signals: the agent census
    /// (a key just arrived) and a fresh connection coming up (the
    /// network is back).
    fn pf_reissue_pending_now(&mut self) -> Task<Message> {
        let now = Instant::now();
        for retry in self.port_forward_retry.values_mut() {
            retry.next_at = now;
            // New conditions, fresh ladder: a forward that fails again
            // from here starts back at the short intervals instead of
            // resuming the old ceiling.
            retry.attempts = 0;
        }
        self.pf_issue_due_retries(now)
    }

    /// A connection just dialed successfully somewhere in the app,
    /// which after a local outage is the proof that the network is
    /// back (issue #144): re-attempt every pending forward right away
    /// instead of letting each wait out a backoff that may have
    /// climbed to the 120 s ceiling. Host tabs already reconnect on
    /// their own, so riding their success is what keeps forwards
    /// symmetric with them. No-op while nothing is pending, so the
    /// connect hot path stays free.
    pub(crate) fn pf_kick_pending_retries(&mut self) -> Task<Message> {
        if self.port_forward_retry.is_empty() {
            return Task::none();
        }
        tracing::info!("a connection came up, re-attempting pending port forwards");
        self.pf_reissue_pending_now()
    }

    /// Issue a connect for every pending rule whose backoff has elapsed.
    /// Prunes entries that are no longer eligible (rule deleted, no longer
    /// wanted per `pf_retry_still_wanted`, or already up) so the
    /// subscription unmounts once nothing is pending. Shared by the
    /// heartbeat and the census kick.
    fn pf_issue_due_retries(&mut self, now: Instant) -> Task<Message> {
        let ids: Vec<Uuid> = self.port_forward_retry.keys().copied().collect();
        let auto_reconnect = self.auto_reconnect;
        let mut due = Vec::new();
        for id in ids {
            let still_wanted = self
                .port_forward_rules
                .iter()
                .any(|r| r.id == id && pf_retry_still_wanted(r.auto_start, auto_reconnect));
            if !still_wanted || self.active_forwards.contains_key(&id) {
                self.port_forward_retry.remove(&id);
                continue;
            }
            // An attempt is already in flight (or the connect just landed);
            // don't stack a second one. `start_port_forward` also guards
            // this, but skipping here keeps the backoff honest.
            if self.port_forward_starting.contains(&id) {
                continue;
            }
            if self
                .port_forward_retry
                .get(&id)
                .is_some_and(|r| r.next_at <= now)
            {
                due.push(id);
            }
        }

        let mut tasks = Vec::new();
        for id in due {
            // Advance the backoff BEFORE issuing: a failure that lands after
            // this tick keeps climbing, and a success clears the entry.
            if let Some(retry) = self.port_forward_retry.get_mut(&id) {
                retry.attempts = retry.attempts.saturating_add(1);
                retry.next_at = now + pf_retry_backoff(retry.attempts);
            }
            tracing::info!("retrying auto-start port forward {id}");
            tasks.push(self.start_port_forward(id, true));
        }
        Task::batch(tasks)
    }
}
/// Runtime state of a host's shared forward connection (issue #126).
/// Every live forward to one host multiplexes its channels over a single
/// PTY-less SSH connection, the way OpenSSH stacks `-L`/`-R`/`-D` flags
/// on one invocation, instead of each rule dialing its own.
#[derive(Debug)]
pub(crate) enum PfHostConn {
    /// A dial for this host is in flight; rules toggled on meanwhile
    /// queue here and attach when `PortForwardConnReady` lands.
    Connecting { pending: Vec<Uuid> },
    /// The connection is up; further rules attach onto it immediately.
    Up(ForwardConn),
}

/// Retry bookkeeping for an `auto_start` forward that is down. `next_at` is
/// the earliest wall-clock instant to re-attempt; `attempts` is how many
/// re-attempts have been issued so far, driving the backoff. The attempt
/// count is never capped, only the interval is: an `auto_start` forward is
/// meant to stay up, so it keeps trying (cheaply) until the key/network
/// comes back, rather than giving up like the SSH-tab reconnect does.
#[derive(Debug, Clone)]
pub(crate) struct PfRetry {
    pub next_at: Instant,
    pub attempts: u32,
}

/// Backoff for the Nth retry: 15s, 30s, 60s, then a 120s ceiling. Cheap
/// enough to poll a dead endpoint indefinitely (≤ ~720 attempts/day) yet
/// snappy enough that a forward comes up seconds after its key lands.
fn pf_retry_backoff(attempts: u32) -> Duration {
    let secs = 15u64.saturating_mul(1u64 << attempts.min(3));
    Duration::from_secs(secs.min(120))
}

/// Whether a down forward rule should keep its retry entry. This mirrors
/// the gate `pf_mark_retry_pending` uses for the `Dropped` cause: an
/// `auto_start` rule always retries, and any rule retries while the user
/// has `auto_reconnect` on. Keeping the prune side in sync with the mark
/// side is what stops a manually started `auto_reconnect` forward from
/// being pruned on the first tick and never retrying again (issue #144).
fn pf_retry_still_wanted(auto_start: bool, auto_reconnect: bool) -> bool {
    auto_start || auto_reconnect
}

/// A reading of the keys an agent-backed forward could authenticate
/// with: what every reachable ssh-agent holds, plus how many keys
/// external tools have pushed into our own agent server.
///
/// Compared reading-to-reading by the retry healer. The backoff alone
/// answers "the endpoint is still down"; it cannot answer "the missing
/// key just arrived", which is exactly the issue-#101 shape: KeePassXC
/// hands its keys to an agent only when its database unlocks, so a
/// forward that started before it can sit out up to 120 s of backoff
/// after the key is already available. A change here means the cause of
/// the failure may have just disappeared, so every pending rule becomes
/// due immediately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PfAgentWatch {
    /// `oryxis_ssh::agent_key_census()`: the rosters of the reachable
    /// agents. Down to fingerprints rather than endpoints, because the
    /// common case is a key landing in an agent that was ALREADY
    /// running (the always-on Windows OpenSSH service), which moves no
    /// endpoint at all.
    agents: Vec<String>,
    /// `AgentRuntime::external_add_generation()`, 0 when our agent
    /// server is off. Covers the mirror case: a tool pushing a key into
    /// US, where there is no external roster to poll.
    added_keys: u64,
}

/// Whether the agent picture moved since the last reading. The FIRST
/// reading is a baseline and never a kick: with nothing to compare
/// against, "changed" would fire on an unchanged environment.
fn pf_agent_changed(prev: Option<&PfAgentWatch>, now: &PfAgentWatch) -> bool {
    prev.is_some_and(|p| p != now)
}

/// Why a forward is being queued for another attempt. The two cases have
/// different answers because one of them the user is watching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PfRetryCause {
    /// It was running and the connection died.
    Dropped,
    /// The connect attempt itself failed.
    StartFailed,
}

#[cfg(test)]
mod tests {
    use super::{PfAgentWatch, pf_agent_changed, pf_retry_backoff, pf_retry_still_wanted};
    use std::time::Duration;

    struct RejectDialer;
    impl super::ForwardDialer for RejectDialer {
        fn connect(&self, _: uuid::Uuid, _: bool) -> super::DialFuture {
            Box::pin(async { Err("fixture rejected".into()) })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn retry_deadlines_advance_and_stop_clears_intent() {
        use super::*;
        use oryxis_core::models::port_forward_rule::ForwardKind;
        let mut r = PortForwardRule::new("auto", ForwardKind::Local, Uuid::new_v4());
        r.auto_start = true;
        let mut state = RuntimeState::new(Arc::new(RejectDialer), vec![r.clone()], true);
        state.pf_mark_retry_pending(r.id, PfRetryCause::StartFailed);
        assert_eq!(
            state.port_forward_retry[&r.id].next_at - Instant::now(),
            Duration::from_secs(15)
        );
        for expected in [30, 60, 120, 120] {
            let due = state.port_forward_retry[&r.id].next_at;
            tokio::time::advance(due - Instant::now()).await;
            let work = state.pf_issue_due_retries(Instant::now());
            assert_eq!(
                state.port_forward_retry[&r.id].next_at - Instant::now(),
                Duration::from_secs(expected)
            );
            for f in work.0 {
                let done = state.handle_message(f.await);
                for f in done.0 {
                    state.handle_message(f.await);
                }
            }
        }
        let (tx, rx) = oneshot::channel();
        state.stop_port_forward(r.id, tx);
        assert!(rx.await.unwrap().is_ok());
        tokio::time::advance(Duration::from_secs(600)).await;
        assert!(state.pf_issue_due_retries(Instant::now()).0.is_empty());
        assert_eq!(state.snapshot().tunnels[0].status, TunnelStatus::Stopped);
    }

    #[tokio::test]
    async fn stop_waits_for_attach_failure_before_restart_and_late_error_cannot_rearm_retry() {
        use super::*;
        use oryxis_core::models::port_forward_rule::ForwardKind;
        let mut r = PortForwardRule::new("auto", ForwardKind::Remote, Uuid::new_v4());
        r.auto_start = true;
        let mut state = RuntimeState::new(Arc::new(RejectDialer), vec![r.clone()], true);
        state.port_forward_starting.insert(r.id);
        state.attaching.insert(r.id);
        let (tx, mut rx) = oneshot::channel();
        state.stop_port_forward(r.id, tx);
        assert!(rx.try_recv().is_err());
        assert!(state.start_port_forward(r.id, false).0.is_empty());
        state.handle_message(Message::Started(r.id, Err("late failure".into())));
        assert!(rx.await.unwrap().is_ok());
        assert!(state.port_forward_retry.is_empty());
        assert!(!state.start_port_forward(r.id, false).0.is_empty());
    }

    fn watch(agents: &[&str], added_keys: u64) -> PfAgentWatch {
        PfAgentWatch {
            agents: agents.iter().map(|s| s.to_string()).collect(),
            added_keys,
        }
    }

    #[test]
    fn first_agent_reading_is_a_baseline_not_a_kick() {
        // Nothing to compare against: an unchanged environment must not
        // read as "the keys moved" on the very first tick.
        assert!(!pf_agent_changed(None, &watch(&[], 0)));
        assert!(!pf_agent_changed(
            None,
            &watch(&["/tmp/agent.sock SHA256:aaa"], 3)
        ));
    }

    #[test]
    fn steady_agent_picture_never_kicks() {
        let prev = watch(&["/tmp/agent.sock SHA256:aaa"], 2);
        assert!(!pf_agent_changed(
            Some(&prev),
            &watch(&["/tmp/agent.sock SHA256:aaa"], 2)
        ));
    }

    #[test]
    fn key_arriving_in_a_running_agent_kicks() {
        // The issue-#101 edge, and the one an endpoint-only watch would
        // miss: the agent (the always-on Windows OpenSSH service) was
        // there all along, KeePassXC just handed it the key.
        let before = watch(&[r"\\.\pipe\openssh-ssh-agent <empty>"], 0);
        let after = watch(&[r"\\.\pipe\openssh-ssh-agent SHA256:aaa"], 0);
        assert!(pf_agent_changed(Some(&before), &after));
        // The reverse (KeePassXC locked, keys pulled) also counts: the
        // picture moved, and one extra dial is cheap next to leaving a
        // forward down for two minutes.
        assert!(pf_agent_changed(Some(&after), &before));
    }

    #[test]
    fn whole_agent_appearing_kicks() {
        // KeePassXC in Pageant mode publishes its own pipe on launch.
        let before = watch(&[], 0);
        let after = watch(&[r"\\.\pipe\pageant.alice.abcd SHA256:aaa"], 0);
        assert!(pf_agent_changed(Some(&before), &after));
    }

    #[test]
    fn key_pushed_into_our_own_agent_kicks() {
        // Same endpoint set, but KeePassXC just ADDed a key to the
        // Oryxis agent server: the missing credential is now available.
        let before = watch(&[r"\\.\pipe\openssh-ssh-agent"], 0);
        let after = watch(&[r"\\.\pipe\openssh-ssh-agent"], 1);
        assert!(pf_agent_changed(Some(&before), &after));
    }

    #[test]
    fn backoff_climbs_then_caps_at_120s() {
        assert_eq!(pf_retry_backoff(0), Duration::from_secs(15));
        assert_eq!(pf_retry_backoff(1), Duration::from_secs(30));
        assert_eq!(pf_retry_backoff(2), Duration::from_secs(60));
        assert_eq!(pf_retry_backoff(3), Duration::from_secs(120));
        // The ceiling holds for every further attempt, and the bounded
        // shift (`attempts.min(3)`) means a huge count can't overflow.
        assert_eq!(pf_retry_backoff(4), Duration::from_secs(120));
        assert_eq!(pf_retry_backoff(50), Duration::from_secs(120));
        assert_eq!(pf_retry_backoff(u32::MAX), Duration::from_secs(120));
    }

    #[test]
    fn auto_reconnect_keeps_a_manual_forward_retrying() {
        // An auto_start forward always keeps retrying.
        assert!(pf_retry_still_wanted(true, false));
        // A manually started forward keeps retrying while auto_reconnect is
        // on. Regression for issue #144: the prune gate only checked
        // auto_start, so such a forward was dropped from the retry set on
        // the first tick (the retry subscription then unmounted) and never
        // came back after the local network dropped, unlike host sessions.
        assert!(pf_retry_still_wanted(false, true));
        // A manual forward with auto_reconnect off is not retried.
        assert!(!pf_retry_still_wanted(false, false));
    }
}
