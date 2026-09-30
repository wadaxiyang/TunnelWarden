TunnelWarden 0.2.1 adds lightweight native tray controls.

- **Hover status panel**: running/saved count, each tunnel's status, upload and
  download payload rates, and short error details. Native Win32 text controls,
  no WebView, no focus stealing. Long lists scroll. The panel is created after
  a 500 ms hover and destroyed after dismissal; rates refresh only while visible.
- **Per-tunnel tray controls**: right click → Tunnels → name/status → Start or
  Stop. Connecting/retrying rules can be stopped. Open, Start all, Stop all and
  Quit remain available. All operations use the existing application runtime.
- **Accurate per-rule counters**: independent Local/Remote/Dynamic payload counts,
  including rules sharing one SSH connection. One-second sampled rates reset
  their baseline on reconnect. Upload is this PC → SSH; download is SSH → this
  PC. SSH/TCP/SOCKS overhead is excluded; successful writes do not prove delivery
  to the final application or external/public reachability.

Windows x64 NSIS `x64-setup.exe` and MSI installers are attached, with matching
source, license notices and SHA256SUMS.txt. Existing profiles remain compatible.
Packages are unsigned; installers obtain Microsoft WebView2 when needed for the
main configuration interface. Hovering and basic tray controls need no WebView.

Oryxis remains at 16ad2af83b424780ff60b6420a6c20a3b30c7143. The complete SSH crate
is vendored at that exact pin with passive writer accounting only. Its provenance
and full checked diff are included in the source archive. Original transport,
authentication, forwarding, cancellation and retry logic is retained.
AGPL-3.0-or-later; all license and modification notices are included.

Known limits: Windows x64; unsigned packages. A newly opened panel needs one
sampling interval before showing a measured rate. Manual encrypted-key unlock
is still needed after full app restart, or use an external agent. Unknown/changed
host keys require manual verification. Remote/public reachability depends on SSH
server policies and the network. The app must stay running to maintain tunnels.
