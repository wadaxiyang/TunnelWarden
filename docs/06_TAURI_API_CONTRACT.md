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
