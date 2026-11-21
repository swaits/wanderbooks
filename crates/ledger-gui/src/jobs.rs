//! One owner for every background thread a tab starts.
//!
//! ```text
//! GOAL: a tab's threads (its loader, its receipts' workers, an export, an
//!       owner's job, the dock's start and its agent's reaper) have one
//!       owner that can tell them all to stop when the tab closes, and say
//!       when every one of them has ended; and a job that must not be cut
//!       off (an owner's change to the books) holds the tab and the window
//!       open until it is done.
//! SOURCE: plan/p15-app-and-relay.md 3.2 (D-78: a tab per open company,
//!         and what the tab started ends with it); CONVENTION (a change
//!         an owner has begun on the books is never cut off half-way: the
//!         tab and the window wait for it).
//! INVARIANTS:
//!   1. Every thread started through a [`Spawner`] is counted from before
//!      it starts until it has ended, however it ends (a panic too): the
//!      count is its [`Ticket`], dropped last.
//!   2. [`Jobs`] is dropped (or [`Jobs::stop`]ped) with its tab: every
//!      [`Stop`] it handed out then says so, for good. A thread checks it
//!      at its phase boundaries and between files, and stops there.
//!   3. A [`Watch`] outlives the tab and says, on a condition and not by
//!      looking now and then, when no thread of the tab is left
//!      ([`Watch::wait_ended`]).
//!   4. A thread started with [`Hold::Keep`] is one the tab's close
//!      control and the window's close are held for: [`Jobs::holding`]
//!      says what is running, in the words the close control shows.
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Whether a running thread may be cut off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hold {
    /// It may: it is told to stop when its tab closes.
    Free,
    /// It may not: the tab and the window stay open until it ends. The
    /// words say what is running ("Restoring these books: wait for it
    /// to finish").
    Keep(String),
}

struct Live {
    id: u64,
    name: String,
    hold: Hold,
}

#[derive(Default)]
struct Table {
    next: u64,
    live: Vec<Live>,
}

struct Shared {
    stop: AtomicBool,
    table: Mutex<Table>,
    ended: Condvar,
}

impl Shared {
    fn new() -> Arc<Shared> {
        Arc::new(Shared {
            stop: AtomicBool::new(false),
            table: Mutex::new(Table::default()),
            ended: Condvar::new(),
        })
    }

    fn table(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A tab's threads, and the means to stop them. Dropping it tells every
/// thread to stop (INVARIANT 2).
pub struct Jobs(Spawner);

/// A handle for starting threads in a tab's [`Jobs`] (cloneable; it does
/// not stop anything when dropped).
#[derive(Clone)]
pub struct Spawner(Arc<Shared>);

/// Tells a thread to stop (INVARIANT 2).
#[derive(Clone)]
pub struct Stop(Arc<Shared>, Option<Arc<std::sync::atomic::AtomicUsize>>);

/// The count of one thread: held from before it starts to after it ends.
pub struct Ticket {
    shared: Arc<Shared>,
    id: u64,
}

/// Says when a tab's threads have ended, after the tab is gone.
#[derive(Clone)]
pub struct Watch(Arc<Shared>);

impl Default for Jobs {
    fn default() -> Self {
        Self::new()
    }
}

impl Jobs {
    pub fn new() -> Jobs {
        Jobs(Spawner(Shared::new()))
    }

    /// A handle to start threads with.
    pub fn spawner(&self) -> Spawner {
        self.0.clone()
    }

    /// Tell every thread to stop, now and for good.
    pub fn stop(&self) {
        self.0.0.stop.store(true, Ordering::SeqCst);
    }

    /// A handle that outlives the tab and says when its threads ended.
    pub fn watch(&self) -> Watch {
        Watch(self.0.0.clone())
    }

    /// The names of the threads running now.
    pub fn running(&self) -> Vec<String> {
        self.watch().running()
    }

    /// What the tab is held open for, if a [`Hold::Keep`] thread runs
    /// (INVARIANT 4): its words.
    pub fn holding(&self) -> Option<String> {
        self.0.holding()
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Default for Spawner {
    /// A handle to a registry of its own that nothing owns: nothing ever
    /// tells its threads to stop (a menu made outside a tab, in tests).
    fn default() -> Self {
        Spawner(Shared::new())
    }
}

impl Spawner {
    /// The stop signal of this registry.
    pub fn stop_signal(&self) -> Stop {
        Stop(self.0.clone(), None)
    }

    /// Count a thread to be started; the count ends when the ticket is
    /// dropped.
    pub fn ticket(&self, name: &str, hold: Hold) -> Ticket {
        let mut table = self.0.table();
        let id = table.next;
        table.next += 1;
        table.live.push(Live {
            id,
            name: name.to_string(),
            hold,
        });
        Ticket {
            shared: self.0.clone(),
            id,
        }
    }

    /// Run `work` on a thread of its own, counted (INVARIANT 1). It is
    /// handed the stop signal to check at its phase boundaries. False when
    /// the system refused the thread (nothing runs, nothing is counted).
    pub fn spawn(&self, name: &str, hold: Hold, work: impl FnOnce(&Stop) + Send + 'static) -> bool {
        let ticket = self.ticket(name, hold);
        let stop = self.stop_signal();
        std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                // Dropped last, whatever happens in `work`.
                let _ticket = ticket;
                work(&stop);
            })
            .is_ok()
    }

    /// What is held open for, if a [`Hold::Keep`] thread runs.
    pub fn holding(&self) -> Option<String> {
        self.0.table().live.iter().find_map(|l| match &l.hold {
            Hold::Keep(words) => Some(words.clone()),
            Hold::Free => None,
        })
    }
}

impl Stop {
    /// Whether the tab asked for its threads to stop.
    pub fn requested(&self) -> bool {
        if self.0.stop.load(Ordering::SeqCst) {
            return true;
        }
        // A signal that trips after so many looks (tests).
        self.1.as_ref().is_some_and(|left| {
            left.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_err()
        })
    }

    /// A signal that is not given until it has been looked at `looks`
    /// times, and is given at the next (tests: a job's every phase
    /// boundary, one after another).
    pub fn given_after(looks: usize) -> Stop {
        Stop(
            Shared::new(),
            Some(Arc::new(std::sync::atomic::AtomicUsize::new(looks))),
        )
    }

    /// A signal already given (tests of what a thread does once told to
    /// stop).
    pub fn given() -> Stop {
        let shared = Shared::new();
        shared.stop.store(true, Ordering::SeqCst);
        Stop(shared, None)
    }

    /// A signal never given (what runs outside a tab).
    pub fn never() -> Stop {
        Stop(Shared::new(), None)
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut table = self.shared.table();
        table.live.retain(|l| l.id != self.id);
        drop(table);
        self.shared.ended.notify_all();
    }
}

impl Watch {
    /// The names of the threads running now.
    pub fn running(&self) -> Vec<String> {
        self.0.table().live.iter().map(|l| l.name.clone()).collect()
    }

    /// Whether every thread has ended.
    pub fn ended(&self) -> bool {
        self.0.table().live.is_empty()
    }

    /// Wait until every thread has ended, for at most `limit` (a failure
    /// guard, never a stand-in for the other side getting there): whether
    /// they have.
    pub fn wait_ended(&self, limit: Duration) -> bool {
        let end = Instant::now() + limit;
        let mut table = self.0.table();
        while !table.live.is_empty() {
            let Some(left) = end.checked_duration_since(Instant::now()) else {
                return false;
            };
            table = self
                .0
                .ended
                .wait_timeout(table, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_is_counted_until_it_has_ended_and_stop_reaches_it() {
        let jobs = Jobs::new();
        let watch = jobs.watch();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let (seen_tx, seen_rx) = std::sync::mpsc::channel::<bool>();
        assert!(jobs.spawner().spawn("w", Hold::Free, move |stop| {
            let _ = go_rx.recv();
            let _ = seen_tx.send(stop.requested());
        }));
        assert_eq!(watch.running(), ["w"]);
        assert!(!watch.ended());
        drop(jobs);
        go_tx.send(()).unwrap();
        assert!(seen_rx.recv().unwrap(), "dropping the jobs says stop");
        assert!(watch.wait_ended(Duration::from_secs(30)));
        assert!(watch.running().is_empty());
    }

    #[test]
    fn a_kept_thread_is_what_the_tab_is_held_for() {
        let jobs = Jobs::new();
        let free = jobs.spawner().ticket("free", Hold::Free);
        assert_eq!(jobs.holding(), None);
        let keep = jobs
            .spawner()
            .ticket("restore", Hold::Keep("Restoring".into()));
        assert_eq!(jobs.holding().as_deref(), Some("Restoring"));
        drop(keep);
        assert_eq!(jobs.holding(), None);
        drop(free);
        assert!(jobs.watch().ended());
    }

    #[test]
    fn a_panic_still_ends_the_count() {
        let jobs = Jobs::new();
        let watch = jobs.watch();
        jobs.spawner()
            .spawn("boom", Hold::Free, |_| panic!("on purpose"));
        assert!(watch.wait_ended(Duration::from_secs(30)));
    }
}
