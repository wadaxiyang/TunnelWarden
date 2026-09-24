//! SSH data plane. The session owns its cancellation token and russh handle.

mod cancellable_stream;
mod chain;
mod client;
mod host_key;

pub use chain::{HopSpec, SshChain, SshChainError};
pub use client::{
    DirectSshSession, ForwardingFailure, RemoteForwardRegistration, SshConnectError, SshCredential,
};
pub use host_key::{HostKeyApproval, HostKeyDecision, HostKeyError, HostKeyPrompt};

pub type DirectTcpStream = russh::ChannelStream<russh::client::Msg>;
