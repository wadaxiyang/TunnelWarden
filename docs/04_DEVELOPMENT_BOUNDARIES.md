# 04 — Development Boundaries

## Boundary 1 — no second SSH engine

TunnelWarden must not contain a handwritten replacement for behavior already present in the pinned `oryxis-ssh` engine. This includes transport setup, authentication, jump-host dialing, `-L`, `-R`, `-D`, SOCKS protocol handling, direct/forwarded TCP channels, stream bridging, remote-forward release, and connection liveness.

## Boundary 2 — no second reconnect architecture

Use the standalone forward retry model already implemented by Oryxis app: runtime registry + starting set + shared host connection + liveness tick + retry map/backoff. Port it; do not invent a generic supervisor framework.

## Boundary 3 — keep upstream code recognizable

When app-layer code is copied, preserve structure and comments until behavioral parity is verified. Avoid simultaneous style rewrites, abstraction changes, or broad renaming during the first port.

## Boundary 4 — independent product shell

Oryxis' iced UI is not part of TunnelWarden. Tauri/Svelte may be designed specifically for tunnel management, but it must not change backend semantics merely to simplify frontend code.

## Boundary 5 — configuration semantics

UI forms use mode-specific concepts:

- `-L`: Local listener → SSH host → Remote target
- `-R`: Remote listener → SSH host → Local target
- `-D`: Local SOCKS listener → SSH host

Never expose ambiguous generic labels such as “Local host / target” or “Remote host / listener”.

## Boundary 6 — runtime is not persisted

Do not persist `running`, `connecting`, `retrying`, errors, live handles, or derived liveness. Persist rule configuration and user preferences only.

## Boundary 7 — scope ceiling

Do not add terminal/SFTP/X11/remote-desktop/cloud/sync/AI/plugin functionality in the first implementation. If a dependency exposes these capabilities, leave them unused rather than surfacing them.

## Boundary 8 — tests before cleanup

Do not aggressively strip “unused-looking” upstream paths until forwarding parity tests are green. A small amount of unused upstream capability is preferable to accidentally deleting a dependency of auth/jump/forward behavior.
