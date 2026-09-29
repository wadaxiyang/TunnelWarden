#!/usr/bin/env bash
set -euo pipefail
# Cheap guardrail only; architectural review still matters.
if grep -RIn --exclude-dir=target --exclude-dir=.git --exclude='*.md' -E 'russh::|channel_open_direct_tcpip|tcpip_forward\(' crates/tunnelwarden-core src-tauri; then
  echo "Direct russh/forwarding calls found outside Oryxis dependency. Review against AGENTS.md copy-first boundary." >&2
  exit 1
fi
