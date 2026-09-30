# TunnelWarden 0.2.0

## Install and use

Download the Windows x64 NSIS setup EXE or MSI from
[GitHub Releases](https://github.com/wadaxiyang/TunnelWarden/releases).
NSIS installs for the current user. The installer downloads Microsoft WebView2
when absent. This release is unsigned.

1. Add a profile in **SSH Hosts**, including its independent SSH server port.
   Passwords use Windows Credential Manager; private keys stay in their files.
   Agent uses external Windows OpenSSH/Pageant-compatible agents via Oryxis.
   Jump routes list saved hosts in order, nearest hop first.
   Use **Test connection** to verify the saved SSH route, fingerprint and
   authentication without creating a tunnel. Tests can be cancelled.
   Encrypted key files prompt for their passphrase when testing/starting.
   Oryxis decrypts the key; the passphrase is never saved. The unlocked key is
   cached only in process memory until quit (including while the UI is closed).
2. Add a Local, Remote or Dynamic rule in **Tunnels**. Remote exposes a server
   listener pointing back to a local target; Dynamic exposes a local SOCKS5 port.
3. Start and verify the first-use fingerprint with the server administrator.
   Running appears only after backend attachment succeeds. For remote forwards,
   it does **not** prove external/public reachability: verify the server's actual
   bind address, SSH forwarding/GatewayPorts policy, firewall/security group,
   NAT/routing and target service, then connect from a separate external machine.
4. Stop releases listeners, including remote binds. Edit stops the old rule
   before saving; Delete cancels before removing configuration.

Auto-start applies at each application launch. Auto-reconnect uses Oryxis'
15/30/60/120-second backoff and a 15-second retry tick (an attempt can occur one
tick later). Manual Stop clears retries for the current run. Unattended starts
refuse unknown/changed host keys; start manually to inspect and approve them.

Close **destroys the interface/WebView** when background mode is enabled.
Only the Rust runtime and tray remain; tunnels keep running. The tray or a
second launch recreates the interface in the same process. Unsaved drafts get
a close confirmation. Launch at login starts in the background with no WebView.
Tray offers Open, Start all, Stop all and Quit. Quit cancels tunnels before exit.
Reopening the UI does not restart the app or rerun auto-start.

In **Logs**, use **Export error logs** to save a JSON diagnostic report with
errors, up to 1,000 recent entries, runtime status, version and upstream SHA.
This covers the current process session; rotated disk logs remain available
for earlier sessions. Credentials/key contents are excluded and Oryxis masks
known secret patterns. Server addresses and error text can remain; review the
report before sharing.

## Data and limitations

- Config/trusted fingerprints: `%APPDATA%\dev.tunnelwarden.app\config.json`.
- Passwords: Windows Credential Manager, service `dev.tunnelwarden.app`.
- Logs: `%LOCALAPPDATA%\dev.tunnelwarden.app\logs` (1 MB, three archives).
- Transient runtime state is never persisted. Corrupt/future config fails closed
  and remains untouched; back up the file before repairing it.
- After a full app restart, encrypted files need manual unlock before unattended
  auto-start can authenticate. Use an external SSH agent for unattended unlock.
- No certificate/interactive OTP UI, terminal, SFTP or built-in agent is included.
- Remote listeners remain subject to SSH server forwarding/GatewayPorts policy.
- Windows x64 is the only packaged and validated target for 0.2.0.

## Build and validate

Windows prerequisites: Rust **1.94.1**, MSVC build tools/Windows SDK, Node.js
**24**, npm, Python **3.13** and WebView2. The workspace enables aws-lc prebuilt
NASM objects and UTF-8 C compilation; separate NASM is unnecessary.

```powershell
npm ci
python -m pip install -r tests/requirements.txt
./scripts/fetch-oryxis-reference.ps1
npm run licenses
./scripts/check.ps1 -Frontend
npm run tauri -- dev
# Release:
npm run bundle
```

Tests use temporary loopback AsyncSSH servers and fresh keys. The Windows agent
test uses a disposable pipe and russh's public server API; production auth uses
Oryxis exclusively. Tests never modify the user's agent. The UI smoke suite uses
a real isolated Tauri WebView; see `docs/12_VALIDATION.md`.

CI runs all Rust/frontend gates on Windows. Tag releases repeat the gates, build
both installers and verify NSIS installation/uninstallation on an ephemeral
runner before publishing installers, source, notices and SHA-256 checksums.

Licensed **AGPL-3.0-or-later**. Source and lockfiles accompany the release; the
pinned upstream is available at
[Oryxis](https://github.com/wilsonglasser/oryxis/tree/16ad2af83b424780ff60b6420a6c20a3b30c7143).
`npm run licenses` collects dependency notices into `THIRD_PARTY_LICENSES.txt`,
which is included in installers and release assets. Preserve LICENSE,
NOTICE.ORYXIS and modification notices when redistributing.

TunnelWarden is a small desktop SSH port-forward manager built around **Oryxis' existing SSH implementation**, not a new SSH implementation.

## Non-negotiable implementation strategy

1. **`oryxis-core`, `oryxis-ssh` and the key-import API of `oryxis-vault` are upstream dependencies**, pinned to Oryxis commit `16ad2af83b424780ff60b6420a6c20a3b30c7143`.
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
