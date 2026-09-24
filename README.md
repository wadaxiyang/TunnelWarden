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

This repository is at the beginning of the internal work packages in
[TunnelWarden_SPEC.md](TunnelWarden_SPEC.md). It currently contains the Rust
workspace, domain types, and a pure lifecycle state machine with regression and
property-based tests. It does **not** yet connect to SSH, open a real listener,
or provide the GUI. None of the v1.0 release gates should be considered passed.

## Local checks

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Architecture

`tunnel-domain` contains configuration, errors, and state types without runtime
or UI dependencies. `tunnel-core` owns the single-writer lifecycle reducer and
its bounded transition journal. Its effects are instructions for a future
resource-owning supervisor; they do not themselves open or close sockets.

The complete design, implementation order, and release gates are in the spec.
