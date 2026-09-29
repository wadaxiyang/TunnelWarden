//! OS credential adapter, separate from plaintext configuration.
use uuid::Uuid;

pub trait SecretStore: Send + Sync + 'static {
    fn get(&self, host: Uuid) -> Result<Option<String>, String>;
    fn set(&self, host: Uuid, password: &str) -> Result<(), String>;
    fn delete(&self, host: Uuid) -> Result<(), String>;
}
pub struct PlatformSecrets;

#[cfg(windows)]
impl SecretStore for PlatformSecrets {
    fn get(&self, host: Uuid) -> Result<Option<String>, String> {
        match entry(host)?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Windows Credential Manager: {e}")),
        }
    }
    fn set(&self, host: Uuid, password: &str) -> Result<(), String> {
        entry(host)?
            .set_password(password)
            .map_err(|e| format!("Windows Credential Manager: {e}"))
    }
    fn delete(&self, host: Uuid) -> Result<(), String> {
        match entry(host)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("Windows Credential Manager: {e}")),
        }
    }
}
#[cfg(windows)]
fn entry(host: Uuid) -> Result<keyring::Entry, String> {
    keyring::Entry::new("dev.tunnelwarden.app", &host.to_string()).map_err(|e| e.to_string())
}
#[cfg(not(windows))]
impl SecretStore for PlatformSecrets {
    fn get(&self, _: Uuid) -> Result<Option<String>, String> {
        Err("This release supports Windows Credential Manager only".into())
    }
    fn set(&self, _: Uuid, _: &str) -> Result<(), String> {
        Err("This release supports Windows Credential Manager only".into())
    }
    fn delete(&self, _: Uuid) -> Result<(), String> {
        Err("This release supports Windows Credential Manager only".into())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn windows_credential_roundtrip_and_delete() {
        let id = Uuid::new_v4();
        let store = PlatformSecrets;
        assert_eq!(store.get(id).unwrap(), None);
        store.set(id, "throwaway-test-value").unwrap();
        let read = store.get(id);
        let deleted = store.delete(id);
        assert_eq!(read.unwrap().as_deref(), Some("throwaway-test-value"));
        deleted.unwrap();
        assert_eq!(store.get(id).unwrap(), None);
    }
}
