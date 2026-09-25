use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;

pub const MAX_PROMPTS_PER_ROUND: usize = 8;
pub const MAX_ROUNDS: usize = 8;
pub const MAX_DISPLAY_BYTES: usize = 4096;
pub const MAX_ANSWER_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub struct InteractiveQuestion {
    pub text: String,
    pub echo: bool,
}

pub struct KeyboardInteractivePrompt {
    pub tunnel_id: String,
    pub generation: u64,
    pub attempt: u64,
    pub hop: usize,
    pub host: String,
    pub port: u16,
    pub round: usize,
    pub name: String,
    pub instructions: String,
    pub questions: Vec<InteractiveQuestion>,
    pub reply: oneshot::Sender<Option<Zeroizing<Vec<String>>>>,
}

pub(crate) struct InteractiveChallenge {
    pub name: String,
    pub instructions: String,
    pub questions: Vec<InteractiveQuestion>,
}

#[derive(Clone)]
pub struct KeyboardInteractiveApproval {
    pub prompts: mpsc::Sender<KeyboardInteractivePrompt>,
    pub tunnel_id: String,
    generation: u64,
    attempts: Arc<AtomicU64>,
    attempt: u64,
    hop: usize,
}

impl KeyboardInteractiveApproval {
    pub fn new(prompts: mpsc::Sender<KeyboardInteractivePrompt>, tunnel_id: String) -> Self {
        Self {
            prompts,
            tunnel_id,
            generation: 0,
            attempts: Arc::new(AtomicU64::new(0)),
            attempt: 0,
            hop: 0,
        }
    }

    pub fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    pub(crate) fn next_attempt(&self) -> Self {
        let mut next = self.clone();
        next.attempt = self.attempts.fetch_add(1, Ordering::Relaxed) + 1;
        next
    }

    pub(crate) fn for_hop(&self, hop: usize) -> Self {
        let mut next = self.clone();
        next.hop = hop;
        next
    }

    pub(crate) fn prompt(
        &self,
        host: &str,
        port: u16,
        round: usize,
        challenge: InteractiveChallenge,
        reply: oneshot::Sender<Option<Zeroizing<Vec<String>>>>,
    ) -> KeyboardInteractivePrompt {
        KeyboardInteractivePrompt {
            tunnel_id: self.tunnel_id.clone(),
            generation: self.generation,
            attempt: self.attempt,
            hop: self.hop,
            host: host.to_owned(),
            port,
            round,
            name: challenge.name,
            instructions: challenge.instructions,
            questions: challenge.questions,
            reply,
        }
    }
}

/// Treat server text as display-only data. Reject oversized input before allocating a copy.
pub(crate) fn display_text(text: &str) -> Result<String, &'static str> {
    if text.len() > MAX_DISPLAY_BYTES {
        return Err("keyboard-interactive display text exceeds 4 KiB");
    }
    Ok(text
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                '�'
            } else {
                c
            }
        })
        .collect())
}
