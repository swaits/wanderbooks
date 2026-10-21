//! Journal payloads of 1099 decisions and payment methods (plan P3,
//! defects 12, 18 and 23; D-40). One encoder and one decoder per event,
//! so the writer and every reader (replay, the projection, the viewer)
//! read a line the same way.
//!
//! ```text
//! GOAL: every event round-trips: decode(encode(x)) == x, and every line
//!       written before decisions were kept reads as it always meant.
//! SOURCE: plan/served.md 5.2, 5.5 (a legacy null is undetermined);
//!         rules.md B-27 (an older reader says "newer", never "corrupt").
//! INVARIANTS:
//!   1. vendor.1099_set {vendor_id, form_1099: "nec"|"misc"|
//!      "not_reportable"|null, reason_kind?, reason?, effective?,
//!      tin_on_file, source?}. A line with no source is one written before
//!      decisions were kept: undated, read as its form or undetermined.
//!      A line with a source is the vendor_1099_decisions feature's.
//!   2. not_reportable needs a source, a reason_kind and a reason; a date
//!      needs a source; a reason or reason_kind beside anything else is
//!      corrupt. A form or reason kind this version doesn't know is newer
//!      (format::newer_content), never corrupt.
//!   3. vendor.w9_flag_set {vendor_id, tin_on_file}.
//!   4. instrument.added {instrument_id, kind, label, last4?,
//!      owner_party_id?, account_id}; entry.instrument_set {entry_id,
//!      instrument_id|null}. Optional keys are left out when absent.
//!   5. Decoding checks shape only (rules.md B-28): what a command checks
//!      (limits, the chart) stays with the command.
//! ```

use crate::error::StoreError;
use crate::journal::JournalLine;
use crate::meta::{source_json, source_of};
use chrono::NaiveDate;
use ledger_core::form1099::{Dated1099, Decision1099, ExemptReason, Form1099};
use ledger_core::{Instrument, InstrumentId, InstrumentKind, VendorId};
use serde_json::{Map, Value, json};

const SET: &str = "vendor.1099_set";

fn corrupt(event: &str, what: &str) -> StoreError {
    StoreError::JournalCorrupt(format!("{event}: missing or invalid '{what}'"))
}

fn u64_of(p: &Value, event: &str, k: &str) -> Result<u64, StoreError> {
    p.get(k)
        .and_then(Value::as_u64)
        .ok_or_else(|| corrupt(event, k))
}

fn opt_u64(p: &Value, event: &str, k: &str) -> Result<Option<u64>, StoreError> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| corrupt(event, k)),
    }
}

fn opt_str<'a>(p: &'a Value, event: &str, k: &str) -> Result<Option<&'a str>, StoreError> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(corrupt(event, k)),
    }
}

/// `vendor.1099_set` for a decision with its source (INVARIANT 1).
pub fn decision_set(vendor: VendorId, d: &Dated1099, tin_on_file: bool) -> Value {
    let mut o = Map::new();
    o.insert("vendor_id".into(), json!(vendor));
    match &d.decision {
        Decision1099::Undetermined => {
            o.insert("form_1099".into(), Value::Null);
        }
        Decision1099::Files(f) => {
            o.insert("form_1099".into(), json!(f.key()));
        }
        Decision1099::NotReportable { kind, reason } => {
            o.insert("form_1099".into(), json!("not_reportable"));
            o.insert("reason_kind".into(), json!(kind.key()));
            o.insert("reason".into(), json!(reason));
        }
    }
    if let Some(e) = d.effective {
        o.insert("effective".into(), json!(e.to_string()));
    }
    o.insert("tin_on_file".into(), json!(tin_on_file));
    if let Some(s) = &d.source {
        o.insert("source".into(), source_json(s));
    }
    Value::Object(o)
}

/// Read a `vendor.1099_set` (INVARIANTS 1-2): the vendor, its decision,
/// and its W-9 flag.
pub fn decision_of(line: &JournalLine) -> Result<(VendorId, Dated1099, bool), StoreError> {
    let p = &line.payload;
    let vendor = u64_of(p, SET, "vendor_id")?;
    // Every line has carried the flag; a missing one was false.
    let tin_on_file = match p.get("tin_on_file") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or_else(|| corrupt(SET, "tin_on_file"))?,
    };
    let source = match p.get("source") {
        None | Some(Value::Null) => None,
        Some(_) => Some(source_of(p, SET, "source")?),
    };
    let effective = match opt_str(p, SET, "effective")? {
        None => None,
        Some(s) => {
            Some(NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| corrupt(SET, "effective"))?)
        }
    };
    let kind = opt_str(p, SET, "reason_kind")?;
    let reason = opt_str(p, SET, "reason")?;
    let decision = match opt_str(p, SET, "form_1099")? {
        None => Decision1099::Undetermined,
        Some("not_reportable") => {
            let kind = kind.ok_or_else(|| corrupt(SET, "reason_kind"))?;
            let reason = reason
                .filter(|r| !r.trim().is_empty())
                .ok_or_else(|| corrupt(SET, "reason"))?;
            Decision1099::NotReportable {
                kind: ExemptReason::parse(kind).ok_or_else(|| {
                    crate::format::newer_content(line, format!("the 1099 reason kind {kind:?}"))
                })?,
                reason: reason.to_string(),
            }
        }
        Some(s) => Decision1099::Files(Form1099::parse(s).ok_or_else(|| {
            crate::format::newer_content(line, format!("the 1099 decision {s:?}"))
        })?),
    };
    let not_reportable = matches!(decision, Decision1099::NotReportable { .. });
    if !not_reportable && (kind.is_some() || reason.is_some()) {
        return Err(corrupt(SET, "reason"));
    }
    if source.is_none() && (not_reportable || effective.is_some()) {
        return Err(corrupt(SET, "source"));
    }
    Ok((
        vendor,
        Dated1099 {
            effective,
            decision,
            source,
        },
        tin_on_file,
    ))
}

/// `vendor.w9_flag_set {vendor_id, tin_on_file}` (INVARIANT 3).
pub fn w9_flag_set(vendor: VendorId, tin_on_file: bool) -> Value {
    json!({ "vendor_id": vendor, "tin_on_file": tin_on_file })
}

/// Read a `vendor.w9_flag_set`.
pub fn w9_flag_of(p: &Value) -> Result<(VendorId, bool), StoreError> {
    const E: &str = "vendor.w9_flag_set";
    Ok((
        u64_of(p, E, "vendor_id")?,
        p.get("tin_on_file")
            .and_then(Value::as_bool)
            .ok_or_else(|| corrupt(E, "tin_on_file"))?,
    ))
}

/// `instrument.added` (INVARIANT 4).
pub fn instrument_added(i: &Instrument) -> Value {
    let mut o = Map::new();
    o.insert("instrument_id".into(), json!(i.id));
    o.insert("kind".into(), json!(i.kind.key()));
    o.insert("label".into(), json!(i.label));
    if let Some(l) = &i.last4 {
        o.insert("last4".into(), json!(l));
    }
    if let Some(p) = i.owner_party {
        o.insert("owner_party_id".into(), json!(p));
    }
    o.insert("account_id".into(), json!(i.account));
    Value::Object(o)
}

/// Read an `instrument.added`.
pub fn instrument_of(line: &JournalLine) -> Result<Instrument, StoreError> {
    const E: &str = "instrument.added";
    let p = &line.payload;
    let kind = opt_str(p, E, "kind")?.ok_or_else(|| corrupt(E, "kind"))?;
    Ok(Instrument {
        id: u64_of(p, E, "instrument_id")?,
        kind: InstrumentKind::parse(kind).ok_or_else(|| {
            crate::format::newer_content(line, format!("the payment method kind {kind:?}"))
        })?,
        label: opt_str(p, E, "label")?
            .ok_or_else(|| corrupt(E, "label"))?
            .to_string(),
        last4: opt_str(p, E, "last4")?.map(str::to_string),
        owner_party: opt_u64(p, E, "owner_party_id")?,
        account: u64_of(p, E, "account_id")?,
    })
}

/// `entry.instrument_set {entry_id, instrument_id|null}` (INVARIANT 4).
pub fn entry_instrument_set(
    entry: ledger_core::EntryId,
    instrument: Option<InstrumentId>,
) -> Value {
    json!({ "entry_id": entry, "instrument_id": instrument })
}

/// Read an `entry.instrument_set`.
pub fn entry_instrument_of(
    p: &Value,
) -> Result<(ledger_core::EntryId, Option<InstrumentId>), StoreError> {
    const E: &str = "entry.instrument_set";
    Ok((u64_of(p, E, "entry_id")?, opt_u64(p, E, "instrument_id")?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::settings::Source;

    fn line(line_type: &str, payload: Value) -> JournalLine {
        JournalLine {
            seq: 7,
            ts: "2026-10-20T00:00:00Z".into(),
            line_type: line_type.into(),
            payload,
            prev: String::new(),
        }
    }

    fn source() -> Source {
        Source {
            decided_by: "Riley Chen".into(),
            decided_on: "2026-10-16".parse().unwrap(),
            citation: Some("Treas. Reg. §1.6041-3(p)(1)".into()),
            note: None,
        }
    }

    /// INVARIANT 1: every decision round-trips.
    #[test]
    fn decisions_round_trip() {
        for (effective, decision) in [
            (None, Decision1099::Undetermined),
            (None, Decision1099::Files(Form1099::Nec)),
            (
                Some("2026-12-30".parse().unwrap()),
                Decision1099::not_reportable(ExemptReason::Corporation, "a corporation by name")
                    .unwrap(),
            ),
        ] {
            let d = Dated1099 {
                effective,
                decision,
                source: Some(source()),
            };
            let got = decision_of(&line(SET, decision_set(4, &d, true))).unwrap();
            assert_eq!(got, (4, d, true));
        }
    }

    /// INVARIANT 1, plan 5.5: lines written before decisions were kept
    /// read as their form, undated, with no source; a null (or no form)
    /// as undetermined.
    #[test]
    fn legacy_lines_read_as_they_always_meant() {
        let legacy = |form: Value| {
            decision_of(&line(
                SET,
                json!({"vendor_id": 2, "form_1099": form, "tin_on_file": true}),
            ))
            .unwrap()
        };
        let nec = legacy(json!("nec"));
        assert_eq!(
            nec,
            (
                2,
                Dated1099::from_beginning(Decision1099::Files(Form1099::Nec), None),
                true
            )
        );
        assert_eq!(
            legacy(Value::Null).1,
            Dated1099::from_beginning(Decision1099::Undetermined, None)
        );
        let bare = decision_of(&line(SET, json!({"vendor_id": 2}))).unwrap();
        assert_eq!(
            bare,
            (
                2,
                Dated1099::from_beginning(Decision1099::Undetermined, None),
                false
            )
        );
    }

    /// INVARIANT 2.
    #[test]
    fn malformed_decisions_are_corrupt_and_newer_ones_newer() {
        let src = source_json(&source());
        let bad = [
            // not_reportable without a reason, its kind, or a source.
            json!({"vendor_id": 1, "form_1099": "not_reportable", "reason_kind": "corporation", "tin_on_file": false, "source": src}),
            json!({"vendor_id": 1, "form_1099": "not_reportable", "reason": "x", "tin_on_file": false, "source": src}),
            json!({"vendor_id": 1, "form_1099": "not_reportable", "reason_kind": "corporation", "reason": "x", "tin_on_file": false}),
            // A date without a source; a reason beside a form.
            json!({"vendor_id": 1, "form_1099": "nec", "effective": "2026-01-01", "tin_on_file": false}),
            json!({"vendor_id": 1, "form_1099": "nec", "reason": "x", "tin_on_file": false, "source": src}),
            json!({"vendor_id": 1, "form_1099": 7, "tin_on_file": false}),
            json!({"vendor_id": 1, "form_1099": "nec", "effective": "2026-13-01", "tin_on_file": false, "source": src}),
        ];
        for p in bad {
            let e = decision_of(&line(SET, p.clone())).unwrap_err();
            assert!(matches!(e, StoreError::JournalCorrupt(_)), "{p}: {e:?}");
        }
        let newer = [
            json!({"vendor_id": 1, "form_1099": "k", "tin_on_file": false, "source": src}),
            json!({"vendor_id": 1, "form_1099": "not_reportable", "reason_kind": "martian", "reason": "x", "tin_on_file": false, "source": src}),
        ];
        for p in newer {
            let e = decision_of(&line(SET, p.clone())).unwrap_err();
            assert!(matches!(e, StoreError::NewerJournal { .. }), "{p}: {e:?}");
        }
    }

    /// INVARIANT 4.
    #[test]
    fn instruments_round_trip() {
        for i in [
            Instrument {
                id: 3,
                kind: InstrumentKind::BusinessCard,
                label: "Harbor Visa".into(),
                last4: Some("0042".into()),
                owner_party: None,
                account: 14,
            },
            Instrument {
                id: 4,
                kind: InstrumentKind::MemberCashOrCheck,
                label: "Riley's checks".into(),
                last4: None,
                owner_party: Some(2),
                account: 9,
            },
        ] {
            let p = instrument_added(&i);
            assert!(p.get("last4").is_none() || i.last4.is_some());
            assert_eq!(instrument_of(&line("instrument.added", p)).unwrap(), i);
        }
        let e = instrument_of(&line(
            "instrument.added",
            json!({"instrument_id": 1, "kind": "crypto", "label": "x", "account_id": 1}),
        ))
        .unwrap_err();
        assert!(matches!(e, StoreError::NewerJournal { .. }), "{e:?}");
        assert_eq!(
            entry_instrument_of(&entry_instrument_set(5, None)).unwrap(),
            (5, None)
        );
        assert_eq!(
            entry_instrument_of(&entry_instrument_set(5, Some(2))).unwrap(),
            (5, Some(2))
        );
        assert_eq!(w9_flag_of(&w9_flag_set(3, true)).unwrap(), (3, true));
    }
}
