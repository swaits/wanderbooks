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
//!      alike: the same ledger, journal seq, attachments (withdrawn ones
//!      included), entry links, and bytes.
//! ```

use ledger_core::Ledger;
use ledger_store::{Attachment, Snapshot, Store, StoreError, Subject};

/// The books, read-only, as the CPA package reads them.
pub trait PackageBooks {
    /// The ledger every figure comes from.
    fn ledger(&self) -> &Ledger;

    /// Sequence number of the last journal line: the journal state the
    /// package reflects.
    fn journal_seq(&self) -> u64;

    /// Every attachment (receipts and documents), withdrawn ones
    /// included, in id order.
    fn all_attachments(&self) -> Result<Vec<Attachment>, StoreError>;

    /// The subjects attachment `attachment_id` is linked to beyond the
    /// entry it is filed on (an implementation may list that entry too:
    /// the package takes the union).
    fn links_of(&self, attachment_id: u64) -> Result<Vec<Subject>, StoreError>;

    /// An attachment's bytes, verified against its hash.
    fn attachment_bytes(&self, attachment_id: u64) -> Result<Vec<u8>, StoreError>;
}

impl PackageBooks for Store {
    fn ledger(&self) -> &Ledger {
        Store::ledger(self)
    }

    fn journal_seq(&self) -> u64 {
        Store::journal_seq(self)
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
}

impl PackageBooks for Snapshot {
    fn ledger(&self) -> &Ledger {
        Snapshot::ledger(self)
    }

    fn journal_seq(&self) -> u64 {
        Snapshot::journal_seq(self)
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
}
