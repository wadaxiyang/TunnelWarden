use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};

/// Cancels the actual transport even if russh has already moved it into its
/// internal session task while `connect_stream` is still awaiting the KEX.
pub(crate) struct CancellableStream {
    inner: TcpStream,
    cancelled: Pin<Box<WaitForCancellationFutureOwned>>,
}

impl CancellableStream {
    pub(crate) fn new(inner: TcpStream, cancellation: CancellationToken) -> Self {
        Self {
            inner,
            cancelled: Box::pin(cancellation.cancelled_owned()),
        }
    }

    fn poll_cancelled(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.cancelled.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "SSH transport cancelled",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncRead for CancellableStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Poll::Ready(result) = this.poll_cancelled(cx) {
            return Poll::Ready(result);
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for CancellableStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if let Poll::Ready(result) = this.poll_cancelled(cx) {
            return Poll::Ready(result.map(|()| 0));
        }
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Poll::Ready(result) = this.poll_cancelled(cx) {
            return Poll::Ready(result);
        }
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}
