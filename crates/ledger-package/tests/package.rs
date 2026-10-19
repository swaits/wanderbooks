//! The CPA package and the 1099 worksheet, built over a small set of books
//! through both views of them (`Store`, `Snapshot`): the files, their
//! order, their contents, and the TINs, with a TIN source and without.

use chrono::NaiveDate;
use ledger_core::settings::{Company, Member};
use ledger_core::{AccountType, Currency, Form1099, Money, PartyId, Role, Side, TinFacts, TinType};
use ledger_package::{
    Package, PackageBooks, PackageRequest, Settings, build_package, vendor_1099_csv,
    vendor_1099_csv_name, vendor_1099_worksheet,
};
use ledger_store::{Snapshot, Store, Subject};
use rust_decimal::Decimal;
use std::io::Read;
use std::path::Path;
use std::str::FromStr;

const MEMBER_SSN: &str = "241-40-4915";
const EIN: &str = "12-3279105";

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn usd(s: &str) -> Money {
    Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
}

struct Books {
    store: Store,
    rent: u64,
    sale: u64,
    contribution: u64,
    acme: PartyId,
    tobin: PartyId,
    mo: PartyId,
    vendor: u64,
}

fn id_of(s: &Store, code: &str) -> ledger_core::AccountId {
    s.ledger().chart().find_by_code(code).unwrap().id
}

fn post(s: &mut Store, date: &str, desc: &str, lines: &[(&str, Side, &str)]) -> u64 {
    let e = s.create_entry(d(date), desc).unwrap();
    for (code, side, a) in lines {
        let id = id_of(s, code);
        s.add_line(e, id, *side, usd(a)).unwrap();
    }
    s.post_entry(e).unwrap();
    e
}

/// Two members (owner, with TIN facts; Mo, without), a contractor paid
/// past 2026's threshold, a receipt filed on the rent and one linked to
/// the sale, a company document, a loose receipt, and a withdrawn file.
fn books(dir: &Path) -> Books {
    let mut s = Store::create(&dir.join("books-data"), Currency::USD).unwrap();
    for (code, name, ty) in [
        ("1000", "Checking", AccountType::Asset),
        ("3000", "Capital, Tobin", AccountType::Equity),
        ("3001", "Capital, Mo", AccountType::Equity),
        ("4000", "Sales", AccountType::Revenue),
        ("6000", "Rent", AccountType::Expense),
        ("6100", "Contract labor", AccountType::Expense),
    ] {
        s.add_account(code, name, ty, None, false).unwrap();
    }
    let tobin = s.add_party("Tobin Example", &[Role::Member]).unwrap();
    let mo = s.add_party("Mo Example", &[Role::Member]).unwrap();
    s.set_party_tin(
        tobin,
        Some(TinFacts {
            tin_type: TinType::Ssn,
            last4: "4915".into(),
            vault_record: None,
            purged: None,
        }),
    )
    .unwrap();
    let vendor = s.add_vendor("Acme, \"Design\"").unwrap();
    let acme = s.ledger().party_of_vendor(vendor).unwrap();
    s.set_vendor_1099(vendor, Some(Form1099::Nec), true)
        .unwrap();
    s.set_party_tin(
        acme,
        Some(TinFacts {
            tin_type: TinType::Ein,
            last4: "9105".into(),
            vault_record: None,
            purged: None,
        }),
    )
    .unwrap();

    let contribution = post(
        &mut s,
        "2026-01-02",
        "Members' contributions",
        &[
            ("1000", Side::Debit, "10000.00"),
            ("3000", Side::Credit, "5000.00"),
            ("3001", Side::Credit, "5000.00"),
        ],
    );
    let sale = post(
        &mut s,
        "2026-02-01",
        "Sale",
        &[
            ("1000", Side::Debit, "4000"),
            ("4000", Side::Credit, "4000"),
        ],
    );
    let rent = post(
        &mut s,
        "2026-03-01",
        "Rent, \"March\"",
        &[
            ("6000", Side::Debit, "1200"),
            ("1000", Side::Credit, "1200"),
        ],
    );
    let e = s.create_entry(d("2026-04-01"), "Acme design work").unwrap();
    s.add_line(e, id_of(&s, "6100"), Side::Debit, usd("2500"))
        .unwrap();
    s.add_line(e, id_of(&s, "1000"), Side::Credit, usd("2500"))
        .unwrap();
    s.set_entry_vendor(e, Some(vendor)).unwrap();
    s.post_entry(e).unwrap();
    // After as_of: not in the package.
    post(
        &mut s,
        "2027-01-05",
        "Next year's rent",
        &[
            ("6000", Side::Debit, "1200"),
            ("1000", Side::Credit, "1200"),
        ],
    );

    s.attach_receipt(
        Some(rent),
        "rent/march.pdf",
        "application/pdf",
        b"%PDF rent",
        None,
    )
    .unwrap();
    let (linked, _, _) = s
        .attach_receipt(None, "invoice.pdf", "application/pdf", b"%PDF sale", None)
        .unwrap();
    s.link_attachment(linked, Subject::Entry(sale)).unwrap();
    s.file_document(
        "legal",
        "operating agreement.pdf",
        "application/pdf",
        b"%PDF agreement",
        None,
    )
    .unwrap();
    s.attach_receipt(None, "loose.pdf", "application/pdf", b"%PDF loose", None)
        .unwrap();
    let (gone, _, _) = s
        .file_document("tax", "mistake.pdf", "application/pdf", b"%PDF oops", None)
        .unwrap();
    s.withdraw_attachment(gone, "filed by mistake").unwrap();
    Books {
        store: s,
        rent,
        sale,
        contribution,
        acme,
        tobin,
        mo,
        vendor,
    }
}

fn company(members: bool) -> Company {
    let member = |name: &str, capital: &str| Member {
        name: name.into(),
        share: Decimal::from_str("0.5").unwrap(),
        capital_account: capital.into(),
        distribution_account: None,
        files_jointly_with: None,
    };
    Company {
        name: "Example Labs LLC".into(),
        basis: "Cash".into(),
        members: if members {
            // Spouses: each names the other (settings require both).
            let mut tobin = member("Tobin Example", "3000");
            tobin.files_jointly_with = Some("Mo Example".into());
            let mut mo = member("Mo Example", "3001");
            mo.files_jointly_with = Some("Tobin Example".into());
            vec![tobin, mo]
        } else {
            Vec::new()
        },
        ..Company::default()
    }
}

fn cash() -> Vec<String> {
    vec!["1000".to_string()]
}

fn settings<'a>(company: &'a Company, cash: &'a [String]) -> Settings<'a> {
    Settings {
        company,
        cash_accounts: cash,
        financing_accounts: &[],
        investing_accounts: &[],
    }
}

fn request<'a>(
    include_receipts: bool,
    tins: Option<ledger_package::TinSource<'a>>,
) -> PackageRequest<'a> {
    PackageRequest {
        as_of: d("2026-12-31"),
        prepared_on: d("2027-01-15"),
        cash_accounts: &[],
        financing_accounts: &[],
        investing_accounts: &[],
        include_receipts,
        tins,
    }
}

fn names(p: &Package) -> Vec<&str> {
    p.files.iter().map(|(n, _)| n.as_str()).collect()
}

fn text<'p>(p: &'p Package, name: &str) -> &'p str {
    let (_, bytes) = p
        .files
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("{name} in {:?}", names(p)));
    std::str::from_utf8(bytes).unwrap()
}

/// `files` with each XLSX as its parts, its creation stamp
/// (docProps/core.xml's dcterms:created and dcterms:modified, the clock's
/// time when it is written) blanked: the only bytes of a package that
/// depend on when it is built.
fn stable(files: &[(String, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
    files
        .iter()
        .map(|(name, bytes)| {
            if !name.ends_with(".xlsx") {
                return (name.clone(), bytes.clone());
            }
            let mut x = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
            let mut out = Vec::new();
            for i in 0..x.len() {
                let mut part = x.by_index(i).unwrap();
                let part_name = part.name().unwrap().to_string();
                let mut b = Vec::new();
                part.read_to_end(&mut b).unwrap();
                if part_name == "docProps/core.xml" {
                    let mut t = String::from_utf8(b).unwrap();
                    for tag in ["dcterms:created", "dcterms:modified"] {
                        let open = t.find(&format!("<{tag}")).unwrap();
                        let start = open + t[open..].find('>').unwrap() + 1;
                        let end = start + t[start..].find(&format!("</{tag}>")).unwrap();
                        t.replace_range(start..end, "STAMP");
                    }
                    b = t.into_bytes();
                }
                out.extend_from_slice(part_name.as_bytes());
                out.push(0);
                out.extend_from_slice(&b);
                out.push(0);
            }
            (name.clone(), out)
        })
        .collect()
}

/// The zip holds exactly the files, in order, byte for byte.
fn assert_zip_is_the_files(p: &Package) {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&p.zip)).unwrap();
    assert_eq!(z.len(), p.files.len());
    for (i, (name, bytes)) in p.files.iter().enumerate() {
        let mut f = z.by_index(i).unwrap();
        assert_eq!(f.name().unwrap(), name.as_str());
        let mut got = Vec::new();
        f.read_to_end(&mut got).unwrap();
        assert_eq!(&got, bytes, "{name}");
    }
}

#[test]
fn a_single_member_package_holds_the_documented_files_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let company = company(false);
    let cash = cash();
    let p = build_package(&b.store, &settings(&company, &cash), &request(true, None)).unwrap();
    assert_eq!(p.file_name, "cpa-package-2026-12-31.zip");
    let sale_date = "2026-02-01";
    let want = vec![
        "README.txt".to_string(),
        "profit-and-loss.pdf".into(),
        "profit-and-loss.xlsx".into(),
        "balance-sheet.pdf".into(),
        "balance-sheet.xlsx".into(),
        "cash-flow.pdf".into(),
        "cash-flow.xlsx".into(),
        "tax-summary.pdf".into(),
        "tax-summary.xlsx".into(),
        "trial-balance.pdf".into(),
        "trial-balance.xlsx".into(),
        "general-ledger.xlsx".into(),
        "general-journal.xlsx".into(),
        "1099-worksheet-2026.csv".into(),
        // Attachment order: the rent's receipt (its name cleaned), the
        // sale's linked one, the company's document; never the loose
        // receipt or the withdrawn file.
        format!("receipts/2026-03-01-entry-{}-rent_march.pdf", b.rent),
        format!("receipts/{sale_date}-entry-{}-invoice.pdf", b.sale),
        "documents/legal/operating agreement.pdf".into(),
        "receipts/index.csv".into(),
        "general-journal.iif".into(),
    ];
    assert_eq!(names(&p), want);
    assert_eq!(p.receipt_count, 2);
    assert!(p.tin_gaps.is_empty());
    for (name, bytes) in &p.files {
        if name.ends_with(".pdf") && !name.contains('/') {
            assert!(bytes.starts_with(b"%PDF-"), "{name}");
        }
        if name.ends_with(".xlsx") {
            assert!(bytes.starts_with(b"PK"), "{name}");
        }
    }
    let rent_sha = ledger_store::journal::sha256_hex(b"%PDF rent");
    let sale_sha = ledger_store::journal::sha256_hex(b"%PDF sale");
    assert_eq!(
        text(&p, "receipts/index.csv"),
        format!(
            "entry_id,date,description,amount,file,sha256\n\
             {rent},2026-03-01,\"Rent, \"\"March\"\"\",1200.00,receipts/2026-03-01-entry-{rent}-rent_march.pdf,{rent_sha}\n\
             {sale},2026-02-01,\"Sale\",4000.00,receipts/2026-02-01-entry-{sale}-invoice.pdf,{sale_sha}\n",
            rent = b.rent,
            sale = b.sale,
        )
    );
    assert_eq!(
        text(&p, "1099-worksheet-2026.csv"),
        "vendor,form,w9_on_file,paid,reportable\n\"Acme, \"\"Design\"\"\",1099-NEC,true,2500.00,true\n"
    );
    let readme = text(&p, "README.txt");
    assert!(readme.starts_with("Example Labs LLC — books package for the CPA\n\nFiscal year to date: 2026-01-01 through 2026-12-31. Cash basis.\n"), "{readme}");
    assert!(
        readme.contains(&format!("({} events, verified)", b.store.journal_seq())),
        "{readme}"
    );
    assert!(readme.contains("receipts/: the 2 receipts behind the period's entries"));
    assert!(readme.contains("cash flow, tax summary, trial balance."));
    assert!(!readme.contains("k1-members.csv") && !readme.contains("full taxpayer"));
    assert!(!names(&p).contains(&"k1-members.csv"));
    let iif = text(&p, "general-journal.iif");
    assert!(iif.contains("Checking") && iif.contains("BANK"), "{iif}");
    // Every posted entry, the year after as_of's included.
    assert!(iif.contains("Next year's rent"), "{iif}");
    assert_zip_is_the_files(&p);
}

#[test]
fn without_receipts_the_package_holds_no_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let company = company(false);
    let cash = cash();
    let with = build_package(&b.store, &settings(&company, &cash), &request(true, None)).unwrap();
    let p = build_package(&b.store, &settings(&company, &cash), &request(false, None)).unwrap();
    assert!(
        !names(&p)
            .iter()
            .any(|n| n.starts_with("receipts/") || n.starts_with("documents/")),
        "{:?}",
        names(&p)
    );
    assert_eq!(p.receipt_count, 0);
    // Everything else is the same, byte for byte, but the README's
    // evidence lines.
    let rest = |p: &Package| -> Vec<(String, Vec<u8>)> {
        p.files
            .iter()
            .filter(|(n, _)| {
                n != "README.txt" && !n.starts_with("receipts/") && !n.starts_with("documents/")
            })
            .cloned()
            .collect()
    };
    assert_eq!(stable(&rest(&p)), stable(&rest(&with)));
    let readme = text(&p, "README.txt");
    assert!(!readme.contains("receipts/") && !readme.contains("documents/"));
    assert_eq!(
        text(&with, "README.txt").replace(
            "receipts/: the 2 receipts behind the period's entries; index.csv ties each to its entry, date, amount, and SHA-256.\ndocuments/: the company's formation, legal, and tax documents.\n",
            ""
        ),
        readme
    );
    assert_zip_is_the_files(&p);
}

/// Tobin's and Acme's TINs from the source; Mo's isn't there.
fn tin_source(b: &Books) -> impl Fn(PartyId) -> Result<ledger_vault::Tin, String> + '_ {
    move |id| {
        if id == b.tobin {
            Ok(ledger_vault::Tin::new(MEMBER_SSN, ledger_vault::TinType::Ssn).unwrap())
        } else if id == b.acme {
            Ok(ledger_vault::Tin::new(EIN, ledger_vault::TinType::Ein).unwrap())
        } else {
            Err("TIN not in the vault: record it with books.set_tin".into())
        }
    }
}

#[test]
fn members_get_the_k1_worksheet_with_full_tins_only_from_a_source() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let company = company(true);
    let cash = cash();
    let source = tin_source(&b);
    let with = build_package(
        &b.store,
        &settings(&company, &cash),
        &request(true, Some(&source)),
    )
    .unwrap();
    let without =
        build_package(&b.store, &settings(&company, &cash), &request(true, None)).unwrap();

    for p in [&with, &without] {
        let n = names(p);
        assert!(n.contains(&"members-equity.pdf") && n.contains(&"members-equity.xlsx"));
        // The K-1 worksheet comes after the evidence, before the IIF.
        assert_eq!(&n[n.len() - 2..], ["k1-members.csv", "general-journal.iif"]);
        assert_zip_is_the_files(p);
    }
    // Capital: 5000 in each; income 4000 - 1200 - 2500 = 300, half each.
    assert_eq!(
        text(&with, "k1-members.csv"),
        format!(
            "member,party_id,share,capital_account,tin,tin_type,tin_note,beginning_capital,contributions_cash,contributions_property,net_income,distributions,other,ending_capital\n\
             Tobin Example,{s},0.50,3000,{MEMBER_SSN},ssn,,0.00,5000.00,0.00,150.00,0.00,0.00,5150.00\n\
             Mo Example,{k},0.50,3001,,,TIN not in the vault: record it with books.set_tin,0.00,5000.00,0.00,150.00,0.00,0.00,5150.00\n\
             \"Total, all members (not a K-1)\",,1.00,,,,,0.00,10000.00,0.00,300.00,0.00,0.00,10300.00\n",
            s = b.tobin,
            k = b.mo,
        )
    );
    assert_eq!(
        text(&without, "k1-members.csv"),
        format!(
            "member,party_id,share,capital_account,tin_last4,tin_type,beginning_capital,contributions_cash,contributions_property,net_income,distributions,other,ending_capital\n\
             Tobin Example,{s},0.50,3000,4915,ssn,0.00,5000.00,0.00,150.00,0.00,0.00,5150.00\n\
             Mo Example,{k},0.50,3001,,,0.00,5000.00,0.00,150.00,0.00,0.00,5150.00\n\
             \"Total, all members (not a K-1)\",,1.00,,,,0.00,10000.00,0.00,300.00,0.00,0.00,10300.00\n",
            s = b.tobin,
            k = b.mo,
        )
    );
    assert_eq!(
        text(&with, "1099-worksheet-2026.csv"),
        format!(
            "vendor,form,w9_on_file,paid,reportable,party_id,tin_last4,tin,tin_type,tin_note\n\
             \"Acme, \"\"Design\"\"\",1099-NEC,true,2500.00,true,{},9105,{EIN},ein,\n",
            b.acme
        )
    );
    assert!(!text(&without, "1099-worksheet-2026.csv").contains(EIN));

    // The gap: Mo's, in tin_gaps and the README.
    assert_eq!(
        with.tin_gaps,
        ["member Mo Example: TIN not in the vault: record it with books.set_tin"]
    );
    assert!(without.tin_gaps.is_empty());
    let readme = text(&with, "README.txt");
    assert!(readme.contains("cash flow, members' equity, tax summary"));
    assert!(readme.contains(", with each payee's TIN.\n"));
    assert!(readme.contains("with their taxpayer ID; the last row totals"));
    // Said once for the couple.
    assert_eq!(
        readme
            .matches("file a joint individual return: each receives their own Schedule K-1; both are reported on the one Form 1040 (Schedule E, Part II).\n")
            .count(),
        1
    );
    assert!(readme.contains("\nTobin Example and Mo Example file a joint individual return"));
    assert!(readme.contains(
        "\nThis package holds full taxpayer identification numbers: keep it out of email and shared folders, and delete copies once filed.\nTINs not included:\n- member Mo Example: TIN not in the vault: record it with books.set_tin\n"
    ));
    let plain = text(&without, "README.txt");
    assert!(plain.contains("with their taxpayer ID's last four;"));
    assert!(!plain.contains("full taxpayer") && !plain.contains("payee's TIN"));
    for (_, bytes) in &without.files {
        let s = String::from_utf8_lossy(bytes);
        assert!(!s.contains(MEMBER_SSN) && !s.contains(EIN));
    }
    let _ = b.contribution;
}

#[test]
fn a_store_and_a_snapshot_of_the_same_books_build_the_same_package() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let snap = Snapshot::load(&dir.path().join("books-data")).unwrap();
    assert_eq!(
        PackageBooks::journal_seq(&snap),
        PackageBooks::journal_seq(&b.store)
    );
    for a in b.store.list_attachments(None).unwrap() {
        let mut from_store: Vec<Subject> = b.store.links_of(a.id).unwrap();
        from_store.retain(|s| a.entry_id.map(Subject::Entry) != Some(*s));
        assert_eq!(
            snap.links_of(a.id).unwrap(),
            from_store,
            "attachment {}",
            a.id
        );
    }
    let company = company(true);
    let cash = cash();
    let source = tin_source(&b);
    for tins in [None, Some(&source as ledger_package::TinSource<'_>)] {
        for receipts in [true, false] {
            let s = settings(&company, &cash);
            let from_store = build_package(&b.store, &s, &request(receipts, tins)).unwrap();
            let from_snap = build_package(&snap, &s, &request(receipts, tins)).unwrap();
            assert_eq!(stable(&from_store.files), stable(&from_snap.files));
            assert_eq!(from_store.tin_gaps, from_snap.tin_gaps);
            assert_eq!(from_store.receipt_count, from_snap.receipt_count);
        }
    }
}

#[test]
fn a_package_needs_a_cash_account() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let company = company(false);
    let err = build_package(&b.store, &settings(&company, &[]), &request(true, None)).unwrap_err();
    assert_eq!(
        err,
        "cash_accounts is required for the cash flow statement (pass it, or set the books' cash accounts setting: books.settings shows it)"
    );
    // A request's own accounts override the settings'.
    let mut req = request(true, None);
    let bad = vec!["9999".to_string()];
    req.cash_accounts = &bad;
    let err = build_package(&b.store, &settings(&company, &cash()), &req).unwrap_err();
    assert_eq!(err, "unknown cash account code: 9999");
}

#[test]
fn the_1099_worksheet_carries_tins_only_from_a_source() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let company = company(false);
    let cash = cash();
    let s = settings(&company, &cash);
    let ledger = b.store.ledger();
    let plain = vendor_1099_worksheet(ledger, &s, 2026, None, None).unwrap();
    assert_eq!(plain.year, 2026);
    assert_eq!(plain.threshold, Decimal::from(2000));
    assert!(!plain.tins_included);
    assert_eq!(plain.lines.len(), 1);
    let l = &plain.lines[0];
    assert_eq!(l.vendor_id, b.vendor);
    assert_eq!(l.party_id, Some(b.acme));
    assert_eq!(l.paid, Decimal::from(2500));
    assert!(l.reportable && l.tin_on_file);
    assert_eq!(l.tin_last4.as_deref(), Some("9105"));
    assert_eq!((&l.tin, &l.tin_type, &l.tin_note), (&None, &None, &None));
    assert!(plain.missing_w9.is_empty());
    // Five posted entries in 2026, one tagged.
    assert_eq!(
        plain.notes,
        [
            "Only entries tagged with a vendor count (books.set_entry_vendor; books.pay_bill tags automatically).",
            "Corporations are generally exempt, except attorneys' fees and medical payments; royalties report at $10. Confirm with the CPA.",
            "3 posted entries in 2026 have no vendor tag; review any that paid contractors or rent.",
        ]
    );
    assert_eq!(
        vendor_1099_csv(&plain, false),
        format!(
            "vendor_id,name,form_1099,paid,reportable,w9_on_file,tin_last4\n{},\"Acme, \"\"Design\"\"\",1099-NEC,2500.00,true,true,9105\n",
            b.vendor
        )
    );
    let source = tin_source(&b);
    let full = vendor_1099_worksheet(ledger, &s, 2026, None, Some(&source)).unwrap();
    assert!(full.tins_included);
    assert_eq!(full.lines[0].tin.as_deref(), Some(EIN));
    assert_eq!(full.lines[0].tin_type.as_deref(), Some("ein"));
    assert_eq!(
        vendor_1099_csv(&full, true),
        format!(
            "vendor_id,name,form_1099,paid,reportable,w9_on_file,tin_last4,tin,tin_type,tin_note\n{},\"Acme, \"\"Design\"\"\",1099-NEC,2500.00,true,true,9105,{EIN},ein,\n",
            b.vendor
        )
    );
    // A source without the payee's TIN: the note says why.
    let none = |_: PartyId| -> Result<ledger_vault::Tin, String> {
        Err("the TIN vault is locked, \"for now\"".into())
    };
    let locked = vendor_1099_worksheet(ledger, &s, 2026, None, Some(&none)).unwrap();
    assert_eq!(
        vendor_1099_csv(&locked, true).lines().nth(1).unwrap(),
        format!(
            "{},\"Acme, \"\"Design\"\"\",1099-NEC,2500.00,true,true,9105,,,\"the TIN vault is locked, \"\"for now\"\"\"",
            b.vendor
        )
    );
    assert_eq!(vendor_1099_csv_name(2026, false), "1099-worksheet-2026.csv");
    assert_eq!(
        vendor_1099_csv_name(2026, true),
        "1099-worksheet-2026-with-tins.csv"
    );
    // A year whose threshold isn't known is refused, never guessed; given
    // one, it is used.
    let far = vendor_1099_worksheet(ledger, &s, 2031, None, None);
    assert!(far.is_err(), "{far:?}");
    let given = vendor_1099_worksheet(ledger, &s, 2031, Some(Decimal::from(2100)), None).unwrap();
    assert_eq!(given.threshold, Decimal::from(2100));
    assert!(
        given
            .notes
            .iter()
            .any(|n| n.contains("indexed for inflation after 2026; confirm the 2031 amount"))
    );
}

/// Small books with no members: a checking account and rent, one rent
/// payment, and whatever the test files.
fn plain_books(dir: &Path) -> (Store, u64) {
    let mut s = Store::create(&dir.join("books-data"), Currency::USD).unwrap();
    for (code, name, ty) in [
        ("1000", "Checking", AccountType::Asset),
        ("3000", "Capital", AccountType::Equity),
        ("6000", "Rent", AccountType::Expense),
    ] {
        s.add_account(code, name, ty, None, false).unwrap();
    }
    let rent = post(
        &mut s,
        "2026-03-01",
        "March rent",
        &[
            ("6000", Side::Debit, "1200.00"),
            ("1000", Side::Credit, "1200.00"),
        ],
    );
    (s, rent)
}

#[test]
fn files_of_the_same_name_each_get_their_own_place() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, rent) = plain_books(dir.path());
    // Two receipts named alike on one entry, the second in other case; two
    // documents named alike in one category.
    s.attach_receipt(
        Some(rent),
        "r.pdf",
        "application/pdf",
        b"%PDF-1.4 one",
        None,
    )
    .unwrap();
    s.attach_receipt(
        Some(rent),
        "R.pdf",
        "application/pdf",
        b"%PDF-1.4 two",
        None,
    )
    .unwrap();
    s.file_document(
        "legal",
        "deed.pdf",
        "application/pdf",
        b"%PDF-1.4 deed one",
        None,
    )
    .unwrap();
    s.file_document(
        "legal",
        "deed.pdf",
        "application/pdf",
        b"%PDF-1.4 deed two",
        None,
    )
    .unwrap();
    let company = company(false);
    let cash = cash();
    let p = build_package(&s, &settings(&company, &cash), &request(true, None)).unwrap();
    let receipt = |suffix: &str| format!("receipts/2026-03-01-entry-{rent}-{suffix}");
    let all = names(&p);
    for name in [
        receipt("r.pdf"),
        receipt("R (2).pdf"),
        "documents/legal/deed.pdf".to_string(),
        "documents/legal/deed (2).pdf".to_string(),
    ] {
        assert!(all.contains(&name.as_str()), "{name} in {all:?}");
    }
    assert_eq!(text(&p, &receipt("r.pdf")), "%PDF-1.4 one");
    assert_eq!(text(&p, &receipt("R (2).pdf")), "%PDF-1.4 two");
    assert_eq!(
        text(&p, "documents/legal/deed (2).pdf"),
        "%PDF-1.4 deed two"
    );
    // The index names each receipt as it is stored.
    let index = text(&p, "receipts/index.csv");
    assert!(
        index.contains(&receipt("r.pdf")) && index.contains(&receipt("R (2).pdf")),
        "{index}"
    );
    assert_eq!(p.receipt_count, 2);
    assert_zip_is_the_files(&p);
    // And the README says the documents are there.
    assert!(text(&p, "README.txt").contains("documents/:"));
}

#[test]
fn the_readme_names_only_what_the_package_holds() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, rent) = plain_books(dir.path());
    s.attach_receipt(
        Some(rent),
        "r.pdf",
        "application/pdf",
        b"%PDF-1.4 one",
        None,
    )
    .unwrap();
    let company = company(false);
    let cash = cash();
    let p = build_package(&s, &settings(&company, &cash), &request(true, None)).unwrap();
    let readme = text(&p, "README.txt");
    assert!(!readme.contains("documents/"), "no documents: {readme}");
    assert!(readme.contains("1099-worksheet-2026.csv:"), "{readme}");
    assert!(names(&p).contains(&"1099-worksheet-2026.csv"));
    // A year whose 1099 threshold isn't known: the worksheet is left out,
    // and the README says what is there instead.
    let mut late = request(true, None);
    late.as_of = d("2031-06-30");
    let p = build_package(&s, &settings(&company, &cash), &late).unwrap();
    let readme = text(&p, "README.txt");
    assert!(
        names(&p).contains(&"1099-worksheet-2031-NOT-INCLUDED.txt"),
        "{:?}",
        names(&p)
    );
    assert!(
        readme.contains("1099-worksheet-2031-NOT-INCLUDED.txt:"),
        "{readme}"
    );
    assert!(!readme.contains("1099-worksheet-2031.csv"), "{readme}");
}

#[test]
fn a_single_members_readme_promises_no_totals_row() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = plain_books(dir.path());
    let mut solo = company(false);
    solo.members = vec![Member {
        name: "Solo Owner".into(),
        share: Decimal::ONE,
        capital_account: "3000".into(),
        distribution_account: None,
        files_jointly_with: None,
    }];
    let cash = cash();
    let p = build_package(&s, &settings(&solo, &cash), &request(false, None)).unwrap();
    let readme = text(&p, "README.txt");
    assert!(readme.contains("k1-members.csv:"), "{readme}");
    assert!(!readme.contains("the last row totals"), "{readme}");
    let k1 = text(&p, "k1-members.csv");
    assert_eq!(k1.lines().count(), 2, "a header and the one member: {k1}");
    // Two members: the totals row, and the README says so.
    let two = company(true);
    let b = books(dir.path().join("two").as_path());
    let p = build_package(&b.store, &settings(&two, &cash), &request(false, None)).unwrap();
    assert!(text(&p, "README.txt").contains("the last row totals"));
}
