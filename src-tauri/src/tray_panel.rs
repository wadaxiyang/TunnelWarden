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
        System::{LibraryLoader::GetModuleHandleW, Registry::*},
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
            Controls::{EM_GETFIRSTVISIBLELINE, EM_LINESCROLL, SetWindowTheme},
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
                    let lines = panel_text(
                        &service.list_tunnels(),
                        &service.get_runtime_snapshot(),
                        &Default::default(),
                        service.get_settings().language,
                    )
                    .lines()
                    .count();
                    match Panel::new(rect, lines) {
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
                    service.get_settings().language,
                ));
                sampled = Instant::now();
            }
        } else {
            outside = None;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Palette {
    background: COLORREF,
    text: COLORREF,
    border: COLORREF,
    dark: bool,
    high_contrast: bool,
}
impl Palette {
    const LIGHT: Self = Self {
        background: 0x00f9f9f9,
        text: 0x001b1b1b,
        border: 0x00d0d0d0,
        dark: false,
        high_contrast: false,
    };
    const DARK: Self = Self {
        background: 0x00202020,
        text: 0x00f3f3f3,
        border: 0x00454545,
        dark: true,
        high_contrast: false,
    };
    fn current() -> Self {
        // This is a shell flyout: prefer Windows (taskbar) mode over app mode.
        // Missing/older personalization settings fall back to app mode, then light.
        unsafe {
            let mut contrast = HIGHCONTRASTW {
                cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
                ..std::mem::zeroed()
            };
            if SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                contrast.cbSize,
                (&mut contrast as *mut HIGHCONTRASTW).cast(),
                0,
            ) != 0
                && contrast.dwFlags & HCF_HIGHCONTRASTON != 0
            {
                return Self {
                    background: GetSysColor(COLOR_WINDOW),
                    text: GetSysColor(COLOR_WINDOWTEXT),
                    border: GetSysColor(COLOR_WINDOWFRAME),
                    high_contrast: true,
                    ..Self::LIGHT
                };
            }
            for key in [w!("SystemUsesLightTheme"), w!("AppsUseLightTheme")] {
                let mut light = 1u32;
                let mut size = std::mem::size_of_val(&light) as u32;
                if RegGetValueW(
                    HKEY_CURRENT_USER,
                    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
                    key,
                    RRF_RT_REG_DWORD,
                    std::ptr::null_mut(),
                    (&mut light as *mut u32).cast(),
                    &mut size,
                ) == ERROR_SUCCESS
                    && light <= 1
                {
                    return if light == 0 { Self::DARK } else { Self::LIGHT };
                }
            }
            Self::LIGHT
        }
    }
}

struct PaintState {
    palette: Palette,
    background: HBRUSH,
    border: HBRUSH,
}
impl PaintState {
    fn new(palette: Palette) -> std::io::Result<Self> {
        let state = Self {
            palette,
            background: unsafe { CreateSolidBrush(palette.background) },
            border: unsafe { CreateSolidBrush(palette.border) },
        };
        if state.background.is_null() || state.border.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        Ok(state)
    }
}
impl Drop for PaintState {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.background);
            DeleteObject(self.border);
        }
    }
}

// SAFETY: the boxed state outlives HWND, is accessed only on its owner thread,
// and no borrow into it spans a call that can reenter this window procedure.
unsafe fn apply_palette(hwnd: HWND, palette: Palette) {
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PaintState;
        if state.is_null() {
            return;
        }
        if (*state).palette != palette {
            let Ok(replacement) = PaintState::new(palette) else {
                return;
            };
            *state = replacement;
        }
        let edit = GetWindow(hwnd, GW_CHILD);
        if !edit.is_null() {
            // Best effort for native scrollbars; the content palette does not
            // depend on this Windows theme class being available.
            let theme = if palette.high_contrast {
                std::ptr::null()
            } else if palette.dark {
                w!("DarkMode_Explorer")
            } else {
                w!("Explorer")
            };
            SetWindowTheme(edit, theme, std::ptr::null());
        }
        RedrawWindow(
            hwnd,
            std::ptr::null(),
            std::ptr::null_mut(),
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_FRAME,
        );
    }
}

unsafe extern "system" fn panel_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lp as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PaintState;
        if !state.is_null() {
            match message {
                // Read-only EDIT controls request STATIC colors, not EDIT colors.
                WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
                    SetTextColor(wp as HDC, (*state).palette.text);
                    SetBkColor(wp as HDC, (*state).palette.background);
                    return (*state).background as LRESULT;
                }
                WM_ERASEBKGND => return 1, // WM_PAINT fills the entire surface.
                WM_PAINT | WM_PRINTCLIENT => {
                    let mut paint: PAINTSTRUCT = std::mem::zeroed();
                    let dc = if message == WM_PAINT {
                        BeginPaint(hwnd, &mut paint)
                    } else {
                        wp as HDC
                    };
                    let mut rect = RECT::default();
                    GetClientRect(hwnd, &mut rect);
                    FillRect(dc, &rect, (*state).background);
                    FrameRect(dc, &rect, (*state).border);
                    if message == WM_PAINT {
                        EndPaint(hwnd, &paint);
                    }
                    return 0;
                }
                WM_SETTINGCHANGE | WM_THEMECHANGED | WM_SYSCOLORCHANGE => {
                    apply_palette(hwnd, Palette::current());
                }
                WM_NCDESTROY => {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, wp, lp)
    }
}

struct Panel {
    hwnd: HWND,
    edit: HWND,
    font: HFONT,
    bounds: Bounds,
    text: String,
    // Stable address for GWLP_USERDATA; dropped after DestroyWindow.
    _paint: Box<PaintState>,
}
impl Panel {
    fn new(anchor: Bounds, lines: usize) -> std::io::Result<Self> {
        // SAFETY: all pointers refer to live UTF-16 strings/stack structs during
        // synchronous calls. Handles are checked and released on their owner thread.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(panel_proc),
                hInstance: instance,
                lpszClassName: w!("TunnelWardenTrayStatus"),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
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
                scale(360),
                scale((24 + lines.min(100) as i32 * 19).clamp(82, 360)),
                scale(8),
            );
            let mut paint = Box::new(PaintState::new(Palette::current())?);
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                class.lpszClassName,
                w!("TunnelWarden status"),
                WS_POPUP | WS_CLIPCHILDREN,
                bounds.left,
                bounds.top,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                (&mut *paint as *mut PaintState).cast(),
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
                _paint: paint,
            };
            if edit.is_null() || font.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            SendMessageW(edit, WM_SETFONT, font as usize, 1);
            apply_palette(hwnd, panel._paint.palette);
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
            // SetWindowPos avoids ShowWindow's first-call STARTUPINFO override
            // (a headless/login launch can carry SW_HIDE).
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
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
    fn native_theme_repaints_background_text_and_border_without_resetting_content() {
        let _serial = NATIVE_TEST.lock().unwrap();
        unsafe {
            let foreground = GetForegroundWindow();
            let mut panel = Panel::new(Bounds::default(), 100).unwrap();
            let text = "Tunnel — Running\r\nUpload 12 KiB/s\r\n".repeat(40);
            panel.update(&text);
            SendMessageW(panel.edit, EM_LINESCROLL, 0, 12);
            let line = SendMessageW(panel.edit, EM_GETFIRSTVISIBLELINE, 0, 0);
            assert!(line > 0);
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: 1024,
                    biHeight: -1024,
                    biPlanes: 1,
                    biBitCount: 32,
                    ..std::mem::zeroed()
                },
                ..std::mem::zeroed()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap = CreateDIBSection(
                dc,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            assert!(!bitmap.is_null());
            let previous = SelectObject(dc, bitmap);
            // Inject palettes without changing the user's registry or theme.
            for palette in [
                Palette::DARK,
                Palette::LIGHT,
                Palette {
                    background: 0,
                    text: 0x0000ffff,
                    border: 0x00ffffff,
                    high_contrast: true,
                    ..Palette::LIGHT
                },
            ] {
                apply_palette(panel.hwnd, palette);
                panel.update(&text); // Same text must not prevent theme repaint.
                SendMessageW(panel.hwnd, WM_PRINTCLIENT, dc as usize, PRF_CLIENT as isize);
                assert_eq!(GetPixel(dc, 0, 0), palette.border);
                assert_eq!(GetPixel(dc, 5, 5), palette.background);
                let brush = SendMessageW(
                    panel.hwnd,
                    WM_CTLCOLORSTATIC,
                    dc as usize,
                    panel.edit as isize,
                );
                assert_ne!(brush, 0);
                assert_eq!(GetTextColor(dc), palette.text);
                assert_eq!(GetBkColor(dc), palette.background);
                // Render the real read-only EDIT to catch default white fills.
                SendMessageW(panel.edit, WM_PRINTCLIENT, dc as usize, PRF_CLIENT as isize);
                let mut rect = RECT::default();
                GetClientRect(panel.edit, &mut rect);
                assert_eq!(GetPixel(dc, rect.right - 4, 5), palette.background);
                assert_eq!(SendMessageW(panel.edit, EM_GETFIRSTVISIBLELINE, 0, 0), line);
                assert_eq!(GetForegroundWindow(), foreground);
            }
            // System notifications must reload preferences even with static text.
            for message in [WM_SETTINGCHANGE, WM_THEMECHANGED, WM_SYSCOLORCHANGE] {
                apply_palette(panel.hwnd, Palette::DARK);
                SendMessageW(panel.hwnd, message, 0, 0);
                assert_eq!(panel._paint.palette, Palette::current());
                assert_eq!(SendMessageW(panel.edit, EM_GETFIRSTVISIBLELINE, 0, 0), line);
            }
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }

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
                let mut panel = Panel::new(anchor, 120).unwrap();
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
        // Let two real telemetry refreshes run; an empty configuration is still
        // a useful panel, and the background worker must remain responsive.
        std::thread::sleep(Duration::from_millis(2200));
        unsafe {
            let hwnd = FindWindowW(w!("TunnelWardenTrayStatus"), w!("TunnelWarden status"));
            assert_ne!(IsWindowVisible(hwnd), 0);
            let edit = GetWindow(hwnd, GW_CHILD);
            let mut text = [0u16; 2048];
            let n = GetWindowTextW(edit, text.as_mut_ptr(), text.len() as i32);
            assert!(String::from_utf16_lossy(&text[..n as usize]).contains(
                tunnelwarden_core::i18n::text(
                    service.get_settings().language,
                    "No running tunnels"
                )
            ));
        }
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
