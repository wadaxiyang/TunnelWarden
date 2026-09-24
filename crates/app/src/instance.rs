use std::{
    hash::{Hash, Hasher},
    io, ptr,
    thread::{self, JoinHandle},
};

use tokio::sync::mpsc;
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0},
    System::Threading::{CreateEventW, CreateMutexW, SetEvent, WaitForMultipleObjects},
};

pub enum InstanceStart {
    Primary(InstanceGuard),
    Existing,
}

pub struct InstanceGuard {
    mutex: HANDLE,
    wake: HANDLE,
    shutdown: HANDLE,
    thread: Option<JoinHandle<()>>,
}

impl InstanceGuard {
    pub fn acquire(open: mpsc::Sender<()>) -> Result<InstanceStart, String> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .hash(&mut hasher);
        let suffix = hasher.finish();
        let mutex_name = wide(&format!("Local\\TunnelWarden-{suffix:016x}-Mutex"));
        let wake_name = wide(&format!("Local\\TunnelWarden-{suffix:016x}-Wake"));
        let mutex = unsafe { CreateMutexW(ptr::null(), 0, mutex_name.as_ptr()) };
        if mutex.is_null() {
            return Err(io::Error::last_os_error().to_string());
        }
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let wake = unsafe { CreateEventW(ptr::null(), 0, 0, wake_name.as_ptr()) };
        if wake.is_null() {
            unsafe {
                CloseHandle(mutex);
            }
            return Err(io::Error::last_os_error().to_string());
        }
        if exists {
            unsafe {
                SetEvent(wake);
                CloseHandle(wake);
                CloseHandle(mutex);
            }
            return Ok(InstanceStart::Existing);
        }
        let shutdown = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if shutdown.is_null() {
            unsafe {
                CloseHandle(wake);
                CloseHandle(mutex);
            }
            return Err(io::Error::last_os_error().to_string());
        }
        let wake_value = wake as usize;
        let shutdown_value = shutdown as usize;
        let thread = match thread::Builder::new()
            .name("tunnelwarden-instance".into())
            .spawn(move || {
                let handles = [wake_value as HANDLE, shutdown_value as HANDLE];
                loop {
                    let result =
                        unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, u32::MAX) };
                    if result == WAIT_OBJECT_0 {
                        let _ = open.try_send(());
                    } else {
                        break;
                    }
                }
            }) {
            Ok(thread) => thread,
            Err(error) => {
                unsafe {
                    CloseHandle(shutdown);
                    CloseHandle(wake);
                    CloseHandle(mutex);
                }
                return Err(error.to_string());
            }
        };
        Ok(InstanceStart::Primary(Self {
            mutex,
            wake,
            shutdown,
            thread: Some(thread),
        }))
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            SetEvent(self.shutdown);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        unsafe {
            CloseHandle(self.shutdown);
            CloseHandle(self.wake);
            CloseHandle(self.mutex);
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
