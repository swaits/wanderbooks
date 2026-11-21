//! No tax id in a tool's text, through the real server as an agent reaches it
//! (BS-partner-2, WB-partner-15; the owner's choice of 2026-11-21).
//!
//! The law: a string that looks like a tax id (nine digits standing alone, an
//! SSN's 3-2-4, an EIN's 2-7) is refused before any write tool runs, in words
//! that name the field and never the number, and the journal is byte for byte
//! what it was; an invoice number, a phone number and a reference of ten
//! digits or more are no tax id; a bank file's descriptions are kept with such
//! a number masked to its last four digits and the answer says so; a chart file
//! with one is refused, naming its row and column. The fake ids are
//! `000-00-0000` and `00-0000000`.

use common::{Books, Wire, answer_of};
use serde_json::json;

mod common;

const SSN: &str = "000-00-0000";
const EIN: &str = "00-0000000";

fn setup(w: &mut Wire) {
    for (code, name, kind) in [("1000", "Bank", "asset"), ("6000", "Supplies", "expense")] {
        answer_of(&w.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": kind}),
        ))
        .unwrap();
    }
}

/// Free text with a tax id in it is refused by every tool that keeps text,
/// before it runs: the journal does not change by a byte.
#[test]
fn a_tax_id_in_a_tools_text_is_refused_and_nothing_is_written() {
    let books = Books::new("tin-text-refused");
    let mut w = books.session("test");
    setup(&mut w);
    let before = books.journal_raw();
    for (tool, args, field) in [
        (
            "books.add_class",
            json!({"name": format!("Rondo {SSN}")}),
            "the name of books.add_class",
        ),
        (
            "books.add_project",
            json!({"name": "Harbor", "notes": format!("ein {EIN}")}),
            "the notes of books.add_project",
        ),
        (
            "books.draft_entry",
            json!({"date": "2026-03-01", "description": format!("paid {SSN}"), "no_party_reason": "test"}),
            "the description of books.draft_entry",
        ),
        (
            "books.draft_entry",
            json!({"date": "2026-03-01", "description": "ok", "no_party_reason": format!("own account {EIN}")}),
            "the no_party_reason of books.draft_entry",
        ),
        // A party's own fields are strict: any run of nine digits.
        (
            "books.add_customer",
            json!({"name": "Acme 123456789"}),
            "the name of books.add_customer",
        ),
        (
            "books.add_party",
            json!({"name": format!("Pat {SSN}"), "roles": ["contractor"]}),
            "the name of books.add_party",
        ),
        (
            "books.set_setting",
            json!({"changes": [{"key": "company.name", "value": "Acme"}], "note": format!("see {SSN}")}),
            "the note of books.set_setting",
        ),
    ] {
        let why = answer_of(&w.call(tool, args)).unwrap_err();
        assert!(
            why.starts_with(&format!("{field} looks like it holds a tax id")),
            "{tool}: {why}"
        );
        assert!(
            why.contains("books.set_tin") && why.contains("W-9"),
            "{why}"
        );
        assert!(
            !why.contains("0000") && !why.contains("123456789"),
            "never the number: {why}"
        );
        assert_eq!(
            books.journal_raw(),
            before,
            "{tool}: a refusal writes nothing"
        );
    }
}

/// What is no tax id is kept: an invoice number, a phone number, a long
/// reference, an amount and a date.
#[test]
fn a_reference_is_no_tax_id() {
    let books = Books::new("tin-text-kept");
    let mut w = books.session("test");
    setup(&mut w);
    let out = answer_of(&w.call(
        "books.draft_entry",
        json!({"date": "2026-03-01",
               "description": "inv 2026-11-0042, call 801-555-0142, ref 1234567890, $1,234,567.89",
               "no_party_reason": "test"}),
    ))
    .unwrap();
    assert!(out["entry_id"].is_u64(), "{out}");
}

/// A bank file is kept whole with a nine-digit reference masked to its last
/// four digits; the answer says how many and where; the journal never holds
/// the number.
#[test]
fn a_bank_file_is_masked_not_refused() {
    let books = Books::new("tin-bank-masked");
    let mut w = books.session("test");
    setup(&mut w);
    let csv = format!(
        "Date,Description,Amount\n2026-01-02,ACH {SSN} payroll,-12.50\n2026-01-03,CHECK 1234567890,-3.25\n2026-01-04,\"WIRE 000000000 and {EIN}\",-40.00\n"
    );
    let up = w.upload(csv.as_bytes(), "bank.csv", "text/csv", "import");
    let out = answer_of(&w.call(
        "books.import_bank_csv",
        json!({"upload_id": up, "bank_account_code": "1000", "default_offset_code": "6000"}),
    ))
    .unwrap();
    assert_eq!(out["count"], 3, "{out}");
    assert_eq!(out["masked_count"], 3, "{out}");
    assert_eq!(
        out["masked"],
        json!([
            {"row": 2, "column": "Description", "numbers": 1},
            {"row": 4, "column": "Description", "numbers": 2},
        ])
    );
    let note = out["masked_note"].as_str().unwrap();
    assert!(
        note.starts_with("3 values in the Description column read as a tax id"),
        "{note}"
    );
    assert!(
        note.contains("lines 2, 4") && note.contains("books.set_tin"),
        "{note}"
    );
    let descriptions: Vec<&str> = out["imported"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["description"].as_str().unwrap())
        .collect();
    assert_eq!(
        descriptions,
        [
            "ACH \u{2022}\u{2022}\u{2022}-\u{2022}\u{2022}-0000 payroll",
            "CHECK 1234567890",
            "WIRE \u{2022}\u{2022}\u{2022}\u{2022}\u{2022}0000 and \u{2022}\u{2022}-\u{2022}\u{2022}\u{2022}0000",
        ]
    );
    let journal = String::from_utf8(books.journal_raw()).unwrap();
    for needle in [SSN, EIN, "000000000"] {
        for line in journal.lines().filter(|l| l.contains(needle)) {
            let kind = serde_json::from_str::<serde_json::Value>(line).unwrap()["type"].clone();
            // The line that makes the books holds zeros of its own (a hash).
            assert_eq!(
                kind, "store.created",
                "the journal holds a tax id in a {kind} line"
            );
        }
    }
    assert!(
        journal.contains("Bank import \u{2014} ACH \u{2022}"),
        "the drafts are kept masked"
    );
}

/// A chart file is typed by a person: a cell that looks like a tax id
/// refuses it whole, naming its row and column, and nothing is imported.
#[test]
fn a_chart_file_with_a_tax_id_is_refused_naming_its_row_and_column() {
    let books = Books::new("tin-chart-refused");
    let mut w = books.session("test");
    let csv =
        format!("code,name,type,parent_code\n1000,Cash,asset,\n1010,Payroll {SSN},asset,1000\n");
    let up = w.upload(csv.as_bytes(), "chart.csv", "text/csv", "import");
    let before = books.journal_raw();
    let why = answer_of(&w.call("books.import_chart_csv", json!({"upload_id": up}))).unwrap_err();
    assert!(
        why.starts_with("the name cell of row 3 looks like it holds a tax id"),
        "{why}"
    );
    assert!(!why.contains("0000"), "never the number: {why}");
    assert_eq!(books.journal_raw(), before, "nothing was imported");
    let chart = answer_of(&w.call("books.chart", json!({}))).unwrap();
    assert!(!chart.to_string().contains("Payroll"), "{chart}");
}
