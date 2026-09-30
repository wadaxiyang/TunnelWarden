//! A disposable Win32 text window. No WebView, activation, background polling,
//! SSH handles or network implementation. All HWND/GDI use stays on one thread.
use super::tray_view::{Bounds, RateSampler, panel_text, place};
use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
use tunnelwarden_core::service::AppService;
use windows_sys::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{EM_GETFIRSTVISIBLELINE, EM_LINESCROLL},
            HiDpi::*,
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

enum Command {
    Enter(Bounds),
    Leave,
    Hide,
    Quit,
}
pub struct TrayPanel {
    tx: mpsc::Sender<Command>,
}
impl TrayPanel {
    pub fn new(service: Arc<AppService>) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("tray-status".into())
            .spawn(move || worker(service, rx))?;
        Ok(Self { tx })
    }
    pub fn enter(&self, rect: tauri::Rect) {
        let p = rect.position.to_physical::<i32>(1.0);
        let s = rect.size.to_physical::<i32>(1.0);
        let _ = self.tx.send(Command::Enter(Bounds {
            left: p.x,
            top: p.y,
            right: p.x + s.width,
            bottom: p.y + s.height,
        }));
    }
    pub fn leave(&self) {
        let _ = self.tx.send(Command::Leave);
    }
    pub fn hide(&self) {
        let _ = self.tx.send(Command::Hide);
    }
    pub fn shutdown(&self) {
        let _ = self.tx.send(Command::Quit);
    }
}
impl Drop for TrayPanel {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn worker(service: Arc<AppService>, rx: mpsc::Receiver<Command>) {
    // SAFETY: this worker exclusively owns and dispatches its windows. The
    // thread uses per-monitor DPI, and never calls SetForegroundWindow.
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let mut panel: Option<Panel> = None;
    let mut pending: Option<(Bounds, Instant)> = None;
    let mut anchor = Bounds::default();
    let mut blocked = false;
    let mut outside = None;
    let mut sampler = RateSampler::default();
    let mut sampled = Instant::now();
    loop {
        let command = if pending.is_none() && panel.is_none() {
            rx.recv().ok()
        } else {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(c) => Some(c),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };
        match command {
            Some(Command::Enter(rect)) if !blocked => {
                anchor = rect;
                if pending.is_none() && panel.is_none() {
                    pending = Some((rect, Instant::now()));
                }
            }
            Some(Command::Leave) => {
                blocked = false;
            }
            Some(Command::Hide) => {
                panel = None;
                pending = None;
                blocked = true;
            }
            Some(Command::Quit) => break,
            None if pending.is_none() && panel.is_none() => break,
            _ => {}
        }
        // SAFETY: messages and cursor coordinates are used only on this thread.
        unsafe {
            let mut message: MSG = std::mem::zeroed();
            while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            // A menu runs a modal loop; dismiss even before Tauri receives the
            // queued click event. This also covers keyboard-opened native menus.
            let mut gui: GUITHREADINFO = std::mem::zeroed();
            gui.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
            if GetGUIThreadInfo(0, &mut gui) != 0 && gui.flags & GUI_INMENUMODE != 0 {
                panel = None;
                pending = None;
                continue;
            }
            let mut cursor = POINT::default();
            if GetCursorPos(&mut cursor) == 0 {
                panel = None;
                pending = None;
                continue;
            }
            if let Some((rect, since)) = pending {
                if !rect.contains(cursor.x, cursor.y) {
                    pending = None;
                } else if since.elapsed() >= Duration::from_millis(500) {
                    pending = None;
                    match Panel::new(rect) {
                        Ok(created) => {
                            panel = Some(created);
                            sampler = RateSampler::default();
                            sampled = Instant::now() - Duration::from_secs(2);
                        }
                        Err(e) => {
                            service
                                .logs
                                .push("error", None, format!("Tray status panel: {e}"));
                            blocked = true;
                        }
                    }
                }
            }
            if let Some(p) = panel.as_ref() {
                let inside =
                    anchor.contains(cursor.x, cursor.y) || p.bounds.contains(cursor.x, cursor.y);
                if inside {
                    outside = None;
                } else if outside.get_or_insert_with(Instant::now).elapsed()
                    > Duration::from_millis(350)
                {
                    panel = None;
                    outside = None;
                }
            }
        }
        if let Some(p) = panel.as_mut() {
            if sampled.elapsed() >= Duration::from_secs(1) {
                let samples = tauri::async_runtime::block_on(async {
                    tokio::time::timeout(Duration::from_millis(500), service.runtime.traffic())
                        .await
                });
                let rates = sampler.sample(
                    Instant::now(),
                    samples.ok().and_then(Result::ok).unwrap_or_default(),
                );
                p.update(&panel_text(
                    &service.list_tunnels(),
                    &service.get_runtime_snapshot(),
                    &rates,
                ));
                sampled = Instant::now();
            }
        } else {
            outside = None;
        }
    }
}

struct Panel {
    hwnd: HWND,
    edit: HWND,
    font: HFONT,
    bounds: Bounds,
    text: String,
}
impl Panel {
    fn new(anchor: Bounds) -> std::io::Result<Self> {
        // SAFETY: all pointers refer to live UTF-16 strings/stack structs during
        // synchronous calls. Handles are checked and released on their owner thread.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(DefWindowProcW),
                hInstance: instance,
                lpszClassName: w!("TunnelWardenTrayStatus"),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
                ..std::mem::zeroed()
            };
            RegisterClassW(&class); // Already registered after the first hover is fine.
            let monitor = MonitorFromPoint(
                POINT {
                    x: anchor.left,
                    y: anchor.top,
                },
                MONITOR_DEFAULTTONEAREST,
            );
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..std::mem::zeroed()
            };
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut dpi = 96;
            let mut dpi_y = 96;
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);
            let scale = |v: i32| v * dpi as i32 / 96;
            let r = info.rcWork;
            let bounds = place(
                anchor,
                Bounds {
                    left: r.left,
                    top: r.top,
                    right: r.right,
                    bottom: r.bottom,
                },
                scale(480),
                scale(360),
                scale(8),
            );
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                class.lpszClassName,
                w!("TunnelWarden status"),
                WS_POPUP | WS_BORDER,
                bounds.left,
                bounds.top,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let font = CreateFontW(
                -scale(14),
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32,
                w!("Segoe UI"),
            );
            let edit = CreateWindowExW(
                0,
                w!("EDIT"),
                w!(""),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_VSCROLL
                    | ES_MULTILINE as u32
                    | ES_READONLY as u32
                    | ES_AUTOVSCROLL as u32,
                scale(16),
                scale(12),
                bounds.right - bounds.left - scale(32),
                bounds.bottom - bounds.top - scale(24),
                hwnd,
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            let panel = Self {
                hwnd,
                edit,
                font,
                bounds,
                text: String::new(),
            };
            if edit.is_null() || font.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            SendMessageW(edit, WM_SETFONT, font as usize, 1);
            Ok(panel)
        }
    }
    fn update(&mut self, text: &str) {
        if self.text == text {
            return;
        }
        self.text = text.into();
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        // Preserve scroll position across live updates. The read-only native edit
        // exposes its full text to Windows accessibility and supports mouse scrolling.
        unsafe {
            let line = SendMessageW(self.edit, EM_GETFIRSTVISIBLELINE, 0, 0);
            SendMessageW(self.edit, WM_SETREDRAW, 0, 0);
            SetWindowTextW(self.edit, wide.as_ptr());
            SendMessageW(self.edit, EM_LINESCROLL, 0, line);
            SendMessageW(self.edit, WM_SETREDRAW, 1, 0);
            InvalidateRect(self.edit, std::ptr::null(), 1);
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
        }
    }
}
impl Drop for Panel {
    fn drop(&mut self) {
        // Destroy children before deleting their font. No HWND leaves this thread.
        unsafe {
            DestroyWindow(self.hwnd);
            if !self.font.is_null() {
                DeleteObject(self.font);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    static NATIVE_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    #[test]
    fn native_panel_is_read_only_nonactivating_scrollable_and_disposable() {
        let _serial = NATIVE_TEST.lock().unwrap();
        // Exercise real User32/GDI controls on the Windows test desktop.
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let foreground = GetForegroundWindow();
            let mut cursor = POINT::default();
            assert_ne!(GetCursorPos(&mut cursor), 0);
            let anchor = Bounds {
                left: cursor.x,
                top: cursor.y,
                right: cursor.x + 24,
                bottom: cursor.y + 24,
            };
            for _ in 0..3 {
                let mut panel = Panel::new(anchor).unwrap();
                let text = (0..40)
                    .map(|i| {
                        format!(
                            "Tunnel {i} 中文 — Running\r\n  Upload 12 KiB/s  Download 24 KiB/s\r\n"
                        )
                    })
                    .collect::<String>();
                panel.update(&text);
                assert_ne!(IsWindowVisible(panel.hwnd), 0);
                assert_eq!(GetForegroundWindow(), foreground);
                assert_ne!(
                    GetWindowLongW(panel.hwnd, GWL_EXSTYLE) as u32 & WS_EX_NOACTIVATE,
                    0
                );
                assert_ne!(
                    GetWindowLongW(panel.edit, GWL_STYLE) as u32 & ES_READONLY as u32,
                    0
                );
                let mut buffer = vec![0u16; text.encode_utf16().count() + 1];
                let n = GetWindowTextW(panel.edit, buffer.as_mut_ptr(), buffer.len() as i32);
                assert_eq!(String::from_utf16_lossy(&buffer[..n as usize]), text);
                SendMessageW(panel.edit, EM_LINESCROLL, 0, 12);
                let scrolled = SendMessageW(panel.edit, EM_GETFIRSTVISIBLELINE, 0, 0);
                assert!(scrolled > 0);
                panel.update(&text.replace("12 KiB/s", "30 KiB/s"));
                assert_eq!(
                    SendMessageW(panel.edit, EM_GETFIRSTVISIBLELINE, 0, 0),
                    scrolled
                );
                let hwnd = panel.hwnd;
                drop(panel);
                assert_eq!(IsWindow(hwnd), 0);
            }
        }
    }

    #[test]
    fn hover_worker_debounces_cancels_reopens_and_shuts_down() {
        use tunnelwarden_core::{credentials::PlatformSecrets, storage::ConfigStore};
        let _serial = NATIVE_TEST.lock().unwrap();
        let store = Arc::new(
            ConfigStore::open(
                std::env::temp_dir().join(format!("tray-test-{}.json", uuid::Uuid::new_v4())),
            )
            .unwrap(),
        );
        let service = tauri::async_runtime::block_on(async {
            AppService::new(store, Arc::new(PlatformSecrets))
        });
        let panel = TrayPanel::new(service.clone()).unwrap();
        // Inject hover events into the real worker without moving the user's
        // pointer. A broad synthetic anchor keeps this independent of its position.
        let anchor = Bounds {
            left: -32000,
            top: -32000,
            right: 32000,
            bottom: 32000,
        };
        let visible = || unsafe {
            !FindWindowW(w!("TunnelWardenTrayStatus"), w!("TunnelWarden status")).is_null()
        };
        panel.tx.send(Command::Enter(anchor)).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!visible(), "Short hover should not create a window");
        panel.hide();
        std::thread::sleep(Duration::from_millis(600));
        assert!(!visible(), "Click must cancel pending hover");
        panel.leave();
        panel.tx.send(Command::Enter(anchor)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while !visible() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(visible(), "Sustained hover must create the native window");
        panel.hide();
        let deadline = Instant::now() + Duration::from_secs(2);
        while visible() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(!visible(), "Dismiss must destroy the window");
        panel.leave();
        panel.tx.send(Command::Enter(anchor)).unwrap();
        panel.shutdown();
        std::thread::sleep(Duration::from_millis(600));
        assert!(!visible(), "Quit must cancel a pending reopen");
        tauri::async_runtime::block_on(service.runtime.shutdown()).unwrap();
    }
}
