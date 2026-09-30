# 0.1.0 validation record

Target: Windows x64, 2026-09-29. No ignored tests or mock production backend.

| Acceptance area | Evidence |
| --- | --- |
| Password, private key, external agent | Actual AsyncSSH connections through public Oryxis APIs; Windows disposable Pageant-compatible pipe uses russh's public agent server |
| Non-22 SSH port | All SSH fixtures use OS-assigned high ports |
| One and two jumps | ApplicationDialer resolver test, actual nested SSH servers |
| Local, Remote, Dynamic | Banner bytes traverse each listener; SOCKS5 client handshake; one shared ForwardConn |
| Remote Stop/Delete | Explicit cancellation, same-connection remote rebind and live deletion |
| Shared host connections | Runtime dial counter stays at one for sibling rules |
| Stop connecting/running | Delayed success cannot resurrect; queued attach/dial failure cannot overtake Stop/Start |
| Auth/key/listener failure | Wrong password, strict unknown fingerprint and occupied port produce errors |
| Connection drop and retry | Kill server, liveness enters retry; virtual clock verifies 15/30/60/120 ceiling; manual Stop clears retry |
| Launch auto-start | Launch path reaches running; reload restores configuration only |
| Persistence/secrets | Atomic-write failure and malformed/future file preservation; real Windows credential roundtrip |
| IPC lifecycle | Service CRUD/Start/Stop/Restart/Delete/reload integration test |
| UI fields/CRUD | Real Tauri WebView suite; Remote ports and separate SSH port; Dynamic target omitted |
| Tray/window/login | Native Close hides but process lives; second launch focuses same PID; login enable/disable roundtrip; Quit exits |
| Logs | Bounded/filter test and rotating files; no credential values logged |

All four Cargo gates and both frontend gates run via `scripts/check.ps1 -Frontend`.
CI repeats them on windows-latest. Release builds NSIS/MSI and verifies installed
files/notices and uninstall on an ephemeral runner before publishing. GitHub
records workflow runs and their release commit.

## Reproduce UI smoke

Use a separate Tauri identifier/product name for a debug build:
`TAURI_CONFIG={"identifier":"dev.tunnelwarden.smoketest","productName":"TunnelWarden Smoke Test"}`.
Run Vite on 1420. Launch the executable with
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223`.
Install agent-browser in a temporary tooling directory and run
`node tests/ui-smoke.mjs`. Set TUNNELWARDEN_AGENT_BROWSER to its native executable
and TUNNELWARDEN_CDP to the port when different. The script removes its own
fixture.invalid profiles. Do not expose CDP remotely or enable it in normal use.
Screenshots were inspected; accessibility audit reported no violations.

## Test fixture boundary

`tests/windows_agent.rs` is external test-server glue using russh's public agent
server/client API to load one fresh test key into a unique pipe. Production auth
still calls Oryxis, including discovery/signing. No custom agent protocol was
written. Boundary checks are unchanged; russh is a dev-dependency only. No OS
agent service, personal keys or agent configuration is modified by this test.

## v0.2 native acceptance (2026-09-30)

In addition to the original suite, `tests/ui-v02.mjs` exercises encrypted-key
host testing with a wrong then correct passphrase, fingerprint verification,
remote-listen help and a live forwarded SSH banner. Run it against the isolated
Tauri application with an AsyncSSH fixture writing `.upstream/ui-fixture.json`.
After native Alt-F4, verify no WebView2 command line contains the smoke test
identifier and independently read the listener's SSH banner. Launch the same
executable again and run `node tests/ui-v02.mjs reopen`; run its `cleanup` mode
when finished. Native Save As export and unsaved-draft close confirmation were
also exercised. A fresh `--autostart` launch created no WebView2 process.

## v0.2 published build

Tag/source: `v0.2.0` / `5b051896c72cecc7f7f431534175875efc54f1e3`.
CI run `36661515768` and Windows Release run `36661516616` both succeeded.
Both ran 26 unit and 11 real SSH integration tests with zero failures/ignored
cases. Release logs confirm two bundles and installed headless startup with
no WebView2 child. All seven published checksum entries match GitHub digests;
the downloaded source ZIP identifies the exact release commit.

## v0.2.1 acceptance

- 31 unit tests (26 core + 5 native presentation), 12 real SSH integration tests.
- 128 KiB live payload echo in both directions for L/R/D; exact counts before
  EOF; SOCKS framing excluded; sibling counters stay zero; reattach resets.
- Real Win32 HWND/control tests: no foreground change, Unicode/read-only text,
  scroll preservation while rates change, destroyed handles after dismissal.
- Hover worker receives synthetic Enter/Hide/Leave/Quit events with the real
  empty AppService: dwell, cancelled short hover, reopen and quit pass.
- Monotonic rate/reset and negative-monitor/top-taskbar geometry unit tests.
- Real isolated Tauri test `tests/ui-v02.mjs` passed again: encrypted-key retry,
  fingerprint consent, host test, remote help and actual local-tunnel banner.
- Fresh isolated `--autostart` process had no matching WebView2 processes.
- Shell tray pointer automation was not reliable in this environment; physical
  mouse hover and nested-menu selection are not recorded as manual acceptance.
  Worker, native controls, menu state mapping and service lifecycle have executable
  coverage. Multi-monitor coordinates are tested in geometry, not on physical
  multiple displays.

Final desktop checks: native Alt-F4 left zero smoke-instance WebView2 processes
while an SSH banner still traversed the local listener; second launch recreated
the interface and tests reused the in-memory key. Smoke profiles/processes were
cleaned. The actual native panel was visually inspected during the worker test.
Headless first-show uses SetWindowPos(SWP_SHOWWINDOW | SWP_NOACTIVATE) to avoid
ShowWindow's STARTUPINFO SW_HIDE override; tests explicitly assert visibility.
Panel height is compact for short lists and capped with scrolling for long lists.
