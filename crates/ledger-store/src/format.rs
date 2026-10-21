//! The format gate: what this version of wanderbooks can read.
//!
//! Every journal line has a type, and this version knows a fixed set of
//! them ([`EVENT_TYPES`]). Books written by a newer wanderbooks can hold a
//! type this version has never seen, or declare a newer journal format
//! (`journal.format_set`) before the first event that needs it. Either way
//! the line is whole and chained: the books are fine, and this reader is
//! old. It says so ([`StoreError::NewerJournal`]) instead of calling the
//! books corrupt, and stops there: it never skips a line it can't read.
//!
//! The gate runs as each line is read, after its parse, seq and hash
//! checks ([`crate::journal::read_journal_and_format`], and
//! [`crate::Snapshot`] line by line).
//!
//! What every format shares is frozen (rules.md B-27): a line is the
//! envelope `{seq, ts, type, payload, prev}` and nothing beside it, and
//! its hash is the SHA-256 of its canonical JSON (keys sorted, no
//! whitespace). A newer format adds its fields inside `payload`. So this
//! version can always follow the chain one line past the first line it
//! can't read, and say whether that line is intact.
//!
//! A line an earlier version can't read is announced by a feature
//! ([`KNOWN_FEATURES`]): the writer declares it (`journal.format_set`,
//! the features in force and the new one) just before the first line that
//! needs it, so the earlier reader stops at the declaration, and books
//! that never use the feature stay readable by it.

use crate::error::StoreError;
use crate::journal::{JournalLine, ReadLine};
use serde_json::{Value, json};

/// The newest journal format this version reads and writes.
pub const SUPPORTED_JOURNAL_FORMAT: u32 = 2;

/// Optional journal features this version reads. Each marks lines an
/// older version can't read: a new event type (which it would refuse as
/// unknown), or a field or value on an event type it knows (which it
/// would misread silently). A writer declares the feature
/// (`journal.format_set`) before its first such line
/// ([`crate::Store`]'s `ensure_feature`), so the older version stops
/// there instead (rules.md B-27). Books that never use one stay readable
/// by versions before it.
pub const KNOWN_FEATURES: &[&str] = &[
    MEMBERS_CUMULATIVE,
    FEATURE_ACCOUNT_RENAMED,
    FEATURE_REQUIRE_TAX_CATEGORY,
    PARTY_CREDITORS,
    PAYMENT_INSTRUMENTS,
    VENDOR_1099_DECISIONS,
    MONTH_REPOSTING,
    SCHEDULE_CONVENTION,
];

/// Feature: `period.closed` closes to members with `"rounding":
/// "cumulative_since"` and `since_seq`, the count of members closes
/// starting at the first since the members or shares last changed (P3
/// defect 17, rules.md B-13). An older version would call the rounding
/// unknown, or (before the format gate) replay the close under the run
/// rule, which can split it differently.
pub const MEMBERS_CUMULATIVE: &str = "members_cumulative";

/// The feature declared before the first `account.renamed` (P3, defect
/// 15).
pub const FEATURE_ACCOUNT_RENAMED: &str = "account_renamed";

/// The feature declared before the first `settings.set` naming
/// `policy.require_tax_category` (P3, defect 8).
pub const FEATURE_REQUIRE_TAX_CATEGORY: &str = "require_tax_category";

/// The feature a `settings.set` naming `key` needs declared first: a key
/// an earlier version doesn't know.
pub(crate) fn setting_feature(key: &str) -> Option<&'static str> {
    match key {
        "policy.require_tax_category" => Some(FEATURE_REQUIRE_TAX_CATEGORY),
        _ => None,
    }
}

/// The feature `party.creditor_set` lines need: a party named the
/// creditor of a liability (rules.md B-35, P3 defect 24).
pub const PARTY_CREDITORS: &str = "party_creditors";

/// Payment methods (`instrument.added`) and the entries tagged with one
/// (`entry.instrument_set`): plan P3, defect 12.
pub const PAYMENT_INSTRUMENTS: &str = "payment_instruments";

/// 1099 decisions kept as decisions (`vendor.1099_set` with
/// `not_reportable`, its reason, the date it takes effect and who decided:
/// plan P3, defects 18 and 23, D-40) and W-9 flags kept apart from them
/// (`vendor.w9_flag_set`).
pub const VENDOR_1099_DECISIONS: &str = "vendor_1099_decisions";

/// Feature: a schedule month or a depreciation period posted again after a
/// reversal un-posted it. An older version would refuse the second
/// `schedule.posted` as posted twice, and its projection would ignore the
/// second `asset.depreciation_posted` (P3 depreciation review R3).
pub const MONTH_REPOSTING: &str = "month_reposting";

/// Feature: `schedule.registered` carries `convention` (`daily` or
/// `monthly_even`). An older version would read a daily schedule as equal
/// months (P3 defect 9, plan/served.md D-11).
pub const SCHEDULE_CONVENTION: &str = "schedule_convention";

/// The line type that declares the journal format.
pub const FORMAT_SET: &str = "journal.format_set";

/// Every journal line type this version reads. Each has a replay arm and a
/// projection arm in `store.rs` (`tests/format.rs` checks both ways).
pub const EVENT_TYPES: &[&str] = &[
    "account.added",
    "account.renamed",
    "account.renumbered",
    "account.tax_set",
    "asset.acquisition_linked",
    "asset.corrected",
    "asset.depreciation_posted",
    "asset.described",
    "asset.disposed",
    "asset.registered",
    "asset.serial_set",
    "asset.tagged",
    "asset.tax_basis_set",
    "attachment.linked",
    "attachment.unlinked",
    "attachment.withdrawn",
    "bill.created",
    "bill.paid",
    "class.added",
    "convention.retired",
    "convention.set",
    "customer.added",
    "decision.recorded",
    "document.filed",
    "entry.approved",
    "entry.created",
    "entry.instrument_set",
    "entry.line_added",
    "entry.line_class_set",
    "entry.no_party",
    "entry.parties_set",
    "entry.party_set",
    "entry.posted",
    "entry.reversed",
    "entry.vendor_set",
    "fx.rate_set",
    "invoice.created",
    "invoice.payment_received",
    "instrument.added",
    FORMAT_SET,
    "journal.tail_repaired",
    "line.cleared",
    "open_item.raised",
    "open_item.resolved",
    "party.added",
    "party.creditor_set",
    "party.role_set",
    "party.tin_set",
    "party.updated",
    "party.w4_set",
    "party.w9_set",
    "payroll.run",
    "payroll.taxes_remitted",
    "period.closed",
    "period.created",
    "receipt.attached",
    "receipt.linked",
    "receipt.unlinked",
    "reconciliation.completed",
    "reconciliation.voided",
    "recurring.generated",
    "recurring.registered",
    "role.assigned",
    "schedule.posted",
    "schedule.registered",
    "settings.set",
    "statement.recorded",
    "statement.voided",
    "store.created",
    "vendor.1099_set",
    "vendor.added",
    "vendor.w9_flag_set",
];

/// The journal format in force: the last `journal.format_set` read, or
/// format 1 with no features for books that never declared one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalFormat {
    /// Format number: 1 for every journal written before the gate.
    pub format: u32,
    /// Optional features in use, as declared.
    pub features: Vec<String>,
}

impl Default for JournalFormat {
    fn default() -> Self {
        Self {
            format: 1,
            features: Vec::new(),
        }
    }
}

impl JournalFormat {
    /// Pass one verified line through the gate, before it is replayed.
    ///
    /// ```text
    /// GOAL: an older wanderbooks never misreads newer books, and never
    ///       calls them corrupt.
    /// SOURCE: rules.md B-27; plan/served.md 5.1 (format gate).
    /// INVARIANTS:
    ///   1. Only lines that parsed, chained and kept their seq reach the
    ///      gate: damage stays JournalCorrupt.
    ///   2. NEVER admits a line this version can't replay.
    /// SCENARIOS:
    ///   1. IF the type is one this version doesn't know: SHOULD refuse
    ///      with NewerJournal
    ///      BECAUSE skipping it would show books that differ from the
    ///      record without saying so.
    ///   2. IF it is a journal.format_set newer than
    ///      SUPPORTED_JOURNAL_FORMAT, or with a feature not in
    ///      KNOWN_FEATURES: SHOULD refuse with NewerJournal.
    ///   3. IF it is a journal.format_set this version reads: SHOULD
    ///      record it as the format in force.
    ///   4. IF a journal.format_set payload is malformed: SHOULD refuse
    ///      with JournalCorrupt.
    /// ```
    pub fn admit(&mut self, line: &JournalLine) -> Result<(), StoreError> {
        if line.line_type == FORMAT_SET {
            *self = declared(line)?;
            Ok(())
        } else if EVENT_TYPES.contains(&line.line_type.as_str()) {
            Ok(())
        } else {
            Err(unknown_type(line))
        }
    }

    /// Check a declaration this version is about to write: a format it
    /// reads, not lower than the one in force, with features it knows.
    /// Returns the journal payload.
    pub(crate) fn declaration(&self, format: u32, features: &[&str]) -> Result<Value, StoreError> {
        if format == 0 || format > SUPPORTED_JOURNAL_FORMAT {
            return Err(StoreError::FormatRefused(format!(
                "journal format {format}: this version writes 1 through {SUPPORTED_JOURNAL_FORMAT}"
            )));
        }
        if format < self.format {
            return Err(StoreError::FormatRefused(format!(
                "journal format {format}: the books already use format {}, and a format is never lowered",
                self.format
            )));
        }
        if let Some(f) = features.iter().find(|f| !KNOWN_FEATURES.contains(f)) {
            return Err(StoreError::FormatRefused(format!(
                "journal feature {f:?}: this version doesn't know it"
            )));
        }
        let mut features: Vec<&str> = features.to_vec();
        features.sort_unstable();
        features.dedup();
        Ok(json!({ "format": format, "features": features }))
    }
}

/// Read a `journal.format_set` line, refusing a format or feature this
/// version doesn't read.
pub(crate) fn declared(line: &JournalLine) -> Result<JournalFormat, StoreError> {
    declared_knowing(line, KNOWN_FEATURES)
}

/// [`declared`] by a version that knows the features `known`: what an
/// older version, which knew fewer, does with the same line.
fn declared_knowing(line: &JournalLine, known: &[&str]) -> Result<JournalFormat, StoreError> {
    let p = &line.payload;
    let format = p
        .get("format")
        .and_then(Value::as_u64)
        .filter(|&f| f > 0)
        .ok_or_else(|| {
            StoreError::JournalCorrupt(format!("seq {}: {FORMAT_SET} format", line.seq))
        })?;
    // A newer format may shape the rest of its declaration differently:
    // it is newer, whatever its features look like.
    if format > u64::from(SUPPORTED_JOURNAL_FORMAT) {
        return Err(newer(line, format!("journal format {format}")));
    }
    let format = u32::try_from(format).unwrap_or(SUPPORTED_JOURNAL_FORMAT);
    let features = p
        .get("features")
        .and_then(Value::as_array)
        .and_then(|a| {
            a.iter()
                .map(|f| f.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| {
            StoreError::JournalCorrupt(format!("seq {}: {FORMAT_SET} features", line.seq))
        })?;
    let unknown: Vec<&str> = features
        .iter()
        .map(String::as_str)
        .filter(|f| !known.contains(f))
        .collect();
    if !unknown.is_empty() {
        return Err(newer(
            line,
            format!("journal features {}", unknown.join(", ")),
        ));
    }
    Ok(JournalFormat { format, features })
}

/// The refusal for a line of a type this version knows that holds a
/// value only a newer version writes: a setting key, a convention kind
/// (`need` says which). Never "corrupt": the books are fine, this reader
/// is old (rules.md B-27).
pub(crate) fn newer_content(line: &JournalLine, need: String) -> StoreError {
    newer(line, need)
}

/// The refusal for a line whose type this version doesn't know.
pub(crate) fn unknown_type(line: &JournalLine) -> StoreError {
    newer(line, "an event type this version doesn't know".into())
}

/// The gate for one line as read: a line with a key beside the envelope's
/// is newer (rules.md B-27: this version never drops a key it can't
/// read), and any other goes to `admit` (JournalFormat::admit).
pub(crate) fn gate(
    read: &ReadLine,
    admit: &mut impl FnMut(&JournalLine) -> Result<(), StoreError>,
) -> Result<(), StoreError> {
    if read.unknown_keys.is_empty() {
        admit(&read.line)
    } else {
        Err(newer(
            &read.line,
            format!(
                "journal line fields this version doesn't know ({})",
                read.unknown_keys.join(", ")
            ),
        ))
    }
}

/// The gate's refusal, unverified: only the reader knows whether a later
/// line checks this one's hash ([`settle`]).
fn newer(line: &JournalLine, need: String) -> StoreError {
    StoreError::NewerJournal {
        seq: line.seq,
        event_type: line.line_type.clone(),
        need,
        verified: false,
    }
}

/// Settle the gate's refusal of a line whose hash is `hash`, given the raw
/// journal line after it (`next`, None when the refused line is the last).
///
/// ```text
/// GOAL: damage is never reported as books from a newer wanderbooks, and
///       books from a newer wanderbooks never as damage.
/// SOURCE: rules.md B-27 (the envelope and its hash are frozen across
///         formats); docs/receipt-store.md "Line format" (a line's hash is
///         checked only by the next line's prev).
/// INVARIANTS:
///   1. Only a NewerJournal refusal is settled; anything else passes
///      through unchanged.
///   2. Of the next line only `prev` is read, leniently (any JSON object
///      with a string `prev`): it is written in a format this version
///      may not read, with floats, a string seq or keys of its own.
/// SCENARIOS:
///   1. IF the next line has a prev that isn't `hash`: SHOULD be
///      JournalCorrupt
///      BECAUSE the refused line changed after it was written (a flipped
///      bit in its type reads like a type this version doesn't know).
///   2. IF the next line's prev is `hash`: SHOULD be NewerJournal,
///      verified: the refused line is intact (the lines after it are the
///      newer version's to check).
///   3. IF there is no next line, or it has no prev this version can
///      read: SHOULD be NewerJournal, unverified
///      BECAUSE nothing checks the refused line's own bytes, so the
///      message says "newer or damaged", never "intact".
/// ```
pub(crate) fn settle(refusal: StoreError, hash: &str, next: Option<&[u8]>) -> StoreError {
    let StoreError::NewerJournal {
        seq,
        event_type,
        need,
        ..
    } = refusal
    else {
        return refusal;
    };
    let prev = next
        .and_then(|raw| serde_json::from_slice::<Value>(raw).ok())
        .and_then(|v| v.get("prev").and_then(Value::as_str).map(str::to_string));
    match prev {
        Some(prev) if prev != hash => {
            StoreError::JournalCorrupt(format!("hash chain broken at seq {}", seq + 1))
        }
        found => StoreError::NewerJournal {
            seq,
            event_type,
            need,
            verified: found.is_some(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Store, apply_line_to_sqlite, replay_line_into_ledger};
    use ledger_core::{Currency, Ledger};

    fn line(line_type: &str, payload: Value) -> JournalLine {
        JournalLine {
            seq: 7,
            ts: "2026-10-16T00:00:00Z".into(),
            line_type: line_type.into(),
            payload,
            prev: String::new(),
        }
    }

    fn is_newer(r: &Result<(), StoreError>) -> bool {
        matches!(r, Err(StoreError::NewerJournal { .. }))
    }

    /// The list and the code agree: every type the gate admits has a
    /// replay arm and a projection arm (a known type with an empty payload
    /// fails as corrupt, never as newer).
    #[test]
    fn every_known_event_type_has_replay_and_projection_arms() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        Store::migrate(&conn).unwrap();
        for &t in EVENT_TYPES {
            let l = line(t, json!({}));
            let mut ledger = Ledger::new(Currency::USD);
            let replayed = replay_line_into_ledger(&mut ledger, &l, Currency::USD);
            assert!(!is_newer(&replayed), "{t}: no replay arm");
            assert!(
                !is_newer(&apply_line_to_sqlite(&conn, &l)),
                "{t}: no projection arm"
            );
        }
        // A type a later phase adds (plan/served.md 5.2, P8).
        let l = line("period.attested", json!({}));
        let mut ledger = Ledger::new(Currency::USD);
        assert!(is_newer(&replay_line_into_ledger(
            &mut ledger,
            &l,
            Currency::USD
        )));
        assert!(is_newer(&apply_line_to_sqlite(&conn, &l)));
    }

    /// A declaration never lowers the format in force, and never names a
    /// format or feature this version can't read back.
    #[test]
    fn a_declared_format_is_never_lowered_or_unreadable() {
        let at1 = JournalFormat::default();
        assert_eq!(
            at1.declaration(1, &[]).unwrap(),
            json!({"format": 1, "features": []})
        );
        for (format, features) in [(0, &[][..]), (3, &[][..]), (1, &["x"][..])] {
            assert!(
                matches!(
                    at1.declaration(format, features),
                    Err(StoreError::FormatRefused(_))
                ),
                "{format} {features:?}"
            );
        }
        let at2 = JournalFormat {
            format: 2,
            features: Vec::new(),
        };
        assert!(matches!(
            at2.declaration(1, &[]),
            Err(StoreError::FormatRefused(m)) if m.contains("never lowered")
        ));
    }

    /// What the gate records, and what it refuses, line by line.
    #[test]
    fn the_gate_records_a_readable_format_and_refuses_a_newer_one() {
        let mut f = JournalFormat::default();
        f.admit(&line(FORMAT_SET, json!({"format": 1, "features": []})))
            .unwrap();
        assert_eq!(f, JournalFormat::default());
        f.admit(&line("entry.posted", json!({}))).unwrap();
        f.admit(&line(FORMAT_SET, json!({"format": 2, "features": []})))
            .unwrap();
        assert_eq!(f.format, 2, "format 2 is this version's");
        match f.admit(&line(FORMAT_SET, json!({"format": 3, "features": []}))) {
            Err(StoreError::NewerJournal {
                seq,
                event_type,
                need,
                verified,
            }) => {
                // The gate alone never vouches for the line's bytes.
                assert!(!verified);
                assert_eq!((seq, event_type.as_str()), (7, FORMAT_SET));
                assert_eq!(need, "journal format 3");
            }
            other => panic!("expected NewerJournal, got {other:?}"),
        }
        match f.admit(&line(
            FORMAT_SET,
            json!({"format": 1, "features": ["a", "b"]}),
        )) {
            Err(StoreError::NewerJournal { need, .. }) => {
                assert_eq!(need, "journal features a, b")
            }
            other => panic!("expected NewerJournal, got {other:?}"),
        }
        // A refused line leaves the format in force alone.
        assert_eq!(f.format, 2);
        for bad in [
            json!({"features": []}),
            json!({"format": 0, "features": []}),
            json!({"format": "1", "features": []}),
            json!({"format": 1}),
            json!({"format": 1, "features": [1]}),
            json!({"format": 1, "features": "a"}),
        ] {
            assert!(
                matches!(
                    f.admit(&line(FORMAT_SET, bad.clone())),
                    Err(StoreError::JournalCorrupt(_))
                ),
                "{bad}"
            );
        }
    }

    /// P3's features (schedule_convention, month_reposting): this version
    /// reads them; a version before them (KNOWN_FEATURES empty, as through
    /// P2) stops at their declaration with NewerJournal, never misreading a
    /// daily schedule as equal months or a second run of a month as none.
    #[test]
    fn an_older_reader_stops_at_p3s_features() {
        for f in [SCHEDULE_CONVENTION, MONTH_REPOSTING] {
            assert!(KNOWN_FEATURES.contains(&f));
            let l = line(FORMAT_SET, json!({"format": 2, "features": [f]}));
            assert_eq!(
                declared(&l).unwrap(),
                JournalFormat {
                    format: 2,
                    features: vec![f.to_string()]
                }
            );
            match declared_knowing(&l, &[]) {
                Err(StoreError::NewerJournal { need, .. }) => {
                    assert_eq!(need, format!("journal features {f}"))
                }
                other => panic!("expected NewerJournal, got {other:?}"),
            }
        }
        // Declared together, sorted.
        assert_eq!(
            JournalFormat::default()
                .declaration(2, &[SCHEDULE_CONVENTION, MONTH_REPOSTING])
                .unwrap(),
            json!({"format": 2, "features": ["month_reposting", "schedule_convention"]})
        );
    }

    /// A format newer than this version reads is newer whatever shape the
    /// rest of its declaration takes, and however large its number: a
    /// newer format may define its features differently, so they are
    /// judged only for a format this version reads (rules.md B-27).
    #[test]
    fn a_newer_format_is_newer_whatever_its_features_look_like() {
        let mut f = JournalFormat::default();
        for newer in [
            json!({"format": 3, "features": {"actors": true}}),
            json!({"format": 3, "features": [1, 2]}),
            json!({"format": 3}),
            json!({"format": 4_294_967_296_u64, "features": []}),
        ] {
            assert!(
                is_newer(&f.admit(&line(FORMAT_SET, newer.clone()))),
                "{newer}"
            );
        }
        assert_eq!(f, JournalFormat::default());
        // The same shapes under a format this version reads are damage.
        for bad in [
            json!({"format": 1, "features": {"actors": true}}),
            json!({"format": 1, "features": [1, 2]}),
        ] {
            assert!(matches!(
                f.admit(&line(FORMAT_SET, bad)),
                Err(StoreError::JournalCorrupt(_))
            ));
        }
    }

    /// P3 defect 17's feature (members_cumulative): this version reads it;
    /// a version before it (KNOWN_FEATURES empty, as through P2) stops at
    /// its declaration with NewerJournal, never replaying a close counted
    /// from its regime's start under the run rule.
    #[test]
    fn an_older_reader_stops_at_the_members_cumulative_feature() {
        assert!(KNOWN_FEATURES.contains(&MEMBERS_CUMULATIVE));
        let l = line(
            FORMAT_SET,
            json!({"format": 2, "features": [MEMBERS_CUMULATIVE]}),
        );
        assert_eq!(
            declared(&l).unwrap(),
            JournalFormat {
                format: 2,
                features: vec![MEMBERS_CUMULATIVE.to_string()]
            }
        );
        match declared_knowing(&l, &[]) {
            Err(StoreError::NewerJournal { need, .. }) => {
                assert_eq!(need, "journal features members_cumulative")
            }
            other => panic!("expected NewerJournal, got {other:?}"),
        }
    }
}
