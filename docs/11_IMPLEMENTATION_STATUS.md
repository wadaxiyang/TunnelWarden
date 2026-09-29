# Implementation status

Target: TunnelWarden 0.1.0, Windows x64. Development branch: `master`.

## Phase 0 — upstream baseline

- Read AGENTS.md, ASTRA_START_PROMPT.md and docs/00–10 in full.
- Materialized the supplied reference set using scripts/fetch-oryxis-reference.ps1.
- Verified upstream HEAD is `16ad2af83b424780ff60b6420a6c20a3b30c7143`.
- Preserved LICENSE, NOTICE.ORYXIS and THIRD_PARTY_NOTICES.md.
- Resolved the pinned Git crates into Cargo.lock. No engine vendoring or patches.
- Windows build prerequisite: aws-lc's supplied prebuilt NASM objects enabled;
  MSVC C source encoding set to UTF-8, independently of the machine code page.

## Source inspection notes

Read `port_forward_rule.rs`, `engine/forwarding.rs`, `connect_forward_conn`,
`dispatch_port_forwards.rs`, message types, timer cadence and public auth/resolver
types. The public engine supplies all three modes, explicit cancellation, SSH
authentication and jump chains. Runtime cadence is 5s liveness / 15s retry;
backoff is 15/30/60/120s. The app port will retain the shared host registry,
starting set, retry gates, agent census and late completion checks.

Persistence, IPC and OS credential storage are application adapters. No SSH
transport, SOCKS server, channel bridge or reconnect supervisor will be added.

## Remaining phases

1. Public API forwarding harness.
2. Port standalone runtime.
3. Persistence and platform credentials.
4. Thin Tauri service/command API.
5. Svelte UI.
6. Tray, login startup and logs.
7. Acceptance, CI and Windows release.

Each phase records its checks here before its commit is pushed. Git history is
the authoritative phase commit ledger (a commit cannot embed its own SHA).
