//! One multiplexed line: streams in both directions over one connection.
//!
//! A line is a yamux connection over the app's TLS. One task drives it:
//! it answers the peer's frames, accepts the streams the peer opens, and
//! opens streams on request. Opening is by a bounded request queue, so a
//! caller who wants a stream waits its turn and cannot make the line hold
//! more than a request at a time.
//!
//! ```text
//! GOAL: many assistant connections ride one line, with the number of
//!       streams, the bytes buffered per stream and the time a silent
//!       line may live all capped, and a line that is closed or dropped
//!       stops its task.
//! SOURCE: WB-relay-7 (bounds), the yamux specification.
//! INVARIANTS:
//!   1. At most `max_streams` streams exist on a line, in both
//!      directions together. A line asked for one more, from either
//!      side, is ended (that is yamux's rule), so the relay keeps its
//!      own per-line permits below the limit.
//!   2. Each stream's receive window is 256 KiB and a line's windows sum
//!      to at most 256 KiB times `max_streams`: no window grows.
//!   3. A line from which nothing was read for `idle` is closed.
//!   4. Dropping every handle, or `close`, ends the driver.
//! SCENARIOS:
//!   1. Streams opened on one side arrive on the other.
//!   2. With `max_streams` open, one more open does not succeed.
//!   3. A line with a silent peer is closed after `idle`.
//! ```

use std::future::poll_fn;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt};
use tokio_util::sync::CancellationToken;
use yamux::{Config, Connection, Mode};

use crate::front::Activity;

/// A stream on a line, as tokio's `AsyncRead` and `AsyncWrite`.
pub type Stream = Compat<yamux::Stream>;

/// Why a stream could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MuxError {
    /// The line is closed.
    #[error("the line is closed")]
    Closed,
    /// Waited too long.
    #[error("timed out opening a stream")]
    Timeout,
    /// The line refused (too many streams, or the peer is gone).
    #[error("the line refused the stream")]
    Refused,
}

type Open = oneshot::Sender<Result<Stream, MuxError>>;

/// A handle for opening streams on a line. Clones share the line.
#[derive(Clone)]
pub struct Mux {
    cmd: mpsc::Sender<Open>,
    cancel: CancellationToken,
}

/// What a line is allowed.
#[derive(Debug, Clone, Copy)]
pub struct MuxLimits {
    /// Streams at once, both directions together.
    pub max_streams: usize,
    /// A line from which nothing was read for this long is closed.
    pub idle: Duration,
}

fn yamux_config(max_streams: usize) -> Config {
    let mut cfg = Config::default();
    cfg.set_max_num_streams(max_streams);
    // 256 KiB is yamux's initial window per stream. Capping the sum at
    // 256 KiB times the stream limit stops the auto-tuning from growing
    // any window: this is the most a line can ever hold in receive
    // buffers.
    cfg.set_max_connection_receive_window(Some(max_streams * yamux::DEFAULT_CREDIT as usize));
    cfg
}

impl Mux {
    /// Open a stream, waiting at most `wait` for the line to take the
    /// request and for the peer to accept it.
    pub async fn open(&self, wait: Duration) -> Result<Stream, MuxError> {
        let (reply, answer) = oneshot::channel();
        let deadline = Instant::now() + wait;
        match tokio::time::timeout_at(deadline, self.cmd.send(reply)).await {
            Err(_) => return Err(MuxError::Timeout),
            Ok(Err(_)) => return Err(MuxError::Closed),
            Ok(Ok(())) => {}
        }
        match tokio::time::timeout_at(deadline, answer).await {
            Err(_) => Err(MuxError::Timeout),
            Ok(Err(_)) => Err(MuxError::Closed),
            Ok(Ok(r)) => r,
        }
    }

    /// Close the line.
    pub fn close(&self) {
        self.cancel.cancel();
    }

    /// True once the line is closed or its driver has ended.
    pub fn is_closed(&self) -> bool {
        self.cancel.is_cancelled() || self.cmd.is_closed()
    }
}

enum Event {
    Inbound(Stream),
    Ended,
}

fn step<T>(
    conn: &mut Connection<T>,
    cmds: &mut mpsc::Receiver<Open>,
    pending: &mut Option<Open>,
    cx: &mut Context<'_>,
) -> Poll<Event>
where
    T: futures::AsyncRead + futures::AsyncWrite + Unpin,
{
    // The connection makes progress only when polled for inbound streams,
    // so that always comes first.
    match conn.poll_next_inbound(cx) {
        Poll::Ready(Some(Ok(s))) => return Poll::Ready(Event::Inbound(s.compat())),
        Poll::Ready(Some(Err(_)) | None) => return Poll::Ready(Event::Ended),
        Poll::Pending => {}
    }
    loop {
        if pending.is_none() {
            match cmds.poll_recv(cx) {
                Poll::Ready(Some(open)) => *pending = Some(open),
                Poll::Ready(None) => return Poll::Ready(Event::Ended),
                Poll::Pending => return Poll::Pending,
            }
        }
        if pending.as_ref().is_some_and(|p| p.is_closed()) {
            *pending = None; // the asker gave up: do not open for nobody
            continue;
        }
        match conn.poll_new_outbound(cx) {
            Poll::Ready(Ok(s)) => {
                if let Some(p) = pending.take() {
                    let _ = p.send(Ok(s.compat()));
                }
                // Opening queued a frame; only polling for inbound frames
                // sends it, so come round again at once.
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            Poll::Ready(Err(yamux::ConnectionError::TooManyStreams)) => {
                if let Some(p) = pending.take() {
                    let _ = p.send(Err(MuxError::Refused));
                }
            }
            Poll::Ready(Err(_)) => return Poll::Ready(Event::Ended),
            Poll::Pending => return Poll::Pending,
        }
    }
}

/// Start driving a line. `on_inbound` is called, inline and without
/// blocking, with each stream the peer opens; it must spawn or drop.
pub fn spawn<T>(
    socket: T,
    mode: Mode,
    limits: MuxLimits,
    activity: Option<Arc<Activity>>,
    mut on_inbound: impl FnMut(Stream) + Send + 'static,
) -> (Mux, JoinHandle<()>)
where
    T: futures::AsyncRead + futures::AsyncWrite + Unpin + Send + 'static,
{
    let (cmd, mut cmds) = mpsc::channel::<Open>(4);
    let cancel = CancellationToken::new();
    let mux = Mux {
        cmd,
        cancel: cancel.clone(),
    };
    let mut conn = Connection::new(socket, yamux_config(limits.max_streams), mode);
    let task = tokio::spawn(async move {
        let mut pending: Option<Open> = None;
        let idle = limits.idle;
        let timer = tokio::time::sleep(idle);
        tokio::pin!(timer);
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                () = &mut timer => {
                    let quiet = activity.as_ref().map_or(Duration::ZERO, |a| a.since());
                    if quiet >= idle {
                        break;
                    }
                    timer.as_mut().reset(Instant::now() + (idle - quiet));
                }
                event = poll_fn(|cx| step(&mut conn, &mut cmds, &mut pending, cx)) => match event {
                    Event::Inbound(stream) => on_inbound(stream),
                    Event::Ended => break,
                },
            }
        }
        cancel.cancel();
        // Refuse anyone still waiting, then close the connection politely
        // but briefly.
        if let Some(p) = pending.take() {
            let _ = p.send(Err(MuxError::Closed));
        }
        cmds.close();
        let _ =
            tokio::time::timeout(Duration::from_secs(2), poll_fn(|cx| conn.poll_close(cx))).await;
    });
    (mux, task)
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_util::compat::TokioAsyncReadCompatExt;

    use super::*;

    fn pair(
        max: usize,
        idle: Duration,
        on_server_inbound: impl FnMut(Stream) + Send + 'static,
    ) -> (Mux, Mux, mpsc::UnboundedReceiver<Stream>) {
        let (a, b) = tokio::io::duplex(64 * 1024);
        let limits = MuxLimits {
            max_streams: max,
            idle,
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let (server, _) = spawn(a.compat(), Mode::Server, limits, None, on_server_inbound);
        let (client, _) = spawn(b.compat(), Mode::Client, limits, None, move |s| {
            let _ = tx.send(s);
        });
        (server, client, rx)
    }

    #[tokio::test]
    async fn a_stream_opened_by_one_side_arrives_on_the_other() {
        let (server, _client, mut rx) = pair(8, Duration::from_secs(60), |_| {});
        let mut s = server.open(Duration::from_secs(2)).await.unwrap();
        s.write_all(b"ping").await.unwrap();
        s.flush().await.unwrap();
        let mut got = rx.recv().await.unwrap();
        let mut buf = [0u8; 4];
        got.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");
        got.write_all(b"pong").await.unwrap();
        got.flush().await.unwrap();
        s.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"pong");
    }

    #[tokio::test]
    async fn a_line_holds_only_its_streams() {
        let (server, _client, mut rx) = pair(2, Duration::from_secs(60), |_| {});
        let mut held = Vec::new();
        for _ in 0..2 {
            let mut s = server.open(Duration::from_secs(2)).await.unwrap();
            s.write_all(b"x").await.unwrap();
            s.flush().await.unwrap();
            held.push(s);
            held.push(rx.recv().await.unwrap()); // keep the far end open too
        }
        let third = server.open(Duration::from_millis(500)).await;
        assert!(third.is_err(), "a third stream was opened past the limit");
    }

    #[tokio::test]
    async fn a_silent_line_is_closed_after_its_idle_time() {
        let (a, b) = tokio::io::duplex(1024);
        let activity = Activity::new();
        let limits = MuxLimits {
            max_streams: 2,
            idle: Duration::from_millis(150),
        };
        let (mux, task) = spawn(a.compat(), Mode::Server, limits, Some(activity), |_| {});
        let _keep = b; // the peer is there and says nothing
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert!(mux.is_closed());
        assert_eq!(
            mux.open(Duration::from_millis(100)).await.err(),
            Some(MuxError::Closed)
        );
    }

    #[tokio::test]
    async fn closing_ends_the_driver() {
        let (server, client, _rx) = pair(2, Duration::from_secs(60), |_| {});
        server.close();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(server.is_closed());
        assert!(client.open(Duration::from_secs(2)).await.is_err() || client.is_closed());
    }

    #[tokio::test]
    async fn a_large_transfer_both_ways_completes() {
        let (server, _client, mut rx) = pair(8, Duration::from_secs(60), |_| {});
        let mut near = server.open(Duration::from_secs(2)).await.unwrap();
        near.write_all(b"x").await.unwrap();
        near.flush().await.unwrap();
        let mut far = rx.recv().await.unwrap();
        let mut one = [0u8; 1];
        far.read_exact(&mut one).await.unwrap();
        let echo = tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            loop {
                let n = far.read(&mut buf).await.unwrap();
                if n == 0 {
                    far.shutdown().await.unwrap();
                    return;
                }
                far.write_all(&buf[..n]).await.unwrap();
            }
        });
        let (mut rd, mut wr) = tokio::io::split(near);
        let data = vec![9u8; 2_000_000];
        let send = tokio::spawn({
            let data = data.clone();
            async move {
                wr.write_all(&data).await.unwrap();
                wr.shutdown().await.unwrap();
            }
        });
        let mut got = Vec::new();
        tokio::time::timeout(Duration::from_secs(20), rd.read_to_end(&mut got))
            .await
            .expect("the echo completes")
            .unwrap();
        assert_eq!(got.len(), data.len());
        send.await.unwrap();
        echo.await.unwrap();
    }
}
