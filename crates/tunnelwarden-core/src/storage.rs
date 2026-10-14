//! Atomic versioned configuration. No secrets, runtime handles or status.
use crate::dto::Settings;
use oryxis_core::models::{
    connection::Connection, known_host::KnownHost, port_forward_rule::PortForwardRule,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub hosts: Vec<Connection>,
    pub tunnels: Vec<PortForwardRule>,
    pub settings: Settings,
    pub known_hosts: Vec<KnownHost>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            hosts: vec![],
            tunnels: vec![],
            settings: Settings::default(),
            known_hosts: vec![],
        }
    }
}
pub struct ConfigStore {
    path: PathBuf,
    data: Mutex<Config>,
}
impl ConfigStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        let data: Config = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                format!(
                    "Cannot read configuration {}: {e}. The original file was preserved.",
                    path.display()
                )
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(format!("Cannot read configuration {}: {e}", path.display())),
        };
        if data.version != 1 {
            return Err(format!(
                "Unsupported configuration version {}",
                data.version
            ));
        }
        Ok(Self {
            path,
            data: Mutex::new(data),
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn snapshot(&self) -> Config {
        self.data.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    /// Serialize writes and publish in memory only after the atomic replacement.
    pub fn update<R>(
        &self,
        edit: impl FnOnce(&mut Config) -> Result<R, String>,
    ) -> Result<R, String> {
        let mut data = self
            .data
            .lock()
            .map_err(|_| "Configuration lock poisoned".to_string())?;
        let mut next = data.clone();
        let result = edit(&mut next)?;
        let parent = self
            .path
            .parent()
            .ok_or("Configuration has no parent directory")?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create configuration directory: {e}"))?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(&mut file, &next).map_err(|e| e.to_string())?;
        file.write_all(b"\n").map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(&self.path)
            .map_err(|e| format!("Cannot save configuration: {e}"))?;
        *data = next;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_settings_follow_system_and_explicit_language_survives_restart() {
        use crate::i18n::Language;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut old = serde_json::to_value(Config::default()).unwrap();
        old["settings"].as_object_mut().unwrap().remove("language");
        old["settings"]["auto_reconnect"] = false.into();
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let store = ConfigStore::open(&path).unwrap();
        assert_eq!(store.snapshot().settings.language, Language::System);
        for language in [Language::Chinese, Language::English, Language::System] {
            store
                .update(|c| {
                    c.settings.language = language;
                    Ok(())
                })
                .unwrap();
            let reopened = ConfigStore::open(&path).unwrap().snapshot();
            assert_eq!(reopened.settings.language, language);
            assert!(!reopened.settings.auto_reconnect);
        }
    }

    #[test]
    fn roundtrip_and_failed_transaction_preserve_config_without_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let store = ConfigStore::open(&path).unwrap();
        store
            .update(|c| {
                c.hosts.push(Connection::new("saved", "example.test"));
                Ok(())
            })
            .unwrap();
        let before = fs::read(&path).unwrap();
        assert!(
            store
                .update::<()>(|c| {
                    c.hosts.clear();
                    Err("rollback".into())
                })
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        let saved = ConfigStore::open(&path).unwrap().snapshot();
        assert_eq!(saved.hosts.len(), 1);
        let json = String::from_utf8(before).unwrap();
        for key in [
            "active_forwards",
            "last_error",
            "next_retry_at",
            "private_key",
            "password",
        ] {
            assert!(!json.contains(&format!("\"{key}\"")));
        }
    }
    #[test]
    fn corrupt_or_future_config_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, b"corrupt").unwrap();
        assert!(ConfigStore::open(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"corrupt");
        let c = Config {
            version: 99,
            ..Config::default()
        };
        fs::write(&path, serde_json::to_vec(&c).unwrap()).unwrap();
        assert!(ConfigStore::open(&path).is_err());
    }
}
