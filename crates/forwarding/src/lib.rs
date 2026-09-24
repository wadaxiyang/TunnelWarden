//! Forwarding data plane primitives. Each relay is a single owned future with
//! bounded per-direction buffers and explicit cancellation.

mod relay;
mod socks5;

pub use relay::{TrafficCounters, relay_bidirectional};
pub use socks5::{
    Destination, Socks5Frontend, SocksConnectRequest, SocksError, SocksFrontend, SocksReply,
};
