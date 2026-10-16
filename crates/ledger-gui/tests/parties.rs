//! Parties in the viewer: each party's totals are the books' numbers
//! (ledger-core's records and 1099 worksheet, summed exactly as
//! `PartyInfo` defines); search, books.show, the journal, the entry page,
//! and "Needs attention" know parties.

use chrono::NaiveDate;
use egui_kittest::Harness;
use ledger_core::arap::DocumentLine;
use ledger_core::{
    AccountType, Currency, EntryId, Form1099, Money, PartyField, PartyId, PayrollLine, Role, Side,
    TinFacts, TinType, W4Facts, W9Facts, vendor_1099_report,
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
    let e = pay(&mut s, d(2026, 4, 1), "Sam, paid by Tobin", owed, 30_000);
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
    assert_eq!(sam.billed_ytd, bill.total().ok());
    assert_eq!(sam.open_bills, bill.balance().ok());
    assert_eq!(sam.open_bills, Some(cents(30_000)));
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
            .map(|i| i.total().unwrap())
            .sum::<Decimal>()
            .into()
    );
    assert_eq!(
        acme.open_invoices,
        Some(
            invoices
                .iter()
                .map(|i| i.balance().unwrap())
                .sum::<Decimal>()
        )
    );
    assert_eq!(acme.open_invoices, Some(cents(80_000)));
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

    // Pat: both runs (one by party, one by name). The run by party names
    // Pat on its entry (Ledger::run_payroll INVARIANT 3): its net pay,
    // 1,000.00 − 100.00 − 30.00 − 62.00 − 14.50 = 793.50, paid to Pat;
    // the run by name only names no one.
    let pat = info(f.pat);
    assert_eq!(pat.payroll_runs.len(), 2);
    assert_eq!(pat.entries.len(), 1);
    assert_eq!(pat.paid_ytd, cents(79_350));

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

/// pages/journal.rs `party.tin_set`: recorded, purged, and cleared, each
/// in plain language with the TIN's type and last four, never more of it.
#[test]
fn tin_text_never_shows_more_than_the_last_four() {
    let f = fixture("tintext");
    let mut s = Store::open(&f.dir.0.join("books-data")).unwrap();
    let facts = |tin_type, purged: Option<&str>| TinFacts {
        tin_type,
        last4: "4915".into(),
        vault_record: purged.is_none().then(|| "ab".repeat(32)),
        purged: purged.map(str::to_string),
    };
    s.set_party_tin(f.pat, Some(facts(TinType::Ssn, None)))
        .unwrap();
    s.set_party_tin(
        f.pat,
        Some(facts(
            TinType::Ssn,
            Some("purged from the wanderbooks vault on 2026-10-15: retention over"),
        )),
    )
    .unwrap();
    s.set_party_tin(f.pat, None).unwrap();
    s.set_party_tin(f.pat, Some(facts(TinType::Itin, None)))
        .unwrap();
    drop(s);
    let b = load(&f.dir.0);
    let events: Vec<_> = b
        .journal
        .iter()
        .filter(|l| l.line_type == "party.tin_set")
        .filter_map(|l| describe(&b, l))
        .collect();
    let texts: Vec<&str> = events.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(
        texts,
        vec![
            "Taxpayer ID for Pat Lee recorded: SSN ending 4915, kept in the vault",
            "Purged Pat Lee's SSN (••••4915) from the vault",
            "Taxpayer ID facts for Pat Lee cleared",
            "Taxpayer ID for Pat Lee recorded: ITIN ending 4915, kept in the vault",
        ]
    );
    assert!(events.iter().all(|e| e.route == Some(Route::Party(f.pat))));
    // No more than four digits in any of them.
    for t in &texts {
        assert!(t.chars().filter(|c| c.is_ascii_digit()).count() <= 4, "{t}");
    }
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

/// Paid out, received in, net, and what for, over each period, worked by
/// hand from the fixture; the breakdown and the months sum to the money
/// moved, exactly.
#[test]
fn party_money_by_period_and_what_for() {
    use ledger_gui::books::PartyPeriod;
    let f = fixture("periods");
    let b = &f.books;
    let o = &f.oracle;
    let code = |c: &str| o.chart().find_by_code(c).unwrap().id;
    let labor = code("6050");
    let payables = o.role_account(&ledger_core::PAYABLES).unwrap();
    let receivables = o.role_account(&ledger_core::RECEIVABLES).unwrap();
    let flow = |p, period| b.party_flow(p, b.party_span(period), true);

    // The spans.
    assert_eq!(
        b.party_span(PartyPeriod::ThisYear),
        (d(2026, 1, 1), today())
    );
    assert_eq!(
        b.party_span(PartyPeriod::LastYear),
        (d(2025, 1, 1), d(2025, 12, 31))
    );
    assert_eq!(b.party_span(PartyPeriod::AllTime), (d(2025, 1, 1), today()));
    assert_eq!(
        b.party_periods(Some(f.sam)),
        vec![
            PartyPeriod::ThisYear,
            PartyPeriod::LastYear,
            PartyPeriod::AllTime
        ]
    );

    // Sam this year: cash 1,000 + card 200 + owner 300 + (250 − 250
    // reversed) + 500 of the 800 bill paid = 2,000 out. What for: labor
    // 1,000 + 200 + 300 + 250 − 250 = 1,500, the bill's 800 (general
    // expenses, the bill's account), less the 300 still owed on the bill
    // (payables −800 + 500).
    let sam = flow(f.sam, PartyPeriod::ThisYear);
    assert_eq!((sam.paid, sam.received), (cents(200_000), Decimal::ZERO));
    assert_eq!(sam.net(), cents(200_000));
    assert_eq!(
        sam.what_for,
        vec![
            (labor, cents(150_000)),
            (code("6999"), cents(80_000)),
            (payables, cents(-30_000))
        ]
    );
    // Feb, Mar card, Apr owner, May and its reversal, the bill, its
    // payment (the draft isn't posted).
    assert_eq!(sam.entries, 7);
    // Last year: December's 500.
    let last = flow(f.sam, PartyPeriod::LastYear);
    assert_eq!((last.paid, last.received), (cents(50_000), Decimal::ZERO));
    assert_eq!(last.what_for, vec![(labor, cents(50_000))]);
    assert_eq!(last.entries, 1);
    // All time: both.
    let all = flow(f.sam, PartyPeriod::AllTime);
    assert_eq!(all.paid, cents(250_000));
    assert_eq!(all.entries, 8);

    // Acme this year: 600 received on the 1,000 invoice. What for: the
    // revenue (1,000 in), less the 400 still receivable.
    let acme = flow(f.acme, PartyPeriod::ThisYear);
    assert_eq!((acme.paid, acme.received), (Decimal::ZERO, cents(60_000)));
    assert_eq!(acme.net(), cents(-60_000));
    let revenue = acme
        .what_for
        .iter()
        .find(|(a, _)| *a != receivables)
        .unwrap();
    assert_eq!(revenue.1, cents(-100_000));
    assert!(acme.what_for.contains(&(receivables, cents(40_000))));
    // Last year: invoiced 400, nothing moved; what for nets to zero.
    let acme_last = flow(f.acme, PartyPeriod::LastYear);
    assert_eq!(
        (acme_last.paid, acme_last.received),
        (Decimal::ZERO, Decimal::ZERO)
    );
    assert_eq!(
        acme_last.what_for.iter().map(|x| x.1).sum::<Decimal>(),
        Decimal::ZERO
    );

    // Gumroad: the 90 deposit is 100 of sales less a 10 fee.
    let g = flow(f.gumroad, PartyPeriod::ThisYear);
    assert_eq!(g.received, cents(9_000));
    assert_eq!(
        g.what_for,
        vec![(code("4100"), cents(-10_000)), (code("6000"), cents(1_000))]
    );

    // Every party, every period: the breakdown sums to the net, and the
    // months to paid and received, exactly; the totals are activity's.
    for p in o.parties() {
        for period in b.party_periods(Some(p.id)) {
            let span = b.party_span(period);
            let fl = b.party_flow(p.id, span, true);
            let a = ledger_core::parties::activity(o, p.id, span, &b.cash_accounts);
            assert_eq!((fl.paid, fl.received), (a.paid, a.received));
            assert_eq!(
                fl.what_for.iter().map(|x| x.1).sum::<Decimal>(),
                fl.net(),
                "{} {period:?}",
                p.name
            );
            let months = b.party_months(p.id, span);
            assert_eq!(months.iter().map(|m| m.paid).sum::<Decimal>(), fl.paid);
            assert_eq!(
                months.iter().map(|m| m.received).sum::<Decimal>(),
                fl.received
            );
        }
    }
    // Sam's months this year: Feb 1,000, Mar 200, Apr 300, May 0 (paid
    // and reversed), Jun 500.
    let months = b.party_months(f.sam, b.party_span(PartyPeriod::ThisYear));
    assert_eq!(months.len(), 10);
    let paid: Vec<Decimal> = months.iter().map(|m| m.paid).collect();
    assert_eq!(
        &paid[..6],
        &[
            Decimal::ZERO,
            cents(100_000),
            cents(20_000),
            cents(30_000),
            Decimal::ZERO,
            cents(50_000)
        ]
    );

    // The 1099 amount follows the period's calendar year.
    let (line, _) = b.party_1099(f.sam, 2025).unwrap();
    assert_eq!(line.paid, cents(50_000));
    let (line, _) = b.party_1099(f.sam, 2026).unwrap();
    assert_eq!(line.paid, cents(150_000));
}

#[test]
fn directory_sorts_by_the_periods_amounts() {
    use ledger_gui::books::PartyPeriod;
    use ledger_gui::pages::parties::{Sort, sorted};
    let f = fixture("sorted");
    let b = &f.books;
    let names =
        |st: &State| -> Vec<PartyId> { sorted(b, st).into_iter().map(|(p, _)| p.id).collect() };
    let mut st = State::default();
    assert_eq!(
        names(&st),
        listed(b, &st).into_iter().map(|p| p.id).collect::<Vec<_>>()
    );
    st.sort = Sort::PaidOut;
    assert_eq!(names(&st)[0], f.sam);
    st.sort = Sort::ReceivedIn;
    assert_eq!(&names(&st)[..2], &[f.acme, f.gumroad]);
    st.sort = Sort::Net;
    assert_eq!(names(&st)[0], f.sam);
    assert_eq!(*names(&st).last().unwrap(), f.acme);
    // Last year, only Sam was paid.
    st.period = PartyPeriod::LastYear;
    st.sort = Sort::PaidOut;
    let rows = sorted(b, &st);
    assert_eq!(rows[0].0.id, f.sam);
    assert_eq!(rows[0].1.paid, cents(50_000));
    assert!(rows[1..].iter().all(|(_, fl)| fl.paid.is_zero()));
}

/// Books with three posted entries this year: one tagged with a party,
/// one marked as having none, one neither; under `parties` = `policy`
/// (None: unset).
fn counterparty_books(
    tag: &str,
    policy: Option<&str>,
) -> (TempDir, Store, EntryId, EntryId, EntryId) {
    let dir = TempDir::new(tag);
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let savings = s
        .add_account("1010", "Savings", AccountType::Asset, None, false)
        .unwrap();
    let labor = s
        .add_account("6050", "Contract Labor", AccountType::Expense, None, false)
        .unwrap();
    let riley = s.add_party("Riley Chen", &[Role::Contractor]).unwrap();
    let post = |s: &mut Store, date, what: &str, dr, cr| {
        let e = s.create_entry(date, what).unwrap();
        s.add_line(e, dr, Side::Debit, usd(10_000)).unwrap();
        s.add_line(e, cr, Side::Credit, usd(10_000)).unwrap();
        s.attach_receipt(Some(e), "r.pdf", "application/pdf", what.as_bytes(), None)
            .unwrap();
        s.post_entry(e).unwrap();
        e
    };
    let tagged = post(&mut s, d(2026, 3, 1), "Riley", labor, cash);
    s.set_entry_party(tagged, Some(riley)).unwrap();
    let transfer = post(&mut s, d(2026, 4, 1), "To savings", savings, cash);
    s.set_entry_no_party(transfer, "Between the company's own accounts")
        .unwrap();
    let unnamed = post(&mut s, d(2026, 5, 1), "Mystery payment", labor, cash);
    if let Some(p) = policy {
        std::fs::write(
            dir.0.join("wanderbooks.toml"),
            format!("[books]\nparties = \"{p}\"\n"),
        )
        .unwrap();
    }
    (dir, s, tagged, transfer, unnamed)
}

fn load_with_toml(dir: &Path) -> Books {
    data::load_once(&Source {
        data_dir: dir.join("books-data"),
        books_toml: Some(dir.join("wanderbooks.toml")),
        today: Some(today()),
    })
    .unwrap()
}

#[test]
fn entries_without_a_counterparty_need_attention_unless_off() {
    use ledger_gui::books::Severity;
    let item = |b: &Books| {
        attention::find(b)
            .into_iter()
            .find(|a| a.title.ends_with("no counterparty"))
    };
    // Unset is suggest: one note naming just the unmarked entry.
    let (dir, mut s, _, transfer, unnamed) = counterparty_books("np-suggest", None);
    let b = load_with_toml(&dir.0);
    let a = item(&b).expect("an item");
    assert_eq!(a.title, "1 entry names no counterparty");
    assert_eq!(a.severity, Severity::Info);
    assert!(a.prompt.starts_with("In wanderbooks,"));
    assert!(a.prompt.contains("books.set_entry_party"));
    assert!(
        a.prompt
            .contains(&format!("{unnamed} (Mystery payment, 2026-05-01)"))
    );
    assert!(!a.prompt.contains(&format!("{transfer} (")));
    // The mark shows on the entry and reads in the journal.
    assert_eq!(
        b.ledger.entry_no_party(transfer),
        Some("Between the company's own accounts")
    );
    let line = b
        .journal
        .iter()
        .find(|l| l.line_type == "entry.no_party")
        .unwrap();
    let ev = describe(&b, line).unwrap();
    assert_eq!(
        ev.text,
        format!(
            "Entry #{transfer} marked as having no counterparty: Between the company's own accounts"
        )
    );
    assert_eq!(ev.route, Some(Route::Entry(transfer)));
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, b, false, Route::Entry(transfer)));
    h.run_steps(3);
    use egui_kittest::kittest::Queryable;
    h.get_by_label(&format!(
        "{} No counterparty: Between the company's own accounts",
        egui_phosphor::regular::USER_MINUS
    ));
    // Tagging the last one clears it.
    let p = s.add_party("Mystery Co", &[Role::Contractor]).unwrap();
    s.set_entry_party(unnamed, Some(p)).unwrap();
    assert!(item(&load_with_toml(&dir.0)).is_none());

    // Require: a to-do.
    let (dir, _s, ..) = counterparty_books("np-require", Some("require"));
    assert_eq!(
        item(&load_with_toml(&dir.0)).unwrap().severity,
        Severity::Todo
    );
    // Off: never mentioned.
    let (dir, _s, ..) = counterparty_books("np-off", Some("off"));
    assert!(item(&load_with_toml(&dir.0)).is_none());
}

// ------------------------------------------------------- party shares and the capital statement
//
// Oracles: EntryParty (resolved shares), PartyFlow (each party's share),
// CapitalStatement (INVARIANTS 1–6), ledger_core::k1, docs/parties-and-attachments.md
// "Shares" and "A member's capital statement". Values worked by hand.

struct MemberBooks {
    dir: TempDir,
    tobin: PartyId,
    mo: PartyId,
    reimburse: EntryId,
    property: EntryId,
    distribution: EntryId,
}

/// Two 50/50 members. 2025: a joint cash contribution of $1,000.00 each.
/// 2026: equipment contributed ($600.00 each, Feb), an expense paid from
/// joint funds ($300.00 on 2100, Mar), revenue $1,000.00 (Apr), $100.00
/// of 2100 reimbursed to both (May), $200.00 distributed to each (Jun),
/// and Jan–Jun closed to the members ($700.00 net: $350.00 each).
fn member_books(tag: &str) -> MemberBooks {
    let dir = TempDir::new(tag);
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    let acct = |s: &mut Store, c: &str, n: &str, t| s.add_account(c, n, t, None, false).unwrap();
    let bank = acct(&mut s, "1000", "Bluevine", AccountType::Asset);
    let equip = acct(&mut s, "1500", "Equipment", AccountType::Asset);
    let due = acct(
        &mut s,
        "2100",
        "Due to Shareholders",
        AccountType::Liability,
    );
    let cap_s = acct(
        &mut s,
        "3000",
        "Owner Capital — Tobin Harlow",
        AccountType::Equity,
    );
    let cap_k = acct(
        &mut s,
        "3001",
        "Owner Capital — Morgan Ellis",
        AccountType::Equity,
    );
    let rev = acct(&mut s, "4000", "App Revenue", AccountType::Revenue);
    let exp = acct(&mut s, "6050", "Domain & Hosting", AccountType::Expense);
    let tobin = s.add_party("Tobin Harlow", &[Role::Member]).unwrap();
    let mo = s
        .add_party("Morgan Ellis", &[Role::Member])
        .unwrap();
    let both = [
        ledger_core::PartyShare::whole(tobin),
        ledger_core::PartyShare::whole(mo),
    ];
    let post = |s: &mut Store, date, what: &str, lines: &[(u64, Side, i64)]| {
        let e = s.create_entry(date, what).unwrap();
        for &(a, side, c) in lines {
            s.add_line(e, a, side, usd(c)).unwrap();
        }
        s.post_entry(e).unwrap();
        e
    };
    let open = post(
        &mut s,
        d(2025, 6, 1),
        "Opening contribution",
        &[
            (bank, Side::Debit, 200_000),
            (cap_s, Side::Credit, 100_000),
            (cap_k, Side::Credit, 100_000),
        ],
    );
    s.set_entry_parties(open, &both).unwrap();
    let property = post(
        &mut s,
        d(2026, 2, 1),
        "Laptop contributed",
        &[
            (equip, Side::Debit, 120_000),
            (cap_s, Side::Credit, 60_000),
            (cap_k, Side::Credit, 60_000),
        ],
    );
    s.set_entry_parties(property, &both).unwrap();
    post(
        &mut s,
        d(2026, 3, 1),
        "Hosting from joint funds",
        &[(exp, Side::Debit, 30_000), (due, Side::Credit, 30_000)],
    );
    post(
        &mut s,
        d(2026, 4, 1),
        "Sales",
        &[(bank, Side::Debit, 100_000), (rev, Side::Credit, 100_000)],
    );
    let reimburse = post(
        &mut s,
        d(2026, 5, 1),
        "Reimburse the household",
        &[(due, Side::Debit, 10_000), (bank, Side::Credit, 10_000)],
    );
    s.set_entry_parties(reimburse, &both).unwrap();
    let distribution = post(
        &mut s,
        d(2026, 6, 1),
        "Distribution",
        &[
            (cap_s, Side::Debit, 20_000),
            (cap_k, Side::Debit, 20_000),
            (bank, Side::Credit, 40_000),
        ],
    );
    s.set_entry_parties(distribution, &both).unwrap();
    let period = s
        .create_period("H1 2026", d(2026, 1, 1), d(2026, 6, 30))
        .unwrap();
    let half = Decimal::new(5, 1);
    s.close_period_to(
        period,
        &ledger_core::periods::CloseTo::Members(vec![
            ledger_core::MemberClose {
                capital: cap_s,
                share: half,
                distributions: None,
            },
            ledger_core::MemberClose {
                capital: cap_k,
                share: half,
                distributions: None,
            },
        ]),
    )
    .unwrap();
    write_member_toml(&dir.0, "cash_accounts = [\"1000\"]\n");
    MemberBooks {
        dir,
        tobin,
        mo,
        reimburse,
        property,
        distribution,
    }
}

/// The fixture's wanderbooks.toml, with `books` as its `[books]` table.
fn write_member_toml(dir: &Path, books: &str) {
    std::fs::write(
        dir.join("wanderbooks.toml"),
        format!(
            "[books]\n{books}[company]\nname = \"Fernhill Labs LLC\"\n\
             [[company.members]]\nname = \"Tobin Harlow\"\nshare = \"50%\"\ncapital_account = \"3000\"\n\
             [[company.members]]\nname = \"Morgan Ellis\"\nshare = \"50%\"\ncapital_account = \"3001\"\n"
        ),
    )
    .unwrap();
}

/// Post one entry to the fixture's books after the fact.
fn post_more(m: &MemberBooks, date: NaiveDate, lines: &[(&str, Side, i64)]) -> EntryId {
    let mut s = Store::open(&m.dir.0.join("books-data")).unwrap();
    let e = s.create_entry(date, "Later").unwrap();
    for &(code, side, c) in lines {
        let a = s.ledger().chart().find_by_code(code).unwrap().id;
        s.add_line(e, a, side, usd(c)).unwrap();
    }
    s.post_entry(e).unwrap();
    e
}

#[test]
fn entries_list_every_party_with_its_resolved_share() {
    let m = member_books("shares-entries");
    let b = load_with_toml(&m.dir.0);
    let e = b.entry(m.reimburse).unwrap();
    assert_eq!(e.party_id, None, "two parties: no sole party");
    assert_eq!(
        e.parties
            .iter()
            .map(|p| (p.id, p.resolved))
            .collect::<Vec<_>>(),
        vec![(m.tobin, Some(cents(5_000))), (m.mo, Some(cents(5_000)))]
    );
    assert_eq!(
        e.party.as_deref(),
        Some("Tobin Harlow & Morgan Ellis")
    );
    // Property: net zero, shares of the total.
    let e = b.entry(m.property).unwrap();
    assert_eq!(e.parties[1].resolved, Some(cents(60_000)));
    // The journal says who, for parties_set.
    let line = b
        .journal
        .iter()
        .find(|l| l.line_type == "entry.parties_set")
        .unwrap();
    let ev = describe(&b, line).unwrap();
    assert!(
        ev.text.contains("Tobin Harlow and Morgan Ellis"),
        "{}",
        ev.text
    );
}

#[test]
fn a_members_money_is_their_share() {
    let m = member_books("shares-flow");
    let b = load_with_toml(&m.dir.0);
    let span = (d(2026, 1, 1), today());
    for who in [m.tobin, m.mo] {
        let f = b.party_flow(who, span, true);
        // Half the reimbursement plus their own distribution; the
        // property moved no money.
        assert_eq!(f.paid, cents(25_000));
        assert_eq!(f.received, Decimal::ZERO);
        let what: Decimal = f.what_for.iter().map(|(_, v)| *v).sum();
        assert_eq!(what, f.net(), "what for sums to the net");
        assert!(b.party(who).unwrap().entries.contains(&m.distribution));
    }
    assert_eq!(b.party(m.mo).unwrap().paid_ytd, cents(25_000));
}

#[test]
fn a_members_page_is_their_capital_statement() {
    let m = member_books("capital");
    let b = load_with_toml(&m.dir.0);
    let st = b
        .capital_statement(m.tobin, (d(2026, 1, 1), today()))
        .expect("a member");
    assert_eq!(st.share, Decimal::new(5, 1));
    assert_eq!(st.opening, cents(100_000));
    assert_eq!(st.contributions_cash, Decimal::ZERO);
    assert_eq!(st.contributions_property, cents(60_000));
    assert_eq!(st.income_share, cents(35_000));
    assert_eq!(st.distributions_taken, cents(20_000));
    assert_eq!(st.closing, cents(175_000));
    assert_eq!(
        st.closing,
        st.opening + st.contributions_cash + st.contributions_property + st.income_share
            - st.distributions_taken
    );
    assert_eq!(st.other, Decimal::ZERO);
    // No [books] financing_accounts: 2100's name says "due to"; $300.00
    // advanced less $100.00 reimbursed, half each.
    assert_eq!(st.due_to_members, cents(20_000));
    assert_eq!(st.owed_to_member, cents(10_000));
    // The lines behind it (k1_detail INVARIANT 5), in journal order.
    use ledger_gui::books::CapitalKind as K;
    let kinds: Vec<(EntryId, K, Decimal)> = st
        .lines
        .iter()
        .map(|l| (l.entry, l.kind, l.amount))
        .collect();
    assert_eq!(kinds.len(), 3);
    assert_eq!(
        kinds[0],
        (m.property, K::PropertyContribution, cents(60_000))
    );
    assert_eq!(kinds[1], (m.distribution, K::Distribution, cents(-20_000)));
    assert_eq!((kinds[2].1, kinds[2].2), (K::IncomeShare, cents(35_000)));
    // Over all time: the 2025 contribution is cash (1000 is
    // [books] cash_accounts), the laptop property.
    let all = b
        .capital_statement(m.mo, (d(2025, 1, 1), today()))
        .unwrap();
    assert_eq!(all.opening, Decimal::ZERO);
    assert_eq!(all.contributions_cash, cents(100_000));
    assert_eq!(all.contributions_property, cents(60_000));
    assert_eq!(all.income_share, cents(35_000));
    assert_eq!(all.distributions_taken, cents(20_000));
    assert_eq!(all.closing, cents(175_000));
    // Not a member: no statement.
    let mut s = Store::open(&m.dir.0.join("books-data")).unwrap();
    let other = s.add_party("Riley Chen", &[Role::Contractor]).unwrap();
    drop(s);
    let b = load_with_toml(&m.dir.0);
    assert!(
        b.capital_statement(other, (d(2026, 1, 1), today()))
            .is_none()
    );
    // The page shows it.
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 1400.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, b, false, Route::Party(m.tobin)));
    h.run_steps(3);
    use egui_kittest::kittest::Queryable;
    h.get_by_label("Capital account");
    h.get_by_label("Contributions of property (at fair market value)");
}

/// The viewer's statement is the K-1 worksheet's row, field for field,
/// over any span (CapitalStatement INVARIANT 1): ledger_core::k1 called
/// here with the members and cash accounts as the CPA package builds them.
#[test]
fn the_capital_statement_is_the_k1_worksheet_row() {
    let m = member_books("capital-k1");
    // Income not yet closed, so beginnings carry undistributed earnings.
    post_more(
        &m,
        d(2026, 7, 1),
        &[
            ("1000", Side::Debit, 10_000),
            ("4000", Side::Credit, 10_000),
        ],
    );
    for (books, fallback) in [("cash_accounts = [\"1000\"]\n", false), ("", true)] {
        write_member_toml(&m.dir.0, books);
        let b = load_with_toml(&m.dir.0);
        let chart = b.ledger.chart();
        let id = |c: &str| chart.find_by_code(c).unwrap().id;
        let members =
            [("Tobin Harlow", "3000"), ("Morgan Ellis", "3001")].map(|(name, code)| {
                ledger_core::MemberCapital {
                    name: name.into(),
                    capital_account: id(code),
                    distribution_account: None,
                    share: Decimal::new(5, 1),
                }
            });
        let cash: Vec<_> = if fallback {
            let mut all: Vec<_> = ledger_core::parties::money_accounts(&b.ledger, &[])
                .cash
                .into_iter()
                .collect();
            all.sort_unstable();
            all
        } else {
            vec![id("1000")]
        };
        for span in [
            (d(2025, 1, 1), today()),
            (d(2026, 1, 1), today()),
            (d(2026, 2, 1), d(2026, 5, 31)),
            (d(2026, 7, 1), today()),
            (d(2026, 8, 1), today()),
        ] {
            let rows =
                ledger_core::k1::k1_worksheet(&b.ledger, span.0, span.1, &members, &cash).unwrap();
            for (who, row) in [m.tobin, m.mo].into_iter().zip(&rows) {
                let st = b.capital_statement(who, span).unwrap();
                assert_eq!(
                    (
                        st.share,
                        st.opening,
                        st.contributions_cash,
                        st.contributions_property,
                        st.income_share,
                        st.distributions_taken,
                        st.other,
                        st.closing,
                    ),
                    (
                        row.share,
                        row.beginning,
                        row.contributions_cash,
                        row.contributions_property,
                        row.net_income,
                        row.distributions,
                        row.other,
                        row.ending,
                    ),
                    "{} over {span:?} (fallback {fallback})",
                    row.name
                );
                assert_eq!(
                    st.closing,
                    st.opening
                        + st.contributions_cash
                        + st.contributions_property
                        + st.income_share
                        - st.distributions_taken
                        + st.other
                );
            }
        }
    }
}

/// Income not yet closed is the member's (members_equity INVARIANT 3):
/// $100.00 of July revenue, unclosed, is $50.00 each of income in the
/// span and of capital after it. Tobin, 2026: 1,000 + 600 + (350 + 50)
/// − 200 = 1,800. July on: 1,750 + 50 = 1,800. August on: opening 1,800
/// (the July income carried), nothing moved.
#[test]
fn the_capital_statement_counts_income_not_yet_closed() {
    let m = member_books("capital-unclosed");
    post_more(
        &m,
        d(2026, 7, 1),
        &[
            ("1000", Side::Debit, 10_000),
            ("4000", Side::Credit, 10_000),
        ],
    );
    let b = load_with_toml(&m.dir.0);
    let st = b
        .capital_statement(m.tobin, (d(2026, 1, 1), today()))
        .unwrap();
    assert_eq!(st.opening, cents(100_000));
    assert_eq!(st.income_share, cents(40_000));
    assert_eq!(st.closing, cents(180_000));
    // The close is still the only income line: July's has no entry on
    // capital (INVARIANT 3).
    let closes: Decimal = st
        .lines
        .iter()
        .filter(|l| l.kind == ledger_gui::books::CapitalKind::IncomeShare)
        .map(|l| l.amount)
        .sum();
    assert_eq!(closes, cents(35_000));
    let july = b
        .capital_statement(m.tobin, (d(2026, 7, 1), today()))
        .unwrap();
    assert_eq!(
        (july.opening, july.income_share, july.closing),
        (cents(175_000), cents(5_000), cents(180_000))
    );
    assert!(july.lines.is_empty());
    let aug = b
        .capital_statement(m.mo, (d(2026, 8, 1), today()))
        .unwrap();
    assert_eq!(
        (aug.opening, aug.income_share, aug.closing),
        (cents(180_000), Decimal::ZERO, cents(180_000))
    );
}

/// "Owed to them" reads [books] financing_accounts first (INVARIANT 5):
/// with 2200 "Member advances" ($40.00 advanced in July) as the financing
/// account, 2100's "due to" name no longer counts: $40.00, $20.00 each.
/// Without the setting, the name rule: 2100's $200.00 (2200's name says
/// neither "due to" nor "owed to").
#[test]
fn owed_to_members_reads_the_financing_accounts_first() {
    let m = member_books("capital-financing");
    let mut s = Store::open(&m.dir.0.join("books-data")).unwrap();
    s.add_account(
        "2200",
        "Member advances",
        AccountType::Liability,
        None,
        false,
    )
    .unwrap();
    drop(s);
    post_more(
        &m,
        d(2026, 7, 2),
        &[("6050", Side::Debit, 4_000), ("2200", Side::Credit, 4_000)],
    );
    let span = (d(2026, 1, 1), today());
    write_member_toml(
        &m.dir.0,
        "cash_accounts = [\"1000\"]\nfinancing_accounts = [\"2200\"]\n",
    );
    let b = load_with_toml(&m.dir.0);
    let st = b.capital_statement(m.mo, span).unwrap();
    assert_eq!(
        (st.due_to_members, st.owed_to_member),
        (cents(4_000), cents(2_000))
    );
    write_member_toml(&m.dir.0, "cash_accounts = [\"1000\"]\n");
    let b = load_with_toml(&m.dir.0);
    let st = b.capital_statement(m.mo, span).unwrap();
    assert_eq!(
        (st.due_to_members, st.owed_to_member),
        (cents(20_000), cents(10_000))
    );
}

/// Without [books] cash_accounts the capital statement reads money as
/// core does (every asset account but receivables: Books::
/// money_cash_accounts), so the laptop on 1500 reads as cash: Tobin's 2026
/// contributions are $600.00 cash, none property. The Cash page keeps its
/// name-based guess ("Bluevine" names no cash word: no cash accounts).
#[test]
fn without_cash_accounts_the_capital_statement_reads_money_as_core_does() {
    let m = member_books("capital-no-cash");
    write_member_toml(&m.dir.0, "");
    let b = load_with_toml(&m.dir.0);
    let id = |c: &str| b.ledger.chart().find_by_code(c).unwrap().id;
    assert_eq!(b.money_cash_accounts, vec![id("1000"), id("1500")]);
    assert!(
        b.cash_accounts.is_empty(),
        "the Cash page's guess is unchanged"
    );
    let st = b
        .capital_statement(m.tobin, (d(2026, 1, 1), today()))
        .unwrap();
    assert_eq!(
        (st.contributions_cash, st.contributions_property),
        (cents(60_000), Decimal::ZERO)
    );
    assert_eq!(
        st.lines[0].kind,
        ledger_gui::books::CapitalKind::CashContribution
    );
    assert_eq!(st.closing, cents(175_000));
    let all = b
        .capital_statement(m.mo, (d(2025, 1, 1), today()))
        .unwrap();
    assert_eq!(
        (all.contributions_cash, all.contributions_property),
        (cents(160_000), Decimal::ZERO)
    );
}

/// PartyTotalsOut's promise ("the viewer shows the same numbers"): with no
/// `[books] cash_accounts`, party money reads cash as core does (every
/// asset account but receivables, as the server passes it), and the year
/// is the calendar year, so a posted entry dated after today counts.
/// Equipment bought from Checking (an asset for an asset: no money moved,
/// under core's every-asset fallback) and software dated next month, both
/// with Apple: 99.00 paid, in the viewer and the server alike (the viewer
/// used to say 2,000.00: its name-based cash guess, cut at today).
#[test]
fn party_money_is_the_servers_without_cash_accounts_and_after_today() {
    use ledger_gui::books::PartyPeriod;
    let dir = TempDir::new("server-numbers");
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    let checking = s
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let equipment = s
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let capital = s
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let software = s
        .add_account("6200", "Software", AccountType::Expense, None, false)
        .unwrap();
    let e = s.create_entry(d(2026, 1, 1), "Opening").unwrap();
    s.add_line(e, checking, Side::Debit, usd(1_000_000))
        .unwrap();
    s.add_line(e, capital, Side::Credit, usd(1_000_000))
        .unwrap();
    s.post_entry(e).unwrap();
    let apple_v = s.add_vendor("Apple").unwrap();
    let apple = s.ledger().party_of_vendor(apple_v).unwrap();
    for (date, account, c) in [
        (d(2026, 5, 1), equipment, 200_000),
        (d(2026, 11, 20), software, 9_900),
    ] {
        let e = s.create_entry(date, "Apple").unwrap();
        s.add_line(e, account, Side::Debit, usd(c)).unwrap();
        s.add_line(e, checking, Side::Credit, usd(c)).unwrap();
        s.post_entry(e).unwrap();
        s.set_entry_party(e, Some(apple)).unwrap();
    }
    let oracle = s.ledger().clone();
    drop(s);
    let b = load(&dir.0);
    // What books.party computes: activity_with over the calendar year
    // with the (empty) cash_accounts setting.
    let server = ledger_core::parties::activity_with(
        &oracle,
        apple,
        (d(2026, 1, 1), d(2026, 12, 31)),
        &[],
        &[],
    );
    assert_eq!(
        (server.paid, server.received),
        (cents(9_900), Decimal::ZERO)
    );
    let span = b.party_span(PartyPeriod::ThisYear);
    assert_eq!(span, (d(2026, 1, 1), d(2026, 11, 20)));
    let flow = b.party_flow(apple, span, true);
    assert_eq!((flow.paid, flow.received), (server.paid, server.received));
    assert_eq!(b.party(apple).unwrap().paid_ytd, cents(9_900));
    let months = b.party_months(apple, span);
    assert_eq!(months.len(), 11);
    assert_eq!(months.iter().map(|m| m.paid).sum::<Decimal>(), cents(9_900));
    assert_eq!(
        b.party_span(PartyPeriod::AllTime),
        (d(2026, 1, 1), d(2026, 11, 20))
    );
}
