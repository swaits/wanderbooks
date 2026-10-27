//! [`PackageBooks`]: what the package reads of the books, from the
//! server's [`Store`] or the viewer's [`Snapshot`] alike.
//!
//! ```text
//! GOAL: one package builder over either view of the books, and no way
//!       for it to write to them.
//! SOURCE: ledger_store::snapshot (a Snapshot reads attachments and their
//!         bytes "the same as through Store").
//! INVARIANTS:
//!   1. Every method only reads.
//!   2. A Store and a Snapshot of the same journal answer every method
//!      alike: the same ledger, settings, journal, journal seq,
//!      attachments (withdrawn ones included), bytes, statements, and
//!      reconciliations, and the same entry links once the filing entry
//!      is added (a store lists it in `links_of`; a snapshot only lists
//!      the explicit links, so callers take the union with
//!      `all_attachments`, as `build_package` and the attention module
//!      do).
//! ```

use ledger_core::Ledger;
use ledger_store::{
    Attachment, DiskCheck, JournalLine, Reconciliation, Snapshot, Statement, Store, StoreError,
    Subject,
};

/// The books, read-only, as the package reads them.
pub trait PackageBooks {
    /// The ledger every figure comes from.
    fn ledger(&self) -> &Ledger;

    /// The settings journaled in the books (the same ones the server and
    /// the viewer read).
    fn settings(&self) -> &ledger_core::settings::Settings {
        self.ledger().settings()
    }

    /// Sequence number of the last journal line: the journal state the
    /// package reflects.
    fn journal_seq(&self) -> u64;

    /// Every journal line, in order: what an entry's origin and its events
    /// are read from.
    fn journal(&self) -> Result<Vec<JournalLine>, StoreError>;

    /// Every attachment (receipts and documents), withdrawn ones
    /// included, in id order.
    fn all_attachments(&self) -> Result<Vec<Attachment>, StoreError>;

    /// The subjects attachment `attachment_id` is linked to beyond the
    /// entry it is filed on (an implementation may list that entry too:
    /// the package takes the union).
    fn links_of(&self, attachment_id: u64) -> Result<Vec<Subject>, StoreError>;

    /// An attachment's bytes, verified against its hash.
    fn attachment_bytes(&self, attachment_id: u64) -> Result<Vec<u8>, StoreError>;

    /// Recorded bank and card statements, by period end.
    fn statements(&self) -> Result<Vec<Statement>, StoreError>;

    /// Reconciliations, voided ones included, by date then id.
    fn reconciliations(&self) -> Result<Vec<Reconciliation>, StoreError>;

    /// How the writer's database compares with the journal. A snapshot
    /// compares them; the store makes the same comparison (see its
    /// implementation).
    fn disk_check(&self) -> DiskCheck {
        DiskCheck::Matches
    }
}

impl PackageBooks for Store {
    fn ledger(&self) -> &Ledger {
        Store::ledger(self)
    }

    fn journal_seq(&self) -> u64 {
        Store::journal_seq(self)
    }

    fn journal(&self) -> Result<Vec<JournalLine>, StoreError> {
        ledger_store::journal::read_and_verify_journal(
            &Store::data_dir(self).join(ledger_store::JOURNAL_FILE),
        )
    }

    fn all_attachments(&self) -> Result<Vec<Attachment>, StoreError> {
        Store::list_attachments(self, None)
    }

    fn links_of(&self, attachment_id: u64) -> Result<Vec<Subject>, StoreError> {
        Store::attachment_links(self, attachment_id)
    }

    fn attachment_bytes(&self, attachment_id: u64) -> Result<Vec<u8>, StoreError> {
        Store::attachment_bytes(self, attachment_id)
    }

    fn statements(&self) -> Result<Vec<Statement>, StoreError> {
        Store::statements(self, None)
    }

    fn reconciliations(&self) -> Result<Vec<Reconciliation>, StoreError> {
        Store::reconciliations(self, None)
    }

    /// The same comparison the viewer makes, not an assumption: the
    /// store's database is the file the viewer checks, and it can be
    /// damaged or changed from outside. Loading a snapshot of the data
    /// directory compares it against a fresh projection of the journal.
    fn disk_check(&self) -> DiskCheck {
        match Snapshot::load(Store::data_dir(self)) {
            Ok(s) => Snapshot::check_disk(&s),
            Err(e) => DiskCheck::Unreadable(e.to_string()),
        }
    }
}

impl PackageBooks for Snapshot {
    fn ledger(&self) -> &Ledger {
        Snapshot::ledger(self)
    }

    fn journal_seq(&self) -> u64 {
        Snapshot::journal_seq(self)
    }

    fn journal(&self) -> Result<Vec<JournalLine>, StoreError> {
        Ok(Snapshot::journal(self).to_vec())
    }

    fn all_attachments(&self) -> Result<Vec<Attachment>, StoreError> {
        Snapshot::attachments(self, None)
    }

    fn links_of(&self, attachment_id: u64) -> Result<Vec<Subject>, StoreError> {
        Ok(Snapshot::attachment_links(self)?
            .into_iter()
            .filter(|(id, _)| *id == attachment_id)
            .map(|(_, subject)| subject)
            .collect())
    }

    fn attachment_bytes(&self, attachment_id: u64) -> Result<Vec<u8>, StoreError> {
        Snapshot::attachment_bytes(self, attachment_id)
    }

    fn statements(&self) -> Result<Vec<Statement>, StoreError> {
        Snapshot::statements(self, None)
    }

    fn reconciliations(&self) -> Result<Vec<Reconciliation>, StoreError> {
        Snapshot::reconciliations(self, None)
    }

    fn disk_check(&self) -> DiskCheck {
        Snapshot::check_disk(self)
    }
}
