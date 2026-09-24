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
- `TUNNELWARDEN_SOAK_SMOKE=1 cargo test --release -p tunnel-core --test supervisor reconnect_keeps_listener_and_restores_forwarded_traffic`: passed. This exercises periodic Dynamic SOCKS5 traffic, a forced disconnect, and traffic recovery on the same listener using short intervals.
- GPUI Kit `workspace::ui_tests`: passed. Headless interaction tests click sidebar navigation, open/cancel the Group editor, switch tunnel modes, open Settings, and verify host-key confirmation controls.
- `cargo-audit 0.22.2` against RustSec advisory database commit `ef8244d224cb89be53491e0c55a96c1279d9fdf1`: 0 reported vulnerabilities across 998 lockfile dependencies. It reported six unmaintained crates and one `glib 0.18.5` unsoundness advisory. `glib`, `proc-macro-error`, and `rustls-pemfile` are absent from the Windows dependency tree. The remaining unmaintained crates are transitive dependencies of GPUI Kit and its renderer; no safe product-local version bump is available. Review again when upgrading GPUI Kit.
- `scripts/release-memory-probe.ps1 -Cycles 30` on the full Release app:
  passed. Working Set 60.3→64.1 MB, Private Bytes 83.1→115.3 MB with a 137.9 MB
  sampled high and no continuing climb, handles 577→582, threads stayed at
  52, GPU local memory 20→31.5 MB. Each cycle closes the window to the tray,
  launches a second instance that signals the first, and confirms the first
  window reopens. A prior unpatched GPUI Windows build grew
  about 23 MB Private Bytes per window cycle; the vendored drop-target fix
  removed that trend.
- A repeat after the Group, login-startup and UI test additions also passed 30
  cycles: Working Set 62.2→66.5 MB, Private Bytes 83.0→116.2 MB with a 138.0 MB
  sampled high, handles 602→607, threads 52 throughout, and GPU local memory
  20→31.5 MB.

The 24-hour Dynamic SOCKS5 soak remains unrun. The supervisor test accepts
`TUNNELWARDEN_SOAK_HOURS=24`: it sends SOCKS5 traffic each minute, disconnects
the test SSH server every hour, and verifies traffic after each recovery.
Build the Release test and run `scripts/reconnect-memory-probe.ps1 -SoakHours 24`
to collect one-minute resource samples. Physical Wi-Fi disconnect/recovery,
full GUI interaction coverage and Windows login behavior are also
pending. The v1.0 release gate is not yet met.
