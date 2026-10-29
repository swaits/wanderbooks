// Fuzz target: validly chained journals with fuzz-chosen events.
//
// Raw bytes (fuzz_store_open) almost never pass the hash chain, so they
// never reach replay. Here the journal is written by the real
// JournalWriter (whole lines, gapless seq, correct prev), starting with a
// real store.created most of the time (else a store.created with a
// fuzz-chosen currency or payload, or another type: the writer and the
// viewer share one rule for the first line); the events after it are
// fuzz-chosen types with payloads built from the field names replay
// reads. Oracles:
//   1. Opening never panics or hangs (an Err is fine: bad facts are
//      refused).
//   2. Books that open must verify (live ledger = replay = projection).
//   3. The viewer (Snapshot::load) accepts exactly the journals the writer
//      does, and then holds the same ledger.
//   4. When both refuse, they agree on whether the books are newer
//      (NewerJournal: a type or format this version doesn't read) or bad.
#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use serde_json::{Map, Value, json};

const TYPES: &[&str] = &[
    "account.added", "account.renamed", "account.tax_set", "asset.depreciation_posted",
    "asset.disposed",
    "asset.registered", "attachment.withdrawn", "bill.created", "bill.paid", "class.added",
    "customer.added", "document.filed", "entry.created", "entry.line_added",
    "entry.line_class_set", "entry.posted", "entry.reversed", "entry.vendor_set",
    "fx.rate_set", "invoice.created", "invoice.payment_received", "invoice.sent",
    "invoice.written_off",
    "journal.tail_repaired",
    "line.cleared", "payroll.run", "period.closed", "period.created", "receipt.attached",
    "receipt.linked", "receipt.unlinked", "recurring.generated", "recurring.registered",
    "schedule.posted", "schedule.registered", "vendor.1099_set", "vendor.added",
    "journal.format_set", "settings.set", "convention.set", "convention.retired",
    "decision.recorded", "open_item.raised", "open_item.resolved", "entry.approved",
    "vendor.w9_flag_set", "instrument.added", "entry.instrument_set", "asset.basis_set",
    "party.creditor_set", "account.renumbered", "asset.acquisition_linked", "asset.corrected",
    "asset.described", "asset.serial_set", "asset.tagged", "asset.tax_basis_set",
    "attachment.linked", "attachment.unlinked", "entry.no_party", "entry.parties_set",
    "entry.party_set", "party.added", "party.role_set", "party.tin_set", "party.updated",
    "party.w4_set", "party.w9_set", "payroll.taxes_remitted", "reconciliation.completed",
    "reconciliation.voided", "role.assigned", "statement.recorded", "statement.voided",
    "project.set",
];

const FIELDS: &[&str] = &[
    "account", "account_id", "accum_depr_account", "amount", "asset_id", "attachment_id",
    "balance_account", "bill_id", "category", "class_id", "cleared", "close_to",
    "closing_entry", "code", "cost", "cost_account", "currency", "customer_id", "date",
    "deductible_percent", "description", "due_date", "employee_name", "end", "entry",
    "entry_id", "expense_account", "file_name", "foreign_amount", "foreign_currency", "id",
    "into", "invoice_id", "line_index", "lines", "method", "mime_type", "month", "months",
    "name", "parent", "period", "pl_account", "placeholder", "proceeds", "purchase_date",
    "rate", "reason", "recurring_id", "reversal_of", "reverses", "salvage_value",
    "schedule_id", "share", "side", "size_bytes", "start", "tax_category", "tin_on_file",
    "total", "useful_life_months", "vendor_id", "type", "account_type", "rounding",
    "members", "distributions", "sweep", "format", "features", "changes", "key", "value",
    "source", "decided_by", "decided_on", "kind", "title", "body_md", "structured", "subject",
    "decision_md", "supersedes", "resolution", "approver", "previous", "form_1099",
    "reason_kind", "effective", "instrument_id", "label", "last4", "owner_party_id",
    "rule", "contributor_adjusted_basis", "fmv_at_contribution", "sec704c_method",
    "tax_basis_723", "depreciable_basis", "built_in_gain_704c", "on", "convention",
    "status", "party_id", "started_on", "ended_on", "links", "notes", "projects",
    "via", "by", "to", "reference", "note",
];

/// Journal features a `journal.format_set` may declare.
const FEATURES: &[&str] = &[
    "projects", "document_line_projects", "payment_instruments", "month_reposting",
    "a_feature_from_the_future",
];

/// A value replay might plausibly see: small ids, amounts as decimal
/// strings, dates, sides, currencies, account types, nested objects.
fn value(u: &mut Unstructured, depth: u32) -> arbitrary::Result<Value> {
    Ok(match u.int_in_range(0..=13)? {
        0 => json!(u.int_in_range(0u64..=6)?),
        1 => json!(format!("{}.{:02}", u.int_in_range(0u32..=5000)?, u.int_in_range(0u32..=99)?)),
        2 => json!(format!("2026-{:02}-{:02}", u.int_in_range(1u32..=12)?, u.int_in_range(1u32..=28)?)),
        3 => json!(*u.choose(&["debit", "credit"])?),
        4 => json!(*u.choose(&["USD", "EUR", "JPY", "XYZ"])?),
        5 => json!(*u.choose(&["asset", "liability", "equity", "revenue", "expense"])?),
        6 => json!(bool::arbitrary(u)?),
        7 => Value::Null,
        8 => json!(*u.choose(&["1000", "2000", "3000", "4000", "6000", "x"])?),
        9 if depth < 2 => {
            let n = u.int_in_range(0..=3)?;
            Value::Array((0..n).map(|_| object(u, depth + 1)).collect::<arbitrary::Result<_>>()?)
        }
        10 if depth < 2 => object(u, depth + 1)?,
        // 1099 decisions and payment methods (plan P3).
        11 => json!(*u.choose(&["nec", "misc", "not_reportable", "corporation", "business_card", "member_card", "0042"])?),
        // A declaration's features: some this version knows (projects,
        // the invoice and bill lines' projects) and one it doesn't.
        12 => {
            let n = u.int_in_range(0..=3)?;
            Value::Array(
                (0..n)
                    .map(|_| u.choose(FEATURES).map(|f| json!(*f)))
                    .collect::<arbitrary::Result<_>>()?,
            )
        }
        _ => json!(String::arbitrary(u)?),
    })
}

fn object(u: &mut Unstructured, depth: u32) -> arbitrary::Result<Value> {
    let mut m = Map::new();
    for _ in 0..u.int_in_range(0..=7)? {
        m.insert(u.choose(FIELDS)?.to_string(), value(u, depth)?);
    }
    Ok(Value::Object(m))
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let dir = std::env::temp_dir().join(format!("wb-fuzz-replay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    {
        let mut w = ledger_store::JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
        let (first, payload) = match u.int_in_range(0..=7).unwrap_or(0) {
            0 => ("store.created", object(&mut u, 0).unwrap_or_default()),
            1 => (*u.choose(TYPES).unwrap_or(&"account.added"), object(&mut u, 0).unwrap_or_default()),
            2 => ("store.created", json!({"base_currency": value(&mut u, 0).unwrap_or_default()})),
            _ => ("store.created", json!({"base_currency": "USD"})),
        };
        w.append(first, payload).unwrap();
        for _ in 0..u.int_in_range(0..=24).unwrap_or(0) {
            let (Ok(t), Ok(p)) = (u.choose(TYPES), object(&mut u, 0)) else { break };
            w.append(t, p).unwrap();
        }
    }
    let store = ledger_store::Store::open(&dir);
    let snap = ledger_store::Snapshot::load(&dir);
    match (store, snap) {
        (Ok(store), Ok(snap)) => {
            store.verify().expect("books that open must verify");
            assert!(
                store.ledger().differs_from(snap.ledger()).is_empty(),
                "the viewer and the writer read different books"
            );
        }
        (Err(a), Err(b)) => {
            let newer = |e: &ledger_store::StoreError| {
                matches!(e, ledger_store::StoreError::NewerJournal { .. })
            };
            assert_eq!(newer(&a), newer(&b), "the writer says {a}; the viewer says {b}");
        }
        (Ok(_), Err(e)) => panic!("the writer opens books the viewer refuses: {e}"),
        (Err(e), Ok(_)) => panic!("the viewer loads books the writer refuses: {e}"),
    }
});
