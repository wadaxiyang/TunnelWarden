//! Versioned TOML configuration and atomic on-disk storage.

mod schema;
mod store;

pub use schema::{
    AppSettings, AuthType, ConfigDocument, ConfigValidationError, DomainConfig, GroupRecord,
    HostRecord, SCHEMA_VERSION, ThemePreference, TunnelRecord,
};
pub use store::{ConfigStore, ConfigStoreError};
