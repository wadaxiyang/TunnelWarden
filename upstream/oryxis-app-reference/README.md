# Oryxis app-layer reference source

This directory is populated by `scripts/fetch-oryxis-reference.*` from Oryxis commit `16ad2af83b424780ff60b6420a6c20a3b30c7143`.

The fetched files are **reference/copy sources** for Astra. The primary file is `dispatch_port_forwards.rs`. Do not translate its behavior from memory; port directly from the materialized source.

Files expected after fetching:

```text
dispatch_port_forwards.rs
messages/port_forward.rs
app.rs
boot/mod.rs
subscription.rs
state/forms.rs
views/port_forwards.rs
```
