# 09 — Astra Execution Guide

## Model usage

This repository is intentionally structured for an implementation agent such as Astra. The task is **migration and adaptation**, not greenfield SSH design.

## First instruction to Astra

```text
Read AGENTS.md and docs/00 through docs/08 before changing code.
Materialize the pinned Oryxis sources at commit 16ad2af83b424780ff60b6420a6c20a3b30c7143 using the provided script.
Do not implement SSH or port forwarding from scratch.
For every backend behavior, first locate the corresponding Oryxis implementation and either call it or port it with a traceable source path.
Work phase-by-phase from docs/05_IMPLEMENTATION_PHASES.md and stop architecture expansion outside the stated scope.
```

## Context/token discipline

Do not load the whole Oryxis monorepo into context. Search by symbol/path and inspect only the dependency slice needed for the current phase.

Recommended reading order for runtime port:

1. `oryxis-core/src/models/port_forward_rule.rs`
2. `oryxis-ssh/src/engine/forwarding.rs`
3. `oryxis-ssh/src/engine/connect.rs` around `connect_forward_conn`
4. `oryxis-app/src/dispatch_port_forwards.rs`
5. supporting app fields/messages only when a referenced symbol requires them

## Commit discipline

Prefer one implementation phase per commit. Commit messages should name the upstream source when a port is introduced, for example:

```text
port standalone forward runtime from Oryxis 16ad2af
```

## When Astra finds missing behavior

Before writing a new implementation:

1. search pinned `oryxis-ssh`;
2. search pinned `oryxis-core`;
3. search pinned `oryxis-app`;
4. record the search result in the working notes;
5. only then add the smallest TunnelWarden-specific glue.

## Red flags

Stop and reassess if a change starts creating any of these:

- a second SSH client wrapper around russh that bypasses Oryxis;
- a custom SOCKS implementation;
- a new generic supervisor/state-machine framework;
- duplicated host/auth/forward domain models with independent semantics;
- backend code inside Svelte/Tauri command handlers;
- removal of upstream lifecycle checks merely to simplify compilation.
