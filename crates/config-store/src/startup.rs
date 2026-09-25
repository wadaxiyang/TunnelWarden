use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("could not determine TunnelWarden executable: {0}")]
    Executable(#[from] std::io::Error),
    #[error("Windows login startup registry operation failed: {0}")]
    Registry(std::io::Error),
    #[error("login startup command is too long")]
    TooLong,
    #[error("login startup is supported only on Windows")]
    Unsupported,
}

/// Reconciles this user's Run entry with an already committed preference.
/// The caller performs this on a blocking worker and reports failures separately.
pub fn set_run_at_login(enabled: bool) -> Result<(), StartupError> {
    #[cfg(windows)]
    {
        set_windows_run(enabled, &std::env::current_exe()?)
    }
    #[cfg(not(windows))]
    {
        if enabled {
            Err(StartupError::Unsupported)
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
fn set_windows_run(enabled: bool, executable: &Path) -> Result<(), StartupError> {
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{HKEY_CURRENT_USER, REG_SZ, RegDeleteKeyValueW, RegSetKeyValueW},
    };

    let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0"
        .encode_utf16()
        .collect();
    let name: Vec<u16> = "TunnelWarden\0".encode_utf16().collect();
    let status = if enabled {
        let command = login_command(executable);
        let byte_len =
            u32::try_from(command.len().saturating_mul(2)).map_err(|_| StartupError::TooLong)?;
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                command.as_ptr().cast(),
                byte_len,
            )
        }
    } else {
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) }
    };
    if status == ERROR_SUCCESS
        || (!enabled && (status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND))
    {
        Ok(())
    } else {
        Err(StartupError::Registry(std::io::Error::from_raw_os_error(
            status as i32,
        )))
    }
}

#[cfg(windows)]
fn login_command(executable: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    let mut command = Vec::new();
    command.push('"' as u16);
    command.extend(executable.as_os_str().encode_wide());
    command.extend("\" --background\0".encode_utf16());
    command
}

#[cfg(test)]
#[cfg(windows)]
mod tests {
    use super::*;

    #[test]
    fn login_command_quotes_paths_with_spaces_and_runs_in_background() {
        let command = login_command(Path::new(r"C:\Program Files\TunnelWarden\tunnelwarden.exe"));
        assert_eq!(
            String::from_utf16_lossy(&command[..command.len() - 1]),
            r#""C:\Program Files\TunnelWarden\tunnelwarden.exe" --background"#
        );
    }
}
