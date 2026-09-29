//! Tauri-facing application methods. Owns DTO conversion/validation and CRUD,
//! not SSH semantics. Searched Oryxis forms/dispatch/connect_methods before
//! implementing these application-specific adapters (see docs/02_COPY_MAP.md).
use crate::{
    credentials::SecretStore,
    dial::{ApplicationDialer, PromptBroker},
    dto::*,
    logs::LogBook,
    runtime::TunnelRuntimeManager,
    storage::ConfigStore,
};
use oryxis_core::models::{
    connection::{AuthMethod, Connection},
    port_forward_rule::PortForwardRule,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::Mutex;
use uuid::Uuid;

pub struct AppService {
    pub runtime: TunnelRuntimeManager,
    pub prompts: Arc<PromptBroker>,
    pub logs: Arc<LogBook>,
    store: Arc<ConfigStore>,
    secrets: Arc<dyn SecretStore>,
    mutations: Mutex<()>,
}
impl AppService {
    pub fn new(store: Arc<ConfigStore>, secrets: Arc<dyn SecretStore>) -> Arc<Self> {
        let config = store.snapshot();
        let prompts = Arc::new(PromptBroker::new(store.clone()));
        let dialer = Arc::new(ApplicationDialer {
            store: store.clone(),
            secrets: secrets.clone(),
            prompts: prompts.clone(),
        });
        let runtime =
            TunnelRuntimeManager::new(dialer, config.tunnels, config.settings.auto_reconnect);
        let logs = Arc::new(LogBook::default());
        let mut updates = runtime.subscribe();
        let weak_logs = Arc::downgrade(&logs);
        tokio::spawn(async move {
            let mut previous = HashMap::new();
            while updates.changed().await.is_ok() {
                let Some(logs) = weak_logs.upgrade() else {
                    break;
                };
                for r in &updates.borrow_and_update().tunnels {
                    let state = (r.status, r.last_error.clone());
                    if previous.get(&r.id) != Some(&state) {
                        logs.push(
                            if r.last_error.is_some() {
                                "error"
                            } else {
                                "info"
                            },
                            Some(r.id),
                            format!(
                                "{:?}{}",
                                r.status,
                                r.last_error
                                    .as_ref()
                                    .map(|e| format!(": {e}"))
                                    .unwrap_or_default()
                            ),
                        );
                        previous.insert(r.id, state);
                    }
                }
            }
        });
        logs.push("info", None, "TunnelWarden 0.1.0 started");
        Arc::new(Self {
            runtime,
            prompts,
            logs,
            store,
            secrets,
            mutations: Mutex::new(()),
        })
    }
    pub fn list_hosts(&self) -> Result<Vec<HostDto>, String> {
        self.store
            .snapshot()
            .hosts
            .iter()
            .map(|h| self.host_dto(h))
            .collect()
    }
    fn host_dto(&self, h: &Connection) -> Result<HostDto, String> {
        Ok(HostDto {
            id: h.id,
            name: h.label.clone(),
            hostname: h.hostname.clone(),
            ssh_port: h.port,
            username: h.username.clone().unwrap_or_default(),
            auth_method: h.auth_method.clone(),
            identity_file: h.identity_file.clone().unwrap_or_default(),
            jump_chain: h.jump_chain.clone(),
            keepalive_seconds: h.keepalive_interval.unwrap_or(30),
            has_password: self.secrets.get(h.id)?.is_some(),
        })
    }
    pub fn list_tunnels(&self) -> Vec<TunnelDto> {
        self.store
            .snapshot()
            .tunnels
            .iter()
            .map(tunnel_dto)
            .collect()
    }
    pub fn get_settings(&self) -> Settings {
        self.store.snapshot().settings
    }
    pub fn get_runtime_snapshot(&self) -> RuntimeSnapshot {
        self.runtime.snapshot()
    }
    pub fn get_logs(&self, filter: Option<LogFilter>) -> Vec<LogEntry> {
        self.logs.get(filter)
    }

    pub async fn save_host(&self, input: SaveHost) -> Result<HostDto, String> {
        let _serial = self.mutations.lock().await;
        let h = input.profile;
        required("Host name", &h.name)?;
        required("Hostname/IP", &h.hostname)?;
        required("Username", &h.username)?;
        endpoint("Hostname/IP", &h.hostname)?;
        if h.ssh_port == 0 {
            return Err("SSH port must be 1–65535".into());
        }
        if h.keepalive_seconds > 3600 {
            return Err("Keepalive must be 0–3600 seconds".into());
        }
        if !matches!(
            h.auth_method,
            AuthMethod::Password | AuthMethod::Key | AuthMethod::Agent | AuthMethod::Auto
        ) {
            return Err("Unsupported authentication method".into());
        }
        if h.auth_method == AuthMethod::Key && h.identity_file.trim().is_empty() {
            return Err("Private key file is required".into());
        }
        let config = self.store.snapshot();
        let mut conn = config
            .hosts
            .iter()
            .find(|c| c.id == h.id)
            .cloned()
            .unwrap_or_else(|| Connection::new(&h.name, &h.hostname));
        conn.id = h.id;
        conn.label = h.name.trim().into();
        conn.hostname = h.hostname.trim().into();
        conn.port = h.ssh_port;
        conn.username = Some(h.username.trim().into());
        conn.auth_method = h.auth_method;
        conn.identity_file =
            (!h.identity_file.trim().is_empty()).then(|| h.identity_file.trim().into());
        conn.use_disk_key = conn.identity_file.is_some();
        conn.jump_chain = h.jump_chain;
        conn.keepalive_interval = Some(h.keepalive_seconds);
        conn.updated_at = chrono::Utc::now();
        let mut proposed = config.hosts.clone();
        proposed.retain(|c| c.id != conn.id);
        proposed.push(conn.clone());
        validate_routes(&proposed)?;
        // A hop edit also affects every final host whose expanded route uses it.
        for target in &config.hosts {
            if target.id == h.id
                || oryxis_core::jump_route::expanded_jump_chain(
                    target.id,
                    &target.jump_chain,
                    &config.hosts,
                )
                .contains(&h.id)
            {
                self.runtime.ensure_host_idle(target.id).await?;
            }
        }
        let old_password = self.secrets.get(h.id)?;
        if input.clear_password {
            self.secrets.delete(h.id)?;
        } else if let Some(password) = input.password {
            self.secrets.set(h.id, &password)?;
        }
        if let Err(e) = self.store.update(|c| {
            c.hosts = proposed;
            Ok(())
        }) {
            self.restore_secret(h.id, old_password.as_deref())
                .map_err(|rollback| format!("{e}; credential rollback failed: {rollback}"))?;
            return Err(e);
        }
        self.logs
            .push("info", None, format!("Saved SSH host {}", conn.label));
        self.host_dto(&conn)
    }
    fn restore_secret(&self, id: Uuid, value: Option<&str>) -> Result<(), String> {
        match value {
            Some(v) => self.secrets.set(id, v),
            None => self.secrets.delete(id),
        }
    }
    pub async fn delete_host(&self, id: Uuid) -> Result<(), String> {
        let _serial = self.mutations.lock().await;
        let c = self.store.snapshot();
        if c.tunnels.iter().any(|r| r.host_id == id)
            || c.hosts.iter().any(|h| h.jump_chain.contains(&id))
        {
            return Err(
                "This host is used by a tunnel or jump route. Remove those references first."
                    .into(),
            );
        }
        self.runtime.ensure_host_idle(id).await?;
        let old = self.secrets.get(id)?;
        self.secrets.delete(id)?;
        if let Err(e) = self.store.update(|c| {
            c.hosts.retain(|h| h.id != id);
            Ok(())
        }) {
            self.restore_secret(id, old.as_deref())
                .map_err(|rollback| format!("{e}; credential rollback failed: {rollback}"))?;
            return Err(e);
        }
        self.logs.push("info", None, "Deleted SSH host");
        Ok(())
    }
    pub async fn save_tunnel(&self, dto: TunnelDto) -> Result<TunnelDto, String> {
        let _serial = self.mutations.lock().await;
        required("Tunnel name", &dto.name)?;
        endpoint("Listen address", &dto.listen_address)?;
        if dto.listen_port == 0 {
            return Err("Listener port must be 1–65535".into());
        }
        if dto.kind.has_target() {
            endpoint("Target host", &dto.target_host)?;
            if dto.target_port == 0 {
                return Err("Target port must be 1–65535".into());
            }
        }
        let config = self.store.snapshot();
        if !config.hosts.iter().any(|h| h.id == dto.host_id) {
            return Err("Choose a saved SSH host".into());
        }
        let mut r = config
            .tunnels
            .iter()
            .find(|r| r.id == dto.id)
            .cloned()
            .unwrap_or_else(|| PortForwardRule::new(&dto.name, dto.kind, dto.host_id));
        r.id = dto.id;
        r.label = dto.name.trim().into();
        r.kind = dto.kind;
        r.host_id = dto.host_id;
        r.listen_host = dto.listen_address.trim().into();
        r.listen_port = dto.listen_port;
        r.auto_start = dto.auto_start;
        r.target_host = if dto.kind.has_target() {
            dto.target_host.trim().into()
        } else {
            String::new()
        };
        r.target_port = if dto.kind.has_target() {
            dto.target_port
        } else {
            0
        };
        r.updated_at = chrono::Utc::now();
        self.runtime.stop(r.id).await?;
        self.store.update(|c| {
            c.tunnels.retain(|old| old.id != r.id);
            c.tunnels.push(r.clone());
            Ok(())
        })?;
        self.sync_rules().await?;
        self.logs.push(
            "info",
            Some(r.id),
            "Saved tunnel; stopped until explicitly started",
        );
        Ok(tunnel_dto(&r))
    }
    pub async fn delete_tunnel(&self, id: Uuid) -> Result<(), String> {
        let _serial = self.mutations.lock().await;
        self.runtime.stop(id).await?;
        self.store.update(|c| {
            c.tunnels.retain(|r| r.id != id);
            Ok(())
        })?;
        self.sync_rules().await?;
        self.logs
            .push("info", Some(id), "Deleted tunnel after cancellation");
        Ok(())
    }
    async fn sync_rules(&self) -> Result<(), String> {
        let c = self.store.snapshot();
        self.runtime
            .configure(c.tunnels, c.settings.auto_reconnect)
            .await
    }
    pub async fn start_tunnel(&self, id: Uuid) -> Result<(), String> {
        let _serial = self.mutations.lock().await;
        self.runtime.start(id).await
    }
    pub async fn stop_tunnel(&self, id: Uuid) -> Result<(), String> {
        let _serial = self.mutations.lock().await;
        self.runtime.stop(id).await
    }
    pub async fn restart_tunnel(&self, id: Uuid) -> Result<(), String> {
        let _serial = self.mutations.lock().await;
        self.runtime.restart(id).await
    }
    pub async fn set_settings(&self, settings: Settings) -> Result<(), String> {
        let _serial = self.mutations.lock().await;
        self.store.update(|c| {
            c.settings = settings;
            Ok(())
        })?;
        self.sync_rules().await
    }
    pub async fn start_all(&self) -> Result<(), String> {
        let _serial = self.mutations.lock().await;
        for r in self.list_tunnels() {
            self.runtime.start(r.id).await?;
        }
        Ok(())
    }
    pub async fn stop_all(&self) -> Result<(), String> {
        let _serial = self.mutations.lock().await;
        for r in self.list_tunnels() {
            self.runtime.stop(r.id).await?;
        }
        Ok(())
    }
}

fn required(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} is required"))
    } else if value.len() > 1024 || value.chars().any(char::is_control) {
        Err(format!(
            "{label} contains invalid characters or is too long"
        ))
    } else {
        Ok(())
    }
}
fn endpoint(label: &str, value: &str) -> Result<(), String> {
    required(label, value)?;
    if value.trim().chars().any(char::is_whitespace)
        || value.contains('/')
        || value.contains('\\')
        || (value.contains(':') && value.trim().parse::<std::net::Ipv6Addr>().is_err())
    {
        Err(format!(
            "{label} must be a hostname or IP, without a URL scheme or port"
        ))
    } else {
        Ok(())
    }
}
fn validate_routes(hosts: &[Connection]) -> Result<(), String> {
    fn visit(id: Uuid, hosts: &[Connection], path: &mut HashSet<Uuid>) -> Result<(), String> {
        if !path.insert(id) {
            return Err("Jump route contains a cycle".into());
        }
        let host = hosts
            .iter()
            .find(|h| h.id == id)
            .ok_or("Jump route references a missing host")?;
        for hop in &host.jump_chain {
            visit(*hop, hosts, path)?;
        }
        path.remove(&id);
        Ok(())
    }
    for h in hosts {
        visit(h.id, hosts, &mut HashSet::new())?;
    }
    Ok(())
}
fn tunnel_dto(r: &PortForwardRule) -> TunnelDto {
    TunnelDto {
        id: r.id,
        name: r.label.clone(),
        kind: r.kind,
        host_id: r.host_id,
        listen_address: r.listen_host.clone(),
        listen_port: r.listen_port,
        target_host: r.target_host.clone(),
        target_port: r.target_port,
        auto_start: r.auto_start,
    }
}
