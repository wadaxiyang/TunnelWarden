# Pinned Oryxis SSH with passive traffic accounting

Source: https://github.com/wilsonglasser/oryxis
Commit: `16ad2af83b424780ff60b6420a6c20a3b30c7143`
Directory: `crates/oryxis-ssh` (complete crate, including original tests).
License: GNU AGPL-3.0-or-later. Copyright Wilson Glasser and contributors.
Modification date: 2026-09-30.

Upstream core/ssh/app were searched for traffic/byte counters and ForwardSession
APIs. This pin has no per-forward payload telemetry. TunnelWarden therefore uses
the vendoring exception in docs/01_UPSTREAM_AND_LICENSE.md, without upgrading it.

- Cargo.toml: resolve inherited workspace declarations at their original values;
  core stays at the pin; declare license and an isolated dependency workspace.
- forwarding.rs: thread per-session counters through existing accept/bridge calls;
  expose counts and their monotonic start time. The original copy/select/cancel,
  routing, SOCKS, channel and retry control flow is retained.
- connect.rs/session.rs: pass counters through existing auto-close forwards too.
- engine/mod.rs: register the new traffic module.
- traffic.rs: new pass-through AsyncWrite adapter; increment atomics only after
  successful writes. No polling task, buffer, extra socket or protocol is added.
- LICENSE: verbatim upstream root license.

Counts are payload written to SSH / to the local TCP endpoint, excluding SSH,
TCP and SOCKS framing. They reset on each new attachment; they do not establish
delivery to the final application or public reachability.

`upstream/oryxis-ssh-traffic.patch` is the full diff against the exact pin,
including these additions. `python scripts/verify-vendor.py` verifies that diff
against Cargo's pinned git checkout; use --write only when reviewing a change.
The app's real SSH integration tests cover live bidirectional accounting,
all modes, shared-connection isolation, reset and cancellation.
