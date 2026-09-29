TunnelWarden 0.1.0 is a focused Windows x64 SSH tunnel manager.

- Local (-L), remote (-R) and dynamic SOCKS5 (-D) forwarding.
- Password, key-file and external ssh-agent authentication; ordered jump chains.
- Shared connections, start/stop/restart, launch auto-start and reconnect.
- Host-key verification, Windows Credential Manager passwords, atomic settings.
- Tray controls, launch at login, logs and license/source information.

Install the x64-setup.exe (per-user NSIS) or .msi package. Installers download
Microsoft WebView2 when needed. Packages are unsigned; no certificate is
configured. SHA256SUMS.txt records the published file hashes.

SSH engine: Oryxis 16ad2af83b424780ff60b6420a6c20a3b30c7143, unchanged.
The standalone runtime is adapted from that revision with source provenance.
TunnelWarden is AGPL-3.0-or-later; source and dependency/license notices accompany
this release. See README.md for build instructions and operating details.

Known limits: Windows x64 only; encrypted key files require an external agent;
no terminal/SFTP/internal agent, certificate or interactive OTP UI. Remote binds
are subject to SSH server forwarding/GatewayPorts policy. Automatic connections
refuse unknown or changed host keys until manually verified. The app must stay
running (usually in the tray) to maintain tunnels.
