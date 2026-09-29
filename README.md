# TunnelWarden — Astra Development Scaffold

TunnelWarden is a small desktop SSH port-forward manager built around **Oryxis' existing SSH implementation**, not a new SSH implementation.

## Non-negotiable implementation strategy

1. **`oryxis-core` and `oryxis-ssh` are upstream dependencies**, pinned to Oryxis commit `16ad2af83b424780ff60b6420a6c20a3b30c7143`.
2. Standalone port-forward runtime behavior is **ported from Oryxis `oryxis-app`**, especially `dispatch_port_forwards.rs`.
3. If Oryxis already implements a required SSH / forwarding / retry behavior, **copy or call it; do not redesign it**.
4. TunnelWarden-owned Rust code should mostly be adapters: persistence, Tauri commands/events, runtime registry integration, and application-specific DTOs.
5. The GUI is new: **Tauri 2 + Svelte 5 + TypeScript**. Do not copy Oryxis' iced UI.

## Target scope

- Saved SSH hosts
- Local forwarding (`-L`)
- Remote forwarding (`-R`)
- Dynamic SOCKS forwarding (`-D`)
- Jump hosts / jump chains through Oryxis
- Password / private-key / ssh-agent auth through Oryxis
- Start / stop / restart
- Auto-start
- Auto-reconnect using Oryxis' standalone-forward retry semantics
- Shared PTY-less SSH connection for multiple forwards to the same host, as Oryxis already does
- Logs
- Tray and launch-at-login
- Small, precise configuration UI

## Start here

Read in this order:

1. `AGENTS.md`
2. `docs/00_PROJECT_CHARTER.md`
3. `docs/01_UPSTREAM_AND_LICENSE.md`
4. `docs/02_COPY_MAP.md`
5. `docs/03_ARCHITECTURE.md`
6. `docs/04_DEVELOPMENT_BOUNDARIES.md`
7. `docs/05_IMPLEMENTATION_PHASES.md`
8. `docs/06_TAURI_API_CONTRACT.md`
9. `docs/07_UI_SPEC.md`
10. `docs/08_ACCEPTANCE_MATRIX.md`
11. `docs/09_ASTRA_EXECUTION.md`

Run `scripts/fetch-oryxis-reference.ps1` on Windows or `scripts/fetch-oryxis-reference.sh` on Unix before implementation. It materializes the exact upstream source files referenced by the migration plan.
