# Third-Party Notices

## Oryxis

TunnelWarden intentionally reuses and adapts code from **Oryxis**.

- Upstream repository: `https://github.com/wilsonglasser/oryxis`
- Pinned source revision: `16ad2af83b424780ff60b6420a6c20a3b30c7143`
- Upstream copyright: Copyright (C) 2026 Wilson Glasser and Oryxis contributors
- License: **GNU Affero General Public License v3.0 or later**
- Original notice: `NOTICE.ORYXIS`
- License text: `LICENSE`

TunnelWarden source copied or adapted from Oryxis must retain attribution and modification notices required by the license. This file is a project engineering notice, not legal advice.

### v0.2.1 SSH instrumentation

`vendor/oryxis-ssh` is the complete Oryxis SSH crate at the pinned SHA above,
modified 2026-09-30 only for passive per-forward payload counters and manifest
adaptation. See its PROVENANCE.md, LICENSE and the checked source diff in
`upstream/oryxis-ssh-traffic.patch`. GNU AGPL-3.0-or-later.

### Application source provenance

Consolidated 2026-10-14. All Oryxis paths below are relative to the upstream
repository at `16ad2af83b424780ff60b6420a6c20a3b30c7143`. Original copyright,
license headers, and modification dates remain beside the adapted source.

| TunnelWarden source | Oryxis source | Adaptation |
| --- | --- | --- |
| `crates/tunnelwarden-core/src/runtime.rs` | `crates/oryxis-app/src/dispatch_port_forwards.rs`, `app.rs`, `boot/mod.rs`, `subscription.rs`, `messages/port_forward.rs` | Adapted 2026-09-29: replace iced tasks with Tokio futures and application DTOs; retain shared connections, start tracking, retry/backoff, liveness, cancellation, and upstream helper tests. Stop IPC awaits pending cancellation; edits withdraw pending host requests. |
| `crates/tunnelwarden-core/src/dial.rs` | `crates/oryxis-app/src/connect_methods.rs`, `dispatch_port_forwards.rs` | Adapted 2026-09-29 and 2026-09-30: replace vault/group lookups with saved configuration and Windows Credential Manager; bridge host-key prompts, cancellation, connection tests, and in-memory key unlock into the application. |
| `crates/tunnelwarden-core/src/session_redact.rs` | `crates/oryxis-app/src/session_redact.rs` | Copied 2026-09-30 with upstream regression tests; rustfmt-only logic changes, used for application diagnostics. |
| `upstream/oryxis-app-reference/` | `crates/oryxis-app/src/` paths listed in that directory's README | Pinned reference copies for reviewing and adapting standalone forwarding behavior; not linked into the application. |
| `vendor/oryxis-ssh/` | `crates/oryxis-ssh/` | Complete pinned crate, including upstream tests; manifest adaptation and passive per-forward traffic counters. See `vendor/oryxis-ssh/PROVENANCE.md` and the reviewed patch. |

`oryxis-core` and the `oryxis-vault::import_key` API are used directly from the
same pinned revision. Key import follows upstream
`crates/oryxis-app/src/dispatch_keys/import.rs`; no vault database or custom
decryption implementation is introduced. Application forms and connection
adapters were reviewed against upstream `state/forms.rs`,
`views/port_forwards.rs`, and `connect_methods.rs`.
