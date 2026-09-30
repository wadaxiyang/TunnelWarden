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
