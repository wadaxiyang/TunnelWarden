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

## Phase sequence (completed records below)

1. Public API forwarding harness.
2. Port standalone runtime.
3. Persistence and platform credentials.
4. Thin Tauri service/command API.
5. Svelte UI.
6. Tray, login startup and logs.
7. Acceptance, CI and Windows release.

Each phase records its checks here before its commit is pushed. Git history is
the authoritative phase commit ledger (a commit cannot embed its own SHA).

## Phase 1 — passed on Windows, 2026-09-29

- Three executable integration tests (none ignored) use fresh loopback SSH
  servers: password/private-key auth, all forwarding modes on one connection,
  byte transport, explicit cancel/rebind, rejected auth, unknown host key and
  occupied listener errors. The fixture delegates server SSH to AsyncSSH.
- `cargo fmt --all -- --check`, `cargo check --workspace`,
  `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`
  and boundary checks all passed. Scaffold Vite build also passed.
- Python 3 + `pip install -r tests/requirements.txt` are test prerequisites.
- Added a generated application icon because Tauri's workspace build embeds it.
- Source-only upstream reference remains unchanged; the harness uses only
  public Oryxis engine APIs.

## Phase 2 — passed on Windows, 2026-09-29

- Ported the standalone runtime with traceable source comments and Copy Map.
  Copied helpers retain their original comments and tests. Tokio executes the
  task/result flow; Oryxis still owns every network operation.
- Preserved host connection sharing, in-flight queue, explicit remote cancel,
  late-stop guards, liveness, auto-start, retry gates and agent census kicks.
- Added only Stop IPC completion bookkeeping to keep Stop/Start ordered while
  a remote attach is in flight. No generation/attempt supervisor was added.
- All four required Cargo checks and boundary checks passed: 9 unit tests and
  5 real SSH integration tests, none ignored. The tests include virtual-time
  retry deadlines, late attach failure, shared connection call count, remote
  delete/rebind, launch auto-start and connection drop into retry.

## Phase 3 — passed on Windows, 2026-09-29

- Versioned atomic JSON stores upstream host/rule/known-host types and user
  settings, never passwords or runtime state. Corrupt/future files fail closed
  and remain untouched. Failed edits preserve both disk and memory.
- Passwords use Windows Credential Manager (real OS roundtrip test passed).
  Key files are read on connect; encrypted keys can use an external SSH agent.
- Ported resolver and host-key callback from `connect_methods.rs`; direct,
  one-hop and nested two-hop forwarding all passed against actual test servers.
- Manual host-key consent persists the verified fingerprint; unattended unknown
  keys fail without prompting. Stop dismisses that host's pending questions.
- All four Cargo gates plus boundary checks passed: 12 unit + 7 SSH integration
  tests, none ignored. Saving auto-start describes launch policy; a manual Stop
  suppresses retries during the current run, matching Oryxis.

## Phase 4 — passed on Windows, 2026-09-29

- Implemented every query/mutation in docs/06 through AppService and thin Tauri
  commands, with runtime/log/host-key events. No engine handles cross IPC.
- Added DTO validation, jump-reference/cycle validation, OS-secret rollback on
  configuration write failure, reference-safe host deletion, and awaited tunnel
  cancel before edit/delete. Host edits refuse live/pending dependent routes.
- Headless service test exercises CRUD, validation, Start (not false-ready),
  Stop, Restart, live remote Delete and reloading saved configuration.
- All Cargo gates and boundary checks passed: 12 unit + 8 integration tests.

## Phase 5 — passed on Windows, 2026-09-29

- Svelte GUI implements all five navigation surfaces, mode-specific tunnel
  forms, independent SSH port, ordered jump routes, password/key/agent fields,
  authoritative status/events, errors, logs, settings, host-key consent and
  embedded license/attribution. Edit/Delete use explicit dialogs.
- `npm run check`: zero errors and warnings. `npm run build`: passed.
- `tests/ui-smoke.mjs` passed against a real Tauri WebView via CDP, with an
  isolated `dev.tunnelwarden.smoketest` application identifier. It exercises
  host/tunnel CRUD, editable Remote listener and Local target ports, hidden
  Dynamic target fields, editing, delete confirmation, settings and About.
  No mocked IPC backend. Test profiles were removed afterwards.
- Browser page errors: none. Accessibility audit: zero violations (one
  decorative glyph requires no text-contrast judgment). Inspected screenshots.
- Fixed Vite watching Rust DLL outputs on Windows (EBUSY) by excluding build,
  reference and Tauri directories from the frontend watcher.

## Phase 6 — passed on Windows, 2026-09-29

- Tray: open window, start all, stop all and quit. Each operation calls the same
  service/runtime path. Launch auto-start uses `runtime.auto_start()` once.
- Window close hides to tray when enabled. Quit waits for runtime shutdown;
  repeated quit requests cannot bypass the pending cancellation. Single-instance
  plugin focuses the existing process instead of starting another engine.
- Login startup uses the Tauri autostart plugin; settings report actual OS state
  and restore it if configuration saving fails. Enabled/disabled roundtrip
  passed under the separate `TunnelWarden Smoke Test` registration; removed it.
- Structured logs retain 1,000 entries in memory and write to rotating platform
  log files (1 MB, three archives). Added bounded/filter test.
- Full gates passed: 13 unit + 8 SSH integration tests, frontend check/build.
  Repeated the real WebView smoke suite successfully with all shell plugins.
  Native Windows automation confirmed Close removes the window while the process
  remains alive; relaunch focuses the original PID. Normal quit exits it.

## Phase 7 — acceptance and release preparation, 2026-09-29

- Completed target-OS matrix evidence in docs/12_VALIDATION.md, including actual
  external agent discovery/authentication through Oryxis on Windows.
- Closed an IPC scheduling race: queued dial failures use the existing Stop
  completion barrier, so old failures cannot consume a newly restarted intent.
  Added a regression test; upstream SSH and retry semantics remain unchanged.
- Full local gates: 14 unit and 9 real SSH integration tests, none ignored;
  fmt/check/test/clippy and architectural boundary check; Svelte zero errors/
  warnings and production frontend build.
- Added Windows CI and tag/manual release workflows, pinned action revisions,
  lockfile builds, version/upstream guards, EXE/MSI packaging, per-user NSIS
  install/uninstall verification, bundled dependency licenses and SHA-256 hashes.
- Updated user/build instructions and release notes. Signed binaries are not
  available; encrypted keys require an external agent. No scope expansion.
- Publication is complete only when the tag workflow publishes actual assets;
  GitHub Actions and the v0.1.0 release provide the durable publication evidence.
