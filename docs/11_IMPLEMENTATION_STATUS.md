# Implementation status

Current release: **TunnelWarden 0.2.1 — published and verified**, Windows x64.
Development branch: `master`. The v0.1 phase history is retained below;
the v0.2 implementation and publication receipts follow it.

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

## Phase 7 — passed and released, 2026-09-29

- Completed target-OS matrix evidence in docs/12_VALIDATION.md, including actual
  external agent discovery/authentication through Oryxis on Windows.
- Closed an IPC scheduling race: queued dial failures use the existing Stop
  completion barrier, so old failures cannot consume a newly restarted intent.
  Added a regression test; upstream SSH and retry semantics remain unchanged.
- Full local gates: 15 unit and 9 real SSH integration tests, none ignored;
  fmt/check/test/clippy and architectural boundary check; Svelte zero errors/
  warnings and production frontend build.
- Added Windows CI and tag/manual release workflows, pinned action revisions,
  lockfile builds, version/upstream guards, EXE/MSI packaging, per-user NSIS
  install/uninstall verification, bundled dependency licenses and SHA-256 hashes.
- Updated user/build instructions and release notes. Signed binaries are not
  available; encrypted keys require an external agent. No scope expansion.
- Publication is complete only when the tag workflow publishes actual assets;
  GitHub Actions and the v0.1.0 release provide the durable publication evidence.
- Final review also covered stopping a dial and changing its rule's host before
  completion: the stopped ID is removed from pending queues so the old host
  cannot attach a newly edited rule. Added a regression test and reran gates.

### Publication receipt

- Release: https://github.com/wadaxiyang/TunnelWarden/releases/tag/v0.1.0
- Published: 2026-09-29 15:56:17 UTC (23:56:17 Asia/Shanghai).
- Release source/tag commit: `6cebb10d0106bfaf274189dd48126d69478d63f1`.
- CI: https://github.com/wadaxiyang/TunnelWarden/actions/runs/36589328498 — success.
- Windows Release: https://github.com/wadaxiyang/TunnelWarden/actions/runs/36589334135 — success.
- Both remote runs passed 15 unit + 9 SSH integration tests, all Rust gates,
  frontend checks and build. Release produced two bundles; NSIS install,
  installed-resource checks and uninstall commands completed successfully.
- Published NSIS EXE: 4,349,774 bytes; MSI: 6,529,024 bytes. Release also contains
  application source ZIP, LICENSE, NOTICE.ORYXIS, third-party notices/licenses
  and SHA256SUMS.txt. All seven listed file hashes match GitHub asset digests.
- This post-publication documentation receipt does not change the release code.
  Its docs-only commit skips redundant CI; the tag continues to identify the
  exact source verified and built by both successful workflows above.

### Phase commit ledger

| Phase | Commit |
| --- | --- |
| 0 — upstream | `07ed15e72cfd4063d8b16b8a9963045300052e15` |
| 1 — harness | `6aecff3ef465b2abcc1ffc8a2e83934ff3b39f4a` |
| 2 — runtime port | `5558bfafa607b71167a2e549c87b1a0425d854a6` |
| 3 — persistence | `02fb87112266044d672838ecd2c3af337177cbe7` |
| 4 — IPC | `1e2b650fe35d5dad4aee999b33ac7ee207b3d643` |
| 5 — UI | `560dffd081094fdac3c7cc49ffd3e713dcde2f26` |
| 6 — desktop shell | `d5457a4cf8d44c4e4c9995a4df4c6ad379add402` |
| 7 — acceptance/release | `5d6d1c9df8a33df949dfeaf5fd5804b81046550a` |
| Final runtime regression fix | `6cebb10d0106bfaf274189dd48126d69478d63f1` |

## v0.2.0 — user-authorized scope (2026-09-30)

The original Phase order remains: upstream/API review → core adapters and
regression harness → IPC/UI → tray/window lifecycle → release acceptance.
The user explicitly requested host connection tests, error-log export,
encrypted-key passphrase input, remote-listener diagnostics, on-demand UI,
and an actual v0.2.0 Windows release.

- Upstream review complete: `SshEngine::connect_forward_conn` tests SSH without
  attaching a forward; pinned `oryxis-vault::import_key` handles key decryption.
- Core implementation in progress: isolated cancellable host tests, ephemeral
  key unlock/cache, redacted diagnostic report, remote-policy error context.
- UI, lifecycle, acceptance and publication pending.
- Subsequent gates below supersede the initial pending status.
- No public reachability claim/test is inferred from forwarding success.

### v0.2 core gate — passed

26 unit + 11 real SSH integration tests passed, including encrypted key
wrong-passphrase retry/cancel, fresh-session unattended rejection, successful
host testing without listener creation, an active tunnel surviving a test,
redacted export and explicit server refusal. `fmt/check/test/clippy` and
boundary scan all pass. Fixture bcrypt dependency is pinned for CI parity.

### v0.2 IPC/UI gate — passed

Saved host rows expose Test connection / Cancel test and precise test results.
Encrypted files open a password-only modal with wrong-passphrase retry; native
Save As exports a structured JSON error report. Remote configuration explains
server policy and running remote rules state that external reachability is
unverified. `scripts/check.ps1 -Frontend` passed all Rust gates, 26 + 11 tests,
Svelte checks (zero warnings) and production UI build. Native lifecycle and
end-to-end window acceptance are next.

### v0.2 disposable UI/lifecycle gate — passed

Configuration sets `create: false`. Normal launch/tray/single-instance opens a
serialized asynchronous window; `--autostart` creates only runtime + tray.
Close destroys the native WebView and keeps the runtime alive; explicit Quit
still awaits runtime shutdown. Unsaved forms/settings and active questions
receive a close confirmation. Reopening fetches state and subscriptions anew.

- All Rust checks, 26 unit + 11 SSH tests, frontend check/build pass.
- Real Tauri CRUD suite and `tests/ui-v02.mjs` pass.
- Native Save As exported valid JSON; no fixture passphrase/private PEM.
- Native Alt-F4: smoke-instance WebView2 process count became **0**; same Rust
  PID stayed alive; actual SSH banner still traversed the running local tunnel.
- Second launch recreated the UI in the original process, displayed Running,
  and successfully tested the host using the session key cache.
- Fresh `--autostart` process had **0** matching WebView2 processes; subsequent
  launch created a working UI. Debug cold-background working set was about
  25 MB (measurement only, not a release memory guarantee).
- Native close with an unsaved form offered Keep editing; draft was retained.

### v0.2 release candidate — local gates passed

Version is 0.2.0 in both Rust crates, npm/lock, Tauri config and About UI.
Final full acceptance passed (26 unit + 11 SSH integration, no ignored tests,
fmt/check/test/clippy/boundaries, Svelte check/build). The optimized local NSIS
installer built successfully: `TunnelWarden_0.2.0_x64-setup.exe` (5,675,738 bytes).
676 dependency notices were collected from the locked inputs.
CI/release share an immutable Rust-cache action; release also starts the
installed executable with `--autostart` and rejects a WebView2 child process.
The workflow collects exact versioned EXE/MSI paths to avoid stale cached bundles.

v0.2 phase ledger before tagging:
- Core adapters and tests: `97d268c0921eac7eeac8fd4e93f7ca84f421f708`.
- IPC and UI: `477e84f4bdf7475aba64af7ae832763b6106941a`.
- Disposable UI and native acceptance: `a90a3ca776d93bc40b70ad8becd9be32652f47a0`.
- Publication receipt will be appended only after GitHub Release/assets succeed.

### v0.2.0 publication receipt — complete

- Release: https://github.com/wadaxiyang/TunnelWarden/releases/tag/v0.2.0
- Published: 2026-09-30 03:26:20 UTC (11:26:20 Asia/Shanghai); non-draft,
  non-prerelease; eight uploaded assets.
- Release source/tag commit: `5b051896c72cecc7f7f431534175875efc54f1e3`.
- CI: https://github.com/wadaxiyang/TunnelWarden/actions/runs/36661515768 — success.
- Windows Release: https://github.com/wadaxiyang/TunnelWarden/actions/runs/36661516616 — success.
- Both workflows passed 26 unit + 11 SSH integration tests, all Cargo gates,
  boundary scan, frontend check and build. Release built both installers and
  passed NSIS installation, resource checks, installed headless startup with no
  WebView2 child process, and uninstall before publication.
- NSIS: `TunnelWarden_0.2.0_x64-setup.exe`, 5,672,945 bytes.
- MSI: `TunnelWarden_0.2.0_x64_en-US.msi`, 8,364,032 bytes.
- Other assets: `TunnelWarden_v0.2.0_source.zip`, `LICENSE`, `NOTICE.ORYXIS`,
  `THIRD_PARTY_NOTICES.md`, `THIRD_PARTY_LICENSES.txt`, `SHA256SUMS.txt`.
- All seven checksum entries match GitHub's asset digests. Downloaded checksum
  and source ZIP hashes match too. The ZIP's Git archive comment is the exact
  tag commit; it contains the new source and no temporary `.upstream` fixtures.
- Oryxis core/ssh/vault remain pinned to
  `16ad2af83b424780ff60b6420a6c20a3b30c7143`.
- Remaining limits are documented in README/release notes: Windows x64,
  unsigned packages, manual encrypted-key unlock after full process restart
  (or external agent), current-session bounded report, and server/network
  control of remote reachability.
- This post-publication documentation receipt changes no release code and
  skips redundant CI. The tag identifies the source tested by both successful
  workflows; all v0.2 requested development and publication work is complete.

## v0.2.1 — native tray additions (2026-09-30)

User-authorized scope: a text-only native hover panel showing tunnel states
and transfer rates, per-rule Start/Stop submenus, and Windows v0.2.1 publication.
Order: upstream review/telemetry → native shell → acceptance/release.
Upstream review found no per-forward counters; exact pinned vendoring with
passive writer instrumentation follows docs/01. Core validation is in progress.

### v0.2.1 telemetry gate — passed

26 unit + 12 real SSH integration tests pass. Live 128 KiB bidirectional
payload counts are exact for Local/Remote/Dynamic, exclude SOCKS framing,
remain isolated on a shared connection and reset on reattach. Rust fmt/check/
test/clippy, boundary scan and full pinned-source/patch verification pass.


### v0.2.1 native shell gate

Native Win32 text panel and per-rule native Start/Stop submenus implemented.
Panel work is demand-driven, never activates a window or creates a WebView;
scrolling survives live refresh. Native control and injected-hover worker tests
cover real HWND creation/destruction, debounce/click cancellation/reopen/quit,
Unicode/read-only text and foreground preservation. Geometry and rate-reset
tests pass. Original real Tauri encrypted-key/host/tunnel UI regression passes.
Desktop automation could not reliably target the Shell tray, so mouse-driven
Shell hover/menu clicks are not claimed as manually verified; native lifecycle
and service/action behavior are covered by executable tests.
Telemetry phase commit: `0862d020124e4d5f85d68c5d21c10ef097d722ad`.

Full native shell gate passed: 31 unit + 12 real SSH integration tests,
fmt/check/test/clippy, boundary/source-patch scans and frontend check/build.


### v0.2.1 release candidate

Native shell phase commit: `269bce2` (full SHA in Git). Version raised to 0.2.1
in both application crates, npm/lock, Tauri configuration, About and workflow
default tag. Existing data identifier and schema are unchanged. Final gates
and Windows installers are being checked before tagging/publication.

Final candidate gate passed: 31 unit + 12 SSH integration tests, all Cargo
gates, boundary/source diff checks, Svelte check/build. Optimized local NSIS
TunnelWarden_0.2.1_x64-setup.exe built successfully. Native Alt-F4 preserved
actual forwarding with zero smoke WebViews; reopening restored the runtime.
Native popup rendering was visually inspected; first-show visibility and
short-list height were hardened and retested. Ready for v0.2.1 tag workflows.


### v0.2.1 publication receipt — complete

- Release: https://github.com/wadaxiyang/TunnelWarden/releases/tag/v0.2.1
- Published: 2026-09-30T09:07:27Z; non-draft, non-prerelease, eight assets.
- Release/tag source: `ecb30174936f0440ee7cacec898083d4e3091254`.
- Telemetry phase: `0862d020124e4d5f85d68c5d21c10ef097d722ad`.
- Native shell phase: `269bce20d4cb4673b0e2bc17224248f81ea0f5c3`.
- Final native visibility/acceptance/version: `ecb30174936f0440ee7cacec898083d4e3091254`.
- CI: https://github.com/wadaxiyang/TunnelWarden/actions/runs/36691272004 — success.
- Windows Release: https://github.com/wadaxiyang/TunnelWarden/actions/runs/36691274926 — success.
- Both workflows passed 31 unit tests and 12 real SSH integration tests, full
  Rust gates, pinned-source diff/boundary scans and frontend check/build.
- Release built NSIS/MSI and passed installed resources, headless startup with
  no WebView2 child and uninstall before publishing.
- NSIS `TunnelWarden_0.2.1_x64-setup.exe`: 5,677,777 bytes.
- MSI `TunnelWarden_0.2.1_x64_en-US.msi`: 8,384,512 bytes.
- Other assets: source ZIP, LICENSE, NOTICE.ORYXIS, THIRD_PARTY_NOTICES.md,
  THIRD_PARTY_LICENSES.txt and SHA256SUMS.txt.
- All eight assets were downloaded and matched GitHub's SHA-256 digests; all
  seven checksum entries matched actual downloaded files. Installer format
  headers checked. Source ZIP's archive comment matches the tag, contains the
  complete vendored source/provenance/patch, and excludes temporary fixtures.
- Oryxis pin remains `16ad2af83b424780ff60b6420a6c20a3b30c7143`.
- Limits: unsigned Windows x64 packages; first rate needs a sampling interval;
  encrypted-file authentication after process restart needs manual unlock or
  an agent; public reachability remains server/network dependent. Physical
  Shell pointer/menu and physical multi-monitor testing were not performed;
  native rendering, lifecycle, geometry and service paths have executable coverage.
- This documentation-only receipt changes no release code and skips redundant
  CI. The tag stays on the exact source built by the successful workflows.

## v0.2.2 — tunnel detail page (2026-10-03)

User-authorized scope: clickable tunnel details, real traffic rates/trends,
configuration and recent state changes, then Windows v0.2.2 publication.
Searched pinned core port_forward_rule, ssh engine/forwarding and app
standalone dispatch/views: no per-forward traffic/history API exists upstream.
Reuse the v0.2.1 counters unchanged and the existing bounded/redacted LogBook.
The new service query only serializes an active attachment's byte counts,
monotonic sample time/age and opaque attachment token; no engine handles,
new runtime ticks, persistence or changes to the reviewed vendor patch.
GUI samples only while the detail page is visible; no background chart work.

### v0.2.2 telemetry/API gate — passed

31 unit + 12 real SSH tests passed, including service-level active/inactive/
deleted telemetry, live remote payload, monotonic timestamps, serialization and
attachment-token/counter reset after restart. fmt/check/test/clippy, boundary
scan and exact 46-file vendored-source verification all passed.

### v0.2.2 GUI acceptance

Telemetry/API commit: `2bd6d2d` (full SHA in Git). Detail page uses on-demand
real counters with no new chart dependency. 3 frontend regression tests cover
precision, actual elapsed rates, baseline resets, missing data and retention.
Real Tauri ui-smoke, ui-v02 and ui-v022 passed: 3,276,800 bytes each direction
match exact service counters, live SVG paths, reset on restart, bind conflict,
state history, keyboard focus restoration and polling disposal after navigation.
Screenshots inspected at normal and 760px widths; no horizontal overflow.
Axe reports zero violations; SVG text contrast requires manual inspection.
While viewing details, close_ui destroys all test WebViews (count 0), same Rust
PID stays alive and a real SSH banner still traverses the tunnel. Reopening
restores the actual runtime and the existing in-memory authentication cache.
Final combined gates are being run before the GUI phase commit.

GUI phase gate passed: full fmt/check/test/clippy, 31 unit + 12 real SSH tests,
3 frontend tests, zero Svelte diagnostics, production build and boundary/vendor
checks. No new frontend package dependency was introduced.
