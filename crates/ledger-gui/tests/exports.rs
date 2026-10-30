//! What the viewer saves for the human (plan 3.6, P2's viewer menu,
//! D-29): a copy of a document, a party's W-9, the CPA package and the
//! 1099 worksheet, each a new file in the downloads folder, never over
//! one; and for a book in the data root, the vault is the book's own.
//! Nothing here writes the books.

use chrono::NaiveDate;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::{AccountType, Currency, Money, Role, Side, TinFacts, TinType};
use ledger_gui::App;
use ledger_gui::books::Books;
use ledger_gui::data::{self, Source};
use ledger_gui::menu::Job;
use ledger_gui::nav::Route;
use ledger_store::Store;
use ledger_vault::{Tin, Vault, W9File};
use rust_decimal::Decimal;
use std::io::Read;
use std::path::{Path, PathBuf};

const SSN: &str = "620-37-1111";

/// This test binary's downloads folder: its own, never the human's.
static DOWNLOADS: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

unsafe extern "C" {
    /// The C library's: runs `f` as the process exits, which libtest's
    /// main does pass or fail (it returns, or calls `exit`).
    safe fn atexit(f: extern "C" fn()) -> std::ffi::c_int;
}

/// Removes the downloads folder as the test binary exits: its files hold
/// (made-up) TINs, and every test's dir is removed when it ends (the
/// justfile's sweep-test-tmp catches a run killed first: it is `wb-*`).
extern "C" fn remove_downloads() {
    if let Some(d) = DOWNLOADS.get() {
        let _ = std::fs::remove_dir_all(d);
    }
}

fn downloads() -> PathBuf {
    DOWNLOADS
        .get_or_init(|| {
            let d = tempfile::Builder::new()
                .prefix("wb-gui-downloads-")
                .tempdir()
                .unwrap()
                .keep();
            assert_eq!(atexit(remove_downloads), 0, "atexit refused");
            ledger_gui::export::set_downloads_dir(d.clone());
            d
        })
        .clone()
}

/// A fresh folder in the downloads folder for one test (tests run side by
/// side): the files saved there are this test's.
fn saved_files() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(downloads())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 15).unwrap()
}

fn usd(s: &str) -> Money {
    Money::new(s.parse::<Decimal>().unwrap(), Currency::USD)
}

/// A book in a data root (its folder holds the root's marker), with a
/// checking account, the member's capital, contract labor, a contractor
/// paid past the threshold, a member whose SSN is in the book's own vault
/// (key in its own keys/), a contractor's W-9 there too, and a document.
struct RootBook {
    _root: tempfile::TempDir,
    book: PathBuf,
    member: u64,
    payee: u64,
    document: u64,
}

/// A 1099-NEC decision from the beginning, decided by the test.
fn nec() -> ledger_core::Dated1099 {
    ledger_core::Dated1099::from_beginning(
        ledger_core::Decision1099::Files(ledger_core::Form1099::Nec),
        Some(ledger_core::settings::Source {
            decided_by: "test".into(),
            decided_on: today(),
            citation: None,
            note: None,
        }),
    )
}

fn root_book() -> RootBook {
    let root = tempfile::Builder::new()
        .prefix("wb-gui-root-")
        .tempdir()
        .unwrap();
    std::fs::write(root.path().join(ledger_store::root::MARKER_FILE), b"").unwrap();
    let book = root.path().join("books").join("acme");
    std::fs::create_dir_all(&book).unwrap();
    let mut s = Store::create(&book, Currency::USD).unwrap();
    let import = ledger_config::codec::import(
        "[company]\nname = \"Acme LLC\"\n\n[[company.members]]\nname = \"Alex Owner\"\nshare = \"1\"\ncapital_account = \"3000\"\n\n[books]\ncash_accounts = [\"1000\"]\n",
        None,
        s.ledger().settings(),
        "USD",
    )
    .unwrap();
    s.adopt_settings(
        &import.changes,
        &ledger_core::settings::Source {
            decided_by: "test".into(),
            decided_on: today(),
            citation: None,
            note: None,
        },
    )
    .unwrap();
    for (code, name, ty) in [
        ("1000", "Checking", AccountType::Asset),
        ("3000", "Capital", AccountType::Equity),
        ("6100", "Contract labor", AccountType::Expense),
    ] {
        s.add_account(code, name, ty, None, false).unwrap();
    }
    let member = s.add_party("Alex Owner", &[Role::Member]).unwrap();
    let payee = s.add_party("Jo Contractor", &[Role::Contractor]).unwrap();
    // The vault and its key, the book's own (as the server keeps them).
    let key = book
        .join(ledger_store::root::KEYS_DIR)
        .join(ledger_store::root::LEGACY_KEY_FILE);
    std::fs::create_dir_all(key.parent().unwrap()).unwrap();
    let v = Vault::open(&book.join(ledger_store::root::VAULT_DIR), &key, true).unwrap();
    let hash = v
        .put_tin(member, &Tin::new(SSN, ledger_vault::TinType::Ssn).unwrap())
        .unwrap();
    let w9_hash = v
        .put_w9(
            payee,
            &W9File {
                file_name: "jo-w9.pdf".into(),
                mime_type: "application/pdf".into(),
                bytes: b"%PDF-1.4 Jo's W-9".to_vec(),
            },
        )
        .unwrap();
    s.set_party_w9(
        payee,
        Some(ledger_core::W9Facts {
            received: today(),
            entity_type: "individual/sole proprietor".into(),
            exempt_payee_code: None,
            fatca_code: None,
            tin_last4: "2222".into(),
            kept_at: "wanderbooks vault".into(),
            vault_record: Some(w9_hash),
        }),
    )
    .unwrap();
    s.set_party_tin(
        member,
        Some(TinFacts {
            tin_type: TinType::Ssn,
            last4: "1111".into(),
            vault_record: Some(hash),
            purged: None,
        }),
    )
    .unwrap();
    let (document, _, _) = s
        .file_document(
            "legal",
            "operating agreement.pdf",
            "application/pdf",
            b"%PDF-1.4 OA",
            None,
        )
        .unwrap();
    let e = s.create_entry(today(), "Opening contribution").unwrap();
    for (code, side) in [("1000", Side::Debit), ("3000", Side::Credit)] {
        let id = s.ledger().chart().find_by_code(code).unwrap().id;
        s.add_line(e, id, side, usd("5000.00")).unwrap();
    }
    s.post_entry(e).unwrap();
    RootBook {
        _root: root,
        book,
        member,
        payee,
        document,
    }
}

fn load(book: &Path) -> Books {
    data::load_once(&Source {
        data_dir: book.to_path_buf(),
        today: Some(today()),
    })
    .unwrap()
}

#[test]
fn a_book_in_the_root_reads_its_own_vault() {
    let f = root_book();
    let b = load(&f.book);
    assert_eq!(b.tin_from_vault(f.member).unwrap(), SSN);
    let w9 = b.w9_file(f.payee).unwrap();
    assert_eq!(
        (w9.file_name.as_str(), w9.bytes.as_slice()),
        ("jo-w9.pdf", &b"%PDF-1.4 Jo's W-9"[..])
    );
    let e = b.w9_file(f.member).unwrap_err();
    assert!(e.contains("no W-9 file"), "{e}");
}

#[test]
fn a_document_is_saved_as_a_new_copy_from_its_page() {
    downloads();
    let f = root_book();
    let b = load(&f.book);
    let name = |n: &str| format!("operating agreement{n}.pdf");
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, b, false, Route::Document(f.document)));
    h.run_steps(3);
    for n in ["", " (1)"] {
        h.get_by_label_contains("Save a copy").click();
        h.run_steps(3);
        assert!(saved_files().contains(&name(n)), "{:?}", saved_files());
        assert_eq!(
            std::fs::read(downloads().join(name(n))).unwrap(),
            b"%PDF-1.4 OA"
        );
    }
    // What came of it shows on the page.
    let said = h.query_by_label_contains("Saved ").is_some();
    assert!(said, "the page doesn't say where the copy went");
}

#[test]
fn the_menu_makes_the_cpa_package_with_full_tins() {
    downloads();
    let f = root_book();
    let b = load(&f.book);
    let (path, note) = Job::Package.run(&b).unwrap();
    assert_eq!(path.parent().unwrap(), downloads());
    assert!(
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("cpa-package-2026-10-15")
    );
    assert_eq!(note, None, "every TIN is there");
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().unwrap().to_string())
        .collect();
    assert_eq!(names[0], "README.txt");
    assert!(
        names.contains(&"general-journal.iif".to_string()),
        "{names:?}"
    );
    assert!(
        names.contains(&"documents/legal/operating agreement.pdf".to_string()),
        "{names:?}"
    );
    let mut readme = String::new();
    zip.by_name("README.txt")
        .unwrap()
        .read_to_string(&mut readme)
        .unwrap();
    assert!(readme.starts_with("Acme LLC"), "{readme}");
    // The QuickBooks file stops at the package's day, like every file.
    assert!(
        readme
            .contains("general-journal.iif: every posted entry from the first through 2026-10-15,"),
        "{readme}"
    );
    // The member's K-1 row carries their full SSN, from the book's vault.
    let mut k1 = String::new();
    zip.by_name("k1-members.csv")
        .unwrap()
        .read_to_string(&mut k1)
        .unwrap();
    assert!(
        k1.contains(&format!("Alex Owner,{},", f.member)) && k1.contains(SSN),
        "{k1}"
    );
    // The books weren't touched: their head is as it was.
    assert_eq!(load(&f.book).head, b.head);
}

#[test]
fn the_menu_makes_the_1099_worksheet_with_tins() {
    downloads();
    let f = root_book();
    let b = load(&f.book);
    let (path, _) = Job::Worksheet(2026).run(&b).unwrap();
    assert!(
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("1099-worksheet-2026-with-tins"),
        "{}",
        path.display()
    );
    let csv = std::fs::read_to_string(&path).unwrap();
    assert!(csv.starts_with("vendor_id,name,"), "{csv}");
    assert!(csv.lines().next().unwrap().contains(",tin,"), "{csv}");
}

#[test]
fn a_locked_vault_still_makes_the_package_and_says_what_is_missing() {
    downloads();
    let f = root_book();
    std::fs::remove_file(
        f.book
            .join(ledger_store::root::KEYS_DIR)
            .join(ledger_store::root::LEGACY_KEY_FILE),
    )
    .unwrap();
    let b = load(&f.book);
    let (_, note) = Job::Package.run(&b).unwrap();
    let note = note.expect("the missing TIN is noted");
    assert!(note.contains("1 TIN(s)"), "{note}");
    // Never a key made by the viewer.
    assert!(
        !f.book
            .join(ledger_store::root::KEYS_DIR)
            .join(ledger_store::root::LEGACY_KEY_FILE)
            .exists()
    );
}

#[test]
fn a_parties_w9_is_saved_from_its_page() {
    downloads();
    let f = root_book();
    let b = load(&f.book);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 1250.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, b, false, Route::Party(f.payee)));
    h.run_steps(3);
    let before = saved_files();
    h.get_by_label("Save the W-9").click();
    h.run_steps(3);
    // Only this test saves a W-9 (others save into the same folder at the
    // same time).
    let new: Vec<String> = saved_files()
        .into_iter()
        .filter(|n| !before.contains(n) && n.starts_with("jo-w9"))
        .collect();
    assert_eq!(new.len(), 1, "{new:?}");
    assert!(new[0].starts_with("jo-w9"), "{new:?}");
    assert_eq!(
        std::fs::read(downloads().join(&new[0])).unwrap(),
        b"%PDF-1.4 Jo's W-9"
    );
}

/// `wanderbooks gui [BOOK]` (plan 3.6): the book named, else the root's
/// only or first one by id; never a folder that isn't a data root.
#[test]
fn the_viewer_finds_its_book_in_the_root() {
    let home = tempfile::Builder::new()
        .prefix("wb-gui-home-")
        .tempdir()
        .unwrap();
    let (root, _) = ledger_store::root::Root::setup(home.path(), None, None).unwrap();
    let r = root.path().to_path_buf();
    // No books yet.
    let e = ledger_gui::book_in_root(Some(&r), None).unwrap_err();
    assert!(e.contains("no books yet"), "{e}");
    for id in ["zeta", "acme"] {
        root.create_book(
            &ledger_store::root::BookId::parse(id).unwrap(),
            Currency::USD,
        )
        .unwrap();
    }
    // As the system spells it: `book_in_root` answers a canonical path
    // (macOS writes /tmp as /private/tmp), with no Windows `\\?\` prefix
    // (the books never keep one).
    let dir = |id: &str| {
        let p =
            std::fs::canonicalize(root.book_dir(&ledger_store::root::BookId::parse(id).unwrap()))
                .unwrap();
        match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
            Some(plain) if cfg!(windows) => std::path::PathBuf::from(plain),
            _ => p,
        }
    };
    assert_eq!(
        ledger_gui::book_in_root(Some(&r), Some("zeta")).unwrap(),
        dir("zeta")
    );
    assert_eq!(
        ledger_gui::book_in_root(Some(&r), None).unwrap(),
        dir("acme"),
        "the first by id"
    );
    let e = ledger_gui::book_in_root(Some(&r), Some("nope")).unwrap_err();
    assert!(e.contains("no books called \"nope\""), "{e}");
    let e = ledger_gui::book_in_root(Some(&r), Some("../x")).unwrap_err();
    assert!(!e.is_empty());
    // A folder that isn't a data root is never opened as one.
    let plain = tempfile::Builder::new()
        .prefix("wb-gui-plain-")
        .tempdir()
        .unwrap();
    let e = ledger_gui::book_in_root(Some(plain.path()), None).unwrap_err();
    assert!(e.contains("no books to show"), "{e}");
}

/// The 1099 worksheet the menu makes carries a reportable payee's full
/// TIN from the vault.
#[test]
fn the_menus_1099_worksheet_carries_a_payees_full_tin() {
    downloads();
    let f = root_book();
    {
        let mut s = Store::open(&f.book).unwrap();
        // A contractor marked for a 1099-NEC, paid past 2026's threshold.
        let vendor = s.add_vendor("Pat Payee").unwrap();
        s.set_vendor_1099(vendor, &nec(), Some(true)).unwrap();
        let party = s.ledger().party_of_vendor(vendor).unwrap();
        let key = f
            .book
            .join(ledger_store::root::KEYS_DIR)
            .join(ledger_store::root::LEGACY_KEY_FILE);
        let v = Vault::open(&f.book.join(ledger_store::root::VAULT_DIR), &key, false).unwrap();
        let hash = v
            .put_tin(
                party,
                &Tin::new("607-97-2222", ledger_vault::TinType::Ssn).unwrap(),
            )
            .unwrap();
        s.set_party_tin(
            party,
            Some(TinFacts {
                tin_type: TinType::Ssn,
                last4: "2222".into(),
                vault_record: Some(hash),
                purged: None,
            }),
        )
        .unwrap();
        let e = s.create_entry(today(), "Design work").unwrap();
        for (code, side) in [("6100", Side::Debit), ("1000", Side::Credit)] {
            let id = s.ledger().chart().find_by_code(code).unwrap().id;
            s.add_line(e, id, side, usd("2500.00")).unwrap();
        }
        s.set_entry_vendor(e, Some(vendor)).unwrap();
        s.post_entry(e).unwrap();
    }
    let b = load(&f.book);
    let (path, note) = Job::Worksheet(2026).run(&b).unwrap();
    assert_eq!(note, None, "{note:?}");
    let csv = std::fs::read_to_string(&path).unwrap();
    let row = csv
        .lines()
        .find(|l| l.contains("Pat Payee"))
        .unwrap_or_else(|| panic!("{csv}"));
    assert!(row.contains("607-97-2222"), "{csv}");
}

/// The menu reads settings as the rest of the viewer and the server do:
/// the books' own, from their journal; a settings file beside them is
/// never read (D-44).
#[test]
fn the_menu_reads_the_books_own_settings() {
    downloads();
    let dir = tempfile::Builder::new()
        .prefix("wb-gui-kept-")
        .tempdir()
        .unwrap();
    let data = dir.path().join("books-data");
    {
        let mut s = Store::create(&data, Currency::USD).unwrap();
        for (code, name, ty) in [
            ("1000", "Checking", AccountType::Asset),
            ("3000", "Capital", AccountType::Equity),
        ] {
            s.add_account(code, name, ty, None, false).unwrap();
        }
        s.change_settings(
            &[
                ledger_core::settings::Change {
                    key: "company.name".into(),
                    value: Some(serde_json::json!("Journal Kept Co")),
                },
                ledger_core::settings::Change {
                    key: "books.cash_accounts".into(),
                    value: Some(serde_json::json!(["1000"])),
                },
            ],
            &ledger_core::settings::Source {
                decided_by: "Tobin Harlow".into(),
                decided_on: today(),
                citation: None,
                note: None,
            },
        )
        .unwrap();
    }
    std::fs::write(
        dir.path().join("wanderbooks.toml"),
        "[company]\nname = \"File Beside Co\"\n",
    )
    .unwrap();
    let b = load(&data);
    assert_eq!(b.settings.company.name, "Journal Kept Co");
    let (path, _) = Job::Package.run(&b).unwrap();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let mut readme = String::new();
    zip.by_name("README.txt")
        .unwrap()
        .read_to_string(&mut readme)
        .unwrap();
    assert!(readme.starts_with("Journal Kept Co"), "{readme}");
}

/// The worksheet says when a reportable payee's TIN isn't in it (the
/// vault locked).
#[test]
fn the_menus_worksheet_says_which_payees_lack_a_tin() {
    downloads();
    let f = root_book();
    {
        let mut s = Store::open(&f.book).unwrap();
        let vendor = s.add_vendor("Pat Payee").unwrap();
        s.set_vendor_1099(vendor, &nec(), Some(true)).unwrap();
        let e = s.create_entry(today(), "Design work").unwrap();
        for (code, side) in [("6100", Side::Debit), ("1000", Side::Credit)] {
            let id = s.ledger().chart().find_by_code(code).unwrap().id;
            s.add_line(e, id, side, usd("2500.00")).unwrap();
        }
        s.set_entry_vendor(e, Some(vendor)).unwrap();
        s.post_entry(e).unwrap();
    }
    let b = load(&f.book);
    let (_, note) = Job::Worksheet(2026).run(&b).unwrap();
    let note = note.expect("the missing TIN is noted");
    assert!(
        note.starts_with("1 payee(s) a 1099 is or may be due for have no TIN"),
        "{note}"
    );
}
