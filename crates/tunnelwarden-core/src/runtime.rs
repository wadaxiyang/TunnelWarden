//! Standalone forward runtime.
//!
//! ASTRA: do not invent this module from scratch. Port it from:
//! Oryxis `16ad2af83b424780ff60b6420a6c20a3b30c7143`
//! `crates/oryxis-app/src/dispatch_port_forwards.rs`
//!
//! Copy the relevant runtime structures/functions first, then replace iced
//! `Task<Message>` / `Oryxis` state access with Tokio-owned runtime state and
//! TunnelWarden event callbacks. Preserve upstream cancellation, shared
//! connection, late-start/stop race handling, liveness, and retry semantics.
