//! Text the books already hold is never refused when the program writes it
//! again (BS-partner-2, WB-partner-15; BS-entry-4): an asset an earlier
//! version took, whose description holds a number that reads as a tax id,
//! still depreciates and is still disposed of; the lines the program
//! composes from it hold the masked form. And a payment's reference of
//! nine digits is kept, shown masked in the entry's description, and is no
//! finding of the audit.

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Money};
use ledger_store::{JOURNAL_FILE, JournalWriter, Store};
use rust_decimal::Decimal;
use serde_json::json;
use tempfile::tempdir;

fn usd(amount: &str) -> Money {
    Money::new(amount.parse().unwrap(), Currency::USD)
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

struct Accounts {
    cost: u64,
    accum: u64,
    expense: u64,
    bank: u64,
    gain: u64,
}

fn accounts(store: &mut Store) -> Accounts {
    let mut add = |code: &str, name: &str, t: AccountType| {
        store.add_account(code, name, t, None, false).unwrap()
    };
    Accounts {
        cost: add("1500", "Equipment", AccountType::Asset),
        accum: add("1501", "Accumulated Depreciation", AccountType::Asset),
        expense: add("6100", "Depreciation Expense", AccountType::Expense),
        bank: add("1000", "Bank", AccountType::Asset),
        gain: add("4900", "Gain on Disposal", AccountType::Revenue),
    }
}

/// Books with two assets an earlier version took: one whose description
/// holds a bare run of nine digits (a serial number), one an SSN's shape.
/// They are written to the journal as that version wrote them: the engine
/// refuses such a description now, whoever asks.
fn old_books() -> (tempfile::TempDir, Store, Accounts, [u64; 2]) {
    let dir = tempdir().unwrap();
    let mut store = Store::create(dir.path(), Currency::USD).unwrap();
    let a = accounts(&mut store);
    let first = store.ledger().next_asset_id();
    drop(store);
    let mut w = JournalWriter::open(&dir.path().join(JOURNAL_FILE)).unwrap();
    for (id, description) in [
        (first, "MacBook SN 000000000"),
        (first + 1, "Laptop 000-00-0000 kit"),
    ] {
        w.append(
            "asset.registered",
            json!({
                "id": id,
                "description": description,
                "purchase_date": "2026-01-15",
                "cost": {"amount": "1200.00", "currency": "USD"},
                "salvage_value": {"amount": "0", "currency": "USD"},
                "useful_life_months": 12,
                "method": "straight_line",
                "cost_account": a.cost,
                "accum_depr_account": a.accum,
                "expense_account": a.expense,
            }),
        )
        .unwrap();
    }
    drop(w);
    let store = Store::open(dir.path()).expect("books holding such an asset open");
    store.verify().expect("and verify");
    (dir, store, a, [first, first + 1])
}

/// An asset whose description holds the shape depreciates: the entry is
/// written, its description holds the shape masked, and the books verify.
#[test]
fn an_old_asset_still_depreciates() {
    let (_dir, mut store, _, [bare, grouped]) = old_books();
    let run = store.run_depreciation(bare, "2026-02").unwrap();
    let entry = store.ledger().get_entry(run.entry_id.unwrap()).unwrap();
    assert_eq!(
        entry.description(),
        "Depreciation — MacBook SN \u{2022}\u{2022}\u{2022}\u{2022}\u{2022}0000 — 2026-02"
    );
    let run = store.run_depreciation(grouped, "2026-02").unwrap();
    let entry = store.ledger().get_entry(run.entry_id.unwrap()).unwrap();
    assert_eq!(
        entry.description(),
        "Depreciation — Laptop \u{2022}\u{2022}\u{2022}-\u{2022}\u{2022}-0000 kit — 2026-02"
    );
    store.verify().unwrap();
}

/// ... and is disposed of: the final depreciation's entry and the
/// disposal's both hold the masked form.
#[test]
fn an_old_asset_is_still_disposed_of() {
    let (dir, mut store, a, [bare, grouped]) = old_books();
    for (id, masked) in [
        (
            bare,
            "MacBook SN \u{2022}\u{2022}\u{2022}\u{2022}\u{2022}0000",
        ),
        (
            grouped,
            "Laptop \u{2022}\u{2022}\u{2022}-\u{2022}\u{2022}-0000 kit",
        ),
    ] {
        store.run_depreciation(id, "2026-02").unwrap();
        let on = date(2026, 3, 10);
        let disposal = store
            .dispose_asset(id, on, usd("100"), a.bank, a.gain)
            .unwrap();
        let disposal = store.ledger().get_entry(disposal).unwrap();
        assert_eq!(
            disposal.description(),
            format!("Disposal — {masked} — {on}")
        );
        // The final depreciation is the entry just before it.
        let final_run = store.ledger().get_entry(disposal.id() - 1).unwrap();
        assert_eq!(
            final_run.description(),
            format!("Depreciation — {masked} — 2026-03 (final)")
        );
        assert!(matches!(
            store.ledger().get_asset(id).unwrap().status,
            ledger_core::AssetStatus::Disposed { .. }
        ));
    }
    store.verify().unwrap();
    // The books open again, as they were written.
    drop(store);
    let again = Store::open(dir.path()).unwrap();
    again.verify().unwrap();
    assert_eq!(
        again.ledger().get_asset(bare).unwrap().description,
        "MacBook SN 000000000"
    );
}

/// The text of such an asset is not repeated in a line the audit would
/// then name: the lines the program wrote hold no number that reads as a
/// tax id, so the audit names only the line an earlier version wrote.
#[test]
fn what_the_program_writes_again_is_no_finding() {
    let (_dir, mut store, a, [bare, _]) = old_books();
    store.run_depreciation(bare, "2026-02").unwrap();
    store
        .dispose_asset(bare, date(2026, 3, 10), usd("100"), a.bank, a.gain)
        .unwrap();
    let found: Vec<String> = store
        .integrity_findings()
        .into_iter()
        .filter(|f| f.code == "tax_id_in_journal")
        .map(|f| f.message)
        .collect();
    // Two lines are the earlier version's (the two assets, whose
    // description the journal holds as written); none is the program's.
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found.iter().all(|m| m.contains("(asset.registered)")),
        "{found:?}"
    );
}

/// A payment reference of nine digits is a check or confirmation number:
/// kept whole as the reference, shown masked in the entry's description,
/// and no finding of the audit; a printed tax id is refused before
/// anything is journaled.
#[test]
fn a_nine_digit_payment_reference_is_kept_and_is_no_finding() {
    let dir = tempdir().unwrap();
    let mut store = Store::create(dir.path(), Currency::USD).unwrap();
    let a = accounts(&mut store);
    let customer = store.add_customer("Acme").unwrap();
    let invoice = store
        .create_invoice(
            customer,
            date(2026, 3, 1),
            date(2026, 3, 31),
            vec![ledger_core::arap::DocumentLine {
                description: "work".into(),
                amount: Decimal::from(100),
                class_id: None,
                account: None,
                quantity: None,
                unit_price: None,
            }],
        )
        .unwrap();
    let head = store.head();
    let pay = |store: &mut Store, reference: &str| {
        store.receive_payment_with(
            invoice,
            date(2026, 3, 5),
            Decimal::from(10),
            a.bank,
            &ledger_core::PaymentDetails {
                instrument: None,
                reference: Some(reference.into()),
            },
        )
    };
    assert!(pay(&mut store, "000-00-0000").is_err());
    assert_eq!(store.head(), head, "a refusal journals nothing");
    let entry = pay(&mut store, "123456789").unwrap();
    assert_eq!(
        store.ledger().get_entry(entry).unwrap().description(),
        "Payment on invoice 1 (ref \u{2022}\u{2022}\u{2022}\u{2022}\u{2022}6789)"
    );
    let found: Vec<String> = store
        .integrity_findings()
        .into_iter()
        .filter(|f| f.code == "tax_id_in_journal")
        .map(|f| f.message)
        .collect();
    assert!(found.is_empty(), "{found:?}");
    store.verify().unwrap();
}

/// The audit reads each journal field as its class: a reference (a tag, a
/// serial, a file name, an invoice number) may be nine digits, free words
/// may not, and a printed shape is a finding in either.
#[test]
fn the_audit_reads_a_reference_as_a_reference() {
    let dir = tempdir().unwrap();
    let mut store = Store::create(dir.path(), Currency::USD).unwrap();
    let a = accounts(&mut store);
    let asset = store
        .register_asset(ledger_core::NewAsset {
            description: "Van".into(),
            in_service_date: date(2026, 1, 15),
            cost: usd("1200"),
            salvage_value: usd("0"),
            useful_life_months: 12,
            cost_account: a.cost,
            accum_depr_account: a.accum,
            expense_account: a.expense,
        })
        .unwrap();
    let next = store.ledger().next_entry_id();
    drop(store);
    let mut w = JournalWriter::open(&dir.path().join(JOURNAL_FILE)).unwrap();
    // Nine digits as a tag: a reference. As the words of an entry: a finding.
    w.append(
        "asset.tagged",
        json!({"asset_id": asset, "tag": "123456789", "previous_tag": null}),
    )
    .unwrap();
    w.append(
        "entry.created",
        json!({"id": next, "date": "2026-03-01", "description": "paid 123456789"}),
    )
    .unwrap();
    // A printed shape in a tag is a finding too.
    w.append(
        "asset.tagged",
        json!({"asset_id": asset, "tag": "T-000-00-0000", "previous_tag": "123456789"}),
    )
    .unwrap();
    drop(w);
    let store = Store::open(dir.path()).unwrap();
    store.verify().unwrap();
    let fields: Vec<String> = store
        .integrity_findings()
        .into_iter()
        .filter(|f| f.code == "tax_id_in_journal")
        .map(|f| f.message)
        .collect();
    assert_eq!(fields.len(), 2, "{fields:?}");
    assert!(
        fields[0].contains("(entry.created)") && fields[0].contains("`description`"),
        "{fields:?}"
    );
    assert!(
        fields[1].contains("(asset.tagged)") && fields[1].contains("`tag`"),
        "{fields:?}"
    );
}
