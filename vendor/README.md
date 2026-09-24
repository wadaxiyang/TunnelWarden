# Windows GPUI platform patch

`gpui-pre-windows` is a local copy of version 0.3.6 from crates.io, licensed
under Apache-2.0 (see its `LICENSE-APACHE`). Cargo.toml patches that package
because closing a Windows window can destroy the HWND before its deferred
`WindowsWindow::drop` calls `RevokeDragDrop`. The registered COM drop target
holds `WindowsWindowInner` and its DirectX renderer alive. This patch tracks
registration and revokes it on `WM_CLOSE` while the HWND is still valid.

The patch should be removed when the upstream Windows platform crate handles
drop-target lifetime on close. See `window.rs` and `events.rs` for the changes.
