//! Versioned TOML configuration and atomic on-disk storage.

mod schema;
mod secrets;
mod ssh_import;
mod startup;
mod store;

pub use schema::{
    AppSettings, AuthType, ConfigDocument, ConfigValidationError, DomainConfig, GroupRecord,
    HostRecord, SCHEMA_VERSION, ThemePreference, TunnelRecord,
};
pub use secrets::{SecretStore, SecretStoreError};
pub use ssh_import::{SshImportEntry, SshImportError, SshImportPreview, preview_ssh_config};
pub use startup::{StartupError, set_run_at_login};
pub use store::{ConfigStore, ConfigStoreError, SaveOutcome};
