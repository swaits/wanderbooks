//! Piping bytes both ways between a caller and a stream, with nothing
//! held back.
//!
//! Each direction has one buffer. It reads only when the buffer is empty
//! and writes it out before reading again, so a reader that is slow on
//! one side slows the writer on the other, and no more than one buffer
//! per direction ever sits in the relay. The relay reads none of it: the
//! bytes are TLS between the assistant and the app.
//!
//! ```text
//! GOAL: forward encrypted bytes both ways, with a slow, silent or
//!       hostile end unable to make the relay hold more than two small
//!       buffers or hold the connection past its limits.
//! SOURCE: WB-relay-1 (the router and its splice), WB-relay-7 (bounds).
//! INVARIANTS:
//!   1. At most `buffer` bytes are held per direction.
//!   2. A write of `n` bytes that is not finished within `stall` plus `n`
//!      over `min_rate` bytes a second ends the connection: a reader
//!      slower than that floor is closed, never buffered for. (A reader
//!      that takes one byte just inside every stall would otherwise hold
//!      the stream, and the receive window behind it, for hours.)
//!   3. A connection with no byte either way for `idle`, or alive for
//!      `max_age`, is closed.
//!   4. A close in one direction (EOF) is passed on as a close of the
//!      other side's write half; the other direction carries on until it
//!      ends too.
//! SCENARIOS:
//!   1. Bytes written at either end arrive at the other, in order, and a
//!      close at one end reaches the other.
//!   2. A peer that never reads: the splice ends after `stall`. A peer that
//!      reads, but under `min_rate`, ends it too; one that keeps the pace
//!      does not, however long the transfer.
//!   3. A peer that says nothing: the splice ends after `idle`.
//! ```

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{Instant, sleep, timeout};

/// The limits of one splice.
#[derive(Debug, Clone, Copy)]
pub struct SpliceLimits {
    /// Buffer bytes per direction.
    pub buffer: usize,
    /// No byte either way for this long ends it.
    pub idle: Duration,
    /// The grace on every write: a write has at least this long.
    pub stall: Duration,
    /// The slowest a reader may take bytes, per second: a write of `n`
    /// bytes has `stall` plus `n / min_rate` to finish.
    pub min_rate: u32,
    /// Its longest life.
    pub max_age: Duration,
}

/// How a splice ended.
#[derive(Debug, PartialEq, Eq)]
pub enum End {
    /// Both directions closed.
    Done,
    /// No traffic for the idle time.
    Idle,
    /// Lived to its maximum age.
    MaxAge,
    /// A write made no progress for the stall time.
    Stalled,
    /// An end failed.
    Error(io::ErrorKind),
}

/// When bytes last moved, on tokio's clock (so tests can pause it).
struct Stamp {
    base: Instant,
    ms: AtomicU64,
}

impl Stamp {
    fn new() -> Self {
        Stamp {
            base: Instant::now(),
            ms: AtomicU64::new(0),
        }
    }
    fn touch(&self) {
        let ms = u64::try_from(self.base.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.ms.store(ms, Ordering::Relaxed);
    }
    fn quiet(&self) -> Duration {
        let now = u64::try_from(self.base.elapsed().as_millis()).unwrap_or(u64::MAX);
        Duration::from_millis(now.saturating_sub(self.ms.load(Ordering::Relaxed)))
    }
}

/// How long a write of `n` bytes may take: the grace plus the floor rate.
fn write_deadline(stall: Duration, min_rate: u32, n: usize) -> Duration {
    let secs = n as f64 / f64::from(min_rate.max(1));
    stall.saturating_add(Duration::from_secs_f64(secs.min(86_400.0)))
}

async fn pump<R, W>(
    from: &mut R,
    to: &mut W,
    buffer: usize,
    stall: Duration,
    min_rate: u32,
    last: &Stamp,
) -> Result<(), End>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; buffer.max(1)];
    loop {
        let n = from
            .read(&mut buf)
            .await
            .map_err(|e| End::Error(e.kind()))?;
        if n == 0 {
            // Pass the close on. A failure here only means the far end
            // has already gone.
            let _ = timeout(stall, to.shutdown()).await;
            return Ok(());
        }
        last.touch();
        match timeout(write_deadline(stall, min_rate, n), async {
            to.write_all(&buf[..n]).await?;
            to.flush().await
        })
        .await
        {
            Err(_) => return Err(End::Stalled),
            Ok(Err(e)) => return Err(End::Error(e.kind())),
            Ok(Ok(())) => last.touch(),
        }
    }
}

/// Splice `a` and `b` until both close or a limit is hit.
pub async fn splice<A, B>(a: A, b: B, limits: SpliceLimits) -> End
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let (mut ar, mut aw) = tokio::io::split(a);
    let (mut br, mut bw) = tokio::io::split(b);
    let last = Stamp::new();
    let both = async {
        // The first failure ends both; a clean close in one direction
        // lets the other carry on.
        match tokio::try_join!(
            pump(
                &mut ar,
                &mut bw,
                limits.buffer,
                limits.stall,
                limits.min_rate,
                &last
            ),
            pump(
                &mut br,
                &mut aw,
                limits.buffer,
                limits.stall,
                limits.min_rate,
                &last
            ),
        ) {
            Ok(((), ())) => End::Done,
            Err(e) => e,
        }
    };
    let idle = async {
        loop {
            let quiet = last.quiet();
            if quiet >= limits.idle {
                return End::Idle;
            }
            sleep(limits.idle - quiet).await;
        }
    };
    tokio::select! {
        end = both => end,
        end = idle => end,
        () = sleep(limits.max_age) => End::MaxAge,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> SpliceLimits {
        SpliceLimits {
            buffer: 1024,
            idle: Duration::from_secs(30),
            stall: Duration::from_secs(30),
            min_rate: 1024,
            max_age: Duration::from_secs(60),
        }
    }

    #[tokio::test]
    async fn bytes_cross_both_ways_and_closes_are_passed_on() {
        let (mut left, a) = tokio::io::duplex(256);
        let (mut right, b) = tokio::io::duplex(256);
        let task = tokio::spawn(splice(a, b, limits()));
        left.write_all(b"to the right").await.unwrap();
        left.shutdown().await.unwrap();
        let mut got = Vec::new();
        right.read_to_end(&mut got).await.unwrap();
        assert_eq!(got, b"to the right");
        right.write_all(b"and back").await.unwrap();
        right.shutdown().await.unwrap();
        let mut back = Vec::new();
        left.read_to_end(&mut back).await.unwrap();
        assert_eq!(back, b"and back");
        assert_eq!(task.await.unwrap(), End::Done);
    }

    #[tokio::test]
    async fn a_large_transfer_arrives_whole_in_small_buffers() {
        let (mut left, a) = tokio::io::duplex(512);
        let (mut right, b) = tokio::io::duplex(512);
        let task = tokio::spawn(splice(a, b, limits()));
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let send = data.clone();
        let writer = tokio::spawn(async move {
            left.write_all(&send).await.unwrap();
            left.shutdown().await.unwrap();
            left
        });
        let mut got = Vec::new();
        right.read_to_end(&mut got).await.unwrap();
        assert_eq!(got, data);
        right.shutdown().await.unwrap();
        let _left = writer.await.unwrap();
        assert_eq!(task.await.unwrap(), End::Done);
    }

    #[tokio::test(start_paused = true)]
    async fn a_peer_that_never_reads_stalls_the_splice_and_buffers_little() {
        let (mut left, a) = tokio::io::duplex(1024);
        let (_right, b) = tokio::io::duplex(1024); // never read
        let l = SpliceLimits {
            stall: Duration::from_secs(5),
            ..limits()
        };
        let task = tokio::spawn(splice(a, b, l));
        // Keep writing; the relay must stop reading once its buffer and
        // the pipes are full, so this write cannot complete in full.
        let flood = tokio::spawn(async move {
            let chunk = vec![1u8; 4096];
            let mut sent = 0usize;
            while left.write_all(&chunk).await.is_ok() {
                sent += chunk.len();
                if sent > 1_000_000 {
                    break;
                }
            }
            sent
        });
        assert_eq!(task.await.unwrap(), End::Stalled);
        let sent = flood.await.unwrap();
        assert!(sent < 20_000, "{sent} bytes were taken from a stalled pipe");
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_connection_is_closed_after_its_idle_time() {
        let (_left, a) = tokio::io::duplex(64);
        let (_right, b) = tokio::io::duplex(64);
        let l = SpliceLimits {
            idle: Duration::from_secs(10),
            ..limits()
        };
        assert_eq!(splice(a, b, l).await, End::Idle);
    }

    #[tokio::test(start_paused = true)]
    async fn a_busy_connection_still_ends_at_its_maximum_age() {
        let (mut left, a) = tokio::io::duplex(64);
        let (_right, b) = tokio::io::duplex(64);
        let l = SpliceLimits {
            idle: Duration::from_secs(10),
            max_age: Duration::from_secs(25),
            ..limits()
        };
        let task = tokio::spawn(splice(a, b, l));
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_secs(6)).await;
            let _ = left.write_all(b"x").await;
        }
        assert_eq!(task.await.unwrap(), End::MaxAge);
    }

    /// A reader that takes a little, often enough that no single wait is
    /// long, but under the floor rate: cut. The old rule (a stall timer on
    /// each write, here 30 s) would have let it hold the stream for hours.
    #[tokio::test(start_paused = true)]
    async fn a_reader_under_the_floor_rate_is_cut_and_one_at_the_floor_is_not() {
        for (per_tick, expect_cut) in [(32usize, true), (512usize, false)] {
            let (mut left, a) = tokio::io::duplex(1024);
            let (mut right, b) = tokio::io::duplex(1024);
            let l = SpliceLimits {
                buffer: 1024,
                // 2 s of grace and 1 KiB/s: a 1 KiB write has 3 s.
                stall: Duration::from_secs(2),
                min_rate: 1024,
                idle: Duration::from_secs(3600),
                max_age: Duration::from_secs(7200),
            };
            let task = tokio::spawn(splice(a, b, l));
            let feed = tokio::spawn(async move {
                let chunk = vec![5u8; 4096];
                let _ = left.write_all(&chunk).await;
                let _ = left.shutdown().await;
                left
            });
            // The reader takes `per_tick` bytes every 250 ms: 128 B/s or 2 KiB/s.
            let drain = tokio::spawn(async move {
                let mut got = 0usize;
                let mut buf = vec![0u8; per_tick];
                loop {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    match right.read(&mut buf).await {
                        Ok(0) | Err(_) => return got,
                        Ok(n) => got += n,
                    }
                }
            });
            let end = task.await.unwrap();
            let _ = feed.await;
            let got = drain.await.unwrap();
            if expect_cut {
                assert_eq!(end, End::Stalled, "128 B/s is under the 1 KiB/s floor");
                assert!(
                    got < 4096,
                    "{got} bytes got through a reader under the floor"
                );
            } else {
                assert_ne!(end, End::Stalled, "2 KiB/s keeps the pace");
                assert_eq!(got, 4096);
            }
        }
    }
}
