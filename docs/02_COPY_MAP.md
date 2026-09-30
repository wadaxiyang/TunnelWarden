# 02 — Oryxis Copy Map

Pinned revision: `16ad2af83b424780ff60b6420a6c20a3b30c7143`.

## A. Use directly as dependencies — do not rewrite

| Upstream area | TunnelWarden use |
| --- | --- |
| `crates/oryxis-core` | Connection/domain types, jump-route related models, `PortForwardRule`, `ForwardKind`, shared helpers |
| `crates/oryxis-ssh` | SSH connection/authentication, jump hosts, forwarding engine, `ForwardConn`, `ForwardSession`, cancellation, channel bridging |
| `crates/oryxis-ssh/src/engine/forwarding.rs` | Authoritative `-L/-R/-D` implementation and per-rule cancellation |
| `crates/oryxis-ssh/src/engine/connect.rs` | Authoritative connection establishment, including `connect_forward_conn` |

## B. Copy/adapt from `oryxis-app`

Primary source: `crates/oryxis-app/src/dispatch_port_forwards.rs`.

Copy these semantics as a coherent unit rather than re-deriving them:

- `PfHostConn`
- `PfRetry`
- `pf_retry_backoff`
- `pf_retry_still_wanted`
- late Start/Stop protection around `port_forward_starting`
- `active_forwards`
- `forward_conns`
- `port_forward_retry`
- `start_port_forward`
- `handle_port_forward_conn_ready`
- `attach_port_forward_task`
- `pf_gc_host_conns`
- `pf_mark_retry_pending`
- `handle_port_forward_retry_tick`
- liveness handling that removes dead sessions and schedules reconnect when appropriate
- explicit `ForwardSession::cancel().await` when stopping/deleting, especially to release `-R` server binds

### Supporting source files to inspect/copy semantics from

- `crates/oryxis-app/src/messages/port_forward.rs`
- `crates/oryxis-app/src/app.rs` — **only the standalone-forward runtime fields**
- `crates/oryxis-app/src/boot/mod.rs` — runtime initialization
- `crates/oryxis-app/src/subscription.rs` — liveness/retry cadence
- `crates/oryxis-app/src/state/forms.rs` — standalone forward form shape only
- `crates/oryxis-app/src/views/port_forwards.rs` — status/UX semantics only; do not copy iced widgets

## C. Adaptation rule

First copy upstream code into a temporary port with minimal changes. Then replace:

- `Oryxis` application state → `TunnelRuntimeManager`
- `iced::Task<Message>` → Tokio task/result flow
- `PortForwardMessage` UI messages → internal commands/events
- vault reads → TunnelWarden persistence/credential adapter
- iced subscription timers → Tokio interval tasks

Do **not** alter the forwarding lifecycle at the same time as removing iced. Get behavioral parity first; simplify only after acceptance tests pass.

## D. Do not copy

Oryxis terminal, SFTP, X11, remote-desktop launcher, cloud, sync, AI, plugin, iced view hierarchy, or unrelated application state.

## E. Test provenance

- `crates/tunnelwarden-core/tests/forwarding.rs`: adapted from pinned
  `crates/oryxis-ssh/tests/ssh_integration.rs` local/shared/remote/dynamic tests.
  Modification date: 2026-09-29. Replaced Docker setup with a loopback AsyncSSH
  fixture; retained public engine calls, tunneled banner assertions, SOCKS client
  handshake and explicit cancellation. Added same-connection remote rebind.

## F. Implemented runtime port (2026-09-29)

`crates/tunnelwarden-core/src/runtime.rs` copies/adapts the B-section symbols
from `dispatch_port_forwards.rs`, including agent census acceleration and the
upstream helper tests. Runtime fields/initialization follow `app.rs` and
`boot/mod.rs`; timers follow `subscription.rs`; completions follow
`messages/port_forward.rs`. All at the pinned SHA above.

Changes: iced tasks are collections of Tokio futures; the engine dial is an
application adapter; errors are per-rule; monotonic time uses Tokio Instant
(same behavior, controllable in tests). No internal agent server is exposed,
so its added-key count is zero. Stop IPC waits for a pending attach to finish
cancelling before acknowledging; this also serializes immediate Stop/Start
without adding generations. The same IPC barrier covers queued dial failures
before Stop/Start (2026-09-29); tests cover both late completion cases.
Stopping also withdraws the ID from pending host queues, so saving that rule
with a different host cannot reuse the old host's late connection. This is an
application editing requirement; fresh starts use the unchanged upstream queue.

## G. Connection adapter (2026-09-29)

`crates/tunnelwarden-core/src/dial.rs`: `connect_methods.rs`'s
`make_jump_resolver`, `expand_jump_chain`, `make_host_key_check`, and
`dispatch_port_forwards.rs`'s manual/background engine setup. Calls
`oryxis_core::jump_route::expanded_jump_chain` unchanged. Vault/group/identity
lookups become configuration + Windows Credential Manager lookups. No group,
proxy, certificate or TOTP features are surfaced. Host-key prompts use a Tokio
broker with per-question IDs and persistent consent; unattended connections
remain strict. Cancellation dismisses a stopped host's prompts. Key files are
read as supplied; encrypted keys should be loaded into ssh-agent (the pinned
engine accepts decrypted PEM via KeyMaterial). No new key decryption is added.

## H. v0.2.0 adapters (2026-09-30)

- Add `oryxis-vault` at the SAME immutable SHA. Call `import_key` unchanged,
  as upstream `dispatch_keys/import.rs` does. No vault database, new crypto,
  or persistent unlocked keys: only per-process zeroizing key cache. Identity
  edits invalidate that host cache; changed file contents bypass cached keys.
- Host test calls the existing `SshEngine::connect_forward_conn`, then drops
  the connection without attaching a rule. Scoped prompts/cancellation are
  TunnelWarden UI orchestration, separate from forwarding runtime intent.
- Copy `crates/oryxis-app/src/session_redact.rs` to the core with its full
  regression tests, source/header/date. Only rustfmt changes to its logic.
  Structured log input and diagnostic report strings use `redact_command`.
- Remote attachment still calls upstream `ForwardConn::attach`; application
  error context preserves the returned engine message and adds policy hints.

This supersedes section G's v0.1 encrypted-key/agent-only limitation.

## I. v0.2.1 passive traffic instrumentation (2026-09-30)

The pinned core/ssh/app have no per-forward byte telemetry. Vendor the complete
`crates/oryxis-ssh` at the same pin under `vendor/oryxis-ssh`; Cargo's patch table
selects it while core/vault remain git dependencies. Exact modifications and
license are recorded in its PROVENANCE.md and upstream/oryxis-ssh-traffic.patch.
Only pass-through write counters and their plumbing are added; the original
channel bridging, cancellation and accept loops stay intact. The runtime adds
an on-demand native telemetry query, with no persisted state or extra tick.
