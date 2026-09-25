use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use thiserror::Error;

use crate::{ConfigDocument, ConfigValidationError, DomainConfig};

const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_BACKUPS: usize = 5;
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Default)]
pub struct SaveOutcome {
    /// Maintenance failures after the main configuration was committed.
    pub warnings: Vec<String>,
}

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
    #[error(
        "configuration recovery is required; main file is missing but a temporary file or backup remains near {0}"
    )]
    RecoveryRequired(PathBuf),
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
        let _lock = self.lock_directory()?;
        let path = self.directory.join("config.toml");
        match File::open(&path) {
            Ok(file) => {
                let config = Self::read_file(path, file)?;
                Ok(config)
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                if self.has_recovery_artifacts()? {
                    return Err(ConfigStoreError::RecoveryRequired(path));
                }
                Ok(ConfigDocument::default().into_domain()?)
            }
            Err(source) => Err(io_error(path, source)),
        }
    }

    pub fn import_file(path: &Path) -> Result<DomainConfig, ConfigStoreError> {
        let file = File::open(path).map_err(|source| io_error(path.to_path_buf(), source))?;
        let mut config = Self::read_file(path.to_path_buf(), file)?;
        for tunnel in &mut config.tunnels {
            tunnel.exposure_approved = false;
        }
        Ok(config)
    }

    fn read_file(path: PathBuf, file: File) -> Result<DomainConfig, ConfigStoreError> {
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

    /// Export contains only config records and opaque keyring references.
    /// The keyring's password and private-key passphrase values are never read.
    pub fn export_file(path: &Path, config: DomainConfig) -> Result<(), ConfigStoreError> {
        let document = ConfigDocument::try_from(config)?;
        document.clone().into_domain()?;
        let serialized = toml::to_string_pretty(&document)?;
        if serialized.len() as u64 > MAX_CONFIG_BYTES {
            return Err(ConfigStoreError::TooLarge);
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|source| io_error(path.to_path_buf(), source))?;
        if let Err(source) = file
            .write_all(serialized.as_bytes())
            .and_then(|()| file.sync_all())
        {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(io_error(path.to_path_buf(), source));
        }
        Ok(())
    }

    /// Writes a validated schema-v1 document through a synced temporary file.
    /// Existing config is backed up before replacement. A failed rename leaves
    /// the original file untouched and removes the temporary file on drop.
    pub fn save(&self, document: ConfigDocument) -> Result<SaveOutcome, ConfigStoreError> {
        let canonical = document.into_domain()?;
        let document = ConfigDocument::try_from(canonical)?;
        let serialized = toml::to_string_pretty(&document)?;
        if serialized.len() as u64 > MAX_CONFIG_BYTES {
            return Err(ConfigStoreError::TooLarge);
        }
        let _lock = self.lock_directory()?;
        let path = self.directory.join("config.toml");
        match File::open(&path) {
            Ok(file) => {
                Self::read_file(path.clone(), file)?;
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                if self.has_recovery_artifacts()? {
                    return Err(ConfigStoreError::RecoveryRequired(path));
                }
            }
            Err(source) => return Err(io_error(path, source)),
        }
        let (temp_path, mut temp) = self.create_temp()?;
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
        let mut outcome = SaveOutcome::default();
        if let Err(error) = self.prune_backups() {
            outcome.warnings.push(error.to_string());
        }
        if let Err(error) = self.clean_orphaned_temps() {
            outcome.warnings.push(error.to_string());
        }
        Ok(outcome)
    }

    fn lock_directory(&self) -> Result<File, ConfigStoreError> {
        fs::create_dir_all(&self.directory)
            .map_err(|source| io_error(self.directory.clone(), source))?;
        let path = self.directory.join("config.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| io_error(path.clone(), source))?;
        file.lock().map_err(|source| io_error(path, source))?;
        Ok(file)
    }

    fn create_temp(&self) -> Result<(PathBuf, File), ConfigStoreError> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..10 {
            let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = self.directory.join(format!(
                "config.toml.tw-{stamp:x}-{:x}-{sequence:x}.tmp",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((path, file)),
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(source) => return Err(io_error(path, source)),
            }
        }
        Err(ConfigStoreError::BackupNameConflict)
    }

    fn has_recovery_artifacts(&self) -> Result<bool, ConfigStoreError> {
        let entries = fs::read_dir(&self.directory)
            .map_err(|source| io_error(self.directory.clone(), source))?;
        for entry in entries {
            let entry = entry.map_err(|source| io_error(self.directory.clone(), source))?;
            if is_config_temp(&entry.file_name().to_string_lossy()) {
                return Ok(true);
            }
        }
        let backups = self.directory.join("backups");
        if backups.exists() {
            let entries =
                fs::read_dir(&backups).map_err(|source| io_error(backups.clone(), source))?;
            for entry in entries {
                let entry = entry.map_err(|source| io_error(backups.clone(), source))?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("config-")
                    && (name.ends_with(".toml") || name.ends_with(".toml.tmp"))
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn clean_orphaned_temps(&self) -> Result<(), ConfigStoreError> {
        let entries = fs::read_dir(&self.directory)
            .map_err(|source| io_error(self.directory.clone(), source))?;
        for entry in entries {
            let entry = entry.map_err(|source| io_error(self.directory.clone(), source))?;
            if is_config_temp(&entry.file_name().to_string_lossy())
                && entry
                    .file_type()
                    .map_err(|source| io_error(entry.path(), source))?
                    .is_file()
            {
                fs::remove_file(entry.path()).map_err(|source| io_error(entry.path(), source))?;
            }
        }
        let backups = self.directory.join("backups");
        if backups.exists() {
            let entries =
                fs::read_dir(&backups).map_err(|source| io_error(backups.clone(), source))?;
            for entry in entries {
                let entry = entry.map_err(|source| io_error(backups.clone(), source))?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("config-")
                    && name.ends_with(".toml.tmp")
                    && entry
                        .file_type()
                        .map_err(|source| io_error(entry.path(), source))?
                        .is_file()
                {
                    fs::remove_file(entry.path())
                        .map_err(|source| io_error(entry.path(), source))?;
                }
            }
        }
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

fn is_config_temp(name: &str) -> bool {
    name == "config.toml.tmp" || (name.starts_with("config.toml.tw-") && name.ends_with(".tmp"))
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
exposure_approved = true
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
        assert!(loaded.tunnels[0].exposure_approved);
        let imported = ConfigStore::import_file(&directory.path().join("config.toml"))
            .expect("import external config");
        assert!(!imported.tunnels[0].exposure_approved);
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
    fn post_commit_prune_failure_reports_warning_and_keeps_new_config() {
        let directory = TempDir::new().expect("temporary directory");
        let store = ConfigStore::new(directory.path().to_path_buf());
        store.save(ConfigDocument::default()).expect("initial save");
        let backups = directory.path().join("backups");
        fs::create_dir_all(&backups).expect("backup directory");
        let blocked = backups.join("config-0000000000000-0.toml");
        fs::create_dir(&blocked).expect("oldest backup is a directory");
        for index in 1..=4 {
            fs::write(
                backups.join(format!("config-000000000000{index}-0.toml")),
                "old backup",
            )
            .expect("backup fixture");
        }
        let mut updated = ConfigDocument::default();
        updated.app.theme = ThemePreference::Dark;
        let outcome = store.save(updated).expect("main file committed");
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            store.load().expect("new file readable").app.theme,
            ThemePreference::Dark
        );
        assert!(blocked.is_dir());
    }

    #[test]
    fn crashed_temp_does_not_block_next_save() {
        let directory = TempDir::new().expect("temporary directory");
        let store = ConfigStore::new(directory.path().to_path_buf());
        store.save(ConfigDocument::default()).expect("initial save");
        let orphan = directory.path().join("config.toml.tw-crashed-1-0.tmp");
        fs::write(&orphan, "partial write").expect("orphan fixture");
        let mut updated = ConfigDocument::default();
        updated.app.theme = ThemePreference::Light;
        let outcome = store.save(updated).expect("save after crash");
        assert!(outcome.warnings.is_empty());
        assert!(!orphan.exists());
        assert_eq!(
            store.load().expect("reload").app.theme,
            ThemePreference::Light
        );
    }

    #[test]
    fn next_save_recovers_after_child_process_exits_during_temp_write() {
        let directory = TempDir::new().expect("temporary directory");
        let store = ConfigStore::new(directory.path().to_path_buf());
        store.save(ConfigDocument::default()).expect("initial save");
        let status = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .arg("--exact")
            .arg("store::tests::exit_during_temp_write_child")
            .env("TUNNELWARDEN_CONFIG_CRASH_TEST_DIRECTORY", directory.path())
            .status()
            .expect("launch child");
        assert_eq!(status.code(), Some(17));
        let mut updated = ConfigDocument::default();
        updated.app.theme = ThemePreference::Dark;
        store.save(updated).expect("next save after child exit");
        assert_eq!(
            store.load().expect("reload").app.theme,
            ThemePreference::Dark
        );
    }

    #[test]
    fn exit_during_temp_write_child() {
        let Some(directory) = std::env::var_os("TUNNELWARDEN_CONFIG_CRASH_TEST_DIRECTORY") else {
            return;
        };
        let store = ConfigStore::new(PathBuf::from(directory));
        let (_path, mut file) = store.create_temp().expect("create unique temp");
        file.write_all(b"partial configuration")
            .expect("write partial");
        file.sync_all().expect("sync partial");
        std::process::exit(17);
    }

    #[test]
    fn missing_main_file_with_orphan_requires_recovery() {
        let directory = TempDir::new().expect("temporary directory");
        let orphan = directory.path().join("config.toml.tw-crashed-1-0.tmp");
        fs::write(&orphan, "partial write").expect("orphan fixture");
        let store = ConfigStore::new(directory.path().to_path_buf());
        assert!(matches!(
            store.load(),
            Err(ConfigStoreError::RecoveryRequired(_))
        ));
        assert!(matches!(
            store.save(ConfigDocument::default()),
            Err(ConfigStoreError::RecoveryRequired(_))
        ));
        assert!(orphan.exists());
    }

    #[test]
    fn missing_main_file_with_backup_is_not_treated_as_empty_config() {
        let directory = TempDir::new().expect("temporary directory");
        let store = ConfigStore::new(directory.path().to_path_buf());
        store.save(ConfigDocument::default()).expect("initial save");
        let mut updated = ConfigDocument::default();
        updated.app.theme = ThemePreference::Light;
        store.save(updated).expect("second save creates backup");
        fs::remove_file(directory.path().join("config.toml")).expect("simulate missing main");
        assert!(matches!(
            store.load(),
            Err(ConfigStoreError::RecoveryRequired(_))
        ));
        assert!(matches!(
            store.save(ConfigDocument::default()),
            Err(ConfigStoreError::RecoveryRequired(_))
        ));
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

    #[test]
    fn export_import_roundtrip_does_not_overwrite_existing_file() {
        let directory = TempDir::new().expect("temporary directory");
        let path = directory.path().join("tunnelwarden-config.toml");
        let config = ConfigDocument::default()
            .into_domain()
            .expect("default config");
        ConfigStore::export_file(&path, config).expect("export");
        assert!(ConfigStore::import_file(&path).is_ok());
        let before = fs::read(&path).expect("read export");
        let config = ConfigDocument::default()
            .into_domain()
            .expect("default config");
        assert!(ConfigStore::export_file(&path, config).is_err());
        assert_eq!(fs::read(&path).expect("read preserved export"), before);
    }
}
