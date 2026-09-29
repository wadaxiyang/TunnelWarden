# 03 — Architecture

```text
Svelte 5 / TypeScript
        │
        │ Tauri commands + events
        ▼
src-tauri (thin shell)
        │
        ▼
tunnelwarden-core
  ├─ persistence adapter
  ├─ credential adapter
  ├─ TunnelRuntimeManager
  │    ├─ active_forwards     (ported from Oryxis app)
  │    ├─ forward_conns       (ported from Oryxis app)
  │    ├─ starting            (ported from Oryxis app)
  │    └─ retries             (ported from Oryxis app)
  │
  ├───────────────┬────────────────┐
  ▼               ▼                ▼
oryxis-core    oryxis-ssh       Tokio
                  │
                  └─ russh
```

## Layer ownership

### `oryxis-core` / `oryxis-ssh`
Authoritative SSH semantics. TunnelWarden should adapt around them instead of penetrating them with duplicated types or alternate implementations.

### `tunnelwarden-core`
Owns the standalone-forward application runtime and persistence-facing services. Runtime behavior should be a direct adaptation of Oryxis' existing standalone forward implementation.

### `src-tauri`
Serializes/deserializes commands, emits state/log events, creates tray entries, and wires launch-at-login. It contains no SSH decisions.

### Svelte UI
Pure presentation and user intent. It renders DTOs and sends commands; it never infers network truth locally.

## Runtime model

Persisted rule state and live runtime state are separate. A persisted rule describes what should be started. Runtime registries describe what is actually alive.

UI status is intentionally small:

```text
stopped
connecting
running
retrying
error
```

Do not introduce generation/attempt/revision machinery unless a concrete regression proves Oryxis' simpler runtime model insufficient.
