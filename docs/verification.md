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

## 2026-09-24, UI coverage and resource follow-up

- The GPUI Kit headless tests now click every sidebar page, enter and focus the
  New Tunnel name, switch modes, inspect Blocked and Reconnecting labels and
  actions, dispatch a host-key button and a Settings action, and filter and
  search Logs. The runtime event journal retains at most 500 entries and the
  GUI pages them 25 at a time. A unit test verifies old events are evicted.
- The manager's `NetworkRecovered` command is exercised while a real SSH
  connection is reconnecting. It advances the retry attempt early while the
  local port remains reserved; Stop releases that port. This covers the
  callback-to-retry behavior short of physically disabling Wi-Fi.
- When login startup is enabled, the Host thread refreshes the per-user Run
  command from the current executable path on launch. The registry command
  quoting has a Windows unit test; an actual sign-out/sign-in remains manual.
- The tray menu now rebuilds only when its displayed tunnel names/states or
  summary counts change. A 30-cycle Release app window close/reopen probe after
  this change measured Working Set 61.1→60.2 MB, Private Bytes 86.0→113.5 MB
  (peak 137.7 MB then receded), handles 595→599, threads 52→51, and GPU local
  memory 20→31.5 MB. No monotonic Working Set or handle growth appeared.
  The same Release binary in background tray mode with no window, sampled after
  five seconds, used 48.5 MB Working Set, 56.9 MB Private Bytes, 458 handles and
  37 threads. The GUI/GPU window accounts for most of the difference; the
  observed Working Set is within the 60–70 MB comparison target.
- A one-cycle run of the updated PowerShell probe against the current Release
  supervisor test passed. `status.log`, `test.stdout.log` and `resources.csv`
  contained the exit status, test result and live resource sample.

The 24-hour Dynamic SOCKS5 soak remains unrun for manual execution. Build the
current Release test and start the probe from the repository root:

```powershell
cargo test --release -p tunnel-core --test supervisor --no-run
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/reconnect-memory-probe.ps1 -SoakHours 24
```

Each run creates `logs/reconnect-24h-<timestamp>-<pid>/` with `status.log`,
`test.stdout.log`, `test.stderr.log` and `resources.csv`. Resource samples are
written every minute, and the test writes traffic progress every ten minutes
and a reconnect result every hour. Logs remain on disk after failure. Use
`-LogDirectory <path>` to store them elsewhere. The soak checks SOCKS5 traffic
on each minute, disconnects the test SSH server hourly, and verifies recovery
on the same listener.

Physical Wi-Fi disable/reconnect and an actual Windows login are still manual
acceptance gates. For Wi-Fi, leave an autostart tunnel running, disconnect and
reconnect, then verify Reconnecting→Healthy and traffic through the same local
port. For login, turn on “Launch at Windows login” in Settings, sign out and
back in, then verify the tray starts in the background and configured autostart
tunnels connect. The v1.0 release gate remains open until these checks and the
24-hour soak pass.

## 2026-09-25, memory attribution and safe reduction

- The Release `window_probe` example now opens its minimal GPUI Kit window at
  launch, so the same window-cycle script can measure it. After 30 close/reopen
  cycles, this one-label window used 63.5 MB Working Set, 113.0 MB Private Bytes,
  562 handles, 48 threads and 31.5 MB GPU local memory. The full app's GUI
  footprint is therefore close to the framework/window baseline on this host;
  these are separate runs and should not be interpreted as an exact subtraction.
- In a 668,902-byte synthetic configuration with 1,000 stopped tunnels and
  512-byte descriptions, the previous Release background app sampled at
  53.6 MB Working Set and 61.7 MB Private Bytes after eight seconds. Sharing
  the startup configuration between desktop event tasks instead of cloning it
  sampled at 52.1/60.2 MB; two fresh runs were 52.6/60.8 and 52.1/60.3 MB.
  This change preserves one owned UI copy when a window actually opens.
- The changed full Release app completed 60 window close/reopen cycles. Its
  Working Set was 59.5 MB at cycle 1 and 62.8 MB at cycle 60; Private Bytes
  was 107.7→116.2 MB with non-monotonic swings, handles 599→590, threads
  52→48 and GPU local memory settled at 31.5 MB. No sustained linear growth
  appeared.
- After closing a single full-app window, Working Set was 62.4 MB at both five
  and 25 seconds; Private Bytes was 82.5 MB. The minimal GPUI Kit process
  likewise remained at 59.7 MB Working Set and 81.4 MB Private Bytes after
  25 seconds. View/window handles dropped, while framework-renderer memory
  remained resident. No working-set trimming was used.
