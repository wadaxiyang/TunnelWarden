<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="TunnelWarden icon" />
</p>

<h1 align="center">TunnelWarden</h1>

<p align="center">SSH tunnels, always within reach.</p>

<p align="center">
  Save servers and forwarding rules. Connect local services, remote ports, and SOCKS5 proxies with a click.<br />
  Automatic reconnection, live traffic, and tray controls keep your everyday tunnels running quietly in the background.
</p>

<p align="center">
  English · <a href="README_zh.md">简体中文</a>
</p>

<p align="center">
  <a href="https://github.com/wadaxiyang/TunnelWarden/releases/latest">Download</a> ·
  <a href="#features">Features</a> ·
  <a href="#development">Development</a> ·
  <a href="https://github.com/wadaxiyang/TunnelWarden/issues">Report an issue</a>
</p>

TunnelWarden is an open-source SSH tunnel manager for Windows, built with **Tauri 2, Svelte 5, and Rust**, with SSH connections and port forwarding powered by **Oryxis**. Use its graphical interface to access an internal database through a jump host, forward a local service to a remote server, or set up a temporary SOCKS5 proxy.

## Features

- **Three forwarding modes**: Local forwarding (`-L`), remote forwarding (`-R`), and dynamic SOCKS5 forwarding (`-D`), with separate settings for the listener, SSH host, and destination.
- **SSH host management**: Save server addresses, independent SSH ports, usernames, and authentication methods. Use jump hosts and multi-hop routes, and test connections before creating a tunnel.
- **Multiple authentication methods**: Passwords, private keys, encrypted private keys, and external SSH agents. Passwords are stored in Windows Credential Manager; key passphrases are never saved, and unlocked keys remain only in the current process's memory.
- **Host fingerprint verification**: Confirm host keys on first connection or when a fingerprint changes. Unattended connections never automatically trust unknown or changed keys.
- **Tunnel lifecycle controls**: Start, stop, and restart individual rules, or start and stop tunnels in bulk. Multiple forwards to the same host share an SSH connection; stopping one rule leaves the others running.
- **Automatic startup and reconnection**: Enable connection at application startup for each rule, with backoff retries after disconnection. Manually stopping a tunnel cancels its current retries.
- **Live traffic and status**: View upload and download rates, cumulative traffic and duration for the current connection, and traffic charts covering the last minute or five minutes.
- **Activity and diagnostics**: Review activity per tunnel, filter errors, and export a JSON diagnostic report to help troubleshoot connection issues.
- **Tray and background operation**: Keep tunnels running after closing the window. Start and stop rules from the tray, and hover to see traffic for running tunnels. Launch in the background at Windows login without keeping a WebView window open.
- **English and Chinese**: Follow the system language by default: Chinese for Chinese locales, English for all others. Switch between following the system, Simplified Chinese, and English in Settings.

## Installation

Download a Windows x64 installer from **[GitHub Releases](https://github.com/wadaxiyang/TunnelWarden/releases/latest)**:

| File | Installation method |
| --- | --- |
| `TunnelWarden_<version>_x64-setup.exe` | Standard installation into the current user's directory |
| `TunnelWarden_<version>_x64_en-US.msi` | Windows Installer package |

After installation, open TunnelWarden:

1. Add a server in **SSH Hosts**, select an authentication method, and use **Test connection** to check it.
2. Create a Local, Remote, or Dynamic rule in **Tunnels**, then enter the listener and destination addresses.
3. Start the tunnel. On the first connection, verify the server fingerprint before confirming trust.

Windows x64 installers are currently available. The installer downloads the Microsoft WebView2 Runtime if it is missing.

A few things to keep in mind:

- External access to remote forwards depends on the SSH server's forwarding policy, `GatewayPorts`, firewall, and network configuration.
- Encrypted private keys must be unlocked again after fully quitting the application. An unlocked external SSH agent can be used for unattended startup.
- Traffic charts collect samples only while the details page is open and clear when you leave it. Activity history belongs to the current process session and clears when the application exits.

## Development

### Prerequisites

Development and CI currently target Windows x64. Install:

- **Git**.
- **Node.js 24**, including npm.
- **Rust 1.94.1**, using the MSVC toolchain, with `rustfmt` and `clippy`.
- **Visual Studio 2022 Build Tools**, with the “Desktop development with C++” workload and Windows SDK.
- **Microsoft Edge WebView2 Runtime**.
- **Python 3.13**, for verifying the vendored Oryxis source and patch.

### Clone and run

```powershell
git clone https://github.com/wadaxiyang/TunnelWarden.git
cd TunnelWarden

rustup toolchain install 1.94.1 --profile minimal --component rustfmt --component clippy
rustup override set 1.94.1

npm ci
npm run licenses
npm run tauri -- dev
```

The first build downloads Rust dependencies and the pinned Oryxis source. `npm run licenses` generates `THIRD_PARTY_LICENSES.txt` from the lockfiles for packaging; regenerate it whenever dependencies change.

### Check and build

```powershell
# Rust formatting, compilation, unit tests, Clippy, architecture and upstream patch checks,
# plus frontend checks and build
./scripts/check.ps1 -Frontend

# Build Windows EXE / MSI installers
npm run bundle
```

Installers are written to `target/release/bundle/nsis/` and `target/release/bundle/msi/`.

### Project structure

```text
src/                         Svelte interface and TypeScript
src-tauri/                   Tauri commands, events, tray, and desktop integration
crates/tunnelwarden-core/    Tunnel runtime, configuration, credentials, and application logic
vendor/oryxis-ssh/           Pinned SSH engine with traffic accounting patches
upstream/                   Oryxis application reference source and patch records
scripts/                    Build checks, license collection, and release tools
```

SSH, authentication, and forwarding logic reuse Oryxis wherever possible. Read [AGENTS.md](AGENTS.md) and the [third-party source notices](THIRD_PARTY_NOTICES.md) before changing this code. Run `./scripts/fetch-oryxis-reference.ps1` to retrieve the upstream application reference files again.

## Acknowledgments

TunnelWarden builds on the work of these open-source projects:

- **[Oryxis](https://github.com/wilsonglasser/oryxis)** — Thanks to Wilson Glasser and all contributors. TunnelWarden reuses its SSH engine, authentication, jump routing, port forwarding, and key import, and adapts its standalone forwarding runtime and log redaction. It is this project's most important upstream dependency.
- **[russh](https://github.com/Eugeny/russh)** — The Rust SSH implementation used by Oryxis.
- **[Tauri](https://github.com/tauri-apps/tauri)** — The desktop application framework and system integration.
- **[Svelte](https://github.com/sveltejs/svelte)**, **[Vite](https://github.com/vitejs/vite)**, and **[TypeScript](https://github.com/microsoft/TypeScript)** — The UI and frontend toolchain.
- **[Tokio](https://github.com/tokio-rs/tokio)** — The Rust asynchronous runtime.

Thanks also to the maintainers and contributors of every transitive dependency. Complete dependency licenses are collected by `npm run licenses` and distributed with the installers.

## License

TunnelWarden is licensed under **[GNU AGPL-3.0-or-later](LICENSE)**.

Oryxis is pinned to commit [`16ad2af8`](https://github.com/wilsonglasser/oryxis/tree/16ad2af83b424780ff60b6420a6c20a3b30c7143). See [NOTICE.ORYXIS](NOTICE.ORYXIS), [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), and [PROVENANCE.md](vendor/oryxis-ssh/PROVENANCE.md) for upstream copyright, adaptation provenance, and SSH patch details.
