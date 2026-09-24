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
workspace, domain types, a pure lifecycle state machine, and a Local/Dynamic
listener owner. Tests bind a real loopback port, confirm a conflict is reported,
and confirm Stop releases the port. The `ssh-engine` crate now uses `russh`
0.63.x for a direct password-authenticated SSH connection with strict OpenSSH
`known_hosts` checks and session ping; integration tests run against an
in-process SSH server. The SSH session is not yet wired to the listener, and
there is no forwarding or GUI. None of the v1.0 release gates should be
considered passed.

## Local checks

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Architecture

`tunnel-domain` contains configuration, errors, and state types without runtime
or UI dependencies. `tunnel-core` owns the single-writer lifecycle reducer and
its bounded transition journal. `ListenerRuntime` is the first resource owner;
the future supervisor will own it together with the SSH chain and relay tasks.
`ssh-engine` owns direct SSH sessions and rejects unknown or changed host keys
under the default strict policy.

The complete design, implementation order, and release gates are in the spec.
