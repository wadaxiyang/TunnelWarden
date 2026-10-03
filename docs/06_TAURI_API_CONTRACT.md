# 06 — Tauri API Contract

The Tauri layer stays intentionally thin. Names below are the target public surface; exact Rust type names may evolve without changing semantics.

## Query commands

```text
list_hosts() -> HostDto[]
list_tunnels() -> TunnelDto[]
get_runtime_snapshot() -> RuntimeSnapshotDto
get_logs(filter?) -> LogEntryDto[]
get_settings() -> SettingsDto
```

## Mutation commands

```text
save_host(host)
delete_host(host_id)
save_tunnel(tunnel)
delete_tunnel(tunnel_id)
start_tunnel(tunnel_id)
stop_tunnel(tunnel_id)
restart_tunnel(tunnel_id)
set_settings(settings)
```

## Events

```text
tunnelwarden://runtime
    full or per-rule authoritative runtime update

tunnelwarden://log
    structured log entry
```

Avoid a high-frequency event stream for data the first release does not need. Do not expose `russh::Handle`, `ForwardConn`, `ForwardSession`, channels, mutexes, or Tokio task handles across IPC.

## Runtime DTO

Minimum per-rule fields:

```text
id
status: stopped | connecting | running | retrying | error
last_error?: string
next_retry_at?: timestamp
```

The backend is authoritative. The frontend must not optimistically claim `running` before the backend reports it.

## Implemented host-key consent extension

`get_host_key_prompts()` and `respond_host_key(prompt_id, trust)` expose Oryxis'
manual host-key question. `tunnelwarden://host-key` broadcasts the pending
question list. A question includes hostname, SSH port, key algorithm, SHA256
fingerprint and (when changed) the old fingerprint. Accept persists trust before
responding to the engine. Background starts never prompt or accept unknown keys.

Stop/Delete wait for any attached forward's explicit cancellation. Save Tunnel
stops the old runtime before persisting the edited rule. Editing an SSH host
requires its own and dependent jump-route connections to be idle. Password input
is accepted only by save_host, stored by the OS, and never returned by queries.

## v0.2 additions

- `test_host(host_id) -> HostTestResult`, `cancel_host_test(host_id)`.
- `get_key_passphrase_prompts()`, `respond_key_passphrase(prompt_id, passphrase | null)`;
  `tunnelwarden://key-passphrase` emits questions only, never the response/secret.
- `export_error_logs() -> string | null`: native Save As; returns the saved path
  or null on cancellation. No arbitrary frontend-supplied file path is accepted.

## v0.2.2 detail telemetry

`get_tunnel_traffic(tunnel_id) -> TrafficSample | null` returns null for an
inactive saved rule; a deleted/unknown rule returns an error. It reuses the
runtime's passive counters. Decimal byte strings preserve u64 precision;
`attachment` is an opaque token which changes on reattach. `sampled_ms` and
`elapsed_ms` use monotonic time, not wall-clock time. No query starts a tunnel.
UI history uses the existing per-tunnel filtered logs/events. Poll only while
viewing visible details; discard rate baselines on pause, gaps or reattach.
