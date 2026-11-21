//! A connection with its first bytes put back, and a clock on its reads.
//!
//! The router reads the ClientHello itself to find the server name. For
//! the relay's own host the same bytes must then go to TLS, so the stream
//! is wrapped to give them back first. The wrapper also stamps the time of
//! every read, which is how a silent line is told from a quiet one.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Instant;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// The time of the last read, in milliseconds since `base`.
#[derive(Debug)]
pub struct Activity {
    base: Instant,
    last_ms: AtomicU64,
}

impl Activity {
    /// A clock that reads "now" as its last activity.
    pub fn new() -> Arc<Self> {
        Arc::new(Activity {
            base: Instant::now(),
            last_ms: AtomicU64::new(0),
        })
    }

    /// Record activity.
    pub fn touch(&self) {
        let ms = u64::try_from(self.base.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.last_ms.store(ms, Ordering::Relaxed);
    }

    /// How long since the last activity.
    pub fn since(&self) -> std::time::Duration {
        let now = u64::try_from(self.base.elapsed().as_millis()).unwrap_or(u64::MAX);
        std::time::Duration::from_millis(now.saturating_sub(self.last_ms.load(Ordering::Relaxed)))
    }
}

/// `inner`, with `prefix` read first, and every read stamped on `activity`.
pub struct Front<S> {
    prefix: Vec<u8>,
    at: usize,
    inner: S,
    activity: Option<Arc<Activity>>,
}

impl<S> Front<S> {
    /// Give `prefix` back before anything from `inner`.
    pub fn new(prefix: Vec<u8>, inner: S, activity: Option<Arc<Activity>>) -> Self {
        Front {
            prefix,
            at: 0,
            inner,
            activity,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Front<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if this.at < this.prefix.len() {
            let rest = &this.prefix[this.at..];
            let n = rest.len().min(buf.remaining());
            buf.put_slice(&rest[..n]);
            this.at += n;
            if this.at == this.prefix.len() {
                // Give the memory back: a line lives for days.
                this.prefix = Vec::new();
                this.at = 0;
            }
            return Poll::Ready(Ok(()));
        }
        let before = buf.filled().len();
        let r = Pin::new(&mut this.inner).poll_read(cx, buf);
        if matches!(r, Poll::Ready(Ok(())))
            && buf.filled().len() > before
            && let Some(a) = &this.activity
        {
            a.touch();
        }
        r
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Front<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[tokio::test]
    async fn the_prefix_comes_first_then_the_stream() {
        let (mut a, b) = tokio::io::duplex(64);
        a.write_all(b"world").await.unwrap();
        drop(a);
        let act = Activity::new();
        let mut f = Front::new(b"hello ".to_vec(), b, Some(act.clone()));
        let mut out = String::new();
        f.read_to_string(&mut out).await.unwrap();
        assert_eq!(out, "hello world");
        assert!(act.since() < std::time::Duration::from_secs(5));
    }
}
