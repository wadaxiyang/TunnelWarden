# 00 — Project Charter

## Product definition

TunnelWarden is a **small desktop manager for persistent SSH port forwards**. It should feel like a compact native utility even though the presentation layer is WebView-based.

The project exists to expose a reliable, already-proven SSH forwarding engine through a precise desktop GUI. It is **not** a project for inventing a new SSH stack or demonstrating custom concurrency architecture.

## First-release capabilities

- SSH host profiles: hostname, SSH port, username, auth method, identity/key, keepalive-relevant settings exposed by Oryxis when needed.
- Jump hosts / jump chains supported by Oryxis.
- Standalone forwarding rules:
  - Local (`-L`)
  - Remote (`-R`)
  - Dynamic SOCKS (`-D`)
- Per-rule start / stop / restart.
- Auto-start and self-healing reconnect.
- Oryxis-style shared PTY-less SSH connection for rules that use the same host.
- Runtime status and actionable error text.
- Logs.
- Tray controls and launch at login.

## Explicitly outside first-release scope

Terminal emulation, SFTP, X11, remote desktop launching, cloud discovery, MCP, AI features, snippets, sync, serial, telnet, mosh, plugin systems, and Oryxis' iced UI.

## Product-quality target

A user should be able to configure a correct tunnel without knowing TunnelWarden's internal field naming. The UI must use forwarding semantics directly: **listener**, **SSH host**, and **target**. Generic `LocalHost`/`RemoteHost` labels are not acceptable UI terminology.
