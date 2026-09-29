# AGENTS.md — TunnelWarden

## Mission

Build TunnelWarden as a focused desktop SSH tunnel manager. The SSH and forwarding implementation is inherited from Oryxis wherever Oryxis already solves the problem.

## Highest-priority rule: COPY FIRST

For every backend task, search the pinned Oryxis source **before writing implementation code**.

Priority order:

1. **Call an existing `oryxis-core` / `oryxis-ssh` public API unchanged.**
2. If the needed behavior lives in `oryxis-app`, **copy that implementation into `tunnelwarden-core` and adapt only the application/UI glue.**
3. Preserve the upstream control flow, cancellation behavior, retry semantics, comments, and invariants unless a TunnelWarden requirement makes a change necessary.
4. Write new Rust logic only for behavior that Oryxis does not provide: Tauri IPC, TunnelWarden persistence, DTO conversion, tray integration, application-specific orchestration, and tests around those adapters.

**Do not independently reimplement SSH transport, authentication, jump-host routing, `-L`, `-R`, `-D`, channel bridging, remote-forward cancellation, shared forward connections, or reconnect/backoff when the pinned Oryxis code already implements them.**

## Pinned upstream

- Repository: `https://github.com/wilsonglasser/oryxis`
- Commit: `16ad2af83b424780ff60b6420a6c20a3b30c7143`
- License: GNU AGPL-3.0-or-later
- Treat this SHA as immutable for the first implementation pass.
- Upgrading upstream is a separate explicit task with a source diff and regression run.

## Required source provenance

When copying Oryxis code into TunnelWarden:

- retain relevant upstream copyright/license headers;
- add a short modification notice with the TunnelWarden modification date when code is changed;
- record the upstream path and pinned SHA in a nearby comment when practical;
- keep `LICENSE`, `NOTICE.ORYXIS`, and `THIRD_PARTY_NOTICES.md` intact;
- update `docs/02_COPY_MAP.md` if a new upstream source file is copied.

## Architectural boundaries

- `oryxis-core` / `oryxis-ssh`: upstream SSH domain and engine.
- `tunnelwarden-core`: application runtime adapted primarily from Oryxis standalone port-forward logic.
- `src-tauri`: thin Tauri commands/events/tray/autostart shell. No SSH implementation belongs here.
- `src/`: Svelte UI only. It never holds russh/Oryxis runtime handles.
- Runtime state and persisted configuration are separate.

## State-model rule

Do not introduce a multi-generation supervisor framework. Start from Oryxis' standalone forward model:

- saved `PortForwardRule`
- active forward registry
- shared per-host `ForwardConn`
- starting set
- retry map
- liveness tick
- retry tick

Expose a simple UI status derived from that runtime: `stopped | connecting | running | retrying | error`.

## Build discipline

After each backend slice:

```text
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

After each frontend slice:

```text
npm run check
npm run build
```

Do not proceed past a failing stage by masking or deleting tests.

## Scope discipline

First release is a tunnel manager, not a terminal emulator, SFTP client, remote desktop client, proxy suite, cloud manager, or generic agent framework. Features outside `docs/00_PROJECT_CHARTER.md` require an explicit scope change.
