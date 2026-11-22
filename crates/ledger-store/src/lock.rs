//! File locks, behind one seam (P11c, D-62).
//!
//! ```text
//! GOAL: "some process holds this file" is asked and answered the same
//!       way on every system.
//! SOURCE: the standard library's `File::try_lock` and `try_lock_shared`
//!         (flock(2) on Unix, LockFileEx on Windows), plan/served.md 3.9
//!         ("the viewer's presence is a shared lock").
//! INVARIANTS:
//!   1. Only the standard library's locks are used: no second copy of an
//!      `unsafe` flock or LockFileEx call lives anywhere else.
//!   2. A lock is let go when its file is closed, by this process ending
//!      or not (the system does it; nothing here keeps a record).
//!   3. A question that cannot be answered is answered "not held".
//!   4. The writer lock of a store is let go when the store is dropped, for
//!      every process that could still hold it: a child forked by another
//!      thread of this process holds a copy of every open descriptor (this
//!      lock's too) from its fork until its exec, and a `flock` lasts as
//!      long as any copy does. Every child a process that holds a store starts
//!      is started by [`spawn`] (the store itself starts none since backups
//!      became plain files, D-91; the seam stays for any that does), and dropping a [`WriterLock`] returns only once every
//!      [`spawn`] begun before it has ended (its child has exec'd and the
//!      copies are closed), and then the lock is taken afresh and let go
//!      (it is free, or another writer has it, or 500 ms passed: the system
//!      lets go of a closed file's last copy a moment after the exec), so
//!      after a close is answered another open, here or elsewhere, finds
//!      the lock free.
//! ```

use std::fs::{File, TryLockError};
use std::process::{Child, Command};
use std::sync::RwLock;

/// Held (shared on Unix, exclusive on Windows) by every [`spawn`] while it
/// runs, and taken (exclusively, for an instant) by every dropped
/// [`WriterLock`].
static SPAWNING: RwLock<()> = RwLock::new(());

/// Start `command`: returns once the child has exec'd (or failed to), so
/// from then on it holds no copy of any descriptor of this process.
///
/// On Windows one spawn runs at a time (INVARIANT 5): the standard library
/// makes the child's end of each pipe inheritable when it makes the pipe,
/// and starts the child with every inheritable handle of the process, so a
/// process started by another thread in that moment holds a copy of it too.
/// A pipe whose other end is so held has no end-of-file when its own child
/// ends, until the process that took the copy ends.
pub fn spawn(command: &mut Command) -> std::io::Result<Child> {
    #[cfg(windows)]
    let _spawning = SPAWNING.write().unwrap_or_else(|e| e.into_inner());
    #[cfg(not(windows))]
    let _spawning = SPAWNING.read().unwrap_or_else(|e| e.into_inner());
    command.spawn()
}

/// A store's exclusive lock on its books, released when this is dropped
/// (INVARIANT 4): the file is closed; every child still between its fork
/// and its exec, which holds a copy of it, is waited for; and the lock is
/// then taken afresh and let go, so it is known to be free (the system
/// lets go of a file's last copy a moment after the exec that closed it).
#[derive(Debug)]
pub(crate) struct WriterLock {
    file: Option<File>,
    path: std::path::PathBuf,
}

/// How long a dropped [`WriterLock`] waits for the system to let go of the
/// last copy of its file.
const RELEASE_WITHIN: std::time::Duration = std::time::Duration::from_millis(500);

impl WriterLock {
    pub(crate) fn new(file: File, path: std::path::PathBuf) -> Self {
        Self {
            file: Some(file),
            path,
        }
    }
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        drop(self.file.take());
        drop(SPAWNING.write().unwrap_or_else(|e| e.into_inner()));
        // Free, or taken by another writer already (then it is not ours to
        // wait for): looked at until it is one or the other.
        let until = std::time::Instant::now() + RELEASE_WITHIN;
        while let Ok(file) = std::fs::OpenOptions::new().write(true).open(&self.path) {
            match file.try_lock() {
                Err(TryLockError::WouldBlock) if std::time::Instant::now() < until => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Ok(()) => {
                    let _ = file.unlock();
                    return;
                }
                Err(_) => return,
            }
        }
    }
}

/// Take a shared lock on `file` without waiting: whether it is now held.
pub fn try_shared(file: &File) -> bool {
    file.try_lock_shared().is_ok()
}

/// Whether another holder keeps a shared lock on `file` (so that an
/// exclusive one can't be taken now). A lock taken to find out is let go
/// at once.
pub fn held_elsewhere(file: &File) -> bool {
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(_)) => false,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;

    /// A command whose child lingers between its fork and its exec, as one
    /// does on a slow machine, holding a copy of every descriptor; it says
    /// so on `said` (a pipe's write end) as it begins to linger.
    fn lingering(said: libc::c_int) -> Command {
        let mut c = Command::new("true");
        // SAFETY: only `write` and `nanosleep`, both async-signal-safe, run
        // in the child between fork and exec.
        unsafe {
            c.pre_exec(move || {
                libc::write(said, b"x".as_ptr().cast(), 1);
                let t = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 300_000_000,
                };
                libc::nanosleep(&t, std::ptr::null_mut());
                Ok(())
            });
        }
        c
    }

    fn take(path: &std::path::Path) -> Option<File> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .unwrap();
        file.try_lock().ok().map(|()| file)
    }

    /// Start a lingering child (through [`spawn`] or not) and return once
    /// it is between its fork and its exec.
    fn linger(gated: bool) -> std::thread::JoinHandle<()> {
        let mut ends = [0 as libc::c_int; 2];
        // SAFETY: `ends` has room for the two descriptors.
        assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
        let [read, write] = ends;
        let started = std::thread::spawn(move || {
            let mut c = lingering(write);
            let mut child = if gated {
                spawn(&mut c).unwrap()
            } else {
                c.spawn().unwrap()
            };
            child.wait().unwrap();
        });
        let mut byte = 0u8;
        // SAFETY: reads one byte into `byte` from the pipe made above.
        assert_eq!(
            unsafe { libc::read(read, (&mut byte as *mut u8).cast(), 1) },
            1
        );
        // SAFETY: both descriptors are this function's own.
        unsafe {
            libc::close(read);
            libc::close(write);
        }
        started
    }

    /// INVARIANT 4. The control shows the cause (a forked child keeps the
    /// `flock` after the file is closed: an open right after a close was
    /// refused as "another program has these books open"); the second half
    /// shows `spawn` and `WriterLock` close it.
    #[test]
    fn a_writer_lock_is_free_once_dropped_even_with_a_child_between_fork_and_exec() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wanderbooks.lock");

        // Control: a plain file closed under an ungated spawn stays locked.
        let held = take(&path).unwrap();
        let child = linger(false);
        drop(held);
        assert!(
            take(&path).is_none(),
            "the child between fork and exec should still hold the lock"
        );
        child.join().unwrap();
        assert!(
            take(&path).is_some(),
            "the lock is free once the child ends"
        );

        // Gated: the drop returns only when the lock really is free.
        let held = WriterLock::new(take(&path).unwrap(), path.clone());
        let child = linger(true);
        drop(held);
        assert!(
            take(&path).is_some(),
            "a dropped writer lock must be free at once"
        );
        child.join().unwrap();
    }
}
