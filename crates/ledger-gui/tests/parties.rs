//! Parties in the viewer: each party's totals are the books' numbers
//! (ledger-core's records and 1099 worksheet, summed exactly as
//! `PartyInfo` defines); search, books.show, the journal, the entry page,
//! and "Needs attention" know parties.

use chrono::NaiveDate;
use egui_kittest::Harness;
use ledger_core::arap::DocumentLine;
use ledger_core::{
    AccountType, Currency, EntryId, Form1099, Money, PartyField, PartyId, PayrollLine, Role, Side,
    W4Facts, W9Facts, vendor_1099_report,
};
use ledger_gui::books::Books;
use ledger_gui::nav::{Route, Section};
use ledger_gui::pages::journal::describe;
use ledger_gui::pages::parties::{
    State, W9Status, listed, masked_tin, role_counts, roles_text, w9_status,
};
use ledger_gui::{App, Source, attention, data, palette};
use ledger_store::Store;
use ledger_store::viewer::ViewerTarget;
use rust_decimal::Decimal;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static DIRS: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let n = DIRS.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!(
            "ledger-gui-parties-{tag}-{}-{n}",
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

fn doc(c: i64) -> Vec<DocumentLine> {
    vec![DocumentLine {
        description: "work".into(),
        amount: cents(c),
    }]
}

fn w9(last4: &str) -> W9Facts {
    W9Facts {
        received: d(2026, 9, 1),
        entity_type: "Individual/sole proprietor".into(),
        exempt_payee_code: None,
        fatca_code: None,
        tin_last4: last4.into(),
        kept_at: "wanderbooks vault".into(),
        vault_record: None,
    }
}

struct Fixture {
    dir: TempDir,
    books: Books,
    /// The store's own ledger, reopened from disk (the oracle's input).
    oracle: ledger_core::Ledger,
    sam: PartyId,
    sam_vendor: u64,
    riley: PartyId,
    acme: PartyId,
    gumroad: PartyId,
    irs: PartyId,
    pat: PartyId,
    old: PartyId,
    bill_entry: EntryId,
    invoice_entry: EntryId,
    reversal: EntryId,
    draft: EntryId,
    remit: EntryId,
}

/// Sam (a vendor marked for a 1099-NEC, no W-9) paid by cash, card, and
/// an owner, last year and this, with a payment reversed, a bill partly
/// paid, and a draft; Riley (a contractor, no vendor record) paid by
/// cash; Acme (a customer) invoiced this year and last, partly paid;
/// Gumroad paying out; the IRS paid a payroll remittance; Pat on payroll
/// (one run by party, one older run by name); Old Co paid, then archived.
fn fixture(tag: &str) -> Fixture {
    let dir = TempDir::new(tag);
    let data = dir.0.join("books-data");
    let mut s = Store::create(&data, Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let card = s
        .add_account("2050", "Business Card", AccountType::Liability, None, false)
        .unwrap();
    let owed = s
        .add_account(
            "2100",
            "Due to Shareholders",
            AccountType::Liability,
            None,
            false,
        )
        .unwrap();
    let capital = s
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let sales = s
        .add_account("4100", "Template Sales", AccountType::Revenue, None, false)
        .unwrap();
    let fees = s
        .add_account("6000", "Platform Fees", AccountType::Expense, None, false)
        .unwrap();
    let labor = s
        .add_account("6050", "Contract Labor", AccountType::Expense, None, false)
        .unwrap();
    let e = s.create_entry(d(2025, 1, 1), "Owner contribution").unwrap();
    s.add_line(e, cash, Side::Debit, usd(10_000_000)).unwrap();
    s.add_line(e, capital, Side::Credit, usd(10_000_000))
        .unwrap();
    s.post_entry(e).unwrap();
    let pay = |s: &mut Store, date, what: &str, cr, c| {
        let e = s.create_entry(date, what).unwrap();
        s.add_line(e, labor, Side::Debit, usd(c)).unwrap();
        s.add_line(e, cr, Side::Credit, usd(c)).unwrap();
        s.post_entry(e).unwrap();
        e
    };

    let sam_vendor = s.add_vendor("Sam Okafor").unwrap();
    s.set_vendor_1099(sam_vendor, Some(Form1099::Nec), false)
        .unwrap();
    let sam = s.ledger().party_of_vendor(sam_vendor).unwrap();
    s.set_party_role(sam, Role::Contractor, true).unwrap();
    let riley = s.add_party("Riley Chen", &[Role::Contractor]).unwrap();
    let acme_c = s.add_customer("Acme Buyer").unwrap();
    let acme = s.ledger().party_of_customer(acme_c).unwrap();
    let gumroad = s
        .add_party("Gumroad, Inc.", &[Role::PayoutPlatform])
        .unwrap();
    s.update_party(gumroad, PartyField::DisplayName(Some("Gumroad".into())))
        .unwrap();
    let irs = s
        .add_party("Internal Revenue Service", &[Role::TaxAgency])
        .unwrap();
    let pat = s.add_party("Pat Lee", &[Role::Employee]).unwrap();
    s.set_party_w4(
        pat,
        Some(W4Facts {
            received: d(2026, 1, 2),
            state: "UT".into(),
            kept_at: "payroll provider".into(),
        }),
    )
    .unwrap();
    let old = s.add_party("Old Co", &[Role::Contractor]).unwrap();

    // Sam: last year (not this year's), then cash, card, an owner, a
    // reversed payment, a bill partly paid, and a draft.
    let e = pay(&mut s, d(2025, 12, 15), "Sam, December", cash, 50_000);
    s.set_entry_vendor(e, Some(sam_vendor)).unwrap();
    let e = pay(&mut s, d(2026, 2, 1), "Sam, February", cash, 100_000);
    s.set_entry_vendor(e, Some(sam_vendor)).unwrap();
    let e = pay(&mut s, d(2026, 3, 1), "Sam, by card", card, 20_000);
    s.set_entry_party(e, Some(sam)).unwrap();
    let e = pay(&mut s, d(2026, 4, 1), "Sam, paid by Stephen", owed, 30_000);
    s.set_entry_vendor(e, Some(sam_vendor)).unwrap();
    let e = pay(&mut s, d(2026, 5, 1), "Sam, twice by mistake", cash, 25_000);
    s.set_entry_vendor(e, Some(sam_vendor)).unwrap();
    let reversal = s.reverse_entry(e, d(2026, 5, 2), "Paid twice").unwrap();
    let bill = s
        .create_bill(sam_vendor, d(2026, 6, 1), d(2026, 7, 1), doc(80_000))
        .unwrap();
    let bill_entry = s.ledger().get_bill(bill).unwrap().entry_id;
    s.pay_bill(bill, d(2026, 6, 15), cents(50_000), cash)
        .unwrap();
    let draft = s.create_entry(d(2026, 10, 1), "Sam, October").unwrap();
    s.add_line(draft, labor, Side::Debit, usd(9_900)).unwrap();
    s.add_line(draft, cash, Side::Credit, usd(9_900)).unwrap();
    s.set_entry_party(draft, Some(sam)).unwrap();

    // Riley, by cash, tagged by party.
    let e = pay(&mut s, d(2026, 7, 1), "Riley, July", cash, 70_000);
    s.set_entry_party(e, Some(riley)).unwrap();

    // Acme: last year's invoice still open, this year's partly paid.
    s.create_invoice(acme_c, d(2025, 12, 1), d(2025, 12, 31), doc(40_000))
        .unwrap();
    let inv = s
        .create_invoice(acme_c, d(2026, 2, 10), d(2026, 3, 10), doc(100_000))
        .unwrap();
    let invoice_entry = s.ledger().get_invoice(inv).unwrap().entry_id;
    s.receive_payment(inv, d(2026, 3, 1), cents(60_000), cash)
        .unwrap();

    // Gumroad's payout: net to cash, fee expensed, gross revenue.
    let e = s.create_entry(d(2026, 8, 1), "Gumroad payout").unwrap();
    s.add_line(e, cash, Side::Debit, usd(9_000)).unwrap();
    s.add_line(e, fees, Side::Debit, usd(1_000)).unwrap();
    s.add_line(e, sales, Side::Credit, usd(10_000)).unwrap();
    s.post_entry(e).unwrap();
    s.set_entry_party(e, Some(gumroad)).unwrap();

    // Payroll: Pat by party, then an older-style line by name; the
    // remittance goes to the IRS.
    let line = |employee: Option<PartyId>| PayrollLine {
        employee_name: "Pat Lee".into(),
        employee,
        gross_pay: cents(100_000),
        federal_withholding: cents(10_000),
        state_withholding: cents(3_000),
        ss_employee: cents(6_200),
        medicare_employee: cents(1_450),
        ss_employer: cents(6_200),
        medicare_employer: cents(1_450),
        other_deductions: Decimal::ZERO,
    };
    s.run_payroll(d(2026, 8, 15), vec![line(Some(pat))], cash)
        .unwrap();
    s.run_payroll(d(2026, 8, 31), vec![line(None)], cash)
        .unwrap();
    let remit = s.remit_payroll_taxes(d(2026, 9, 5), cash).unwrap();
    s.set_entry_party(remit, Some(irs)).unwrap();

    // Old Co: paid, then archived.
    let e = pay(&mut s, d(2026, 1, 20), "Old Co", cash, 12_300);
    s.set_entry_party(e, Some(old)).unwrap();
    s.update_party(old, PartyField::Archived(true)).unwrap();
    drop(s);
    let oracle = Store::open(&data).unwrap().ledger().clone();
    Fixture {
        books: load(&dir.0),
        dir,
        oracle,
        sam,
        sam_vendor,
        riley,
        acme,
        gumroad,
        irs,
        pat,
        old,
        bill_entry,
        invoice_entry,
        reversal,
        draft,
        remit,
    }
}

#[test]
fn each_partys_totals_are_the_books_numbers() {
    let f = fixture("totals");
    let b = &f.books;
    let o = &f.oracle;
    let info = |p| b.party(p).cloned().unwrap();

    // Sam: this year's cash 1,000 + card 200 + owner 300 + (250 − 250
    // reversed) + 500 of the bill paid. Last year's 500, the bill itself,
    // and the draft move nothing this year.
    let sam = info(f.sam);
    assert_eq!(sam.paid_ytd, cents(200_000));
    let bill = o.get_bill(1).unwrap();
    assert_eq!(sam.billed_ytd, bill.total());
    assert_eq!(sam.open_bills, bill.balance());
    assert_eq!(sam.open_bills, cents(30_000));
    assert_eq!(sam.bills, vec![1]);
    // The 1099 amount is ledger-core's worksheet line, exactly.
    let r = vendor_1099_report(o, 2026, None).unwrap();
    let line = r
        .vendors
        .iter()
        .find(|v| v.vendor_id == f.sam_vendor)
        .unwrap();
    assert_eq!(sam.form_1099.as_ref(), Some(line));
    assert_eq!(sam.threshold_1099, Some(r.threshold));
    // (Cash only: card and owner-paid aren't in it.)
    assert_eq!(line.paid, cents(150_000));
    // Every entry tagged, drafts included, newest first.
    assert!(sam.entries.contains(&f.bill_entry));
    assert!(sam.entries.contains(&f.reversal));
    assert!(sam.entries.contains(&f.draft));
    assert_eq!(sam.entries.len(), 9, "{:?}", sam.entries);
    let dates: Vec<NaiveDate> = sam
        .entries
        .iter()
        .map(|e| b.entry(*e).unwrap().date)
        .collect();
    assert!(dates.windows(2).all(|w| w[0] >= w[1]), "{dates:?}");

    // Riley: 700 by cash.
    assert_eq!(info(f.riley).paid_ytd, cents(70_000));

    // Acme: received 600 on net (negative paid); invoiced this year 1,000;
    // still owes last year's 400 and this year's 400.
    let acme = info(f.acme);
    assert_eq!(acme.paid_ytd, cents(-60_000));
    let invoices: Vec<_> = (1..o.next_invoice_id())
        .filter_map(|i| o.get_invoice(i))
        .collect();
    assert_eq!(
        acme.invoiced_ytd,
        invoices
            .iter()
            .filter(|i| i.date >= d(2026, 1, 1))
            .map(|i| i.total())
            .sum::<Decimal>()
    );
    assert_eq!(
        acme.open_invoices,
        invoices.iter().map(|i| i.balance()).sum::<Decimal>()
    );
    assert_eq!(acme.open_invoices, cents(80_000));
    assert_eq!(acme.invoices.len(), 2);
    assert!(acme.form_1099.is_none());

    // Gumroad: the net deposit came in.
    assert_eq!(info(f.gumroad).paid_ytd, cents(-9_000));

    // The IRS: the remittance's cash, from its own lines.
    let remit = o.get_entry(f.remit).unwrap();
    let cash = o.chart().find_by_code("1000").unwrap().id;
    let out: Decimal = remit
        .lines()
        .iter()
        .filter(|l| l.account == cash && l.side == Side::Credit)
        .map(|l| l.amount.amount)
        .sum();
    assert!(out > Decimal::ZERO);
    assert_eq!(info(f.irs).paid_ytd, out);

    // Pat: both runs (one by party, one by name), no tagged entries.
    let pat = info(f.pat);
    assert_eq!(pat.payroll_runs.len(), 2);
    assert!(pat.entries.is_empty());
    assert_eq!(pat.paid_ytd, Decimal::ZERO);

    // Every entry's party is the one whose list holds it.
    for e in &b.entries {
        if let Some(p) = e.party_id {
            assert!(info(p).entries.contains(&e.id), "#{}", e.id);
            assert_eq!(e.party.as_deref(), Some(b.party_label(p).as_str()));
        }
    }
    assert_eq!(b.entry(f.bill_entry).unwrap().party_id, Some(f.sam));
    assert_eq!(b.entry(f.invoice_entry).unwrap().party_id, Some(f.acme));
    assert_eq!(b.entry(f.reversal).unwrap().party_id, Some(f.sam));
}

#[test]
fn the_directory_lists_filters_and_hides_archived() {
    let f = fixture("list");
    let b = &f.books;
    let names = |st: &State| -> Vec<String> {
        listed(b, st)
            .iter()
            .map(|p| p.label().to_string())
            .collect()
    };
    let mut st = State::default();
    assert_eq!(
        names(&st),
        vec![
            "Acme Buyer",
            "Gumroad",
            "Internal Revenue Service",
            "Pat Lee",
            "Riley Chen",
            "Sam Okafor"
        ]
    );
    st.show_archived = true;
    assert!(names(&st).contains(&"Old Co".to_string()));
    st.role = Some(Role::Contractor);
    assert_eq!(names(&st), vec!["Old Co", "Riley Chen", "Sam Okafor"]);
    st.show_archived = false;
    st.query = "  sam ".into();
    assert_eq!(names(&st), vec!["Sam Okafor"]);
    st.role = None;
    st.query = "gumroad inc".into();
    assert_eq!(names(&st), vec!["Gumroad"]);
    let counts = role_counts(b, false);
    assert!(counts.contains(&(Role::Contractor, 2)));
    assert!(counts.contains(&(Role::Vendor, 1)));
    assert!(!counts.iter().any(|(r, _)| *r == Role::Landlord));
    let sam = b.ledger.get_party(f.sam).unwrap();
    assert_eq!(roles_text(sam), "Vendor, contractor");
    assert_eq!(w9_status(b, sam), W9Status::Missing);
    assert_eq!(
        w9_status(b, b.ledger.get_party(f.gumroad).unwrap()),
        W9Status::NotNeeded
    );
    assert_eq!(masked_tin("1234"), "•••-••-1234");
}

#[test]
fn search_finds_parties_by_name() {
    let f = fixture("search");
    let b = &f.books;
    for q in ["Gumroad", "gumroad, inc.", " GUMROAD "] {
        let items = palette::search(b, q);
        assert_eq!(items[0].route, Route::Party(f.gumroad), "{q}");
        assert_eq!(items[0].score, 1500, "{q}");
    }
    let first_party = |q: &str| {
        palette::search(b, q)
            .into_iter()
            .find(|i| matches!(i.route, Route::Party(_)))
    };
    assert_eq!(first_party("rile").unwrap().route, Route::Party(f.riley));
    // Vendors and customers are found as their parties.
    assert_eq!(
        palette::search(b, "Sam Okafor")[0].route,
        Route::Party(f.sam)
    );
    assert_eq!(first_party("acme").unwrap().route, Route::Party(f.acme));
    // Archived parties are found, and say so.
    let old = first_party("Old Co").unwrap();
    assert_eq!(old.route, Route::Party(f.old));
    assert!(old.detail.contains("archived"), "{}", old.detail);
    assert_eq!(palette::describe(b, &Route::Party(f.gumroad)), "Gumroad");
    // Transactions search by party name too.
    let sam_entry = b.entry(f.bill_entry).unwrap();
    assert!(sam_entry.haystack.contains("sam okafor"));
}

#[test]
fn routes_and_books_show_open_parties() {
    let f = fixture("routes");
    assert_eq!(Route::Parties.section(), Section::Parties);
    assert_eq!(Route::Party(f.sam).section(), Section::Parties);
    assert_eq!(Section::Parties.home(), Route::Parties);
    assert!(Section::ALL.contains(&Section::Parties));
    let books = load(&f.dir.0);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 920.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, Route::Overview));
    h.state_mut().show(ViewerTarget::Page {
        page: "parties".into(),
    });
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &Route::Parties);
    h.state_mut().show(ViewerTarget::Party {
        party_id: f.gumroad,
    });
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &Route::Party(f.gumroad));
    // An unknown party is ignored.
    h.state_mut().show(ViewerTarget::Party { party_id: 999 });
    h.run_steps(1);
    assert_eq!(h.state_mut().route(), &Route::Party(f.gumroad));
    // Every party's page, and the entry page of a tagged entry, render.
    for p in [f.sam, f.riley, f.acme, f.gumroad, f.irs, f.pat, f.old, 999] {
        h.state_mut().goto(Route::Party(p));
        h.run_steps(2);
    }
    h.state_mut().goto(Route::Entry(f.bill_entry));
    h.run_steps(2);
    h.state_mut().state_mut().parties.show_archived = true;
    h.state_mut().goto(Route::Parties);
    h.run_steps(2);
}

#[test]
fn journal_text_for_party_events() {
    let f = fixture("journal");
    let b = &f.books;
    let texts: Vec<(String, Option<Route>, Option<Route>)> = b
        .journal
        .iter()
        .filter(|l| l.line_type.starts_with("party.") || l.line_type == "entry.party_set")
        .filter_map(|l| describe(b, l))
        .map(|e| (e.text, e.route, e.also))
        .collect();
    let has = |t: &str, route: Option<Route>| {
        assert!(
            texts.iter().any(|(x, r, _)| x == t && *r == route),
            "no {t:?} ({route:?}) in {texts:#?}"
        );
    };
    has(
        "Added party Riley Chen (contractor)",
        Some(Route::Party(f.riley)),
    );
    has(
        "Added party Gumroad, Inc. (payout platform)",
        Some(Route::Party(f.gumroad)),
    );
    has(
        "Display name of Gumroad, Inc. set to Gumroad",
        Some(Route::Party(f.gumroad)),
    );
    has("Sam Okafor is now a contractor", Some(Route::Party(f.sam)));
    has(
        "W-4 for Pat Lee recorded, received 2026-01-02 (works in UT)",
        Some(Route::Party(f.pat)),
    );
    has("Archived Old Co", Some(Route::Party(f.old)));
    let tag = texts
        .iter()
        .find(|(x, _, _)| x.starts_with("Tagged #") && x.ends_with("with Gumroad"))
        .expect("entry.party_set text");
    assert!(matches!(tag.1, Some(Route::Entry(_))));
    assert_eq!(tag.2, Some(Route::Party(f.gumroad)));
    // Party history: its own events and its records'.
    let sam = b.party(f.sam).unwrap();
    let kinds: Vec<&str> = sam
        .events
        .iter()
        .map(|&i| b.journal[i].line_type.as_str())
        .collect();
    for k in [
        "vendor.added",
        "vendor.1099_set",
        "party.role_set",
        "entry.vendor_set",
        "entry.party_set",
    ] {
        assert!(kinds.contains(&k), "{k} not in {kinds:?}");
    }
}

#[test]
fn w9_text_never_shows_more_than_the_last_four() {
    let f = fixture("w9text");
    let mut s = Store::open(&f.dir.0.join("books-data")).unwrap();
    s.set_party_w9(f.riley, Some(w9("6789"))).unwrap();
    s.set_party_w9(f.riley, None).unwrap();
    drop(s);
    let b = load(&f.dir.0);
    let texts: Vec<String> = b
        .journal
        .iter()
        .filter(|l| l.line_type == "party.w9_set")
        .filter_map(|l| describe(&b, l))
        .map(|e| e.text)
        .collect();
    assert_eq!(
        texts,
        vec![
            "W-9 for Riley Chen recorded, received 2026-09-01: Individual/sole proprietor, TIN ending 6789",
            "W-9 facts for Riley Chen cleared",
        ]
    );
}

#[test]
fn attention_asks_for_a_w9_until_its_facts_are_recorded() {
    let f = fixture("attention");
    let items = attention::find(&f.books);
    let item = |name: &str| {
        items
            .iter()
            .find(|a| a.title == format!("Collect a W-9 from {name}"))
    };
    let riley = item("Riley Chen").expect("Riley's W-9 item");
    assert_eq!(riley.target, Some(Route::Party(f.riley)));
    assert!(riley.detail.contains("$700.00"), "{}", riley.detail);
    assert!(riley.prompt.starts_with("In wanderbooks,"));
    assert!(riley.prompt.contains("books.set_w9"));
    assert!(riley.prompt.contains("last four"));
    assert!(item("Sam Okafor").is_some(), "Sam is paid, no W-9");
    assert!(item("Old Co").is_none(), "archived");
    assert!(item("Gumroad").is_none(), "no 1099 role");
    // The existing items are still found.
    assert!(
        items
            .iter()
            .any(|a| a.title == "$300.00 owed on Due to Shareholders"),
        "{:#?}",
        items.iter().map(|a| &a.title).collect::<Vec<_>>()
    );

    // Recording the facts clears it.
    let mut s = Store::open(&f.dir.0.join("books-data")).unwrap();
    s.set_party_w9(f.riley, Some(w9("1234"))).unwrap();
    drop(s);
    let b = load(&f.dir.0);
    let items = attention::find(&b);
    assert!(
        !items
            .iter()
            .any(|a| a.title == "Collect a W-9 from Riley Chen")
    );
    assert!(
        items
            .iter()
            .any(|a| a.title == "Collect a W-9 from Sam Okafor")
    );
    assert_eq!(
        w9_status(&b, b.ledger.get_party(f.riley).unwrap()),
        W9Status::OnFile {
            last4: "1234".into(),
            received: d(2026, 9, 1)
        }
    );
}
