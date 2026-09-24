use std::{
    sync::Arc,
    thread::{self, JoinHandle},
};

use config_store::{DomainConfig, SecretStore};
use gpui_kit::Global;
use tunnel_core::{CredentialSource, ManagerHandle, TunnelManager};
use tunnel_domain::SecretRef;
use zeroize::Zeroizing;

struct SystemCredentials;

impl CredentialSource for SystemCredentials {
    fn load(&self, reference: &SecretRef) -> Result<Zeroizing<String>, String> {
        SecretStore::load(reference).map_err(|error| error.to_string())
    }
}

/// Application-owned Tokio host. Closing a window drops only its UI
/// subscription; this global keeps tunnel supervisors alive until app exit.
pub struct RuntimeHost {
    handle: ManagerHandle,
    thread: Option<JoinHandle<()>>,
}

impl Global for RuntimeHost {}

impl RuntimeHost {
    pub fn new(config: DomainConfig) -> Result<Self, String> {
        let (manager, handle) = TunnelManager::new(
            config.hosts,
            config.groups,
            config.tunnels,
            Arc::new(SystemCredentials),
        )
        .map_err(|error| error.to_string())?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let thread = thread::Builder::new()
            .name("tunnelwarden-host".into())
            .spawn(move || runtime.block_on(manager.run()))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            handle,
            thread: Some(thread),
        })
    }

    pub fn handle(&self) -> ManagerHandle {
        self.handle.clone()
    }
}

impl Drop for RuntimeHost {
    fn drop(&mut self) {
        self.handle.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
