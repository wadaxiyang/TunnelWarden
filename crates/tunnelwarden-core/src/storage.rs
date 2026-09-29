//! TunnelWarden persistence adapter.
//!
//! This is application glue, not an SSH implementation. Persist user-visible
//! host/rule settings; keep runtime handles and runtime status out of storage.
