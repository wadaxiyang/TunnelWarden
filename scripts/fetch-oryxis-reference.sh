#!/usr/bin/env bash
set -euo pipefail

SHA="16ad2af83b424780ff60b6420a6c20a3b30c7143"
URL="https://github.com/wilsonglasser/oryxis.git"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UPSTREAM="$ROOT/.upstream/oryxis"
DEST="$ROOT/upstream/oryxis-app-reference"

rm -rf "$UPSTREAM"
mkdir -p "$(dirname "$UPSTREAM")" "$DEST"

git init "$UPSTREAM"
git -C "$UPSTREAM" remote add origin "$URL"
git -C "$UPSTREAM" fetch --depth 1 origin "$SHA"
git -C "$UPSTREAM" checkout --detach FETCH_HEAD

ACTUAL="$(git -C "$UPSTREAM" rev-parse HEAD)"
if [[ "$ACTUAL" != "$SHA" ]]; then
  echo "Oryxis SHA mismatch: $ACTUAL" >&2
  exit 1
fi

copy() { mkdir -p "$DEST/$(dirname "$1")"; cp "$UPSTREAM/crates/oryxis-app/src/$1" "$DEST/$1"; }
copy dispatch_port_forwards.rs
copy messages/port_forward.rs
copy app.rs
copy boot/mod.rs
copy subscription.rs
copy state/forms.rs
copy views/port_forwards.rs

cp "$UPSTREAM/LICENSE" "$ROOT/LICENSE"
cp "$UPSTREAM/NOTICE" "$ROOT/NOTICE.ORYXIS"

echo "Materialized Oryxis reference at $SHA"
