//! Reading what a child process says, without waiting on a pipe it doesn't
//! hold (P11c, found on Windows in the open/close hammer).
//!
//! ```text
//! GOAL: the output of a child that has ended is in hand once everything
//!       it wrote has been read, however long some other process keeps the
//!       other end of its pipe open.
//! SOURCE: the standard library's `Command` (Windows: `CreateProcessW` with
//!         `bInheritHandles`, the child's end of each pipe inheritable from
//!         the moment it is made until the parent closes it, so a process
//!         started by another thread in that moment holds a copy and the
//!         pipe has no end-of-file until that process ends too); found by
//!         the open/close hammer on Windows (plan/p15-app-and-relay.md
//!         3.1, D-77).
//! INVARIANTS:
//!   1. Nothing a child wrote is lost: bytes already in the pipe when the
//!      child ended are read before the reader is let go (the pipe is asked
//!      what it holds; the reader is let go only when it is blocked in its
//!      read with nothing in the pipe, for QUIET together).
//!   2. The reader is let go when the pipe reaches its end (every writer
//!      gone), or when it is idle as in 1, or after `limit`, whichever is
//!      first; which one is reported ([`Closed`]).
//!   3. A reader let go while still blocked ends by itself when the last
//!      holder of the pipe's other end lets go; it holds nothing else.
//! ```

use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a reader must be seen blocked in its read, with nothing in the
/// pipe, before it is let go (INVARIANT 1): longer than any thread waits to
/// run after it has been handed a byte.
pub(crate) const QUIET: Duration = Duration::from_millis(100);

/// How long a reader in its read, with no read returning, is waited for when
/// the pipe can't say whether it holds anything.
const BLIND: Duration = Duration::from_millis(1500);

/// How the wait for a pipe ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Closed {
    /// The pipe reached its end: every writer was gone.
    Eof,
    /// Something else still held the pipe's other end, and nothing was left
    /// in it to read.
    Idle,
    /// The limit passed first; what had been read is what is returned.
    Limit,
}

impl Closed {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Closed::Eof => "eof",
            Closed::Idle => "held-open",
            Closed::Limit => "limit",
        }
    }
}

/// A pipe being read on its own thread.
pub(crate) struct Drain {
    bytes: Arc<Mutex<Vec<u8>>>,
    /// True while the pipe is open (its reader hasn't ended): guards the
    /// raw descriptor or handle asked about below.
    open: Arc<Mutex<bool>>,
    /// True from just before the reader calls `read` until it returns.
    reading: Arc<AtomicBool>,
    /// How many reads have returned.
    reads: Arc<AtomicU64>,
    raw: usize,
    cap: usize,
}

/// The raw descriptor (Unix) or handle (Windows) of a pipe, as a number.
pub(crate) trait Raw {
    fn raw(&self) -> usize;
}

#[cfg(unix)]
impl<T: std::os::unix::io::AsRawFd> Raw for T {
    fn raw(&self) -> usize {
        self.as_raw_fd() as usize
    }
}

#[cfg(windows)]
impl<T: std::os::windows::io::AsRawHandle> Raw for T {
    fn raw(&self) -> usize {
        self.as_raw_handle() as usize
    }
}

/// How many bytes `pipe` holds now, if that can be asked: `None` when it
/// can't (the pipe is at its end, or the question failed), which is never
/// taken as "nothing".
fn pending(raw: usize) -> Option<usize> {
    #[cfg(unix)]
    {
        let mut fd = libc::pollfd {
            fd: raw as libc::c_int,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd, and no wait.
        let ready = unsafe { libc::poll(&mut fd, 1, 0) };
        // Anything else is readable, or hung up (end of file), or failed:
        // never "nothing".
        if ready == 0 { Some(0) } else { None }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Pipes::PeekNamedPipe;
        let mut available: u32 = 0;
        // SAFETY: `raw` is a pipe handle that stays open while `open` is
        // held (the reader closes it only under that lock); no buffer is
        // asked for, only the count.
        let ok = unsafe {
            PeekNamedPipe(
                raw as *mut core::ffi::c_void,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            None
        } else {
            Some(available as usize)
        }
    }
}

impl Drain {
    /// Start reading `pipe` (if any) on a thread of its own, keeping the
    /// last `cap` bytes; `heard` is set to the milliseconds since `started`
    /// whenever a byte arrives.
    pub(crate) fn start<P>(
        pipe: Option<P>,
        cap: usize,
        started: Instant,
        heard: Arc<AtomicU64>,
    ) -> Drain
    where
        P: Read + Raw + Send + 'static,
    {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let open = Arc::new(Mutex::new(pipe.is_some()));
        let reading = Arc::new(AtomicBool::new(false));
        let reads = Arc::new(AtomicU64::new(0));
        let raw = pipe.as_ref().map_or(0, Raw::raw);
        let (b, o, r, n_reads) = (bytes.clone(), open.clone(), reading.clone(), reads.clone());
        if let Some(mut p) = pipe {
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    r.store(true, Ordering::SeqCst);
                    let got = p.read(&mut buf);
                    r.store(false, Ordering::SeqCst);
                    n_reads.fetch_add(1, Ordering::SeqCst);
                    match got {
                        Ok(0) => break,
                        Ok(n) => {
                            heard.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
                            // Bounded: a git that says this much is kept
                            // only in part, its last part (where its
                            // errors are).
                            let mut bytes = b.lock().unwrap_or_else(|e| e.into_inner());
                            bytes.extend_from_slice(&buf[..n]);
                            if bytes.len() > cap.saturating_mul(2) {
                                let cut = bytes.len() - cap;
                                bytes.drain(..cut);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
                // Closed under the lock the question above is asked under,
                // so a descriptor that is gone is never asked about.
                let mut open = o.lock().unwrap_or_else(|e| e.into_inner());
                *open = false;
                drop(p);
            });
        }
        Drain {
            bytes,
            open,
            reading,
            reads,
            raw,
            cap,
        }
    }

    /// Whether the pipe has reached its end.
    fn ended(&self) -> bool {
        !*self.open.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether the reader is blocked in its read with nothing in the pipe;
    /// when the pipe can't say what it holds, whether the reader is in its
    /// read (`None` is returned for the caller to weigh against time).
    fn idle(&self) -> Option<bool> {
        let open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if !*open || !self.reading.load(Ordering::SeqCst) {
            return Some(false);
        }
        pending(self.raw).map(|n| n == 0)
    }

    /// What has been read, in whole (the last `cap` bytes of it).
    fn taken(&self) -> Vec<u8> {
        let mut bytes = self.bytes.lock().unwrap_or_else(|e| e.into_inner());
        if bytes.len() > self.cap {
            let cut = bytes.len() - self.cap;
            bytes.drain(..cut);
        }
        std::mem::take(&mut *bytes)
    }
}

/// Wait, until `limit` after `since`, for every one of `drains` to reach
/// its end or to be idle (INVARIANT 1 and 2), then take what each read: the
/// bytes, and how its wait ended.
pub(crate) fn finish<const N: usize>(
    drains: [&Drain; N],
    since: Instant,
    limit: Duration,
) -> [(Vec<u8>, Closed); N] {
    // When each reader was first seen blocked in its read (with nothing in
    // the pipe, or the pipe unable to say), and the reads it had made then.
    let mut idle_since: [Option<Instant>; N] = [None; N];
    let mut blind_since: [Option<(Instant, u64)>; N] = [None; N];
    let mut how: [Option<Closed>; N] = [None; N];
    loop {
        let now = Instant::now();
        for (i, d) in drains.iter().enumerate() {
            if how[i].is_some() {
                continue;
            }
            if d.ended() {
                how[i] = Some(Closed::Eof);
            } else {
                match d.idle() {
                    Some(true) => {
                        let since = *idle_since[i].get_or_insert(now);
                        if now.duration_since(since) >= QUIET {
                            how[i] = Some(Closed::Idle);
                        }
                    }
                    Some(false) => {
                        idle_since[i] = None;
                        blind_since[i] = None;
                    }
                    // The pipe can't say what it holds: the reader in its
                    // read, with no read returning for BLIND, is idle (a
                    // reader is never kept from a byte for that long).
                    None => {
                        let reads = d.reads.load(Ordering::SeqCst);
                        let (since, was) = *blind_since[i].get_or_insert((now, reads));
                        if was != reads {
                            blind_since[i] = Some((now, reads));
                        } else if now.duration_since(since) >= BLIND {
                            how[i] = Some(Closed::Idle);
                        }
                    }
                }
            }
        }
        if how.iter().all(Option::is_some) {
            break;
        }
        if now.duration_since(since) >= limit {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    // An end reached in the instant before this is read: its last bytes
    // are in, the reader having stored them before it ended.
    std::array::from_fn(|i| (drains[i].taken(), how[i].unwrap_or(Closed::Limit)))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn drained(child: &mut std::process::Child) -> (Drain, Drain) {
        let heard = Arc::new(AtomicU64::new(0));
        let now = Instant::now();
        (
            Drain::start(child.stdout.take(), usize::MAX, now, heard.clone()),
            Drain::start(child.stderr.take(), 1 << 20, now, heard),
        )
    }

    /// INVARIANT 1 and 2. A child that ends, leaving a grandchild that
    /// still holds its pipes (the Windows leak, made by hand), has its
    /// whole output read, and the wait ends in far less than the limit.
    #[test]
    fn a_pipe_another_process_still_holds_is_read_in_whole_without_waiting_for_it() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("echo out-one; echo err-one >&2; (sleep 5 </dev/null) & echo out-two")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let status = child.wait().unwrap();
        assert!(status.success());
        let (out, err) = drained(&mut child);
        let began = Instant::now();
        let got = finish([&out, &err], began, Duration::from_secs(4));
        let took = began.elapsed();
        assert_eq!(String::from_utf8_lossy(&got[0].0), "out-one\nout-two\n");
        assert_eq!(String::from_utf8_lossy(&got[1].0), "err-one\n");
        assert_eq!(got[0].1, Closed::Idle, "held open, nothing left to read");
        assert_eq!(got[1].1, Closed::Idle);
        assert!(took < Duration::from_secs(2), "took {took:?}");
    }

    /// INVARIANT 2. With nobody else holding the pipe, the end is the
    /// pipe's own, at once.
    #[test]
    fn a_pipe_nobody_holds_ends_at_its_end() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("echo said")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.wait().unwrap();
        let (out, err) = drained(&mut child);
        let began = Instant::now();
        let got = finish([&out, &err], began, Duration::from_secs(4));
        assert_eq!(String::from_utf8_lossy(&got[0].0), "said\n");
        assert_eq!((got[0].1, got[1].1), (Closed::Eof, Closed::Eof));
        assert!(began.elapsed() < QUIET * 5, "took {:?}", began.elapsed());
    }

    /// INVARIANT 1. A lot of output, written and the writer gone before the
    /// reader is looked at, is read in whole.
    #[test]
    fn a_lot_of_output_is_read_in_whole() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("i=0; while [ $i -lt 20000 ]; do echo line-$i; i=$((i+1)); done; (sleep 3 </dev/null) &")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let heard = Arc::new(AtomicU64::new(0));
        let out = Drain::start(child.stdout.take(), usize::MAX, Instant::now(), heard);
        child.wait().unwrap();
        let got = finish([&out], Instant::now(), Duration::from_secs(4));
        let text = String::from_utf8_lossy(&got[0].0).to_string();
        assert_eq!(text.lines().count(), 20000);
        assert_eq!(text.lines().last(), Some("line-19999"));
        assert_eq!(got[0].1, Closed::Idle);
    }
}
