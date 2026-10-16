//! Attachment links, statements, and reconciliations in the viewer: each
//! subject shows exactly the files linked to it, each account its
//! statements and reconciliations, "Needs attention" asks for a
//! reconciliation until one covers last month, the journal says what
//! happened and opens it, and search finds statements and
//! reconciliations.

use chrono::NaiveDate;
use egui_kittest::Harness;
use ledger_core::{AccountType, Currency, EntryId, Money, NewAsset, PartyId, Role, Side};
use ledger_gui::books::Books;
use ledger_gui::nav::Route;
use ledger_gui::pages::journal::describe;
use ledger_gui::{App, Source, attention, data, palette};
use ledger_store::{Statement, Store, Subject};
use rust_decimal::Decimal;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static DIRS: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let n = DIRS.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!(
            "ledger-gui-attachments-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        TempDir(d)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

fn usd(c: i64) -> Money {
    Money::new(cents(c), Currency::USD)
}

fn today() -> NaiveDate {
    d(2026, 10, 15)
}

fn load(dir: &Path) -> Books {
    data::load_once(&Source {
        data_dir: dir.join("books-data"),
        books_toml: None,
        today: Some(today()),
    })
    .unwrap()
}

struct Fixture {
    dir: TempDir,
    store: Store,
    checking: u64,
    card: u64,
    party: PartyId,
    asset: u64,
    contribution: EntryId,
    software: EntryId,
    linked_only: EntryId,
    receipt: u64,
    contract: u64,
    photo: u64,
}

/// Checking funded in July; software paid in August (its receipt filed on
/// it, and also linked to a second August entry that has no receipt of
/// its own); a contract linked to a party, the company, and the software
/// entry; a photo linked to an asset; a card account with no activity.
fn fixture(tag: &str) -> Fixture {
    let dir = TempDir::new(tag);
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    let acct = |s: &mut Store, code: &str, name: &str, t| {
        s.add_account(code, name, t, None, false).unwrap()
    };
    let checking = acct(&mut s, "1000", "Checking", AccountType::Asset);
    let equipment = acct(&mut s, "1500", "Equipment", AccountType::Asset);
    let accum = acct(
        &mut s,
        "1510",
        "Accumulated Depreciation",
        AccountType::Asset,
    );
    let card = acct(&mut s, "2050", "Business Card", AccountType::Liability);
    let capital = acct(&mut s, "3000", "Capital", AccountType::Equity);
    let software_acct = acct(&mut s, "6300", "Software", AccountType::Expense);
    let depreciation = acct(&mut s, "6800", "Depreciation", AccountType::Expense);
    let post = |s: &mut Store, date, what: &str, dr, cr, c, receipt: bool| {
        let e = s.create_entry(date, what).unwrap();
        s.add_line(e, dr, Side::Debit, usd(c)).unwrap();
        s.add_line(e, cr, Side::Credit, usd(c)).unwrap();
        if receipt {
            s.attach_receipt(
                Some(e),
                &format!("{what}.pdf"),
                "application/pdf",
                what.as_bytes(),
                None,
            )
            .unwrap();
        }
        s.post_entry(e).unwrap();
        e
    };
    let contribution = post(
        &mut s,
        d(2026, 7, 5),
        "Contribution",
        checking,
        capital,
        500_000,
        true,
    );
    let software = post(
        &mut s,
        d(2026, 8, 10),
        "Software",
        software_acct,
        checking,
        12_000,
        true,
    );
    // Its receipt also covers this one (one invoice, two entries).
    let e = s
        .create_entry(d(2026, 8, 12), "Software, second seat")
        .unwrap();
    s.add_line(e, software_acct, Side::Debit, usd(3_000))
        .unwrap();
    s.add_line(e, checking, Side::Credit, usd(3_000)).unwrap();
    let receipt = s.list_attachments(Some(software)).unwrap()[0].id;
    s.link_attachment(receipt, Subject::Entry(e)).unwrap();
    s.post_entry(e).unwrap();
    let linked_only = e;

    let party = s.add_party("Acme Software", &[Role::Contractor]).unwrap();
    let (contract, _, _) = s
        .file_document(
            "contract",
            "acme-agreement.pdf",
            "application/pdf",
            b"%PDF agreement",
            Some("Signed agreement"),
        )
        .unwrap();
    s.link_attachment(contract, Subject::Party(party)).unwrap();
    s.link_attachment(contract, Subject::Company).unwrap();
    s.link_attachment(contract, Subject::Entry(software))
        .unwrap();
    let asset = s
        .register_asset(NewAsset {
            description: "Laptop".into(),
            purchase_date: d(2026, 7, 6),
            cost: usd(200_000),
            salvage_value: usd(0),
            useful_life_months: 36,
            cost_account: equipment,
            accum_depr_account: accum,
            expense_account: depreciation,
        })
        .unwrap();
    let (photo, _, _) = s
        .file_document("photo", "laptop.png", "image/png", b"png bytes", None)
        .unwrap();
    s.link_attachment(photo, Subject::Asset(asset)).unwrap();
    Fixture {
        dir,
        store: s,
        checking,
        card,
        party,
        asset,
        contribution,
        software,
        linked_only,
        receipt,
        contract,
        photo,
    }
}

/// Clear every line on `account` through `through`, file and record its
/// statement for `month_start..=through`, and reconcile it. Returns the
/// statement's attachment id and the reconciliation id.
fn reconcile(
    s: &mut Store,
    account: u64,
    month_start: NaiveDate,
    through: NaiveDate,
) -> (u64, u64) {
    let mut lines = Vec::new();
    for e in s.ledger().posted_entries() {
        if e.date() <= through {
            for (i, l) in e.lines().iter().enumerate() {
                if l.account == account && !s.ledger().is_cleared(e.id(), i) {
                    lines.push((e.id(), i));
                }
            }
        }
    }
    for (e, i) in lines {
        s.mark_cleared(e, i, true).unwrap();
    }
    let report = |s: &Store, date| {
        s.ledger()
            .reconciliation_report(account, date, Decimal::ZERO)
            .unwrap()
            .cleared_balance
    };
    let opening = report(s, month_start.pred_opt().unwrap());
    let closing = report(s, through);
    let (att, _, _) = s
        .file_document(
            "statement",
            &format!("statement-{through}.pdf"),
            "application/pdf",
            format!("statement {through}").as_bytes(),
            None,
        )
        .unwrap();
    s.record_statement(Statement {
        attachment_id: att,
        account_id: account,
        period_start: month_start,
        period_end: through,
        opening_balance: opening,
        closing_balance: closing,
    })
    .unwrap();
    let r = s.complete_reconciliation(att).unwrap();
    (att, r.id)
}

fn ids(docs: Vec<&ledger_store::Attachment>) -> Vec<u64> {
    docs.into_iter().map(|a| a.id).collect()
}

#[test]
fn each_subject_shows_exactly_its_links() {
    let f = fixture("links");
    let b = load(&f.dir.0);
    // An entry: filed on it, and linked to it.
    assert_eq!(
        ids(b.documents_for(&[Subject::Entry(f.software)])),
        vec![f.receipt, f.contract]
    );
    assert_eq!(
        ids(b.documents_for(&[Subject::Entry(f.linked_only)])),
        vec![f.receipt]
    );
    assert_eq!(
        ids(b.documents_for(&[Subject::Party(f.party)])),
        vec![f.contract]
    );
    assert_eq!(ids(b.documents_for(&[Subject::Company])), vec![f.contract]);
    assert_eq!(
        ids(b.documents_for(&[Subject::Asset(f.asset)])),
        vec![f.photo]
    );
    assert!(b.documents_for(&[Subject::Account(f.checking)]).is_empty());
    // Every link of the contract, in the store's order (by kind, id).
    assert_eq!(
        b.links_of(f.contract),
        f.store.attachment_links(f.contract).unwrap().as_slice()
    );
    assert_eq!(b.links_of(f.receipt)[0], Subject::Entry(f.software));
    assert!(
        b.links_of(f.receipt)
            .contains(&Subject::Entry(f.linked_only))
    );
    // The receipt policy counts a linked receipt (the server's rule).
    let only = b.entry(f.linked_only).unwrap();
    assert!(!only.missing_receipt);
    assert_eq!(only.receipts.len(), 1);
    assert_eq!(b.attachments[only.receipts[0]].id, f.receipt);
    // Every subject the books hold matches the store's own list.
    for sub in [
        Subject::Entry(f.contribution),
        Subject::Entry(f.software),
        Subject::Entry(f.linked_only),
        Subject::Party(f.party),
        Subject::Company,
        Subject::Asset(f.asset),
        Subject::Account(f.checking),
    ] {
        assert_eq!(
            ids(b.documents_for(&[sub])),
            f.store
                .attachments_for(sub)
                .unwrap()
                .iter()
                .map(|a| a.id)
                .collect::<Vec<_>>(),
            "{sub:?}"
        );
    }
    // Each chip opens its subject.
    assert_eq!(
        b.subject_route(Subject::Party(f.party)),
        Some(Route::Party(f.party))
    );
    assert_eq!(b.subject_route(Subject::Company), Some(Route::Company));
    assert_eq!(
        b.subject_route(Subject::Asset(f.asset)),
        Some(Route::Asset(f.asset))
    );
}

#[test]
fn unlinking_and_withdrawing_take_a_file_off_its_subjects() {
    let mut f = fixture("unlink");
    f.store
        .unlink_attachment(f.contract, Subject::Company, "Not company-wide")
        .unwrap();
    // A file comes off its links before it is withdrawn.
    f.store
        .unlink_attachment(f.photo, Subject::Asset(f.asset), "Blurry")
        .unwrap();
    f.store
        .withdraw_attachment(f.photo, "Blurry; retaking it")
        .unwrap();
    let b = load(&f.dir.0);
    assert!(b.documents_for(&[Subject::Company]).is_empty());
    assert!(b.documents_for(&[Subject::Asset(f.asset)]).is_empty());
    assert_eq!(
        ids(b.documents_for(&[Subject::Party(f.party)])),
        vec![f.contract]
    );
    assert!(!b.links_of(f.contract).contains(&Subject::Company));
    assert!(b.links_of(f.photo).is_empty());
}

#[test]
fn statements_and_reconciliations_per_account() {
    let mut f = fixture("recs");
    let (jul_st, jul) = reconcile(&mut f.store, f.checking, d(2026, 7, 1), d(2026, 7, 31));
    f.store.void_reconciliation(jul, "Wrong statement").unwrap();
    let (_, jul2) = reconcile(&mut f.store, f.checking, d(2026, 7, 1), d(2026, 7, 31));
    let (aug_st, aug) = reconcile(&mut f.store, f.checking, d(2026, 8, 1), d(2026, 8, 31));
    let b = load(&f.dir.0);
    let st = b.statements_for(f.checking);
    assert_eq!(st.len(), 3);
    assert_eq!(st[0].attachment_id, jul_st);
    assert_eq!(st.last().unwrap().attachment_id, aug_st);
    assert_eq!(
        st.iter().map(|s| (*s).clone()).collect::<Vec<_>>(),
        f.store.statements(Some(f.checking)).unwrap()
    );
    let recs = b.reconciliations_for(f.checking);
    assert_eq!(
        recs.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![jul, jul2, aug]
    );
    assert_eq!(recs[0].voided.as_deref(), Some("Wrong statement"));
    // Cleared lines: the contribution's, then August's two payments.
    let a = b.reconciliation(aug).unwrap();
    assert_eq!(a.cleared.len(), 3);
    assert_eq!(a.statement_balance, cents(500_000 - 12_000 - 3_000));
    assert!(b.reconciliations_for(f.card).is_empty());
    assert_eq!(b.reconciled_through(f.checking), Some(d(2026, 8, 31)));
    assert_eq!(b.reconciled_through(f.card), None);
    use ledger_gui::pages::reconciliation::{Standing, lines, standing};
    assert_eq!(standing(&b, a), Standing::Current);
    assert_eq!(
        standing(&b, b.reconciliation(jul2).unwrap()),
        Standing::Superseded
    );
    assert_eq!(
        standing(&b, b.reconciliation(jul).unwrap()),
        Standing::Voided("Wrong statement".into())
    );
    // Its lines are the account's through the date, each marked.
    let ls = lines(&b, a);
    assert_eq!(ls.len(), 3);
    assert!(ls.iter().all(|l| l.5));
    let total: Decimal = ls.iter().map(|l| l.4).sum();
    assert_eq!(total, a.statement_balance);
    // The statement is linked to its account and its reconciliation.
    assert!(b.links_of(aug_st).contains(&Subject::Account(f.checking)));
    assert!(b.links_of(aug_st).contains(&Subject::Reconciliation(aug)));
    assert_eq!(
        b.subject_route(Subject::Reconciliation(aug)),
        Some(Route::Reconciliation(aug))
    );
}

fn reconcile_items(b: &Books) -> Vec<String> {
    attention::find(b)
        .into_iter()
        .filter(|a| a.title.starts_with("Reconcile "))
        .map(|a| a.title)
        .collect()
}

#[test]
fn attention_asks_for_a_reconciliation_until_last_month_is_covered() {
    let mut f = fixture("attention");
    let b = load(&f.dir.0);
    // Checking has activity; the card has none (no item).
    assert_eq!(b.last_month_end(), d(2026, 9, 30));
    assert_eq!(
        reconcile_items(&b),
        vec!["Reconcile Checking through Sep 30, 2026".to_string()]
    );
    let item = attention::find(&b)
        .into_iter()
        .find(|a| a.title.starts_with("Reconcile "))
        .unwrap();
    assert!(item.prompt.starts_with("In wanderbooks,"));
    assert!(item.prompt.contains("books.record_statement"));
    assert!(item.prompt.contains("books.reconcile"));
    assert!(item.prompt.contains("1000 Checking"));
    assert!(item.prompt.contains("2026-09-30"));
    assert!(item.prompt.contains("never been reconciled"));
    assert_eq!(item.target, Some(Route::Account(f.checking)));

    // Through August only: still due.
    reconcile(&mut f.store, f.checking, d(2026, 7, 1), d(2026, 8, 31));
    let b = load(&f.dir.0);
    assert_eq!(reconcile_items(&b).len(), 1);
    assert!(
        attention::find(&b)
            .iter()
            .any(|a| a.prompt.contains("reconciled through 2026-08-31"))
    );

    // Through September (last month): clear.
    let (_, sep) = reconcile(&mut f.store, f.checking, d(2026, 9, 1), d(2026, 9, 30));
    let b = load(&f.dir.0);
    assert!(reconcile_items(&b).is_empty(), "{:?}", reconcile_items(&b));
    assert_eq!(b.reconciliation_due(f.checking), None);

    // Voided, it's due again.
    f.store.void_reconciliation(sep, "Redo").unwrap();
    let b = load(&f.dir.0);
    assert_eq!(reconcile_items(&b).len(), 1);

    // Card activity before this month makes the card due too.
    let e = f.store.create_entry(d(2026, 9, 3), "Card charge").unwrap();
    let software = f.store.ledger().chart().find_by_code("6300").unwrap().id;
    f.store
        .add_line(e, software, Side::Debit, usd(1_000))
        .unwrap();
    f.store
        .add_line(e, f.card, Side::Credit, usd(1_000))
        .unwrap();
    f.store
        .attach_receipt(Some(e), "c.pdf", "application/pdf", b"card", None)
        .unwrap();
    f.store.post_entry(e).unwrap();
    let b = load(&f.dir.0);
    assert!(
        reconcile_items(&b).contains(&"Reconcile Business Card through Sep 30, 2026".to_string())
    );
}

#[test]
fn journal_text_names_and_opens_the_subjects() {
    let mut f = fixture("journal");
    let (st, r) = reconcile(&mut f.store, f.checking, d(2026, 7, 1), d(2026, 7, 31));
    f.store.void_reconciliation(r, "Wrong statement").unwrap();
    f.store
        .unlink_attachment(f.contract, Subject::Company, "Not company-wide")
        .unwrap();
    let b = load(&f.dir.0);
    let find = |t: &str| {
        b.journal
            .iter()
            .filter(|l| l.line_type == t)
            .map(|l| describe(&b, l).unwrap())
            .collect::<Vec<_>>()
    };
    let linked = find("attachment.linked");
    let party_link = linked
        .iter()
        .find(|e| e.also == Some(Route::Party(f.party)))
        .unwrap();
    assert_eq!(
        party_link.text,
        "Linked acme-agreement.pdf to Acme Software"
    );
    assert_eq!(party_link.route, Some(Route::Document(f.contract)));
    assert!(
        linked
            .iter()
            .any(|e| e.text == "Linked laptop.png to Laptop"
                && e.also == Some(Route::Asset(f.asset)))
    );
    assert!(
        linked
            .iter()
            .any(|e| e.text == format!("Linked acme-agreement.pdf to #{} Software", f.software))
    );
    assert!(
        linked
            .iter()
            .any(|e| e.also == Some(Route::Reconciliation(r)))
    );
    let unlinked = &find("attachment.unlinked")[0];
    assert_eq!(
        unlinked.text,
        "Unlinked acme-agreement.pdf from the company: Not company-wide"
    );
    assert_eq!(unlinked.also, Some(Route::Company));
    let recorded = &find("statement.recorded")[0];
    assert!(
        recorded.text.contains("1000 Checking") && recorded.text.contains("Jul 31, 2026"),
        "{}",
        recorded.text
    );
    assert_eq!(recorded.route, Some(Route::Document(st)));
    assert_eq!(recorded.also, Some(Route::Account(f.checking)));
    let done = &find("reconciliation.completed")[0];
    assert!(
        done.text
            .starts_with("Reconciled 1000 Checking through Jul 31, 2026"),
        "{}",
        done.text
    );
    assert_eq!(done.route, Some(Route::Reconciliation(r)));
    let voided = &find("reconciliation.voided")[0];
    assert_eq!(
        voided.text,
        "Voided the reconciliation of 1000 Checking through Jul 31, 2026: Wrong statement"
    );
    assert_eq!(voided.route, Some(Route::Reconciliation(r)));
}

#[test]
fn search_finds_statements_and_reconciliations() {
    let mut f = fixture("search");
    let (st, r) = reconcile(&mut f.store, f.checking, d(2026, 7, 1), d(2026, 7, 31));
    let b = load(&f.dir.0);
    let routes = |q: &str| -> Vec<Route> {
        palette::search(&b, q)
            .into_iter()
            .map(|i| i.route)
            .collect()
    };
    assert!(routes("reconciliation checking").contains(&Route::Reconciliation(r)));
    assert!(routes("checking 2026-07-31").contains(&Route::Reconciliation(r)));
    assert!(routes("statement checking jul").contains(&Route::Document(st)));
    assert!(!routes("statement savings").contains(&Route::Document(st)));
}

#[test]
fn every_new_page_renders() {
    let mut f = fixture("render");
    let (st, r) = reconcile(&mut f.store, f.checking, d(2026, 7, 1), d(2026, 7, 31));
    for route in [
        Route::Overview,
        Route::Account(f.checking),
        Route::Account(f.card),
        Route::Reconciliation(r),
        Route::Reconciliation(999),
        Route::Document(st),
        Route::Document(f.contract),
        Route::Entry(f.software),
        Route::Entry(f.linked_only),
        Route::Party(f.party),
        Route::Asset(f.asset),
        Route::Company,
        Route::Parties,
    ] {
        let books = load(&f.dir.0);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1100.0, 900.0))
            .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, route.clone()));
        h.run_steps(3);
        assert_eq!(h.state_mut().route(), &route);
    }
}

#[test]
fn books_show_opens_a_reconciliation() {
    use ledger_store::viewer::ViewerTarget;
    let mut f = fixture("show");
    let (_, r) = reconcile(&mut f.store, f.checking, d(2026, 7, 1), d(2026, 7, 31));
    let books = load(&f.dir.0);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, Route::Overview));
    h.state_mut().show(ViewerTarget::Reconciliation { id: r });
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &Route::Reconciliation(r));
    // An unknown one goes nowhere.
    h.state_mut().show(ViewerTarget::Reconciliation { id: 999 });
    assert_eq!(h.state_mut().route(), &Route::Reconciliation(r));
}

/// ledger_store::reconcilable INVARIANT 1, the digest's rule: a liability
/// not named for a card (an Amex) joins the accounts to reconcile once it
/// has a recorded statement, so Needs attention asks for it as the digest
/// does (it used to be listed by the digest only).
#[test]
fn a_liability_with_a_statement_needs_reconciling_whatever_its_name() {
    let mut f = fixture("amex");
    let amex = f
        .store
        .add_account("2060", "Amex", AccountType::Liability, None, false)
        .unwrap();
    let software = f.store.ledger().chart().find_by_code("6300").unwrap().id;
    let e = f.store.create_entry(d(2026, 9, 3), "Amex charge").unwrap();
    f.store
        .add_line(e, software, Side::Debit, usd(1_000))
        .unwrap();
    f.store.add_line(e, amex, Side::Credit, usd(1_000)).unwrap();
    f.store.post_entry(e).unwrap();
    let b = load(&f.dir.0);
    assert!(!b.reconcilable_accounts().contains(&amex));
    let (att, _, _) = f
        .store
        .file_document(
            "statement",
            "amex-aug.pdf",
            "application/pdf",
            b"amex aug",
            None,
        )
        .unwrap();
    f.store
        .record_statement(Statement {
            attachment_id: att,
            account_id: amex,
            period_start: d(2026, 8, 1),
            period_end: d(2026, 8, 31),
            opening_balance: Decimal::ZERO,
            closing_balance: Decimal::ZERO,
        })
        .unwrap();
    let b = load(&f.dir.0);
    assert!(b.reconcilable_accounts().contains(&amex));
    assert_eq!(b.reconciliation_due(amex), Some(d(2026, 9, 30)));
    assert!(reconcile_items(&b).contains(&"Reconcile Amex through Sep 30, 2026".to_string()));
}
