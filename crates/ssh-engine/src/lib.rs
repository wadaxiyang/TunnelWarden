//! SSH data plane. The session owns its cancellation token and russh handle.

mod cancellable_stream;
mod chain;
mod client;
mod host_key;
mod keyboard_interactive;

pub use chain::{HopSpec, SshChain, SshChainError};
pub use client::{
    DirectSshSession, ForwardingFailure, RemoteForwardRegistration, SshConnectError, SshCredential,
};
pub use host_key::{HostKeyApproval, HostKeyDecision, HostKeyError, HostKeyPrompt};
pub use keyboard_interactive::{
    InteractiveQuestion, KeyboardInteractiveApproval, KeyboardInteractivePrompt, MAX_ANSWER_BYTES,
    MAX_PROMPTS_PER_ROUND,
};

pub type DirectTcpStream = russh::ChannelStream<russh::client::Msg>;
