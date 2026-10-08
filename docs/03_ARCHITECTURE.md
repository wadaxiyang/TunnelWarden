# 03 — Architecture

```text
Svelte 5 / TypeScript
        │
        │ Tauri commands + events
        ▼
src-tauri (thin shell)
        │
        ▼
tunnelwarden-core
  ├─ persistence adapter
  ├─ credential adapter
  ├─ TunnelRuntimeManager
  │    ├─ active_forwards     (ported from Oryxis app)
  │    ├─ forward_conns       (ported from Oryxis app)
  │    ├─ starting            (ported from Oryxis app)
  │    └─ retries             (ported from Oryxis app)
  │
  ├───────────────┬────────────────┐
  ▼               ▼                ▼
oryxis-core    oryxis-ssh       Tokio
                  │
                  └─ russh
```

## Layer ownership

### `oryxis-core` / `oryxis-ssh`
Authoritative SSH semantics. TunnelWarden should adapt around them instead of penetrating them with duplicated types or alternate implementations.

### `tunnelwarden-core`
Owns the standalone-forward application runtime and persistence-facing services. Runtime behavior should be a direct adaptation of Oryxis' existing standalone forward implementation.

### `src-tauri`
Serializes/deserializes commands, emits state/log events, creates tray entries, and wires launch-at-login. It contains no SSH decisions.

### Svelte UI
Pure presentation and user intent. It renders DTOs and sends commands; it never infers network truth locally.

## Runtime model

Persisted rule state and live runtime state are separate. A persisted rule describes what should be started. Runtime registries describe what is actually alive.

UI status is intentionally small:

```text
stopped
connecting
running
retrying
error
```

Do not introduce generation/attempt/revision machinery unless a concrete regression proves Oryxis' simpler runtime model insufficient.

## v0.2 window ownership

The Rust AppService and tray are process-scoped. Main window config is a
creation template (`create: false`), not a startup WebView. `ui::show` serializes
creation on an async thread as required by Tauri/WebView2. Close destroys the
window; implicit last-window exit is prevented. Explicit quit stops SSH first.
Launch-at-login is headless. Opening/reopening the interface does not rerun
runtime auto-start. New manual authentication questions can summon the UI;
unattended retry remains strict and never prompts. Pending questions/tests
are cancelled when the interface is destroyed; established tunnels continue.

## v0.2.1 native tray presentation

The hover panel is a Win32 tool window with a read-only native edit control,
created after a 500 ms hover and destroyed on dismissal. It never creates a
WebView or activates/focuses itself. One shell worker waits without polling
when no hover/panel exists; while visible it samples payload counters once per
second. Rates use elapsed monotonic time and reset their baseline on reattach.
Native menu entries retain handles across status updates; configuration changes
rebuild only the Tunnels children. Every action calls the existing AppService.

Panel theme handling is native shell glue: read the Windows personalization
preference on creation and WM_SETTINGCHANGE/WM_THEMECHANGED/WM_SYSCOLORCHANGE,
with high-contrast system colors taking precedence. Per-window GDI brushes live
until after HWND destruction. No theme watcher thread, WebView or background
polling is added; a closed panel reads the latest preference when next created.

## v0.2.2 detail sampling

Traffic IPC exposes decimal counters and monotonic sample/attachment metadata,
not runtime handles. A mounted visible detail component samples sequentially;
no overlapping requests, application timer or background history is added.
Unmount/visibility cleanup stops polling; reconnect/paused baselines are reset.
Chart memory is capped at 301 points/5 minutes. State history reuses LogBook.
