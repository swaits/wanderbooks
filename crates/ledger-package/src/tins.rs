//! Full TINs for the CPA's worksheets: where they come from, and the check
//! that the vault's TIN is the one the books record.
//!
//! ```text
//! GOAL: a worksheet for the CPA carries a TIN only when the vault and the
//!       books agree on it, and says why when it can't.
//! SOURCE: DECISION (owner, 2026-10-15): the vault holds every member's
//!         and payee's full TIN for the CPA package; docs/parties-and-
//!         attachments.md "The TIN vault".
//! INVARIANTS:
//!   1. The party's TIN facts, when on file, have the vault TIN's type and
//!      last four, and the record's hash when they name one.
//!   2. Its W-9 facts, when on file, have the vault TIN's last four; with
//!      no TIN facts, the W-9 facts' vault record is the record's hash.
//!   3. A vault that can't be opened or read, or holds no TIN for the
//!      party, is a note, never an error (a package never fails on it).
//! ```

use ledger_core::{Ledger, PartyId};

/// Where a worksheet's full TINs come from: a party's TIN, else why not
/// (a note for the worksheet, never holding more than a last four). The
/// server and the viewer give `|id| vault_tin(ledger, &vault, id)`.
pub type TinSource<'a> = &'a dyn Fn(PartyId) -> Result<ledger_vault::Tin, String>;

/// A TIN type's wire name.
pub fn tin_type_key(t: ledger_vault::TinType) -> &'static str {
    match t {
        ledger_vault::TinType::Ssn => "ssn",
        ledger_vault::TinType::Ein => "ein",
        ledger_vault::TinType::Itin => "itin",
    }
}

/// The books' name for a vault TIN's type.
pub fn tin_kind(t: ledger_vault::TinType) -> ledger_core::TinType {
    match t {
        ledger_vault::TinType::Ssn => ledger_core::TinType::Ssn,
        ledger_vault::TinType::Ein => ledger_core::TinType::Ein,
        ledger_vault::TinType::Itin => ledger_core::TinType::Itin,
    }
}

/// The full TIN the vault holds for a party, checked against what the
/// books record of it; else why not, in words for a worksheet's note (the
/// note never holds more than a last four). `vault` is the vault as
/// opened, or why it couldn't be (INVARIANT 3).
pub fn vault_tin(
    ledger: &Ledger,
    vault: &Result<ledger_vault::Vault, String>,
    party: PartyId,
) -> Result<ledger_vault::Tin, String> {
    let vault = vault.as_ref().map_err(String::clone)?;
    let t = vault
        .tin(party)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            "TIN not in the vault: record it with books.set_tin (or books.set_w9 tin, from the W-9)"
                .to_string()
        })?;
    let hash = vault.record_hash(party).ok().flatten();
    let p = ledger.get_party(party);
    let (facts, w9) = (
        p.and_then(|p| p.tin.as_ref()),
        p.and_then(|p| p.w9.as_ref()),
    );
    if facts.is_none() && w9.is_none() {
        return Err(
            "the vault holds a TIN but the books have no record of it: record it again with books.set_tin"
                .into(),
        );
    }
    if let Some(f) = facts {
        if f.last4 != t.last4() || f.tin_type != tin_kind(t.kind) {
            return Err(format!(
                "the vault's TIN ({}, ending {}) isn't the one the books record ({}, ending {}): record the right one with books.set_tin",
                tin_type_key(t.kind),
                t.last4(),
                f.tin_type.key(),
                f.last4
            ));
        }
        if f.vault_record.is_some() && f.vault_record != hash {
            return Err("the vault record isn't the one the books recorded: record the TIN again with books.set_tin".into());
        }
    }
    if let Some(f) = w9 {
        if f.tin_last4 != t.last4() {
            return Err(format!(
                "the vault's TIN ends in {}, the W-9 facts say {}: check the W-9 and fix with books.set_w9",
                t.last4(),
                f.tin_last4
            ));
        }
        if facts.is_none() && f.vault_record.is_some() && f.vault_record != hash {
            return Err("the vault record isn't the one the books recorded: check the W-9 and record it again with books.set_w9 tin".into());
        }
    }
    Ok(t)
}
