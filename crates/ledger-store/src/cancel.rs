//! A stop request the long loops over a book's journal and files look at
//! (a backup's drill or proof, ledger-store `backup`): a step that has to
//! end when the books are let go (a close) can't wait for a replay of a
//! journal of any length to come to its end.
//!
//! ```text
//! GOAL: a verification over a book's journal or files that was asked to
//!       stop stops promptly, at its next step, and leaves nothing of its
//!       own half-made, however large the book.
//! SOURCE: the books server's close (ledger-mcp local.rs INVARIANT 10 and
//!         13: a close is answered only when nothing of its backups still
//!         runs, and is bounded).
//! INVARIANTS:
//!   1. A request is scoped to the thread that made it (`scope`), for as
//!      long as the scope lives; code on other threads, and code outside a
//!      scope, never sees one. Nothing is shared, so no other work of the
//!      process can be stopped by it.
//!   2. `check` is looked at once per journal line (verifying, replaying,
//!      applying to the projection), once per stored file hashed, and at
//!      the boundaries between a verification's phases: the longest it
//!      goes without looking is one line or one file.
//!   3. A stop is an error (`StoreError::Cancelled`), never a partial
//!      success: what a stopped verification had made is the caller's to
//!      remove (the books it was opened on are a scratch folder's).
//! ```

use crate::error::StoreError;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    static FLAG: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// A stop request in force on this thread, until dropped.
pub struct Scope(Option<Arc<AtomicBool>>);

impl Drop for Scope {
    fn drop(&mut self) {
        FLAG.with(|f| *f.borrow_mut() = self.0.take());
    }
}

/// Look for `flag` (INVARIANT 1) on this thread, until the returned scope
/// is dropped (the request in force before it comes back).
#[must_use]
pub fn scope(flag: Arc<AtomicBool>) -> Scope {
    Scope(FLAG.with(|f| f.borrow_mut().replace(flag)))
}

/// Whether a stop was asked for.
pub fn requested() -> bool {
    FLAG.with(|f| {
        f.borrow()
            .as_ref()
            .is_some_and(|c| c.load(Ordering::SeqCst))
    })
}

/// Ok, or `StoreError::Cancelled` when a stop was asked for (INVARIANT 2).
pub fn check() -> Result<(), StoreError> {
    if requested() {
        Err(StoreError::Cancelled)
    } else {
        Ok(())
    }
}
