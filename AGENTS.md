# TunnelWarden — Agent Instructions

## Resource and memory discipline

Rust memory safety is not permission to waste memory. Code must remain bounded and have explicit ownership.

- Every long-lived resource needs a clear owner and end of life: task, subscription, timer, channel, cache, request, file buffer, window/view state, and native handle.
- **No detached or forgotten tasks.** Do not fire-and-forget `tokio::spawn`, `cx.spawn`, or equivalent work. Retain a handle or cancellation token and cancel/finish it when its owner is replaced, dropped, or the app shuts down.
- Tunnel work that survives window hiding is owned by the application runtime, never secretly kept alive by a hidden view.
- **No unbounded queues.** Use bounded channels with an explicit overload policy. A slow UI must recover from the latest/versioned state instead of accumulating infinite deltas.
- **No unbounded in-memory collections.** Any `Vec`, `VecDeque`, `HashMap`, cache, log, history, or pending-work collection that can grow from user/network activity must define a limit, eviction/pagination rule, or persistence boundary.
- Caches and diagnostic journals must be bounded and invalidatable. Configuration is TOML; do not add a database for it.
- **Do not clone to silence the borrow checker.** Avoid repeated `.clone()` of logs, snapshots, configuration, or network buffers, especially in relay and sampling paths. Borrow where possible. Use `Arc<str>`, `Arc<[u8]>`, IDs, or another shared immutable representation only when ownership is genuinely shared.
- Do not wrap broad state in `Arc<Mutex<_>>` merely to make code compile. Keep state ownership narrow and typed.
- Avoid `Arc`/`Rc` reference cycles. Prefer parent ownership plus IDs or `Weak` back-references where a back-reference is necessary.
- Traffic relays count bytes atomically and publish bounded samples at 1 Hz. Do not emit one UI event per packet.
- Render code describes UI only. It must not perform I/O, config/network calls, spawn tasks, create subscriptions, generate persistent IDs, or repeatedly allocate large buffers.
- Create InputState, focus/scroll handles, subscriptions, and other stateful GPUI entities once in their owner, not on each render.
- Hiding the window must not stop application-owned tunnels or retain an expensive hidden UI tree indefinitely.
- Do not add a hidden keeper window or invisible heavyweight UI tree to preserve state.

When changing long-lived state, relay, logs, caching, or window lifecycle, check Release memory behavior. Repeated show/hide, tunnel switching, and completed/cancelled connections should settle rather than show unexplained monotonic growth. Measure Working Set, Private Bytes, handles/threads, and GPU memory separately; never trim a working set to disguise growth.

## GPUI and UI

- Longbridge GPUI Kit is the default component source. Reuse its inputs, buttons, dialogs, menus, themes, scrolling, lists, selection, and Root before creating product-local equivalents.
- Product colors live in `theme.rs`; views use semantic roles and shared metrics.
- Window creation contains no business logic.
- Keep GPUI values on their owning thread and reject stale background UI updates using request/window generations where needed.
- Virtualize or page large lists. Scrolling away from the bottom disables log auto-follow.
- Escape closes the top overlay before hiding the window. File imports require explicit preview and confirmation.

## Code quality

Read and reuse existing code before adding abstractions. Prefer concrete types, enums, guard clauses, and small typed interfaces. Avoid forwarding layers, speculative architecture, unrelated refactors, and duplicated state.

Do not `unwrap` fallible user input, files, network, database, credentials, or Windows operations. Surface actionable failures without corrupting or discarding user data.

A change is not acceptable merely because it compiles. For resource-sensitive code, explicitly check ownership, cancellation, growth bounds, and repeated-use behavior.

## TunnelWarden specification invariants

`TunnelWarden_SPEC.md` defines the complete v1.0 release scope. Internal work packages
may be incremental, but do not describe an incomplete package as v1.0 or silently
defer Local, Remote, Dynamic SOCKS5, multi-hop, import, health, traffic, tray, or
reliability gates.

- Never delete desired, runtime, listener, health, or generation state to make code compile.
- Do not reduce the supervisor to an unowned retry loop. Every async task must identify its owner, cancellation path, generation, and join path.
- Every socket must identify its owner and release point. Stop completes only after listeners and channels have been released or the bounded shutdown deadline has expired and owned tasks have been aborted.
- All lifecycle changes go through the state machine transition function. UI renders Core snapshots; it never infers health from a bound port, task, socket, or process.
- `Healthy` requires the current generation's authenticated SSH chain, established forwarding, successful session ping, and live runtime task.
- Keep local listeners owned through transient reconnects. A stopped tunnel owns no listener, and an old generation cannot bind or report health.
- Never spawn `ssh.exe`, `plink.exe`, or other tunnel subprocesses. Do not introduce telemetry, accounts, license checks, or Pro gating.

