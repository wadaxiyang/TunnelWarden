# 05 — Implementation Phases

## Phase 0 — materialize and pin upstream

Run the reference-fetch script. Confirm Oryxis SHA equals `UPSTREAM.lock`. Run a minimal Cargo resolution using the pinned git dependencies.

**Exit:** exact upstream is available locally and attribution files are intact.

## Phase 1 — backend forwarding harness, no GUI

Create a tiny test/harness around `oryxis-ssh` public forwarding APIs. Verify direct SSH plus `-L`, `-R`, `-D` without inventing new forwarding code.

**Exit:** all three forwarding modes can start and explicitly cancel.

## Phase 2 — port Oryxis standalone runtime

Copy/adapt the runtime subset mapped in `docs/02_COPY_MAP.md`. Replace iced task/message plumbing with Tokio and internal callbacks while preserving behavior.

**Exit:** shared host connection, late-stop protection, liveness, auto-start and retry/backoff behave like Oryxis.

## Phase 3 — persistence and credential adapter

Persist hosts/rules/preferences. Keep runtime separate. Prefer existing platform/keyring facilities for secrets; do not invent cryptography.

**Exit:** app restart reconstructs saved hosts/rules without resurrecting manually stopped runtime state unexpectedly.

## Phase 4 — Tauri API

Implement the narrow command/event contract in `docs/06_TAURI_API_CONTRACT.md`.

**Exit:** a headless frontend test can CRUD and start/stop rules via Tauri-facing service methods.

## Phase 5 — Svelte GUI

Implement the UI specification without backend changes. Forms must be mode-specific and validate ports/required endpoints before submit.

**Exit:** full user flow is possible from GUI.

## Phase 6 — tray/autostart/logging

Wire tray controls and launch-at-login. Auto-start of rules must reuse the exact runtime start path.

## Phase 7 — verification and only then cleanup

Run the acceptance matrix. Only after it passes may unused copied reference/glue be simplified.
