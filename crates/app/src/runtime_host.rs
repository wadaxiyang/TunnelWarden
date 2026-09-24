use std::{
    path::PathBuf,
    sync::Arc,
    thread::{self, JoinHandle},
};

use config_store::{DomainConfig, SecretStore};
use gpui_kit::Global;
use tunnel_core::{CredentialSource, ManagerHandle, TunnelManager};
use tunnel_domain::SecretRef;
use zeroize::Zeroizing;

#[cfg(windows)]
use crate::network::NetworkWatcher;

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
    #[cfg(windows)]
    network: Option<NetworkWatcher>,
}

impl Global for RuntimeHost {}

impl RuntimeHost {
    pub fn new(config: DomainConfig, directory: PathBuf) -> Result<Self, String> {
        let (manager, handle) = TunnelManager::new(
            config.hosts,
            config.groups,
            config.tunnels,
            Arc::new(SystemCredentials),
        )
        .map_err(|error| error.to_string())?;
        let manager = manager
            .with_app_settings(config.app)
            .with_app_known_hosts(directory.join("known_hosts"));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let thread = thread::Builder::new()
            .name("tunnelwarden-host".into())
            .spawn(move || runtime.block_on(manager.run()))
            .map_err(|error| error.to_string())?;
        #[cfg(windows)]
        let network = match NetworkWatcher::new(handle.clone()) {
            Ok(network) => Some(network),
            Err(error) => {
                eprintln!("{error}");
                None
            }
        };
        Ok(Self {
            handle,
            thread: Some(thread),
            #[cfg(windows)]
            network,
        })
    }

    pub fn handle(&self) -> ManagerHandle {
        self.handle.clone()
    }
}

impl Drop for RuntimeHost {
    fn drop(&mut self) {
        #[cfg(windows)]
        self.network.take();
        self.handle.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
