# Verification record

## 2026-09-24, Windows, Rust 1.98.1

- `cargo test --workspace --quiet`: passed. Includes real in-process SSH,
  password/key/agent authentication, two- and three-hop chains, host-key
  approval, local/remote/SOCKS5 forwarding, reconnect, and port release.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed after formatting.
- `TUNNELWARDEN_RECONNECT_CYCLES=1000 cargo test --release -p tunnel-core --test supervisor reconnect_keeps_listener_and_restores_forwarded_traffic`: passed in 70.88 s. Each cycle waited for reconnect and Healthy; the local listener remained bound, traffic resumed, and Stop released the port.
- A repeat of the same Release test sampled every 5 s with
  `scripts/reconnect-memory-probe.ps1 -Cycles 1000`: passed in 67.17 s. Private
  Bytes ranged from 2.0 to 2.5 MB, handles 141–143, threads settled from 6
  to 3, and TCP sockets ranged from 2 to 4.
- `scripts/release-memory-probe.ps1 -Cycles 30` on the full Release app:
  passed. Working Set 60.3→64.1 MB, Private Bytes 83.1→115.3 MB with a 137.9 MB
  sampled high and no continuing climb, handles 577→582, threads stayed at
  52, GPU local memory 20→31.5 MB. A prior unpatched GPUI Windows build grew
  about 23 MB Private Bytes per window cycle; the vendored drop-target fix
  removed that trend.

The 24-hour Dynamic SOCKS5 soak remains unrun. The supervisor test accepts
`TUNNELWARDEN_SOAK_HOURS=24`: it sends SOCKS5 traffic each minute, disconnects
the test SSH server every hour, and verifies traffic after each recovery.
Build the Release test and run `scripts/reconnect-memory-probe.ps1 -SoakHours 24`
to collect one-minute resource samples. Physical Wi-Fi disconnect/recovery,
full GUI interaction coverage, startup integration, and `cargo audit` are also
pending. The v1.0 release gate is not yet met.
