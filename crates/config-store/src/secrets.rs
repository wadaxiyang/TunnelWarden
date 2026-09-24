use thiserror::Error;
use tunnel_domain::SecretRef;
use zeroize::Zeroizing;

const SERVICE: &str = "TunnelWarden";
const MAX_SECRET_BYTES: usize = 16 * 1024;

#[derive(Debug, Error)]
pub enum SecretStoreError {
    #[error("invalid credential reference")]
    InvalidReference,
    #[error("credential exceeds 16 KiB")]
    TooLarge,
    #[error("Windows Credential Manager or system keyring failed: {0}")]
    Keyring(#[from] keyring::Error),
}

/// Stores SSH passwords and key passphrases in the OS credential store.
/// Calls perform blocking platform I/O and belong on a Tokio blocking worker,
/// never the GPUI thread. Configuration files hold only the opaque reference.
pub struct SecretStore;

impl SecretStore {
    pub fn load(reference: &SecretRef) -> Result<Zeroizing<String>, SecretStoreError> {
        let entry = entry(reference)?;
        let secret = Zeroizing::new(entry.get_password()?);
        if secret.len() > MAX_SECRET_BYTES {
            return Err(SecretStoreError::TooLarge);
        }
        Ok(secret)
    }

    pub fn save(reference: &SecretRef, secret: &Zeroizing<String>) -> Result<(), SecretStoreError> {
        if secret.len() > MAX_SECRET_BYTES {
            return Err(SecretStoreError::TooLarge);
        }
        entry(reference)?
            .set_password(secret)
            .map_err(SecretStoreError::Keyring)
    }

    pub fn delete(reference: &SecretRef) -> Result<(), SecretStoreError> {
        entry(reference)?
            .delete_credential()
            .map_err(SecretStoreError::Keyring)
    }
}

fn entry(reference: &SecretRef) -> Result<keyring::Entry, SecretStoreError> {
    if reference.0.is_empty()
        || reference.0.len() > 128
        || !reference
            .0
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(SecretStoreError::InvalidReference);
    }
    keyring::Entry::new(SERVICE, &reference.0).map_err(SecretStoreError::Keyring)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_reference_before_platform_access() {
        assert!(matches!(
            entry(&SecretRef("".into())),
            Err(SecretStoreError::InvalidReference)
        ));
        assert!(matches!(
            entry(&SecretRef("bad/name".into())),
            Err(SecretStoreError::InvalidReference)
        ));
    }
}
