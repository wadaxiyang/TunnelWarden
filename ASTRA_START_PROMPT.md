# Astra Start Prompt

Use this as the first implementation prompt after opening this repository:

> Read `AGENTS.md`, then read every document in `docs/` in numeric order. This project is intentionally **copy-first**. Oryxis is pinned in `UPSTREAM.lock`; materialize its reference sources using the supplied script before implementing the backend. Use `oryxis-core` and `oryxis-ssh` as the SSH dependencies. Port the standalone port-forward runtime from the pinned Oryxis `crates/oryxis-app/src/dispatch_port_forwards.rs` and supporting files named in `docs/02_COPY_MAP.md`. Do not redesign or independently reimplement SSH, `-L/-R/-D`, jump-host logic, connection sharing, cancellation, or reconnect/backoff when Oryxis already provides it. Replace only Oryxis/iced application glue with `tunnelwarden-core` + Tokio, then expose a thin Tauri command/event layer and build the Svelte UI. Work through `docs/05_IMPLEMENTATION_PHASES.md` sequentially and keep the acceptance matrix green before cleanup or refactoring.

The intended first action is **source materialization and inspection**, not writing a new state machine.
