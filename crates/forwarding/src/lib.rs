//! Forwarding data plane primitives. Each relay is a single owned future with
//! bounded per-direction buffers and explicit cancellation.

mod relay;

pub use relay::{TrafficCounters, relay_bidirectional};
