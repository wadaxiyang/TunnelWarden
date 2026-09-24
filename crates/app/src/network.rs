use std::{ffi::c_void, ptr};

use tunnel_core::{CoreCommand, ManagerHandle};
use windows_sys::Win32::{
    Foundation::HANDLE,
    NetworkManagement::IpHelper::{
        CancelMibChangeNotify2, MIB_IPINTERFACE_ROW, MIB_NOTIFICATION_TYPE, MibAddInstance,
        MibDeleteInstance, MibInitialNotification, NotifyIpInterfaceChange,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkEvent {
    Offline,
    Online,
    InterfaceChanged,
}

struct CallbackContext {
    manager: ManagerHandle,
}

pub struct NetworkWatcher {
    handle: HANDLE,
    context: *mut CallbackContext,
}

impl NetworkWatcher {
    pub fn new(manager: ManagerHandle) -> Result<Self, String> {
        let context = Box::into_raw(Box::new(CallbackContext { manager }));
        let mut handle = ptr::null_mut();
        // The callback only performs a bounded try_send and never waits for GPUI or Tokio.
        let status = unsafe {
            NotifyIpInterfaceChange(0, Some(on_change), context.cast(), false, &mut handle)
        };
        if status != 0 {
            unsafe {
                drop(Box::from_raw(context));
            }
            return Err(format!(
                "Windows IP interface notification failed: {status}"
            ));
        }
        Ok(Self { handle, context })
    }
}

unsafe extern "system" fn on_change(
    context: *const c_void,
    row: *const MIB_IPINTERFACE_ROW,
    kind: MIB_NOTIFICATION_TYPE,
) {
    if context.is_null() || row.is_null() || kind == MibInitialNotification {
        return;
    }
    let context = unsafe { &*context.cast::<CallbackContext>() };
    let row = unsafe { &*row };
    let event = if kind == MibDeleteInstance || !row.Connected {
        NetworkEvent::Offline
    } else if kind == MibAddInstance {
        NetworkEvent::Online
    } else {
        NetworkEvent::InterfaceChanged
    };
    if matches!(event, NetworkEvent::Online | NetworkEvent::InterfaceChanged) {
        let _ = context.manager.try_send(CoreCommand::NetworkRecovered);
    }
}

impl Drop for NetworkWatcher {
    fn drop(&mut self) {
        // Windows requires deregistration from outside the notification callback.
        let status = unsafe { CancelMibChangeNotify2(self.handle) };
        if status == 0 {
            unsafe {
                drop(Box::from_raw(self.context));
            }
        } else {
            // Keep the callback context alive if Windows did not deregister it.
            eprintln!("Could not cancel IP interface notification: {status}");
        }
    }
}
