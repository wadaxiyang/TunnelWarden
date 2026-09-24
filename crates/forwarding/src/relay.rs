use std::{
    io,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

const RELAY_BUFFER_BYTES: usize = 16 * 1024;

#[derive(Default)]
pub struct TrafficCounters {
    uploaded: AtomicU64,
    downloaded: AtomicU64,
    active_connections: AtomicU32,
}

impl TrafficCounters {
    pub fn uploaded(&self) -> u64 {
        self.uploaded.load(Ordering::Relaxed)
    }
    pub fn downloaded(&self) -> u64 {
        self.downloaded.load(Ordering::Relaxed)
    }
    pub fn active_connections(&self) -> u32 {
        self.active_connections.load(Ordering::Relaxed)
    }
}

struct ActiveConnection<'a>(&'a TrafficCounters);

impl<'a> ActiveConnection<'a> {
    fn new(counters: &'a TrafficCounters) -> Self {
        counters.active_connections.fetch_add(1, Ordering::Relaxed);
        Self(counters)
    }
}

impl Drop for ActiveConnection<'_> {
    fn drop(&mut self) {
        self.0.active_connections.fetch_sub(1, Ordering::Relaxed);
    }
}

pub async fn relay_bidirectional<L, R>(
    local: L,
    remote: R,
    cancellation: &CancellationToken,
    counters: &TrafficCounters,
) -> io::Result<()>
where
    L: AsyncRead + AsyncWrite + Unpin,
    R: AsyncRead + AsyncWrite + Unpin,
{
    let _active = ActiveConnection::new(counters);
    let (local_read, local_write) = tokio::io::split(local);
    let (remote_read, remote_write) = tokio::io::split(remote);
    tokio::try_join!(
        copy_direction(local_read, remote_write, cancellation, &counters.uploaded),
        copy_direction(remote_read, local_write, cancellation, &counters.downloaded),
    )?;
    Ok(())
}

async fn copy_direction<R, W>(
    mut reader: R,
    mut writer: W,
    cancellation: &CancellationToken,
    counter: &AtomicU64,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = [0u8; RELAY_BUFFER_BYTES];
    loop {
        let count = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Ok(()),
            result = reader.read(&mut buffer) => result?,
        };
        if count == 0 {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Ok(()),
                result = writer.shutdown() => result?,
            }
            return Ok(());
        }
        let mut written = 0;
        while written < count {
            let n = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Ok(()),
                result = writer.write(&buffer[written..count]) => result?,
            };
            if n == 0 {
                return Err(io::Error::from(io::ErrorKind::WriteZero));
            }
            written += n;
            counter.fetch_add(n as u64, Ordering::Relaxed);
        }
    }
}
