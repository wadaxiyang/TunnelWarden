# TunnelWarden

A native Rust GUI that keeps SSH tunnels alive.

- Local, Remote and SOCKS5 forwarding
- Multi-hop Jump Hosts
- Automatic reconnect
- Real SSH health checks
- Live traffic and latency
- No accounts, telemetry, or Pro tier
- Apache-2.0

## Development status

The Rust workspace implements real `russh` connections with password,
private-key and Windows SSH Agent authentication; strict host-key checking with
an in-app trust prompt; local, remote and dynamic SOCKS5 forwarding; two- and
three-hop Jump Chains; bounded reconnect supervisors; and SSH session health
checks. Integration tests use an in-process SSH server and exercise bytes over
forwarded channels, reconnect, host-key approval and listener release.

The Windows GPUI Kit app has structured jumper and tunnel editors, a tray,
single-instance wakeup, network change notifications, versioned TOML storage,
OS keyring-backed secrets, and import/export with previews. OpenSSH config
import previews supported fields, conflicts and unmapped directives before
adding jumpers. The `ProxyJump` directive is shown as a warning because imported
jumpers do not automatically become tunnel Jump Chains.

This is still an internal development build. The full v1.0 gates in
[TunnelWarden_SPEC.md](TunnelWarden_SPEC.md) include a 24-hour soak, 1000 real
reconnects, UI interaction checks, startup integration and an audit review.
Passing a shorter test does not imply those gates have passed.

## Local checks

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
pwsh -NoProfile -File scripts/release-memory-probe.ps1 -Cycles 30
```

## Architecture

`config-store` translates the versioned TOML schema to domain types and owns
atomic file replacement under `%APPDATA%\\TunnelWarden` or a chosen directory.
`tunnel-domain` contains configuration, errors, and state types without runtime
or UI dependencies. Its `TunnelConfig::validate` checks mode-specific endpoints,
jump-chain references, group references, and reconnect settings before startup.
`tunnel-core` owns the lifecycle reducer, bounded transition journal, the
single-writer tunnel manager, and Local/Dynamic and Remote reconnect supervisors.
The manager owns a bounded command queue and latest-state watch snapshot.
`ListenerRuntime` owns the Local/Dynamic port throughout retries.
`ssh-engine` owns direct SSH sessions and rejects unknown or changed host keys
under the default strict policy. Private-key input is capped at 1 MiB and its
source text is zeroized after decoding. `forwarding` contains the counted bidirectional
relay and a bounded SOCKS5 parser used by the Local and Dynamic worker.

The complete design, implementation order, and release gates are in the spec.
