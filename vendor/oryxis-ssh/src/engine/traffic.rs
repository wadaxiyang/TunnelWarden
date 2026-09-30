// TunnelWarden addition, 2026-09-30. SPDX-License-Identifier: AGPL-3.0-or-later
// Passive accounting around the pinned Oryxis writers. No buffering, transport,
// task, cancellation or retry decisions live here.
use std::{
    io, pin::Pin,
    sync::{Arc, atomic::{AtomicU64, Ordering}},
    task::{Context, Poll},
};
use tokio::io::AsyncWrite;

#[derive(Clone)]
pub(crate) struct ForwardTraffic {
    pub started_at: std::time::Instant,
    pub sent: Arc<AtomicU64>,
    pub received: Arc<AtomicU64>,
}
impl Default for ForwardTraffic {
    fn default() -> Self { Self { started_at: std::time::Instant::now(), sent: Arc::default(), received: Arc::default() } }
}
impl ForwardTraffic {
    pub fn bytes(&self) -> (u64, u64) {
        (self.sent.load(Ordering::Relaxed), self.received.load(Ordering::Relaxed))
    }
}
pub(crate) struct CountedWriter<W> { inner: W, bytes: Arc<AtomicU64> }
impl<W> CountedWriter<W> {
    pub fn new(inner: W, bytes: Arc<AtomicU64>) -> Self { Self { inner, bytes } }
}
impl<W: AsyncWrite + Unpin> AsyncWrite for CountedWriter<W> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = result { self.bytes.fetch_add(n as u64, Ordering::Relaxed); }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
