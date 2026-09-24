use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use thiserror::Error;

use crate::{ConfigDocument, ConfigValidationError, DomainConfig};

const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_BACKUPS: usize = 5;

#[derive(Debug, Error)]
pub enum ConfigStoreError {
    #[error("APPDATA is unavailable; choose a configuration directory")]
    MissingAppData,
    #[error("configuration file exceeds 4 MiB")]
    TooLarge,
    #[error("configuration I/O at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid TOML configuration: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("cannot serialize configuration: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error(transparent)]
    Validation(#[from] ConfigValidationError),
    #[error("too many backup names share the current timestamp")]
    BackupNameConflict,
}

pub struct ConfigStore {
    directory: PathBuf,
}

impl ConfigStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn default_windows() -> Result<Self, ConfigStoreError> {
        let app_data = std::env::var_os("APPDATA").ok_or(ConfigStoreError::MissingAppData)?;
        Ok(Self::new(PathBuf::from(app_data).join("TunnelWarden")))
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn load(&self) -> Result<DomainConfig, ConfigStoreError> {
        let path = self.directory.join("config.toml");
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(ConfigDocument::default().into_domain()?);
            }
            Err(source) => return Err(io_error(path, source)),
        };
        let mut bytes = String::new();
        file.take(MAX_CONFIG_BYTES + 1)
            .read_to_string(&mut bytes)
            .map_err(|source| io_error(path.clone(), source))?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(ConfigStoreError::TooLarge);
        }
        let document: ConfigDocument = toml::from_str(&bytes)?;
        Ok(document.into_domain()?)
    }

    /// Writes a validated schema-v1 document through a synced temporary file.
    /// Existing config is backed up before replacement. A failed rename leaves
    /// the original file untouched and removes the temporary file on drop.
    pub fn save(&self, document: ConfigDocument) -> Result<(), ConfigStoreError> {
        let serialized = toml::to_string_pretty(&document)?;
        document.into_domain()?;
        if serialized.len() as u64 > MAX_CONFIG_BYTES {
            return Err(ConfigStoreError::TooLarge);
        }
        fs::create_dir_all(&self.directory)
            .map_err(|source| io_error(self.directory.clone(), source))?;
        let path = self.directory.join("config.toml");
        let temp_path = self.directory.join("config.toml.tmp");
        let mut temp = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|source| io_error(temp_path.clone(), source))?;
        let cleanup = TempCleanup(&temp_path);
        temp.write_all(serialized.as_bytes())
            .and_then(|()| temp.sync_all())
            .map_err(|source| io_error(temp_path.clone(), source))?;
        drop(temp);
        if path.exists() {
            self.backup_current(&path)?;
        }
        fs::rename(&temp_path, &path).map_err(|source| io_error(path, source))?;
        drop(cleanup);
        self.prune_backups()?;
        Ok(())
    }

    fn backup_current(&self, path: &Path) -> Result<(), ConfigStoreError> {
        let size = fs::metadata(path)
            .map_err(|source| io_error(path.to_path_buf(), source))?
            .len();
        if size > MAX_CONFIG_BYTES {
            return Err(ConfigStoreError::TooLarge);
        }
        let backups = self.directory.join("backups");
        fs::create_dir_all(&backups).map_err(|source| io_error(backups.clone(), source))?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        for suffix in 0..10 {
            let backup = backups.join(format!("config-{stamp}-{suffix}.toml"));
            if backup.exists() {
                continue;
            }
            let staged = backups.join(format!("config-{stamp}-{suffix}.toml.tmp"));
            let mut source =
                File::open(path).map_err(|error| io_error(path.to_path_buf(), error))?;
            let mut target = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged)
            {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error(staged, error)),
            };
            let cleanup = TempCleanup(&staged);
            io::copy(&mut source, &mut target).map_err(|error| io_error(staged.clone(), error))?;
            target
                .sync_all()
                .map_err(|error| io_error(staged.clone(), error))?;
            drop(target);
            fs::rename(&staged, &backup).map_err(|error| io_error(backup, error))?;
            drop(cleanup);
            return Ok(());
        }
        Err(ConfigStoreError::BackupNameConflict)
    }

    fn prune_backups(&self) -> Result<(), ConfigStoreError> {
        let backups = self.directory.join("backups");
        if !backups.exists() {
            return Ok(());
        }
        let entries = fs::read_dir(&backups).map_err(|source| io_error(backups.clone(), source))?;
        let mut newest = Vec::<(String, PathBuf)>::with_capacity(MAX_BACKUPS + 1);
        for entry in entries {
            let entry = entry.map_err(|source| io_error(backups.clone(), source))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("config-") || !name.ends_with(".toml") {
                continue;
            }
            newest.push((name, entry.path()));
            newest.sort_unstable_by(|a, b| b.0.cmp(&a.0));
            if newest.len() > MAX_BACKUPS {
                let (_, oldest) = newest.pop().expect("length exceeds backup limit");
                fs::remove_file(&oldest).map_err(|source| io_error(oldest, source))?;
            }
        }
        Ok(())
    }
}

struct TempCleanup<'a>(&'a Path);

impl Drop for TempCleanup<'_> {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0);
    }
}

fn io_error(path: PathBuf, source: io::Error) -> ConfigStoreError {
    ConfigStoreError::Io { path, source }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ThemePreference;
    use tempfile::TempDir;
    use tunnel_domain::{AuthConfig, TunnelMode};

    #[test]
    fn spec_style_toml_loads_as_domain_config() {
        let directory = TempDir::new().expect("temporary directory");
        fs::write(
            directory.path().join("config.toml"),
            r#"
schema_version = 1

[app]
theme = "system"
run_at_startup = true
minimize_to_tray = true
traffic_monitor = true

[[hosts]]
id = "lab"
name = "Lab A40"
hostname = "lab.example.com"
port = 22
username = "user"
auth_type = "private_key"
identity_file = "~/.ssh/id_ed25519"
connect_timeout_ms = 8000
keepalive_interval_ms = 10000
keepalive_max = 3

[[groups]]
id = "proxy"
name = "Proxy"

[[tunnels]]
id = "lab-socks"
name = "Lab SOCKS"
group_id = "proxy"
mode = "dynamic"
jump_chain = ["lab"]
local_host = "127.0.0.1"
local_port = 1080
auto_start = true
description = "Primary SOCKS5 proxy"
"#,
        )
        .expect("write config");
        let loaded = ConfigStore::new(directory.path().to_path_buf())
            .load()
            .expect("load spec config");
        assert_eq!(loaded.hosts.len(), 1);
        assert!(matches!(
            loaded.hosts[0].auth,
            AuthConfig::PrivateKey { .. }
        ));
        assert_eq!(loaded.tunnels[0].mode, TunnelMode::Dynamic);
        assert!(loaded.app.run_at_startup);
        let document = ConfigDocument::try_from(loaded).expect("runtime to file model");
        ConfigStore::new(directory.path().to_path_buf())
            .save(document)
            .expect("save converted configuration");
        let reloaded = ConfigStore::new(directory.path().to_path_buf())
            .load()
            .expect("reload converted configuration");
        assert_eq!(reloaded.tunnels[0].jump_chain[0].0, "lab");
    }

    #[test]
    fn atomic_replace_preserves_five_backups() {
        let directory = TempDir::new().expect("temporary directory");
        let store = ConfigStore::new(directory.path().to_path_buf());
        for index in 0..8 {
            let mut document = ConfigDocument::default();
            document.app.theme = if index % 2 == 0 {
                ThemePreference::Light
            } else {
                ThemePreference::Dark
            };
            store.save(document).expect("atomic save");
        }
        let loaded = store.load().expect("load final config");
        assert_eq!(loaded.app.theme, ThemePreference::Dark);
        let backups = fs::read_dir(directory.path().join("backups"))
            .expect("backups directory")
            .count();
        assert_eq!(backups, MAX_BACKUPS);
        assert!(!directory.path().join("config.toml.tmp").exists());
    }

    #[test]
    fn unsupported_version_never_replaces_existing_file() {
        let directory = TempDir::new().expect("temporary directory");
        let store = ConfigStore::new(directory.path().to_path_buf());
        store.save(ConfigDocument::default()).expect("initial save");
        let before = fs::read(directory.path().join("config.toml")).expect("read initial");
        let invalid = ConfigDocument {
            schema_version: 2,
            ..ConfigDocument::default()
        };
        assert!(matches!(
            store.save(invalid),
            Err(ConfigStoreError::Validation(
                ConfigValidationError::UnsupportedVersion(2)
            ))
        ));
        assert_eq!(
            fs::read(directory.path().join("config.toml")).expect("read preserved"),
            before
        );
    }
}
