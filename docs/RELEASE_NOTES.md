TunnelWarden 0.2.2 adds tunnel details and live traffic trends.

- Click a tunnel card/name for **Overview / Traffic / History** tabs with fixed
  navigation. Inspect its live status, error/retry details,
  configuration, connection age and exact per-connection sent/received bytes.
  Start, Stop, Restart and Edit are available directly from the detail page.
- Upload/download line chart with 1-minute and 5-minute views, plus a table of
  recent exact rates in a separate dialog. Measurements use real elapsed time and preserve integer
  counter precision. Reconnects, pauses and missing samples appear as gaps.
- Per-tunnel state/activity history, errors-only filter and older-entry paging.
  History comes from the latest 1,000 application log entries in this process.
- Lightweight on-demand sampling: only the visible detail page polls, once per
  second. Leaving details clears chart history and stops polling. Closing the
  UI releases WebView2 while the Rust runtime and tray keep tunnels alive.

Windows x64 NSIS EXE and MSI installers are attached. Existing profiles remain
compatible. Source, license notices and SHA256SUMS.txt accompany the binaries.
Packages are unsigned. Main-window WebView2 is obtained by the installer when
needed; the native tray controls still do not create a WebView.

Oryxis remains pinned to 16ad2af83b424780ff60b6420a6c20a3b30c7143.
This release reuses the existing v0.2.1 passive payload counters unchanged.
The reviewed vendor patch, provenance and AGPL-3.0-or-later notices are included.

Limits: chart history starts when details open, retains at most five minutes,
and is not saved. Activity history clears on process exit. Byte totals reset
on reconnect. Payload measurements exclude protocol overhead and cannot prove
final delivery or public reachability. Remote accessibility depends on server
policy and the network. Encrypted files need manual unlock after full app
restart, or an external SSH agent for unattended operation.
