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
//! ```

use std::fs::{File, TryLockError};

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
