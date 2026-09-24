//! SSH data plane. The session owns its cancellation token and russh handle.

mod cancellable_stream;
mod client;
mod host_key;

pub use client::{DirectSshSession, ForwardingFailure, RemoteForwardRegistration, SshConnectError};
pub use host_key::HostKeyError;

pub type DirectTcpStream = russh::ChannelStream<russh::client::Msg>;
