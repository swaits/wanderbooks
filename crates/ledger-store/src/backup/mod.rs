//! Backups in plain folders (plan/p15-app-and-relay.md section 9, D-91,
//! D-92): a backup is a folder the person picks, holding one folder per
//! set of books, made only of files that are written once.
//!
//! ```text
//! GOAL: a book's every write survives the loss of this computer, a
//!       backup is proven whole before it is trusted or restored, and the
//!       person needs nothing installed and nothing explained to have one.
//! SOURCE: plan/p15-app-and-relay.md section 9 (the form, D-91: git leaves
//!         backups entirely; D-92: a passphrase, off by default), plan 3.7
//!         (what a backup holds, when it runs, the proof after it, the
//!         weekly drill), 3.9 and D-39 (a proven local backup with the
//!         keys before a format upgrade), WB-backup-1.
//! INVARIANTS:
//!   1. Nothing in a destination is ever deleted or rewritten. Every file
//!      is put once (io.rs): a piece under a name made from what is in it,
//!      a manifest under its number, taken exclusively. The one thing this
//!      code removes from a destination is the scratch file it made itself
//!      a moment before. So a destination only grows, a syncing service
//!      never sees a file change under it, and an earlier state of the
//!      books is always still there.
//!   2. A backup is whole when its manifest is in place, and not before.
//!      The manifest is written last, after every piece it names is there;
//!      a run that dies leaves pieces no manifest names, which are
//!      harmless, and at most a short manifest, which no reader accepts
//!      (it carries its own check) and whose number is spent.
//!   3. A manifest names every piece it needs, by where it is, how long it
//!      is and what is in it (SHA-256), either itself or by naming the
//!      manifest before it (by number and by what its body hashes to) and
//!      only what is new, a chain of at most `DELTA_CHAIN` links; a reader
//!      proves the chain link by link. A reader tries the manifests newest
//!      first and takes the first whose pieces are all there at the length
//!      it names and that proves (`verify_found`), reading no more: the
//!      journal's chain replays to the head the manifest records, the books
//!      open, every stored file re-hashes, every vault record the books name
//!      and the key that opens them are there. A piece is accepted under
//!      any generation of it whose contents hash right.
//!   4. A backup is written only onto a destination whose newest whole
//!      manifest is a point on the live journal (same line, same hash,
//!      same place in the file). Otherwise it is said, and nothing is
//!      written: the live journal is the beginning of a longer one
//!      (`Ahead`: the books are behind the folder, and a new set may be
//!      begun beside the old at the person's request), or the books are
//!      others or have diverged (`Elsewhere`).
//!   5. Two programs, or two computers, writing one destination cannot
//!      corrupt it: pieces are named by what is in them, so two writers of
//!      one piece write the same bytes; the only decision is which of them
//!      takes the next manifest number, and the disk decides it
//!      (exclusive create). The loser looks again and goes on from what the
//!      winner wrote, or finds there is nothing left to write.
//!   6. A manifest that is lost, or replaced by a sync service's conflict
//!      copy, makes unreadable only the manifests chained on it (at most
//!      the rest of its chain of 64); every earlier one, and every later
//!      one that names everything, stands.
//!   7. With a passphrase every piece, every manifest, is sealed
//!      (seal.rs); names say no more than counts and sizes; the key is
//!      derived by a memory-hard function; this computer keeps it in the
//!      book's `local/` folder, owner-only, never in a backup. A wrong
//!      passphrase opens nothing, and is told from damage.
//!   8. A step that has to end when the books are let go looks for the
//!      stop between pieces, and once per chunk of a file read; a stopped
//!      backup leaves a destination that proves whole at its previous
//!      manifest.
//!   9. Nothing here names a path of the data root: errors speak of "the
//!      backup folder".
//!  10. A backup is proven before it is recorded good (write.rs
//!      `prove_and_repair`, read.rs `prove_written`): the manifest just put
//!      is read back, every journal and access segment re-hashed, the chain
//!      replayed to its head, the vault and keys present, and the stored
//!      files this backup added and one sixteenth of the rest re-hashed (a
//!      share that comes round within 16 backups); what is found damaged is
//!      written again under a new generation beside it and proven again.
//!      A manifest whose proof was cut short is marked unproven (no mark in
//!      `proofs/`) and proven by the next backup.
//!  11. A drill passes only on the newest manifest, and only if it reaches
//!      the last head recorded good (`drill`).
//! ```

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod cut;
mod form;
mod io;
mod read;
mod seal;
mod single;
#[cfg(test)]
mod tests;
mod write;

pub use cut::Cut;
pub use read::list;
pub use read::{Candidate, Found, Verified, drill, find, prove_local, verify_found};
pub use seal::{BackupKey, key_derivations};
pub use single::{FILE_EXTENSION, save_file};
pub use write::{
    Destination, Options, Written, holds_only_sealed, set_name_for, write_destination,
    write_destination_with, write_local,
};

/// What a backup holds, of a book's folder.
pub const BACKED_UP: [&str; 5] = [
    crate::JOURNAL_FILE,
    "blobs",
    crate::root::VAULT_DIR,
    crate::root::KEYS_DIR,
    crate::access::ACCESS_FILE,
];

/// A book's own local backup, in its folder: the same form as a
/// destination, written under the store's lock before a format upgrade.
pub const LOCAL_DIR: &str = "local-backup";

/// Why a backup couldn't be made, proven or restored. Never names a path
/// of the data root (INVARIANT 9).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackupError {
    /// A folder that isn't there (an external disk not connected).
    #[error("the backup folder isn't there (is its disk connected?)")]
    NoFolder,
    /// A folder that took these books' backups but holds none now (a disk
    /// not mounted on its mount point, or a new disk).
    #[error(
        "the backup folder holds no backup of these books: connect the disk that does; if it is a new disk, back up now saying so (new_disk)"
    )]
    NoBackupThere,
    /// The destination holds other books, or another history of these.
    #[error(
        "the backup folder holds a backup of other books, or of a different history of these books (they have diverged): nothing was written there"
    )]
    Elsewhere,
    /// The destination holds a longer history of these same books.
    #[error(
        "the backup folder holds a longer history of these books (through event {0}) than this computer has (event {1}), as after restoring from an older backup: nothing was written there. If these books are the ones to carry on with, start a new backup set beside the old one: the old one is never touched"
    )]
    Ahead(u64, u64),
    /// Another program kept taking the next place in the destination.
    #[error("another program kept writing to the backup folder: try again in a moment")]
    Busy,
    /// A backup that isn't whole.
    #[error("that backup can't be trusted: {0}")]
    Damaged(String),
    /// The backup is protected by a passphrase and none was given.
    #[error("that backup is protected by a passphrase: it needs the passphrase it was made with")]
    NeedsPassphrase,
    /// The backups were sealed with a passphrase and this computer has
    /// lost its key.
    #[error(
        "these backups are protected by a passphrase, but this computer no longer has its key: nothing was written. Enter the passphrase in the backup settings to carry on, or choose remove the passphrase there to back up without one"
    )]
    KeyMissing,
    /// A file of the backup folder is online only: its sync service has
    /// not put it on this computer.
    #[error(
        "some files in the backup folder are online only and not downloaded to this computer yet: open the folder in your sync app, keep it available offline, wait until it has finished, and try again"
    )]
    NotDownloaded,
    /// The newest backup does not prove whole; an older one does.
    #[error("{}", newest_damaged_words(*.number, .older_from.as_deref(), .newest_from.as_deref()))]
    NewestDamaged {
        /// The newest manifest's number.
        number: u64,
        /// When the newest was made, if it can be read.
        newest_from: Option<String>,
        /// When the newest one that proves whole was made (None: none does).
        older_from: Option<String>,
    },
    /// The passphrase given opens nothing.
    #[error("that isn't the passphrase this backup was made with")]
    WrongPassphrase,
    /// A passphrase of a length not taken.
    #[error(
        "a backup passphrase is {} to {} characters long",
        crate::passphrase::MIN_CHARS,
        crate::passphrase::MAX_CHARS
    )]
    PassphraseLength,
    /// More than one set of books is there, and which was not said.
    #[error(
        "that folder holds backups of more than one set of books ({0}): choose the folder of one"
    )]
    Several(String),
    /// Nothing there is a backup.
    #[error("there is no backup there")]
    NotABackup,
    /// Stopped because the books are closing.
    #[error("{0} was stopped: the books are closing")]
    Cancelled(String),
    /// A test's death at a write point; never made otherwise.
    #[doc(hidden)]
    #[error("the program died here")]
    Died,
    /// The disk refused.
    #[error("the backup couldn't be made: {0}")]
    Io(std::io::ErrorKind),
}

impl From<std::io::Error> for BackupError {
    fn from(e: std::io::Error) -> Self {
        io::uncarry(e)
    }
}

impl From<crate::StoreError> for BackupError {
    fn from(e: crate::StoreError) -> Self {
        match e {
            crate::StoreError::Cancelled => BackupError::Cancelled("verifying".into()),
            e => BackupError::Damaged(e.to_string()),
        }
    }
}

fn newest_damaged_words(number: u64, older: Option<&str>, newest: Option<&str>) -> String {
    let which = match newest {
        Some(d) => format!("the newest backup (number {number}, from {d})"),
        None => format!("the newest backup (number {number})"),
    };
    match older {
        Some(d) => format!(
            "{which} is damaged; an older one from {d} is whole, and a restore would give that one, not the newest"
        ),
        None => format!("{which} is damaged, and no older one proves whole either"),
    }
}

/// Told the name of each phase of a long step as it begins (the server's
/// test knob waits in them).
pub type PhaseWatch = Arc<dyn Fn(&str) + Send + Sync>;

/// A test's way to die at a write point: told the kind of point and the
/// file it is for, answers true to die there.
pub type Faults = Arc<dyn Fn(&str, &str) -> bool + Send + Sync>;

/// What a step is run under: the stop it looks for (INVARIANT 8) and, in
/// tests, where it dies.
#[derive(Clone, Default)]
pub struct Ctx {
    stop: Option<Arc<AtomicBool>>,
    faults: Option<Faults>,
    phase: Option<PhaseWatch>,
    /// Tests: a disk that makes no hard links, and one whose move cannot
    /// refuse to replace.
    no_links: bool,
    no_noreplace: bool,
    /// Tests: nothing is synced to disk (a destination of thousands of
    /// backups is made quickly; a real one never is).
    no_sync: bool,
    /// Tests: the proof after a backup is left out (for the tests that make
    /// many backups only to see what is written, which prove what they
    /// made themselves).
    no_proof: bool,
    /// Tests: told each read and length look at a file of the set.
    trace: Option<Trace>,
}

/// A test's look at which files of a set are read or measured.
pub type Trace = Arc<dyn Fn(&str, &str) + Send + Sync>;

impl std::fmt::Debug for Ctx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Ctx")
    }
}

impl Ctx {
    /// No stop, no faults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stopped when `flag` is set.
    pub fn with_stop(mut self, flag: Arc<AtomicBool>) -> Self {
        self.stop = Some(flag);
        self
    }

    /// Dies where `faults` says (tests).
    #[doc(hidden)]
    pub fn with_faults(mut self, faults: Faults) -> Self {
        self.faults = Some(faults);
        self
    }

    /// A disk that makes no hard links (tests: the commonest other disk,
    /// FAT or exFAT on a stick).
    #[doc(hidden)]
    pub fn with_no_hard_links(mut self) -> Self {
        self.no_links = true;
        self
    }

    /// A disk that makes no links and whose move cannot refuse to replace
    /// (tests: the last resort of `io.rs`).
    #[doc(hidden)]
    pub fn with_no_exclusive_move(mut self) -> Self {
        self.no_links = true;
        self.no_noreplace = true;
        self
    }

    /// Told, in tests, each `read` and `len` of a file of a set.
    #[doc(hidden)]
    pub fn with_trace(mut self, trace: Trace) -> Self {
        self.trace = Some(trace);
        self
    }

    pub(crate) fn traced(&self, kind: &str, rel: &str) {
        if let Some(t) = &self.trace {
            t(kind, rel);
        }
    }

    /// Nothing is synced to disk (tests that make thousands of backups).
    #[doc(hidden)]
    pub fn with_no_sync(mut self) -> Self {
        self.no_sync = true;
        self
    }

    /// The proof after a backup is left out (tests that make many backups
    /// only to see what is written; the proof has tests of its own).
    #[doc(hidden)]
    pub fn with_proof_skipped(mut self) -> Self {
        self.no_proof = true;
        self
    }

    pub(crate) fn skips_proof(&self) -> bool {
        self.no_proof
    }

    pub(crate) fn syncs(&self) -> bool {
        !self.no_sync
    }

    pub(crate) fn refuses_links(&self) -> bool {
        self.no_links
    }

    pub(crate) fn refuses_noreplace(&self) -> bool {
        self.no_noreplace
    }

    /// Told each phase of a long step as it begins.
    pub fn with_phase_watch(mut self, watch: PhaseWatch) -> Self {
        self.phase = Some(watch);
        self
    }

    /// A phase of a step begins.
    pub(crate) fn phase(&self, name: &str) {
        if let Some(w) = &self.phase {
            w(name);
        }
    }

    /// The stop flag, if there is one.
    pub fn stop_flag(&self) -> Option<Arc<AtomicBool>> {
        self.stop.clone()
    }

    /// Ok, or [`BackupError::Cancelled`] when a stop was asked for.
    pub fn check(&self, what: &str) -> Result<(), BackupError> {
        if self.stop.as_ref().is_some_and(|f| f.load(Ordering::SeqCst)) {
            Err(BackupError::Cancelled(what.to_string()))
        } else {
            Ok(())
        }
    }

    /// A write point: where a test's fault may die.
    pub(crate) fn point(&self, kind: &str, rel: &str) -> Result<(), BackupError> {
        match &self.faults {
            Some(f) if f(kind, rel) => Err(BackupError::Died),
            _ => Ok(()),
        }
    }
}

/// What a person sees of a backup's standing in one line of the record:
/// serialisable pieces for the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reached {
    /// The journal seq the backup reaches.
    pub seq: u64,
    /// Its hash.
    pub hash: String,
    /// The number of the manifest that made it whole.
    pub number: u64,
}

/// The words in a set's READ-ME, for a person who opens the folder.
pub(crate) const READ_ME_TEXT: &str = "\
This folder is a backup of a set of books kept with Wanderbooks.

Nothing in it needs to be opened or changed by hand, and nothing in it is
ever deleted or rewritten by Wanderbooks: a backup only adds files. If your
disk is short of room you may delete the whole folder, but not parts of it.

To get the books back, open Wanderbooks and choose Restore, then point it at
this folder (or at the folder above it). It checks everything before it
puts anything in place.

If these backups are protected by a passphrase, the files here are sealed:
only Wanderbooks with that passphrase can read them, and nobody can recover
a forgotten passphrase.
";
