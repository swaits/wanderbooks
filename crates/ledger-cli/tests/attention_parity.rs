//! The viewer's "Needs attention" and `books.start`'s next steps are one
//! list: both come from `ledger_package::attention`, the viewer over its
//! `Snapshot`, the server over its `Store`. This drives both on the same
//! books (the demo company, a book written entry by entry, and that book
//! after its stored file and its database are damaged) and compares them
//! item for item.
#![cfg(feature = "gui")]

use std::collections::HashSet;
use std::time::Instant;

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_gui::{Source, data};
use rust_decimal::Decimal;
use serde_json::Value;
use serde_json::json;

mod common;

const SETTINGS: &str = "[company]\nname = \"Attention Parity LLC\"\nfiscal_year_start = \"01-01\"\n\
     [books]\nreceipt_threshold = \"0.00\"\ncash_accounts = [\"1000\"]\nparties = \"require\"\n";

fn usd(cents: i64) -> Money {
    Money::new(Decimal::new(cents, 2), Currency::USD)
}

fn start(wire: &mut common::Wire) -> Value {
    common::answer_of(&wire.call("books.start", json!({"focus": "daily"})))
        .unwrap_or_else(|e| panic!("books.start: {e}"))
}

/// The items the viewer draws and the next steps the server lists: the
/// server's attention steps are the ones whose `why` is one of the viewer's
/// titles, and they must be the same titles, in the same order. The viewer
/// has at most 12 (books.start lists the top 12). Returns the viewer's
/// titles.
fn compare(place: &common::Place, wire: &mut common::Wire, today: NaiveDate) -> Vec<String> {
    let answer = start(wire);
    let books = data::load_once(&Source {
        data_dir: place.books_data(),
        today: Some(today),
    })
    .unwrap_or_else(|e| panic!("the viewer can't read the books: {e}"));
    let viewer: Vec<String> = books.attention.iter().map(|a| a.title.clone()).collect();
    assert!(
        viewer.len() <= 12,
        "the fixture is meant to fit books.start's 12 attention steps: {viewer:?}"
    );
    let steps = answer["next_steps"]
        .as_array()
        .unwrap_or_else(|| panic!("no next_steps: {answer}"));
    let server: Vec<String> = steps
        .iter()
        .filter_map(|s| s["why"].as_str())
        .filter(|why| viewer.iter().any(|t| t == why))
        .map(str::to_string)
        .collect();
    assert_eq!(server, viewer, "books.start: {answer}");
    // Item for item: severity, title, and detail are the viewer's own; the
    // prompt names the books and says what to do.
    for (a, title) in books.attention.iter().zip(&viewer) {
        assert_eq!(&a.title, title);
        assert!(a.prompt.contains("wanderbooks"), "{}", a.prompt);
    }
    viewer
}

/// A book written entry by entry, pseudo-randomly but deterministically (a
/// fixed seed, so a failure reproduces): a chart, three drafts, posted
/// entries without receipts, one that names no counterparty, and one with
/// a filed receipt (the blob the damage test removes).
fn write_random(dir: &std::path::Path, today: NaiveDate) {
    let _ = std::fs::remove_dir_all(dir);
    let mut st = ledger_store::Store::create(dir, Currency::USD).unwrap();
    let account = |code: &str, name: &str, ty, st: &mut ledger_store::Store| {
        st.add_account(code, name, ty, None, false).unwrap()
    };
    let checking = account("1000", "Checking", AccountType::Asset, &mut st);
    let owners = account("2100", "Due to owners", AccountType::Liability, &mut st);
    let capital = account("3000", "Capital", AccountType::Equity, &mut st);
    let revenue = account("4000", "Design", AccountType::Revenue, &mut st);
    let supplies = account("6100", "Supplies", AccountType::Expense, &mut st);
    let contractors = account("6200", "Contractors", AccountType::Expense, &mut st);
    // A member paid a cost personally: money owed to an owner.
    let post = |st: &mut ledger_store::Store, day: NaiveDate, dr, cr, cents: i64| {
        let e = st.create_entry(day, "Entry").unwrap();
        st.add_line(e, dr, Side::Debit, usd(cents)).unwrap();
        st.add_line(e, cr, Side::Credit, usd(cents)).unwrap();
        st.post_entry(e).unwrap();
        e
    };
    post(
        &mut st,
        today - chrono::Days::new(40),
        supplies,
        checking,
        12_345,
    );
    // Three drafts, alternating accounts.
    for i in 0..3i64 {
        let e = st
            .create_entry(today - chrono::Days::new(30 - i as u64), "A draft")
            .unwrap();
        st.add_line(
            e,
            if i % 2 == 0 { supplies } else { contractors },
            Side::Debit,
            usd(5_000 + i),
        )
        .unwrap();
        st.add_line(e, checking, Side::Credit, usd(5_000 + i))
            .unwrap();
    }
    // Posted without receipts, one naming no counterparty.
    for i in 0..4i64 {
        let day = today - chrono::Days::new(20 - i as u64);
        let e = st.create_entry(day, "No receipt").unwrap();
        st.add_line(
            e,
            if i % 2 == 0 { supplies } else { contractors },
            Side::Debit,
            usd(7_000 + i),
        )
        .unwrap();
        st.add_line(e, checking, Side::Credit, usd(7_000 + i))
            .unwrap();
        st.post_entry(e).unwrap();
    }
    // A reverse of the first entry (its original carries the evidence).
    let first = st.ledger().posted_entries().next().map(|e| e.id()).unwrap();
    st.reverse_entry(first, today - chrono::Days::new(1), "Wrong account")
        .unwrap();
    // One entry with a receipt, so not every posted entry is flagged.
    let e = st
        .create_entry(today - chrono::Days::new(3), "With receipt")
        .unwrap();
    st.add_line(e, revenue, Side::Credit, usd(50_000)).unwrap();
    st.add_line(e, checking, Side::Debit, usd(50_000)).unwrap();
    st.attach_receipt(Some(e), "r.pdf", "application/pdf", b"%PDF r", None)
        .unwrap();
    st.post_entry(e).unwrap();
    let _ = (owners, capital);
    drop(st);
}

/// A blob's file in the content-addressed store (`<aa>/<bb>/<sha>`).
fn blob_path(data_dir: &std::path::Path, sha: &str) -> std::path::PathBuf {
    data_dir
        .join("blobs")
        .join(&sha[0..2])
        .join(&sha[2..4])
        .join(sha)
}

/// The viewer's items and the server's steps, and both name `expected`.
fn compare_and_expect(
    place: &common::Place,
    wire: &mut common::Wire,
    today: NaiveDate,
    expected: &str,
) {
    let viewer = compare(place, wire, today);
    assert!(
        viewer.iter().any(|t| t.contains(expected)),
        "the viewer should say {expected:?}: {viewer:?}"
    );
}

#[test]
fn the_viewer_and_books_start_see_the_same_attention() {
    let today = ledger_core::settings::today_in(None);
    // The demo company (ledger_gui::demo): the books the viewer ships with,
    // with a few loose ends of every kind. Built in a folder of its own and
    // adopted into the data root, as old books come in.
    {
        let place = common::Place::new("attention-parity-demo");
        let source = tempfile::Builder::new()
            .prefix("wb-attention-demo-")
            .tempdir()
            .unwrap();
        ledger_gui::demo::build(source.path(), today).unwrap();
        std::fs::create_dir_all(place.books_data().parent().unwrap()).unwrap();
        std::fs::rename(source.path().join("books-data"), place.books_data()).unwrap();
        let mut wire = place.start(None);
        wire.initialize("attention-parity");
        compare(&place, &mut wire, today);
        // What the real integrity check costs books.start: the cold call
        // (which also opens the store) and a warm one, and how many stored
        // files it hashes (one per distinct blob, as the viewer does).
        let books = data::load_once(&Source {
            data_dir: place.books_data(),
            today: Some(today),
        })
        .unwrap();
        let blobs: HashSet<&str> = books
            .attachments
            .iter()
            .map(|a| a.blob_sha.as_str())
            .collect();
        let cold = Instant::now();
        let _ = start(&mut wire);
        let cold = cold.elapsed();
        let warm = Instant::now();
        let _ = start(&mut wire);
        let warm = warm.elapsed();
        println!(
            "books.start on the demo: {} attachments, {} distinct blobs; cold {cold:?}, warm {warm:?}",
            books.attachments.len(),
            blobs.len()
        );
    }
    // A book written entry by entry, then damaged: a stored file removed
    // and the database changed from outside. Both lists must still agree,
    // and both must name the damage (a clean-books-only test missed this).
    {
        let place = common::Place::new("attention-parity-random");
        write_random(&place.books_data(), today);
        let mut wire = place.start(Some(SETTINGS));
        wire.initialize("attention-parity");
        compare(&place, &mut wire, today);

        // The stored file: remove its bytes (the blob store never deletes;
        // the test does, on its own books).
        let books = data::load_once(&Source {
            data_dir: place.books_data(),
            today: Some(today),
        })
        .unwrap();
        let sha = books
            .attachments
            .first()
            .expect("the fixture filed a receipt")
            .blob_sha
            .clone();
        let path = blob_path(&place.books_data(), &sha);
        std::fs::remove_file(&path).unwrap();
        compare_and_expect(&place, &mut wire, today, "damaged or missing");

        // The database: change a projected row the journal doesn't say.
        let db = ledger_store::database_in_use(&place.books_data());
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE accounts SET name = name || ' (changed outside the journal)' \
             WHERE id = (SELECT MIN(id) FROM accounts)",
            [],
        )
        .unwrap();
        drop(conn);
        compare_and_expect(&place, &mut wire, today, "doesn't match the journal");
    }
}

/// Plan P4b S5, step 4 (R-F): an issued invoice nobody marked sent is on
/// the viewer's list and in `books.start`'s next steps, from the one
/// engine, with the step naming `books.mark_sent`.
#[test]
fn an_unsent_invoice_is_on_both_lists() {
    let today = ledger_core::settings::today_in(None);
    let place = common::Place::new("attention-unsent-parity");
    let dir = place.books_data();
    let _ = std::fs::remove_dir_all(&dir);
    let mut st = ledger_store::Store::create(&dir, Currency::USD).unwrap();
    st.add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let client = st.add_customer("Harbor Client").unwrap();
    st.create_invoice_with(
        client,
        today - chrono::Days::new(40),
        today + chrono::Days::new(20),
        vec![ledger_core::arap::DocumentLine {
            description: "Consulting".into(),
            amount: Decimal::new(50_000, 2),
            class_id: None,
            account: None,
            quantity: None,
            unit_price: None,
        }],
        ledger_core::arap::InvoiceFields {
            number: Some("INV-0001".into()),
            ..Default::default()
        },
    )
    .unwrap();
    drop(st);
    let mut wire = place.start(Some(SETTINGS));
    wire.initialize("attention-unsent-parity");
    compare_and_expect(&place, &mut wire, today, "Invoice INV-0001 was never sent");
    // The step offers the fix: books.mark_sent on that invoice.
    let answer = start(&mut wire);
    let step = answer["next_steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["why"] == json!("Invoice INV-0001 was never sent"))
        .unwrap_or_else(|| panic!("no unsent step: {answer}"));
    assert_eq!(step["tool"], json!("books.mark_sent"), "{answer}");
    assert_eq!(step["args"]["invoice"], json!("invoice:1"), "{answer}");
}

/// Plan P4 step 6 (R-F): a draft that says it waits for its evidence is
/// one item, "draft awaiting receipt", and a draft still being built is
/// another, "draft not posted": both on the viewer's list and in
/// `books.start`'s next steps, from the one engine.
#[test]
fn a_waiting_draft_and_a_building_draft_are_two_items_on_both_lists() {
    let today = ledger_core::settings::today_in(None);
    let place = common::Place::new("attention-drafts-parity");
    let dir = place.books_data();
    let _ = std::fs::remove_dir_all(&dir);
    let mut st = ledger_store::Store::create(&dir, Currency::USD).unwrap();
    let cash = st
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let rent = st
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let mut draft = |what: &str| {
        let e = st.create_entry(today - chrono::Days::new(5), what).unwrap();
        st.add_line(e, rent, Side::Debit, usd(4_000)).unwrap();
        st.add_line(e, cash, Side::Credit, usd(4_000)).unwrap();
        e
    };
    let waiting = draft("Figma annual plan");
    let _building = draft("Coworking day pass");
    st.set_draft_state(
        waiting,
        ledger_core::DraftState::AwaitingEvidence {
            note: "invoice due".into(),
        },
    )
    .unwrap();
    drop(st);
    let mut wire = place.start(Some(SETTINGS));
    wire.initialize("attention-drafts-parity");
    compare_and_expect(
        &place,
        &mut wire,
        today,
        &format!("Draft #{waiting} awaiting receipt: Figma annual plan"),
    );
    let titles: Vec<String> = start(&mut wire)["next_steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["why"].as_str().map(str::to_string))
        .collect();
    assert!(
        titles.contains(&"Draft not posted: Coworking day pass".to_string()),
        "{titles:?}"
    );
}
