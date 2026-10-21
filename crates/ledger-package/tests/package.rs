//! The CPA package and the 1099 worksheet, built over a small set of books
//! through both views of them (`Store`, `Snapshot`): the files, their
//! order, their contents, and the TINs, with a TIN source and without.

use chrono::NaiveDate;
use ledger_core::settings::{Company, Member};
use ledger_core::{
    AccountType, Currency, Form1099, Ledger, Money, PartyId, Role, Side, TinFacts, TinType,
};
use ledger_package::{
    Package, PackageBooks, PackageRequest, Settings, build_package, vendor_1099_csv,
    vendor_1099_csv_name, vendor_1099_worksheet,
};
use ledger_store::{Snapshot, Store, Subject};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashMap};
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
    // Every posted entry through as_of; the next year's rent is not.
    assert!(!iif.contains("Next year's rent"), "{iif}");
    // The rent's memo, `Rent, "March"`: its double quotes written single
    // (QuickBooks reads one as a text delimiter), and said.
    assert!(iif.contains("\tRent, 'March'\r\n"), "{iif}");
    assert_eq!(
        p.warnings,
        [format!(
            "\"\\\"\" is written \"'\" in the memo of entry {}",
            b.rent
        )]
    );
    assert_eq!(p.iif_changes, 1);
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

#[test]
fn a_dotfiles_twin_keeps_its_whole_name_and_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = plain_books(dir.path());
    // A name that is all extension (".env"): its twin is ".env (2)", never
    // " (2).env", a name made of nothing but a number.
    for bytes in [b"one".as_slice(), b"two".as_slice()] {
        s.file_document("legal", ".env", "text/plain", bytes, None)
            .unwrap();
    }
    let company = company(false);
    let cash = cash();
    let p = build_package(&s, &settings(&company, &cash), &request(true, None)).unwrap();
    let docs: Vec<&str> = names(&p)
        .into_iter()
        .filter(|n| n.starts_with("documents/"))
        .collect();
    assert_eq!(docs, ["documents/legal/.env", "documents/legal/.env (2)"]);
    assert_eq!(text(&p, "documents/legal/.env"), "one");
    assert_eq!(text(&p, "documents/legal/.env (2)"), "two");
    assert_zip_is_the_files(&p);
}

#[test]
fn a_twins_number_goes_on_its_own_name_and_no_name_climbs_out_of_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = plain_books(dir.path());
    // A dot in the folder is not the file's extension: the second "notes"
    // in "a.b" is "a.b/notes (2)", never "a (2).b/notes" in another
    // folder. And a name of dots alone would climb out of its folder when
    // the zip is unpacked: it becomes underscores.
    for bytes in [b"one".as_slice(), b"two".as_slice()] {
        s.file_document("a.b", "notes", "text/plain", bytes, None)
            .unwrap();
    }
    s.file_document("..", "..", "text/plain", b"three", None)
        .unwrap();
    s.file_document("legal", ".", "text/plain", b"four", None)
        .unwrap();
    let company = company(false);
    let cash = cash();
    let p = build_package(&s, &settings(&company, &cash), &request(true, None)).unwrap();
    let docs: Vec<&str> = names(&p)
        .into_iter()
        .filter(|n| n.starts_with("documents/"))
        .collect();
    assert_eq!(
        docs,
        [
            "documents/a.b/notes",
            "documents/a.b/notes (2)",
            "documents/__/__",
            "documents/legal/_"
        ]
    );
    assert_eq!(text(&p, "documents/a.b/notes (2)"), "two");
    assert_eq!(text(&p, "documents/__/__"), "three");
    assert!(
        names(&p).iter().all(|n| n
            .split('/')
            .all(|seg| seg != ".." && seg != "." && !seg.is_empty())),
        "{:?}",
        names(&p)
    );
    assert_zip_is_the_files(&p);
}

#[test]
fn the_years_first_and_last_days_receipts_are_in_and_the_days_beyond_are_not() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = plain_books(dir.path());
    // The fiscal year is 2026 (the default start, January 1) and as_of is
    // its last day: the receipts on both edges go in, the ones a day
    // outside stay out.
    let mut on = Vec::new();
    for date in ["2025-12-31", "2026-01-01", "2026-12-31", "2027-01-01"] {
        let e = post(
            &mut s,
            date,
            "Rent",
            &[
                ("6000", Side::Debit, "100.00"),
                ("1000", Side::Credit, "100.00"),
            ],
        );
        s.attach_receipt(Some(e), "r.pdf", "application/pdf", date.as_bytes(), None)
            .unwrap();
        on.push((date, e));
    }
    let company = company(false);
    let cash = cash();
    let p = build_package(&s, &settings(&company, &cash), &request(true, None)).unwrap();
    let receipts: Vec<&str> = names(&p)
        .into_iter()
        .filter(|n| n.starts_with("receipts/") && *n != "receipts/index.csv")
        .collect();
    let first = format!("receipts/2026-01-01-entry-{}-r.pdf", on[1].1);
    let last = format!("receipts/2026-12-31-entry-{}-r.pdf", on[2].1);
    assert_eq!(receipts, [first.as_str(), last.as_str()]);
    assert_eq!(p.receipt_count, 2);
    assert_eq!(text(&p, &first), "2026-01-01");
    assert_eq!(text(&p, &last), "2026-12-31");
    assert_eq!(text(&p, "receipts/index.csv").lines().count(), 3);
}

/// Contract labor paid three ways: from checking (cash), from a prepaid
/// asset that isn't cash, and by a member's advance (financing).
#[test]
fn payments_to_a_vendor_come_from_the_settings_accounts_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = plain_books(dir.path());
    for (code, name, ty) in [
        ("1300", "Prepaid expenses", AccountType::Asset),
        ("2100", "Member advances", AccountType::Liability),
        ("6100", "Contract labor", AccountType::Expense),
    ] {
        s.add_account(code, name, ty, None, false).unwrap();
    }
    let vendor = s.add_vendor("Acme Design").unwrap();
    s.set_vendor_1099(vendor, Some(Form1099::Nec), true)
        .unwrap();
    for (from, amount) in [("1000", "1000.00"), ("1300", "400.00"), ("2100", "700.00")] {
        let e = s.create_entry(d("2026-05-01"), "Acme design work").unwrap();
        s.add_line(e, id_of(&s, "6100"), Side::Debit, usd(amount))
            .unwrap();
        s.add_line(e, id_of(&s, from), Side::Credit, usd(amount))
            .unwrap();
        s.set_entry_vendor(e, Some(vendor)).unwrap();
        s.post_entry(e).unwrap();
    }
    let company = company(false);
    // A code the chart lacks is skipped.
    let cash = vec!["1000".to_string(), "1999".to_string()];
    let financing = vec!["2100".to_string()];
    let st = Settings {
        company: &company,
        cash_accounts: &cash,
        financing_accounts: &financing,
        investing_accounts: &[],
    };
    let ledger = s.ledger();
    assert_eq!(
        ledger_package::payment_accounts(ledger, &st),
        ledger_core::PaymentAccounts {
            cash: vec![id_of(&s, "1000")],
            financing: vec![id_of(&s, "2100")],
        }
    );
    // Checking and the advance count; the prepaid asset doesn't.
    let w = vendor_1099_worksheet(ledger, &st, 2026, None, None).unwrap();
    assert_eq!(w.lines.len(), 1);
    assert_eq!(w.lines[0].vendor_id, vendor);
    assert_eq!(w.lines[0].paid, Decimal::from(1700));
}

#[test]
fn a_receipts_index_amount_is_the_entrys_debit_total_as_written() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = plain_books(dir.path());
    // A tool takes "100.000" as written (it is whole cents): the two sides
    // are equal in value but not in spelling. The index prints the debit
    // side's total, padded to cents, never the credit side's "100.000".
    let e = post(
        &mut s,
        "2026-06-01",
        "Rent, June",
        &[
            ("6000", Side::Debit, "60"),
            ("6000", Side::Debit, "40.0"),
            ("1000", Side::Credit, "100.000"),
        ],
    );
    s.attach_receipt(Some(e), "june.pdf", "application/pdf", b"%PDF june", None)
        .unwrap();
    let company = company(false);
    let cash = cash();
    let p = build_package(&s, &settings(&company, &cash), &request(true, None)).unwrap();
    let index = text(&p, "receipts/index.csv");
    let row = index
        .lines()
        .find(|l| l.starts_with(&format!("{e},")))
        .unwrap();
    let fields: Vec<&str> = row.split(',').collect();
    assert_eq!(
        fields[..4],
        [e.to_string().as_str(), "2026-06-01", "\"Rent", " June\""]
    );
    assert_eq!(fields[4], "100.00", "{row}");
    assert_eq!(fields[5], format!("receipts/2026-06-01-entry-{e}-june.pdf"));
}

// ---------------------------------------------------------------------------
// As of one day (rules.md B-33), and the QuickBooks file (B-34)
// ---------------------------------------------------------------------------

/// The IIF's transactions in file order: each one's DOCNUM, memo, and
/// lines as (account NAME, signed amount).
type IifTxn = (u64, String, Vec<(String, Decimal)>);

fn iif_txns(iif: &str) -> Vec<IifTxn> {
    let mut out: Vec<IifTxn> = Vec::new();
    for l in iif.split("\r\n") {
        let f: Vec<&str> = l.split('\t').collect();
        let line = || (f[4].to_string(), Decimal::from_str(f[6]).unwrap());
        match f[0] {
            "TRNS" => out.push((f[7].parse().unwrap(), f[8].to_string(), vec![line()])),
            "SPL" => out.last_mut().unwrap().2.push(line()),
            _ => {}
        }
    }
    out
}

/// Each account's balance in the IIF, debit positive, by ACCNUM; zero
/// balances left out.
fn iif_balances(iif: &str) -> BTreeMap<String, Decimal> {
    let number: HashMap<String, String> = iif
        .split("\r\n")
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .filter(|f| f[0] == "ACCNT")
        .map(|f| (f[1].to_string(), f[3].to_string()))
        .collect();
    let mut out: BTreeMap<String, Decimal> = BTreeMap::new();
    for (_, _, lines) in iif_txns(iif) {
        for (name, a) in lines {
            *out.entry(number[&name].clone()).or_default() += a;
        }
    }
    out.retain(|_, v| !v.is_zero());
    out
}

/// The trial balance as of `as_of` (ledger_core::reports), debit
/// positive, by code, without the closing entries' lines; zero balances
/// left out.
fn tb_without_closes(ledger: &Ledger, as_of: NaiveDate) -> BTreeMap<String, Decimal> {
    let mut out: BTreeMap<String, Decimal> =
        ledger_core::reports::trial_balance_as_of(ledger, as_of)
            .lines
            .into_iter()
            .map(|l| (l.code, l.debit - l.credit))
            .collect();
    for id in ledger.closing_entry_ids() {
        let e = ledger.get_entry(id).unwrap();
        if e.date() > as_of {
            continue;
        }
        for l in e.lines() {
            let code = ledger.chart().get(l.account).unwrap().code.clone();
            let signed = match l.side {
                Side::Debit => l.amount.amount,
                Side::Credit => -l.amount.amount,
            };
            *out.entry(code).or_default() -= signed;
        }
    }
    out.retain(|_, v| !v.is_zero());
    out
}

/// Defect 21 (P3; found in P0, 2026-10-16): the package's
/// general-journal.iif held every posted entry, the ones after as_of too,
/// while every other file stopped at as_of. Now it holds every posted
/// entry from the first through as_of, each with its id as DOCNUM, and its
/// balances are the trial balance as of as_of.
#[test]
fn as_of_package_iif_stops_at_as_of() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let company = company(false);
    let cash = cash();
    let p = build_package(&b.store, &settings(&company, &cash), &request(false, None)).unwrap();
    let iif = text(&p, "general-journal.iif");
    let ledger = b.store.ledger();
    let as_of = d("2026-12-31");
    let through: Vec<u64> = ledger
        .posted_entries()
        .filter(|e| e.date() <= as_of)
        .map(|e| e.id())
        .collect();
    assert_eq!(
        through.len(),
        4,
        "the year's four entries; next year's rent is the fifth"
    );
    let txns = iif_txns(iif);
    assert_eq!(txns.iter().map(|t| t.0).collect::<Vec<_>>(), through);
    assert!(txns.iter().all(|t| t.1 != "Next year's rent"), "{iif}");
    assert_eq!(iif_balances(iif), tb_without_closes(ledger, as_of));
    assert!(text(&p, "README.txt").contains(
        "general-journal.iif: every posted entry from the first through 2026-12-31, for import into QuickBooks Desktop. Plain ASCII text (its import turns anything else into \"?\"), CR LF line endings; DOCNUM is each entry's id in these books."
    ));
}

/// B-33 at its edges, through ledger_package::export_iif (the package's
/// own): an entry dated as_of is in and one dated the day after is not; a
/// reversal after as_of leaves its original in, under the original's id;
/// as of a day before the first entry the file declares the accounts and
/// holds no transaction.
#[test]
fn the_iif_through_as_of_at_its_edges() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, rent) = plain_books(dir.path());
    let fee = post(
        &mut s,
        "2026-03-31",
        "Bank fee",
        &[
            ("6000", Side::Debit, "5.00"),
            ("1000", Side::Credit, "5.00"),
        ],
    );
    let reversal = s
        .reverse_entry(rent, d("2026-04-01"), "Rent refunded")
        .unwrap();
    let ledger = s.ledger();
    let cash = cash();
    let ids = |as_of: &str| -> Vec<u64> {
        let iif = ledger_package::export_iif(ledger, &cash, d(as_of)).unwrap();
        assert!(iif.changes.is_empty());
        assert_eq!(iif_balances(&iif.text), tb_without_closes(ledger, d(as_of)));
        iif_txns(&iif.text).iter().map(|t| t.0).collect()
    };
    assert_eq!(ids("2026-02-28"), Vec::<u64>::new());
    assert_eq!(ids("2026-03-01"), [rent]);
    assert_eq!(ids("2026-03-30"), [rent]);
    assert_eq!(ids("2026-03-31"), [rent, fee]);
    assert_eq!(ids("2026-04-01"), [rent, fee, reversal]);
    // Before the first entry: every account, no transaction.
    let empty = ledger_package::export_iif(ledger, &cash, d("2026-02-28")).unwrap();
    let accnt = empty
        .text
        .split("\r\n")
        .filter(|l| l.starts_with("ACCNT\t"))
        .count();
    assert_eq!(accnt, ledger.chart().len());
    assert!(!empty.text.contains("\r\nTRNS\t"));
    assert!(empty.text.ends_with("!ENDTRNS\r\n"));
}

/// The books as the package reads them, the journal's length fixed: it is
/// the one thing later activity rightly changes in a package as of an
/// earlier day (every report's footer and the README name it).
struct AtSeq<'a>(&'a Store, u64);

impl PackageBooks for AtSeq<'_> {
    fn ledger(&self) -> &Ledger {
        PackageBooks::ledger(self.0)
    }
    fn journal_seq(&self) -> u64 {
        self.1
    }
    fn all_attachments(&self) -> Result<Vec<ledger_store::Attachment>, ledger_store::StoreError> {
        PackageBooks::all_attachments(self.0)
    }
    fn links_of(&self, id: u64) -> Result<Vec<Subject>, ledger_store::StoreError> {
        PackageBooks::links_of(self.0, id)
    }
    fn attachment_bytes(&self, id: u64) -> Result<Vec<u8>, ledger_store::StoreError> {
        PackageBooks::attachment_bytes(self.0, id)
    }
}

/// [`books`] with equipment accounts and a bank fee dated `as_of` with its
/// receipt; with `desk`, a desk bought in January and registered, its
/// depreciation posted through as_of's month. With `later`, the days after
/// as well: a payment to the 1099 vendor marked "AFTER AS_OF" with its
/// receipt, a laptop bought and registered as a fixed asset, the bank fee
/// reversed, the members contributing again, and the desk (if any) sold.
fn books_around(dir: &Path, as_of: NaiveDate, later: bool, desk: bool) -> Books {
    let mut b = books(dir);
    let s = &mut b.store;
    for (code, name, ty) in [
        ("1500", "Equipment", AccountType::Asset),
        ("1510", "Accumulated depreciation", AccountType::Asset),
        ("1600", "Furniture", AccountType::Asset),
        (
            "1610",
            "Accumulated depreciation, furniture",
            AccountType::Asset,
        ),
        ("6500", "Depreciation", AccountType::Expense),
        ("7900", "Gain or loss on disposal", AccountType::Revenue),
    ] {
        s.add_account(code, name, ty, None, false).unwrap();
    }
    let fee = post(
        s,
        &as_of.to_string(),
        "Bank fee on the day",
        &[
            ("6000", Side::Debit, "35.00"),
            ("1000", Side::Credit, "35.00"),
        ],
    );
    s.attach_receipt(Some(fee), "fee.pdf", "application/pdf", b"%PDF fee", None)
        .unwrap();
    let desk = desk.then(|| {
        post(
            s,
            "2026-01-10",
            "Desk",
            &[
                ("1600", Side::Debit, "1200.00"),
                ("1000", Side::Credit, "1200.00"),
            ],
        );
        let (cost, accum, expense) = (id_of(s, "1600"), id_of(s, "1610"), id_of(s, "6500"));
        let id = s
            .register_asset(ledger_core::NewAsset {
                description: "Desk".into(),
                purchase_date: d("2026-01-10"),
                cost: usd("1200.00"),
                salvage_value: usd("0.00"),
                useful_life_months: 60,
                cost_account: cost,
                accum_depr_account: accum,
                expense_account: expense,
            })
            .unwrap();
        for m in 1..=chrono::Datelike::month(&as_of) {
            s.run_depreciation(id, &format!("2026-{m:02}")).unwrap();
        }
        id
    });
    if later {
        let next = as_of.succ_opt().unwrap();
        let e = s.create_entry(next, "AFTER AS_OF").unwrap();
        s.add_line(e, id_of(s, "6100"), Side::Debit, usd("777.77"))
            .unwrap();
        s.add_line(e, id_of(s, "1000"), Side::Credit, usd("777.77"))
            .unwrap();
        s.set_entry_vendor(e, Some(b.vendor)).unwrap();
        s.post_entry(e).unwrap();
        s.attach_receipt(Some(e), "after.pdf", "application/pdf", b"%PDF after", None)
            .unwrap();
        let bought = next.succ_opt().unwrap();
        post(
            s,
            &bought.to_string(),
            "Laptop",
            &[
                ("1500", Side::Debit, "1800.00"),
                ("1000", Side::Credit, "1800.00"),
            ],
        );
        let (cost, accum, expense) = (id_of(s, "1500"), id_of(s, "1510"), id_of(s, "6500"));
        s.register_asset(ledger_core::NewAsset {
            description: "Laptop".into(),
            purchase_date: bought,
            cost: usd("1800.00"),
            salvage_value: usd("0.00"),
            useful_life_months: 36,
            cost_account: cost,
            accum_depr_account: accum,
            expense_account: expense,
        })
        .unwrap();
        s.reverse_entry(fee, next, "Bank fee refunded").unwrap();
        post(
            s,
            &next.to_string(),
            "Members' contributions, again",
            &[
                ("1000", Side::Debit, "600.00"),
                ("3000", Side::Credit, "300.00"),
                ("3001", Side::Credit, "300.00"),
            ],
        );
        if let Some(desk) = desk {
            let (cash, gain) = (id_of(s, "1000"), id_of(s, "7900"));
            // Sold at a gain (its book value is at most 1,080.00), credited
            // to 7900.
            s.dispose_asset(desk, bought, usd("1150.00"), cash, gain)
                .unwrap();
        }
    }
    b
}

/// Plan P3, defect 21: "a test pins every file in an as-of package to
/// it". B-33: no figure, entry, receipt or worksheet row reflects anything
/// dated after as_of. Two sets of books, the same through as_of, one with
/// activity in the days after (a 1099 payment with its receipt, an asset
/// bought, a reversal of an entry dated as_of, the members' contributions,
/// a desk bought before as_of sold), make the same package byte for byte,
/// every file and which files there are: statements (PDF and XLSX),
/// ledgers, the fixed asset register, the IIF, the 1099 and K-1
/// worksheets, the receipts and the README; with and without TINs and
/// receipts; with and without the desk (so with and without a register);
/// mid-year and at year end. And the later activity is no phantom: a
/// package a few days later shows it.
#[test]
fn every_file_in_an_as_of_package_stops_at_as_of() {
    let company = company(true);
    let cash = cash();
    for (as_of, desk) in [
        ("2026-06-30", false),
        ("2026-06-30", true),
        ("2026-12-31", false),
        ("2026-12-31", true),
    ] {
        let as_of = d(as_of);
        let dir = tempfile::tempdir().unwrap();
        let without = books_around(&dir.path().join("without"), as_of, false, desk);
        let with = books_around(&dir.path().join("with"), as_of, true, desk);
        let (src_without, src_with) = (tin_source(&without), tin_source(&with));
        for receipts in [true, false] {
            for tins in [false, true] {
                let build =
                    |b: &Books, src: &dyn Fn(PartyId) -> Result<ledger_vault::Tin, String>| {
                        let req = PackageRequest {
                            as_of,
                            ..request(receipts, tins.then_some(src))
                        };
                        build_package(&AtSeq(&b.store, 99), &settings(&company, &cash), &req)
                            .unwrap()
                    };
                let a = build(&without, &src_without);
                let b = build(&with, &src_with);
                assert_eq!(names(&a), names(&b), "as of {as_of}");
                assert_eq!(
                    names(&a).contains(&"fixed-asset-register.pdf"),
                    desk,
                    "as of {as_of}"
                );
                for ((name, x), (_, y)) in stable(&a.files).iter().zip(stable(&b.files)) {
                    assert!(*x == y, "{name}, as of {as_of}, shows what came after");
                }
                assert_eq!(a.tin_gaps, b.tin_gaps);
                assert_eq!(a.receipt_count, b.receipt_count);
                assert_eq!(a.warnings, b.warnings);
                assert_eq!(a.iif_changes, b.iif_changes);
            }
        }
        // A package days on sees the later activity.
        let later = PackageRequest {
            as_of: as_of + chrono::Duration::days(3),
            ..request(true, None)
        };
        let p = build_package(&with.store, &settings(&company, &cash), &later).unwrap();
        let iif = text(&p, "general-journal.iif");
        for memo in [
            "AFTER AS_OF",
            "Bank fee refunded",
            "Members' contributions, again",
        ] {
            assert!(iif.contains(memo), "{memo}");
        }
        assert!(names(&p).contains(&"fixed-asset-register.pdf"));
        assert!(names(&p).iter().any(|n| n.ends_with("-after.pdf")));
    }
}

/// Plan D-36 (P3, defect 21): the 1099 worksheet stops at the package's
/// as_of, like every other file in it: the payments from January 1
/// through as_of (§6041(a) and §6041A(a)(2) count what is paid in the
/// calendar year, so a package as of June 30 shows the year to date, and a
/// vendor not yet at the threshold is not yet reportable). A payment dated
/// as_of counts; one the day after doesn't. A year-end package is
/// unchanged, its README too.
#[test]
fn the_1099_worksheet_stops_at_as_of() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = plain_books(dir.path());
    s.add_account("6100", "Contract labor", AccountType::Expense, None, false)
        .unwrap();
    let vendor = s.add_vendor("Contractor Example").unwrap();
    s.set_vendor_1099(vendor, Some(Form1099::Nec), false)
        .unwrap();
    for (date, amount) in [("2026-06-30", "1500.00"), ("2026-07-01", "1000.00")] {
        let e = s.create_entry(d(date), "Contract work").unwrap();
        s.add_line(e, id_of(&s, "6100"), Side::Debit, usd(amount))
            .unwrap();
        s.add_line(e, id_of(&s, "1000"), Side::Credit, usd(amount))
            .unwrap();
        s.set_entry_vendor(e, Some(vendor)).unwrap();
        s.post_entry(e).unwrap();
    }
    let company = company(false);
    let cash = cash();
    let at = |as_of: &str| {
        let req = PackageRequest {
            as_of: d(as_of),
            ..request(false, None)
        };
        build_package(&s, &settings(&company, &cash), &req).unwrap()
    };
    let mid = at("2026-06-30");
    assert_eq!(
        text(&mid, "1099-worksheet-2026.csv"),
        "vendor,form,w9_on_file,paid,reportable\n\"Contractor Example\",1099-NEC,false,1500.00,false\n"
    );
    assert!(text(&mid, "README.txt").contains(
        "\n1099-worksheet-2026.csv: cash paid per vendor marked for a 1099 from January 1 through 2026-06-30, against the year's threshold.\n"
    ));
    let year_end = at("2026-12-31");
    assert_eq!(
        text(&year_end, "1099-worksheet-2026.csv"),
        "vendor,form,w9_on_file,paid,reportable\n\"Contractor Example\",1099-NEC,false,2500.00,true\n"
    );
    assert!(text(&year_end, "README.txt").contains(
        "\n1099-worksheet-2026.csv: cash paid per vendor marked for a 1099, against the year's threshold.\n"
    ));
    // The worksheet the tools make for the whole year is unchanged.
    let w =
        vendor_1099_worksheet(s.ledger(), &settings(&company, &cash), 2026, None, None).unwrap();
    assert_eq!(w.lines[0].paid, Decimal::from(2500));
}

/// Plan D-36: the IIF leaves the books' period closes out. QuickBooks
/// keeps no closing entries (it works out retained earnings itself), so an
/// imported close would zero its period's profit there. Two months
/// closed: no close is written; the file's income and expenses are the
/// profit and loss for the year to date; its balances are the trial
/// balance without the closes; and the README says so.
#[test]
fn iif_leaves_out_period_closes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = plain_books(dir.path());
    s.add_account("4000", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    post(
        &mut s,
        "2026-01-15",
        "January sale",
        &[
            ("1000", Side::Debit, "5000.00"),
            ("4000", Side::Credit, "5000.00"),
        ],
    );
    post(
        &mut s,
        "2026-02-10",
        "February sale",
        &[
            ("1000", Side::Debit, "700.00"),
            ("4000", Side::Credit, "700.00"),
        ],
    );
    let mut closes = Vec::new();
    for (name, start, end) in [
        ("2026-01", "2026-01-01", "2026-01-31"),
        ("2026-02", "2026-02-01", "2026-02-28"),
    ] {
        let id = s.create_period(name, d(start), d(end)).unwrap();
        closes.push(s.close_period(id).unwrap().expect("income to close"));
    }
    let ledger = s.ledger();
    assert_eq!(
        ledger.closing_entry_ids().into_iter().collect::<Vec<_>>(),
        closes
    );
    let as_of = d("2026-03-31");
    let company = company(false);
    let cash = cash();
    let req = PackageRequest {
        as_of,
        ..request(false, None)
    };
    let p = build_package(&s, &settings(&company, &cash), &req).unwrap();
    let iif = text(&p, "general-journal.iif");
    let written: Vec<u64> = iif_txns(iif).iter().map(|t| t.0).collect();
    let want: Vec<u64> = ledger
        .posted_entries()
        .filter(|e| e.date() <= as_of && !closes.contains(&e.id()))
        .map(|e| e.id())
        .collect();
    assert_eq!(written, want);
    assert_eq!(want.len(), 3, "two sales and the rent");
    let balances = iif_balances(iif);
    assert_eq!(balances, tb_without_closes(ledger, as_of));
    // Income and expenses: the year to date's profit and loss.
    let pl = ledger_core::reports::profit_and_loss_for_period(ledger, d("2026-01-01"), as_of, None)
        .unwrap();
    for l in &pl.revenue {
        assert_eq!(balances[&l.code], -l.amount, "{}", l.code);
    }
    for l in &pl.expenses {
        assert_eq!(balances[&l.code], l.amount, "{}", l.code);
    }
    assert_eq!(pl.total_revenue - pl.total_expenses, Decimal::from(4500));
    // The package's own trial balance keeps the closes: income zeroed into
    // Retained Earnings through February.
    let with_closes = ledger_core::reports::trial_balance_as_of(ledger, as_of);
    assert!(with_closes.lines.iter().any(|l| l.code == "3999"));
    assert!(!balances.contains_key("3999"));
    assert!(text(&p, "README.txt").contains(
        "Period-closing entries are left out: QuickBooks keeps none and works out retained earnings itself, so an imported close would zero its period's profit; trial-balance includes them.\n"
    ));
}

/// Plan P3 review R18: one list of cash accounts per package. The
/// request's (savings with checking) types the IIF's BANK accounts and
/// counts the 1099 payments the same as it reconciles the cash flow;
/// without one, the settings' (checking alone) do all three.
#[test]
fn one_list_of_cash_accounts_per_package() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = plain_books(dir.path());
    s.add_account("1010", "Savings", AccountType::Asset, None, false)
        .unwrap();
    s.add_account("6100", "Contract labor", AccountType::Expense, None, false)
        .unwrap();
    post(
        &mut s,
        "2026-01-05",
        "Transfer to savings",
        &[
            ("1010", Side::Debit, "3000.00"),
            ("1000", Side::Credit, "3000.00"),
        ],
    );
    let vendor = s.add_vendor("Contractor Example").unwrap();
    s.set_vendor_1099(vendor, Some(Form1099::Nec), false)
        .unwrap();
    let e = s.create_entry(d("2026-05-01"), "Contract work").unwrap();
    s.add_line(e, id_of(&s, "6100"), Side::Debit, usd("2100.00"))
        .unwrap();
    s.add_line(e, id_of(&s, "1010"), Side::Credit, usd("2100.00"))
        .unwrap();
    s.set_entry_vendor(e, Some(vendor)).unwrap();
    s.post_entry(e).unwrap();
    let company = company(false);
    let cash = cash();
    let both = vec!["1000".to_string(), "1010".to_string()];
    let mut req = request(false, None);
    req.cash_accounts = &both;
    let given = build_package(&s, &settings(&company, &cash), &req).unwrap();
    let plain = build_package(&s, &settings(&company, &cash), &request(false, None)).unwrap();
    let savings = |p: &Package| {
        text(p, "general-journal.iif")
            .split("\r\n")
            .find(|l| l.starts_with("ACCNT\tSavings\t"))
            .unwrap()
            .to_string()
    };
    assert_eq!(savings(&given), "ACCNT\tSavings\tBANK\t1010");
    assert_eq!(savings(&plain), "ACCNT\tSavings\tOCASSET\t1010");
    assert!(text(&given, "1099-worksheet-2026.csv").contains(",2100.00,true\n"));
    assert!(text(&plain, "1099-worksheet-2026.csv").contains(",0.00,false\n"));
}

/// Plan D-36 (P3, defect 7): general-journal.iif is plain ASCII, and
/// everything it writes differently is in the package's warnings and the
/// README: each account name a line; the memos grouped by what is written
/// differently, each with the entries it is in (lead's call, 2026-10-20),
/// so the em dash of many memos is one line; every other file keeps the
/// books' text. Books whose names and memos are plain ASCII get no list.
#[test]
fn the_readme_lists_what_the_iif_writes_differently() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = plain_books(dir.path());
    s.add_account("6200", "Café 北京", AccountType::Expense, None, false)
        .unwrap();
    let lunch = post(
        &mut s,
        "2026-04-02",
        "Lunch \u{2014} \u{201C}Zoë\u{201D}\nteam",
        &[
            ("6200", Side::Debit, "42.00"),
            ("1000", Side::Credit, "42.00"),
        ],
    );
    let mut months = Vec::new();
    for m in 4..=9 {
        months.push(post(
            &mut s,
            &format!("2026-{m:02}-28"),
            &format!("Depreciation \u{2014} Laptop \u{2014} 2026-{m:02}"),
            &[
                ("6000", Side::Debit, "50.00"),
                ("1000", Side::Credit, "50.00"),
            ],
        ));
    }
    let company = company(false);
    let cash = cash();
    let p = build_package(&s, &settings(&company, &cash), &request(false, None)).unwrap();
    let dashes: Vec<String> = std::iter::once(lunch)
        .chain(months.iter().copied())
        .map(|e| e.to_string())
        .collect();
    let want = [
        "account 6200's name \"Café 北京\" is written \"Cafe Bei Jing\"".to_string(),
        format!(
            "\"—\" is written \"--\" in the memos of entries {}",
            dashes.join(", ")
        ),
        format!("\"“\" is written \"'\" in the memo of entry {lunch}"),
        // "ë", "”" and the line break are side by side: one run.
        format!("\"ë”\\n\" is written \"e' \" in the memo of entry {lunch}"),
    ];
    assert_eq!(p.warnings, want);
    assert_eq!(p.iif_changes, 1 + 1 + 6, "the name, the lunch, six months");
    let readme = text(&p, "README.txt");
    let mut block = String::from(
        "Written differently in general-journal.iif to fit QuickBooks Desktop (plain ASCII; account names at most 31 characters a level, no two alike; a subaccount of another QuickBooks type than its parent's at the top level; account numbers at most 7 characters, no two alike; memos at most 4095 characters); every other file keeps the books' text:\n",
    );
    for w in &want {
        block.push_str(&format!("- {w}\n"));
    }
    assert!(readme.contains(&block), "{readme}");
    let iif = text(&p, "general-journal.iif");
    assert!(iif.is_ascii() && iif.contains("\tCafe Bei Jing\t"));
    // The general journal workbook keeps the books' own text.
    let gj = xlsx_strings(&p, "general-journal.xlsx");
    assert!(
        gj.contains("Café 北京") && gj.contains("Lunch — “Zoë”"),
        "{gj}"
    );
    // ASCII books: no list.
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = plain_books(dir.path());
    let p = build_package(&s, &settings(&company, &cash), &request(false, None)).unwrap();
    assert!(p.warnings.is_empty() && p.iif_changes == 0);
    assert!(!text(&p, "README.txt").contains("Written differently"));
}

/// B-34 rule 7 (P3 review round 2): the file names no customer or vendor
/// (Intuit states no such need for an IIF journal; QuickBooks Desktop asks
/// for one on a journal entry to accounts receivable or payable, and takes
/// one such account a transaction: sources/intuit-iif.md), so the README
/// names the entries with a line on either, in file order, and says what
/// QuickBooks may ask. An entry on both is among them. Books with neither
/// get no such sentence.
#[test]
fn the_readme_names_the_entries_on_receivables_and_payables() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, rent) = plain_books(dir.path());
    for (code, name, ty) in [
        ("1200", "Accounts Receivable", AccountType::Asset),
        ("2000", "Accounts Payable", AccountType::Liability),
        ("4000", "Sales", AccountType::Revenue),
    ] {
        s.add_account(code, name, ty, None, false).unwrap();
    }
    let invoiced = post(
        &mut s,
        "2026-04-01",
        "Invoice 7",
        &[
            ("1200", Side::Debit, "500.00"),
            ("4000", Side::Credit, "500.00"),
        ],
    );
    let billed = post(
        &mut s,
        "2026-04-02",
        "Bill 3",
        &[
            ("6000", Side::Debit, "80.00"),
            ("2000", Side::Credit, "80.00"),
        ],
    );
    let netted = post(
        &mut s,
        "2026-04-03",
        "Netted",
        &[
            ("2000", Side::Debit, "80.00"),
            ("1200", Side::Credit, "80.00"),
        ],
    );
    let company = company(false);
    let cash = cash();
    let p = build_package(&s, &settings(&company, &cash), &request(false, None)).unwrap();
    let iif = text(&p, "general-journal.iif");
    assert!(
        iif.contains("ACCNT\tAccounts Receivable\tAR\t1200\r\n"),
        "{iif}"
    );
    assert!(
        iif.contains("ACCNT\tAccounts Payable\tAP\t2000\r\n"),
        "{iif}"
    );
    // No NAME column on the transactions.
    assert!(
        iif.contains("\r\n!TRNS\tTRNSID\tTRNSTYPE\tDATE\tACCNT\tCLASS\tAMOUNT\tDOCNUM\tMEMO\r\n")
    );
    assert_eq!(
        iif_txns(iif).iter().map(|t| t.0).collect::<Vec<_>>(),
        [rent, invoiced, billed, netted]
    );
    let readme = text(&p, "README.txt");
    assert!(
        readme.contains(&format!(
            "\nIt names no customer or vendor on its lines to accounts receivable or payable, in entries {invoiced}, {billed}, {netted}: QuickBooks Desktop asks for one on a journal entry to A/R or A/P (and takes only one A/R or A/P account a transaction); add it there if QuickBooks asks.\n"
        )),
        "{readme}"
    );
    // One entry: "entry", not "entries".
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = plain_books(dir.path());
    s.add_account(
        "1200",
        "Accounts Receivable",
        AccountType::Asset,
        None,
        false,
    )
    .unwrap();
    let one = post(
        &mut s,
        "2026-04-01",
        "Owed",
        &[
            ("1200", Side::Debit, "5.00"),
            ("1000", Side::Credit, "5.00"),
        ],
    );
    let p = build_package(&s, &settings(&company, &cash), &request(false, None)).unwrap();
    assert!(text(&p, "README.txt").contains(&format!(
        "lines to accounts receivable or payable, in entry {one}: "
    )));
    // Neither: no sentence.
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = plain_books(dir.path());
    let p = build_package(&s, &settings(&company, &cash), &request(false, None)).unwrap();
    assert!(!text(&p, "README.txt").contains("customer or vendor"));
}

/// Every text the package's XLSX `name` holds: its XML parts (the shared
/// strings and the sheets), concatenated, `&amp;` read as `&`.
fn xlsx_strings(p: &Package, name: &str) -> String {
    let (_, bytes) = p
        .files
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("{name} in {:?}", names(p)));
    let mut x = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = String::new();
    for i in 0..x.len() {
        let mut part = x.by_index(i).unwrap();
        if part.name().unwrap().ends_with(".xml") {
            part.read_to_string(&mut out).unwrap();
        }
    }
    out.replace("&amp;", "&")
}

/// Random books for [`iif_ties_to_the_trial_balance_as_of`]: entries of
/// (day of 2026, debit account, credit account, cents), reversals of some
/// of them a few days on, and the first `closed` months closed.
fn random_ledger(
    entries: &[(i64, usize, usize, i64)],
    reversals: &[(proptest::sample::Index, i64)],
    closed: u32,
) -> Ledger {
    let mut l = Ledger::new(Currency::USD);
    let mut ids = Vec::new();
    for (code, name, ty) in [
        ("1000", "Checking", AccountType::Asset),
        ("1100", "Receivables", AccountType::Asset),
        ("2000", "Card", AccountType::Liability),
        ("3000", "Capital", AccountType::Equity),
        ("4000", "Sales", AccountType::Revenue),
        ("6000", "Rent", AccountType::Expense),
    ] {
        ids.push(l.add_account(code, name, ty, None, false).unwrap());
    }
    let jan1 = d("2026-01-01");
    let mut posted = Vec::new();
    for (i, &(day, dr, cr, cents)) in entries.iter().enumerate() {
        let cr = if cr == dr { (dr + 1) % ids.len() } else { cr };
        let e = l.create_entry(jan1 + chrono::Duration::days(day), &format!("Entry {i}"));
        let m = Money::new(Decimal::new(cents, 2), Currency::USD);
        l.add_line(e, ids[dr], Side::Debit, m).unwrap();
        l.add_line(e, ids[cr], Side::Credit, m).unwrap();
        l.post_entry(e).unwrap();
        posted.push(e);
    }
    for (pick, days) in reversals {
        if posted.is_empty() {
            break;
        }
        let e = posted[pick.index(posted.len())];
        let on = l.get_entry(e).unwrap().date() + chrono::Duration::days(*days);
        // An entry is reversed at most once (B-10): a second pick is skipped.
        let _ = l.reverse_entry(e, on, "Reversal");
    }
    for m in 1..=closed {
        let start = NaiveDate::from_ymd_opt(2026, m, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, m + 1, 1)
            .unwrap()
            .pred_opt()
            .unwrap();
        let p = l
            .create_period(&format!("2026-{m:02}"), start, end)
            .unwrap();
        l.close_period(p).unwrap();
    }
    l
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2000))]

    /// B-33 and B-34 over random books, random as_of (before the first
    /// entry, on an entry's date, after the last), reversals on either side
    /// of it and closed months: the IIF holds exactly the posted entries
    /// dated on or before as_of that aren't closes, in posting order, each
    /// numbered with its id and summing to zero; it declares every account;
    /// its balances are the trial balance as of as_of without the closes.
    #[test]
    fn iif_ties_to_the_trial_balance_as_of(
        entries in proptest::collection::vec((0i64..120, 0usize..6, 0usize..6, 1i64..10_000_000), 0..20),
        reversals in proptest::collection::vec((proptest::prelude::any::<proptest::sample::Index>(), 0i64..40), 0..4),
        closed in 0u32..4,
        as_of in -5i64..170,
    ) {
        let l = random_ledger(&entries, &reversals, closed);
        let as_of = d("2026-01-01") + chrono::Duration::days(as_of);
        let iif = ledger_package::export_iif(&l, &cash(), as_of).unwrap();
        let closes = l.closing_entry_ids();
        let want: Vec<u64> = l
            .posted_entries()
            .filter(|e| e.date() <= as_of && !closes.contains(&e.id()))
            .map(|e| e.id())
            .collect();
        let txns = iif_txns(&iif.text);
        proptest::prop_assert_eq!(txns.iter().map(|t| t.0).collect::<Vec<_>>(), want);
        for (id, _, lines) in &txns {
            proptest::prop_assert_eq!(lines.iter().map(|(_, a)| *a).sum::<Decimal>(), Decimal::ZERO, "entry {}", id);
        }
        let declared = iif.text.split("\r\n").filter(|l| l.starts_with("ACCNT\t")).count();
        proptest::prop_assert_eq!(declared, l.chart().len());
        proptest::prop_assert_eq!(iif_balances(&iif.text), tb_without_closes(&l, as_of));
        proptest::prop_assert!(iif.changes.is_empty());
    }
}

/// The bookspec example `bookspec/examples/iif-quickbooks-journal-file.toml`
/// (P3's accept: each regression test also a bookspec example), its
/// expected values derived by hand, against the engine: the example's
/// books, built from its setup and events, export the very file and
/// changes it says.
#[test]
fn the_bookspec_iif_example_matches_the_engine() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../bookspec/examples/iif-quickbooks-journal-file.toml"
    );
    let spec: toml::Value = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let setup = &spec["setup"];
    let mut l = Ledger::new(Currency::USD);
    for a in setup["accounts"].as_array().unwrap() {
        let ty = match a["type"].as_str().unwrap() {
            "asset" => AccountType::Asset,
            "liability" => AccountType::Liability,
            "equity" => AccountType::Equity,
            "revenue" => AccountType::Revenue,
            _ => AccountType::Expense,
        };
        let parent = a
            .get("parent")
            .map(|p| l.chart().find_by_code(p.as_str().unwrap()).unwrap().id);
        l.add_account(
            a["code"].as_str().unwrap(),
            a["name"].as_str().unwrap(),
            ty,
            parent,
            false,
        )
        .unwrap();
    }
    for (i, e) in spec["events"].as_array().unwrap().iter().enumerate() {
        let id = l.create_entry(
            d(e["date"].as_str().unwrap()),
            e["description"].as_str().unwrap(),
        );
        assert_eq!(
            id,
            i as u64 + 1,
            "an event's entry id is its index plus one"
        );
        for line in e["lines"].as_array().unwrap() {
            let account = l
                .chart()
                .find_by_code(line["account"].as_str().unwrap())
                .unwrap()
                .id;
            let side = match line["side"].as_str().unwrap() {
                "debit" => Side::Debit,
                _ => Side::Credit,
            };
            l.add_line(id, account, side, usd(line["amount"].as_str().unwrap()))
                .unwrap();
        }
        l.post_entry(id).unwrap();
    }
    let cash: Vec<String> = setup["cash_accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    let strs = |v: &toml::Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect()
    };
    let mut checked = 0;
    for x in spec["expect"].as_array().unwrap() {
        let iif = ledger_package::export_iif(&l, &cash, d(x["as_of"].as_str().unwrap())).unwrap();
        match x["query"].as_str().unwrap() {
            "iif_lines" => {
                let mut want = strs(&x["values"]).join("\r\n");
                want.push_str("\r\n");
                assert_eq!(iif.text, want);
            }
            "iif_changes" => {
                let got: Vec<Vec<String>> = iif
                    .changes
                    .iter()
                    .map(|c| {
                        let what = match &c.field {
                            ledger_import::IifField::AccountName { code } => format!("name {code}"),
                            ledger_import::IifField::Memo { entry } => format!("memo {entry}"),
                            ledger_import::IifField::AccountNumber
                            | ledger_import::IifField::AccountNumberLeftOut { .. } => {
                                "number".to_string()
                            }
                            ledger_import::IifField::AtTopLevel { code, .. } => {
                                format!("top level {code}")
                            }
                        };
                        vec![what, c.books.clone(), c.written.clone()]
                    })
                    .collect();
                let want: Vec<Vec<String>> =
                    x["values"].as_array().unwrap().iter().map(strs).collect();
                assert_eq!(got, want);
            }
            q => panic!("unknown query {q}"),
        }
        checked += 1;
    }
    assert_eq!(checked, 2);
}

/// The package's statement of cash flows is the direct method's (rules.md
/// B-14, D-35): operating by ASC 230-10-45-25 class, financing by kind
/// with each member named, and both schedules: the reconciliation of net
/// income to operating cash (45-30) and the noncash investing and
/// financing activities (50-3; "None" here: every entry moved cash).
#[test]
fn the_cash_flow_carries_both_schedules() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let company = company(true);
    let cash = cash();
    let p = build_package(&b.store, &settings(&company, &cash), &request(true, None)).unwrap();
    let text = xlsx_strings(&p, "cash-flow.xlsx");
    for m in &company.members {
        let want = format!("Contributions from members — {}", m.name);
        assert!(text.contains(&want), "{want:?} not in the cash flow");
    }
    for want in [
        "Statement of Cash Flows",
        "Cash received from customers",
        "Cash paid to suppliers and employees",
        "Members' contributions (#",
        "Reconciliation of net income (loss) to net cash from operating activities",
        "Noncash investing and financing activities",
        "None",
    ] {
        assert!(text.contains(want), "{want:?} not in the cash flow");
    }
    assert!(!text.contains("Member distributions"));
    assert!(p.files.iter().any(|(n, _)| n == "cash-flow.pdf"));
}
