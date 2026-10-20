//! The data root (root.rs): one private folder for every book, found
//! without the environment, and books adopted into it whole.
#![cfg(unix)]

use chrono::NaiveDate;
use ledger_core::settings::{Change, Source};
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::root::{
    AdoptOptions, BookId, LOCATION_FILE, MARKER_FILE, Root, RootError, default_root,
    platform_data_dir,
};
use ledger_store::{JOURNAL_FILE, JournalLine, Snapshot, Store, StoreError};
use rust_decimal::Decimal;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn tmp(tag: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("wb-root-{tag}-"))
        .tempdir()
        .unwrap()
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn src(by: &str) -> Source {
    Source {
        decided_by: by.into(),
        decided_on: NaiveDate::from_ymd_opt(2026, 10, 18).unwrap(),
        citation: None,
        note: None,
    }
}

fn set(key: &str, value: serde_json::Value) -> Change {
    Change {
        key: key.into(),
        value: Some(value),
    }
}

/// Books with accounts, a posted entry with its receipt, a filed document,
/// and (with `settings`) settings of their own: what adoption must carry.
fn source_books(dir: &Path, settings: bool) -> Store {
    let mut s = Store::create(dir, Currency::USD).unwrap();
    if settings {
        s.adopt_settings(&[set("company.name", json!("Adopted LLC"))], &src("Owner"))
            .unwrap();
    }
    let bank = s
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let fees = s
        .add_account("6000", "Fees", AccountType::Expense, None, false)
        .unwrap();
    let e = s
        .create_entry(NaiveDate::from_ymd_opt(2026, 10, 9).unwrap(), "Filing fee")
        .unwrap();
    let m = Money::new(Decimal::new(5900, 2), Currency::USD);
    s.add_line(e, fees, Side::Debit, m).unwrap();
    s.add_line(e, bank, Side::Credit, m).unwrap();
    s.attach_receipt(
        Some(e),
        "fee.pdf",
        "application/pdf",
        b"%PDF fee receipt",
        None,
    )
    .unwrap();
    s.post_entry(e).unwrap();
    s.file_document(
        "formation",
        "cert.pdf",
        "application/pdf",
        b"%PDF certificate",
        None,
    )
    .unwrap();
    s
}

/// Every file under `dir`, with its bytes, for before/after comparison.
fn snapshot_files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// The error of a call that must fail (a Store isn't Debug).
fn refused<T>(r: Result<T, RootError>) -> RootError {
    match r {
        Ok(_) => panic!("expected a refusal"),
        Err(e) => e,
    }
}

fn id(s: &str) -> BookId {
    BookId::parse(s).unwrap()
}

#[test]
fn book_ids_are_plain_lowercase_names() {
    for ok in ["a", "fernhill", "acme-2026", "0", &"x".repeat(40)] {
        assert!(BookId::parse(ok).is_ok(), "{ok:?}");
    }
    for bad in [
        "",
        "-a",
        "A",
        "acme_llc",
        "acme llc",
        "acme.",
        "..",
        "a/b",
        "é",
        &"x".repeat(41),
    ] {
        assert!(
            matches!(BookId::parse(bad), Err(RootError::BadId(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn the_root_is_found_without_the_environment() {
    let home = tmp("find");
    // Nothing recorded: the platform default under the home.
    assert_eq!(
        Root::locate(home.path(), None).unwrap(),
        default_root(home.path())
    );
    assert_eq!(
        default_root(home.path()),
        home.path().join(".local/share/wanderbooks")
    );
    // An explicit root wins.
    let elsewhere = home.path().join("elsewhere");
    assert_eq!(
        Root::locate(home.path(), Some(&elsewhere)).unwrap(),
        elsewhere
    );
    // A recorded root is used when none is given.
    std::fs::create_dir_all(platform_data_dir(home.path())).unwrap();
    std::fs::write(
        platform_data_dir(home.path()).join(LOCATION_FILE),
        format!("{}\n", elsewhere.display()),
    )
    .unwrap();
    assert_eq!(Root::locate(home.path(), None).unwrap(), elsewhere);
    // A relative record is refused, not guessed at.
    std::fs::write(
        platform_data_dir(home.path()).join(LOCATION_FILE),
        "relative/root\n",
    )
    .unwrap();
    assert!(matches!(
        Root::locate(home.path(), None),
        Err(RootError::Unsafe(_))
    ));
}

#[test]
fn setup_makes_a_private_root_and_its_folders() {
    let home = tmp("setup");
    let (root, report) = Root::setup(home.path(), None, None).unwrap();
    assert_eq!(root.path(), default_root(home.path()));
    assert!(report.created && !report.tightened && !report.recorded);
    assert_eq!(mode(root.path()), 0o700);
    for sub in ["books", "run", "legacy"] {
        assert_eq!(mode(&root.path().join(sub)), 0o700, "{sub}");
    }
    // Again: nothing to do, nothing recorded.
    let (_, again) = Root::setup(home.path(), None, None).unwrap();
    assert!(!again.created && !again.tightened && !again.recorded);
    assert!(Root::find_in(home.path(), None).is_ok());
}

#[test]
fn setup_records_a_root_chosen_elsewhere_and_forgets_it_when_back_to_default() {
    let home = tmp("setup-elsewhere");
    let chosen = home.path().join("my books");
    let (_, report) = Root::setup(home.path(), Some(&chosen), None).unwrap();
    assert!(report.recorded);
    let pointer = platform_data_dir(home.path()).join(LOCATION_FILE);
    assert_eq!(mode(&pointer), 0o600);
    assert_eq!(Root::find_in(home.path(), None).unwrap().path(), chosen);
    // Back to the default (it holds no books): the record goes.
    let (root, report) = Root::setup(home.path(), Some(&default_root(home.path())), None).unwrap();
    assert_eq!(root.path(), default_root(home.path()));
    assert!(!report.recorded && !pointer.exists());

    // $XDG_DATA_HOME is honoured at setup, when nothing is recorded, and
    // recorded.
    let fresh = tmp("setup-xdg");
    let xdg = fresh.path().join("xdg");
    let (root, report) = Root::setup(fresh.path(), None, Some(&xdg)).unwrap();
    assert_eq!(root.path(), xdg.join("wanderbooks"));
    assert!(report.recorded);
    assert_eq!(
        Root::find_in(fresh.path(), None).unwrap().path(),
        xdg.join("wanderbooks")
    );
    // A relative XDG_DATA_HOME is ignored, as the spec says.
    let other = tmp("setup-rel");
    let (root, report) = Root::setup(other.path(), None, Some(Path::new("rel"))).unwrap();
    assert_eq!(root.path(), default_root(other.path()));
    assert!(!report.recorded);
}

#[test]
fn setup_tightens_a_root_others_can_enter_and_open_refuses_one() {
    let home = tmp("tighten");
    let root = default_root(home.path());
    std::fs::create_dir_all(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(Root::open(&root), Err(RootError::Unsafe(_))));
    let (_, report) = Root::setup(home.path(), None, None).unwrap();
    assert!(report.tightened && !report.created);
    assert_eq!(mode(&root), 0o700);
    // A folder inside that others can enter is refused too.
    std::fs::set_permissions(root.join("books"), std::fs::Permissions::from_mode(0o750)).unwrap();
    let opened = Root::open(&root).unwrap();
    assert!(matches!(opened.list_books(), Err(RootError::Unsafe(_))));
    // Setup makes it private again.
    let (_, report) = Root::setup(home.path(), None, None).unwrap();
    assert!(report.tightened);
    assert_eq!(mode(&root.join("books")), 0o700);
    assert!(opened.list_books().unwrap().is_empty());
}

#[test]
fn a_root_that_is_a_link_or_a_file_or_missing_is_refused() {
    let home = tmp("unsafe");
    let real = home.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
    let link = home.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(matches!(Root::open(&link), Err(RootError::Unsafe(_))));
    assert!(matches!(
        Root::setup(home.path(), Some(&link), None),
        Err(RootError::Unsafe(_))
    ));
    let file = home.path().join("file");
    std::fs::write(&file, "x").unwrap();
    assert!(matches!(Root::open(&file), Err(RootError::Unsafe(_))));
    assert!(matches!(
        Root::open(&home.path().join("none")),
        Err(RootError::Missing)
    ));
    // A private folder setup didn't make (no marker) isn't a root.
    assert!(matches!(Root::open(&real), Err(RootError::Unsafe(why)) if why.contains("no marker")));
}

#[test]
fn books_are_created_listed_and_never_made_twice() {
    let home = tmp("create");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    assert!(root.list_books().unwrap().is_empty());
    let store = root.create_book(&id("acme"), Currency::USD).unwrap();
    assert_eq!(store.data_dir(), root.book_dir(&id("acme")));
    drop(store);
    assert_eq!(mode(&root.book_dir(&id("acme"))), 0o700);
    assert!(matches!(
        root.create_book(&id("acme"), Currency::USD),
        Err(RootError::Exists(_))
    ));
    root.create_book(&id("beta"), Currency::USD).unwrap();
    // Folders that aren't books, or aren't named as books are, are not listed.
    std::fs::create_dir(root.path().join("books/.adopt-x-1")).unwrap();
    std::fs::create_dir(root.path().join("books/empty")).unwrap();
    std::fs::create_dir(root.path().join("books/Bad_Name")).unwrap();
    std::fs::write(root.path().join("books/Bad_Name").join(JOURNAL_FILE), "").unwrap();
    assert_eq!(root.list_books().unwrap(), vec![id("acme"), id("beta")]);
}

#[test]
fn adopted_books_are_the_same_books_and_the_source_is_untouched() {
    let home = tmp("adopt");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let old = tmp("adopt-src");
    let data = old.path().join("books-data");
    let source = source_books(&data, true);
    let source_head = source.head();
    let source_ledger = source.ledger().clone();
    drop(source);
    // A vault and a key beside them, as the old layout kept them.
    let vault = old.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    std::fs::write(vault.join("tin-7.bin"), b"sealed record").unwrap();
    let key = old.path().join("vault.key");
    std::fs::write(&key, b"0123456789abcdef0123456789abcdef").unwrap();
    let before = snapshot_files(old.path());

    let report = root
        .adopt(
            &data,
            &id("acme"),
            &AdoptOptions {
                settings: None,
                vault_dir: Some(vault.clone()),
                vault_key: Some(key.clone()),
                // As a preview read it: the same head, so it goes ahead.
                expect_head: Some(source_head.clone()),
            },
        )
        .unwrap();
    assert_eq!(report.source_head, source_head);
    assert_eq!(
        report.head, source_head,
        "books with settings gain no event"
    );
    assert_eq!(report.blob_files, 2);
    assert_eq!(report.blobs_verified, 2);
    assert_eq!(
        (
            report.vault_files,
            report.key_copied,
            report.settings_imported
        ),
        (1, true, 0)
    );

    // The same books: every reader agrees with the source's ledger.
    let dir = root.book_dir(&id("acme"));
    let adopted = Store::open(&dir).unwrap();
    adopted.verify().unwrap();
    assert!(adopted.ledger().differs_from(&source_ledger).is_empty());
    drop(adopted);
    assert!(
        Snapshot::load(&dir)
            .unwrap()
            .ledger()
            .differs_from(&source_ledger)
            .is_empty()
    );
    // Private, inside and out.
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&dir.join("journal.jsonl")), 0o600);
    assert_eq!(mode(&dir.join("vault")), 0o700);
    assert_eq!(mode(&dir.join("vault/tin-7.bin")), 0o600);
    assert_eq!(mode(&dir.join("keys/legacy.key")), 0o600);
    assert_eq!(
        std::fs::read(dir.join("keys/legacy.key")).unwrap(),
        std::fs::read(&key).unwrap()
    );
    // The source: byte for byte as it was, nothing added or gone.
    assert_eq!(snapshot_files(old.path()), before);
    // No staging left behind; the book is listed.
    assert_eq!(root.list_books().unwrap(), vec![id("acme")]);
    assert_eq!(
        std::fs::read_dir(root.path().join("books"))
            .unwrap()
            .count(),
        1
    );
    // Adopted once only.
    assert!(matches!(
        root.adopt(&data, &id("acme"), &AdoptOptions::default()),
        Err(RootError::Exists(_))
    ));
}

#[test]
fn books_without_settings_need_them_and_get_them_on_the_way_in() {
    let home = tmp("adopt-settings");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let old = tmp("adopt-settings-src");
    let source = source_books(old.path(), false);
    let head = source.head();
    drop(source);
    match root.adopt(old.path(), &id("acme"), &AdoptOptions::default()) {
        Err(RootError::Adopt(why)) => assert!(why.contains("keep no settings"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert!(root.list_books().unwrap().is_empty());
    let report = root
        .adopt(
            old.path(),
            &id("acme"),
            &AdoptOptions {
                settings: Some((
                    vec![set("company.name", json!("From The File LLC"))],
                    src("Owner"),
                )),
                ..AdoptOptions::default()
            },
        )
        .unwrap();
    assert_eq!(report.source_head, head);
    assert_eq!(report.settings_imported, 1);
    // What was added: the journal format the settings need, declared
    // first (older books), then the settings.
    let journal = std::fs::read_to_string(root.book_dir(&id("acme")).join(JOURNAL_FILE)).unwrap();
    let added: Vec<String> = journal
        .lines()
        .skip(usize::try_from(head.0).unwrap())
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    let format_declaration = Store::create(&tmp("probe").path().join("d"), Currency::USD)
        .map(|mut s| {
            s.adopt_settings(&[], &src("x")).unwrap();
            let j = std::fs::read_to_string(s.data_dir().join(JOURNAL_FILE)).unwrap();
            j.lines()
                .map(|l| {
                    serde_json::from_str::<serde_json::Value>(l).unwrap()["type"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .collect::<Vec<_>>()
        })
        .unwrap();
    // The same steps a fresh store takes to keep settings, after store.created.
    assert_eq!(added, format_declaration[1..].to_vec());
    assert_eq!(added.last().map(String::as_str), Some("settings.set"));
    assert_eq!(report.head.0, head.0 + added.len() as u64);
    let adopted = Store::open(&root.book_dir(&id("acme"))).unwrap();
    assert!(adopted.ledger().settings().is_journaled());
    assert_eq!(
        adopted.ledger().settings().company("USD").name,
        "From The File LLC"
    );
}

#[test]
fn books_another_writer_has_open_are_refused() {
    let home = tmp("adopt-locked");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let old = tmp("adopt-locked-src");
    let open = source_books(old.path(), true);
    let err = root
        .adopt(old.path(), &id("acme"), &AdoptOptions::default())
        .unwrap_err();
    assert!(
        matches!(err.store_error(), Some(StoreError::Locked(_))),
        "{err:?}"
    );
    drop(open);
    assert!(root.list_books().unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(root.path().join("books"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn a_cut_off_or_damaged_source_is_refused_and_nothing_is_left_behind() {
    let home = tmp("adopt-bad");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();

    // A cut-off last line: refused, the source not repaired.
    let torn = tmp("adopt-torn");
    drop(source_books(torn.path(), true));
    let journal = torn.path().join(JOURNAL_FILE);
    let mut bytes = std::fs::read(&journal).unwrap();
    bytes.extend_from_slice(b"{\"seq\":99");
    std::fs::write(&journal, &bytes).unwrap();
    match root.adopt(torn.path(), &id("torn"), &AdoptOptions::default()) {
        Err(RootError::Adopt(why)) => assert!(why.contains("cut off"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(std::fs::read(&journal).unwrap(), bytes);

    // A blob whose bytes changed: the copy fails its re-hash, and no book
    // (nor its staging folder) appears.
    let damaged = tmp("adopt-damaged");
    drop(source_books(damaged.path(), true));
    let blob = walk_files(&damaged.path().join("blobs")).remove(0);
    std::fs::write(&blob, b"not what was stored").unwrap();
    match root.adopt(damaged.path(), &id("damaged"), &AdoptOptions::default()) {
        Err(RootError::Adopt(why)) => {
            assert!(why.contains("doesn't match its fingerprint"), "{why}")
        }
        other => panic!("{other:?}"),
    }
    assert!(root.list_books().unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(root.path().join("books"))
            .unwrap()
            .count(),
        0
    );

    // A link among the blobs is refused.
    let linked = tmp("adopt-linked");
    drop(source_books(linked.path(), true));
    std::os::unix::fs::symlink("/etc/hostname", linked.path().join("blobs/link")).unwrap();
    match root.adopt(linked.path(), &id("linked"), &AdoptOptions::default()) {
        Err(RootError::Adopt(why)) => assert!(why.contains("isn't a stored file"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        std::fs::read_dir(root.path().join("books"))
            .unwrap()
            .count(),
        0
    );

    // Not books at all.
    let empty = tmp("adopt-empty");
    match root.adopt(empty.path(), &id("empty"), &AdoptOptions::default()) {
        Err(RootError::Adopt(why)) => assert!(why.contains("no journal.jsonl"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn books_inside_the_root_are_not_adopted_into_it() {
    let home = tmp("adopt-inside");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    drop(root.create_book(&id("acme"), Currency::USD).unwrap());
    match root.adopt(
        &root.book_dir(&id("acme")),
        &id("copy"),
        &AdoptOptions::default(),
    ) {
        Err(RootError::Adopt(why)) => {
            assert!(why.contains("inside the books' private folder"), "{why}")
        }
        other => panic!("{other:?}"),
    }
    // Nor the folder holding the root.
    match root.adopt(home.path(), &id("copy"), &AdoptOptions::default()) {
        Err(RootError::Adopt(_)) => {}
        other => panic!("{other:?}"),
    }
}

fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk_files(&p));
        } else {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Append a line, properly chained, of a type this version doesn't know:
/// what a newer wanderbooks would write.
fn append_newer_line(data: &Path) {
    use sha2::{Digest, Sha256};
    let journal = data.join(JOURNAL_FILE);
    let text = std::fs::read_to_string(&journal).unwrap();
    let last: JournalLine = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    let next = JournalLine {
        seq: last.seq + 1,
        ts: last.ts.clone(),
        line_type: "future.thing".into(),
        payload: json!({}),
        prev: hex::encode(Sha256::digest(last.to_canonical().as_bytes())),
    };
    std::fs::write(&journal, format!("{text}{}\n", next.to_canonical())).unwrap();
}

#[test]
fn setup_run_again_keeps_the_recorded_root_and_never_moves_books() {
    let home = tmp("setup-keep");
    let chosen = home.path().join("chosen");
    let (root, _) = Root::setup(home.path(), Some(&chosen), None).unwrap();
    drop(root.create_book(&id("acme"), Currency::USD).unwrap());
    // Run again with nothing given (and XDG set): the recorded root stays.
    let (again, report) = Root::setup(home.path(), None, Some(&home.path().join("xdg"))).unwrap();
    assert_eq!(again.path(), chosen);
    assert!(report.recorded);
    assert_eq!(Root::find_in(home.path(), None).unwrap().path(), chosen);
    // A root elsewhere while this one holds books: refused, nothing changed.
    let other = home.path().join("other");
    match Root::setup(home.path(), Some(&other), None) {
        Err(RootError::Unsafe(why)) => assert!(why.contains("may hold books"), "{why}"),
        x => panic!("{x:?}"),
    }
    assert!(!other.exists());
    assert_eq!(Root::find_in(home.path(), None).unwrap().path(), chosen);
    match Root::setup(home.path(), Some(&default_root(home.path())), None) {
        Err(RootError::Unsafe(why)) => assert!(why.contains("may hold books"), "{why}"),
        x => panic!("{x:?}"),
    }
}

#[test]
fn setup_leaves_a_folder_that_holds_other_things_alone() {
    let home = tmp("setup-full");
    let full = home.path().join("Documents");
    std::fs::create_dir(&full).unwrap();
    std::fs::set_permissions(&full, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(full.join("taxes.pdf"), "x").unwrap();
    match Root::setup(home.path(), Some(&full), None) {
        Err(RootError::Unsafe(why)) => assert!(why.contains("holds other things"), "{why}"),
        x => panic!("{x:?}"),
    }
    assert_eq!(mode(&full), 0o755, "its permissions untouched");
    assert!(!full.join("books").exists() && !full.join(MARKER_FILE).exists());
    assert!(!platform_data_dir(home.path()).join(LOCATION_FILE).exists());
    // An empty folder is fine.
    let empty = home.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    std::fs::set_permissions(&empty, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(Root::setup(home.path(), Some(&empty), None).is_ok());
}

#[test]
fn a_root_another_user_could_swap_is_refused() {
    let home = tmp("ancestor");
    let open = home.path().join("open");
    std::fs::create_dir(&open).unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
    match Root::setup(home.path(), Some(&open.join("books-root")), None) {
        Err(RootError::Unsafe(why)) => assert!(why.contains("a folder above"), "{why}"),
        x => panic!("{x:?}"),
    }
    // With the sticky bit (like /tmp), others can't swap our folders.
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o1777)).unwrap();
    let (root, _) = Root::setup(home.path(), Some(&open.join("books-root")), None).unwrap();
    // And a parent opened up later is caught when the root is opened.
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(Root::open(root.path()), Err(RootError::Unsafe(_))));
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn missing_folders_above_the_root_are_made_private() {
    let home = tmp("parents");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    assert_eq!(mode(&home.path().join(".local")), 0o700);
    assert_eq!(mode(&home.path().join(".local/share")), 0o700);
    assert_eq!(mode(&root.path().join(MARKER_FILE)), 0o600);
}

#[test]
fn a_planted_location_note_is_refused() {
    let home = tmp("planted");
    let data = platform_data_dir(home.path());
    std::fs::create_dir_all(&data).unwrap();
    let pointer = data.join(LOCATION_FILE);
    std::fs::write(&pointer, format!("{}\n", home.path().join("x").display())).unwrap();
    std::fs::set_permissions(&pointer, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(matches!(
        Root::locate(home.path(), None),
        Err(RootError::Unsafe(_))
    ));
    assert!(matches!(
        Root::setup(home.path(), None, None),
        Err(RootError::Unsafe(_))
    ));
    std::fs::remove_file(&pointer).unwrap();
    std::os::unix::fs::symlink(home.path().join("elsewhere"), &pointer).unwrap();
    assert!(matches!(
        Root::locate(home.path(), None),
        Err(RootError::Unsafe(_))
    ));
}

#[test]
fn a_new_root_is_named_plainly() {
    let home = tmp("plain");
    std::fs::create_dir(home.path().join("a")).unwrap();
    // `..` anywhere is refused, where it exists or not.
    for odd in ["a/../b", "new/../c"] {
        assert!(
            matches!(
                Root::setup(home.path(), Some(&home.path().join(odd)), None),
                Err(RootError::Unsafe(_))
            ),
            "{odd}"
        );
    }
    // Links in the folders above are resolved; the root's own name is kept.
    std::os::unix::fs::symlink(home.path().join("a"), home.path().join("to-a")).unwrap();
    let (root, _) =
        Root::setup(home.path(), Some(&home.path().join("to-a/new/books")), None).unwrap();
    assert_eq!(
        root.path(),
        std::fs::canonicalize(home.path())
            .unwrap()
            .join("a/new/books")
    );
}

/// Set every folder under `dir` to `dirs` and every file to `files`.
fn chmod_tree(dir: &Path, dirs: u32, files: u32) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            chmod_tree(&p, dirs, files);
        } else {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(files)).unwrap();
        }
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(dirs)).unwrap();
}

#[test]
fn adoption_leaves_a_read_only_source_as_it_was() {
    let home = tmp("adopt-ro");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let old = tmp("adopt-ro-src");
    drop(source_books(old.path(), true));
    let before = snapshot_files(old.path());
    chmod_tree(old.path(), 0o500, 0o400);
    // Writable again however the test ends, so its folder can go.
    struct Writable<'a>(&'a Path);
    impl Drop for Writable<'_> {
        fn drop(&mut self) {
            chmod_tree(self.0, 0o700, 0o600);
        }
    }
    let writable = Writable(old.path());
    let report = root.adopt(old.path(), &id("acme"), &AdoptOptions::default());
    drop(writable);
    let report = report.unwrap();
    assert!(!report.source_lock_created);
    assert_eq!(snapshot_files(old.path()), before, "the lock file included");
}

#[test]
fn a_source_with_no_lock_file_gets_one_and_says_so() {
    let home = tmp("adopt-nolock");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let old = tmp("adopt-nolock-src");
    drop(source_books(old.path(), true));
    std::fs::remove_file(old.path().join("wanderbooks.lock")).unwrap();
    let report = root
        .adopt(old.path(), &id("acme"), &AdoptOptions::default())
        .unwrap();
    assert!(report.source_lock_created);
}

#[test]
fn a_vault_or_key_that_isnt_there_stops_adoption() {
    let home = tmp("adopt-novault");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let old = tmp("adopt-novault-src");
    drop(source_books(old.path(), true));
    for options in [
        AdoptOptions {
            vault_dir: Some(old.path().join("no-vault")),
            ..AdoptOptions::default()
        },
        AdoptOptions {
            vault_key: Some(old.path().join("no.key")),
            ..AdoptOptions::default()
        },
    ] {
        match root.adopt(old.path(), &id("acme"), &options) {
            Err(RootError::Adopt(why)) => assert!(why.contains("isn't there"), "{why}"),
            x => panic!("{x:?}"),
        }
    }
    assert!(root.list_books().unwrap().is_empty());
}

#[test]
fn every_stored_file_is_accounted_for() {
    use sha2::{Digest, Sha256};
    let home = tmp("adopt-blobs");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();

    // A file the books name, missing: named in the refusal.
    let missing = tmp("adopt-missing");
    drop(source_books(missing.path(), true));
    let gone = walk_files(&missing.path().join("blobs")).remove(0);
    let sha = gone.file_name().unwrap().to_string_lossy().into_owned();
    std::fs::remove_file(&gone).unwrap();
    match root.adopt(missing.path(), &id("missing"), &AdoptOptions::default()) {
        Err(RootError::Adopt(why)) => {
            assert!(why.contains("aren't there") && why.contains(&sha), "{why}")
        }
        x => panic!("{x:?}"),
    }

    // A stored file the books don't name, and an unfinished write.
    let extra = tmp("adopt-extra");
    drop(source_books(extra.path(), true));
    let orphan = b"stored, never named";
    let osha = hex::encode(Sha256::digest(orphan));
    let odir = extra
        .path()
        .join("blobs")
        .join(&osha[0..2])
        .join(&osha[2..4]);
    std::fs::create_dir_all(&odir).unwrap();
    std::fs::write(odir.join(&osha), orphan).unwrap();
    std::fs::write(odir.join(format!("{osha}.tmp")), b"half").unwrap();
    // Hidden files a desktop leaves (macOS's .DS_Store), anywhere in it.
    std::fs::write(extra.path().join("blobs/.DS_Store"), b"finder").unwrap();
    std::fs::write(odir.parent().unwrap().join(".DS_Store"), b"finder").unwrap();
    std::fs::write(odir.join(".DS_Store"), b"finder").unwrap();
    let report = root
        .adopt(extra.path(), &id("extra"), &AdoptOptions::default())
        .unwrap();
    assert_eq!((report.blob_files, report.blobs_verified), (3, 2));
    assert_eq!((report.unnamed_blobs, report.skipped_files), (1, 4));
    let dir = root
        .book_dir(&id("extra"))
        .join("blobs")
        .join(&osha[0..2])
        .join(&osha[2..4]);
    assert!(dir.join(&osha).is_file() && !dir.join(format!("{osha}.tmp")).exists());

    // A stray file where only stored files belong: refused.
    let stray = tmp("adopt-stray");
    drop(source_books(stray.path(), true));
    std::fs::write(stray.path().join("blobs/notes.txt"), "x").unwrap();
    assert!(matches!(
        root.adopt(stray.path(), &id("stray"), &AdoptOptions::default()),
        Err(RootError::Adopt(_))
    ));
    assert_eq!(root.list_books().unwrap(), vec![id("extra")]);
}

#[test]
fn books_from_a_newer_wanderbooks_are_refused() {
    let home = tmp("adopt-newer");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let old = tmp("adopt-newer-src");
    drop(source_books(old.path(), true));
    append_newer_line(old.path());
    let err = root
        .adopt(old.path(), &id("acme"), &AdoptOptions::default())
        .unwrap_err();
    assert!(
        matches!(err.store_error(), Some(StoreError::NewerJournal { .. })),
        "{err:?}"
    );
    assert!(root.list_books().unwrap().is_empty());
}

#[test]
fn an_adoption_that_never_finished_is_cleared_away() {
    let home = tmp("adopt-stale");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let stale = root.path().join("books/.adopt-acme-999999");
    std::fs::create_dir(&stale).unwrap();
    std::fs::write(stale.join("journal.jsonl"), "half a copy").unwrap();
    let old = tmp("adopt-stale-src");
    drop(source_books(old.path(), true));
    root.adopt(old.path(), &id("acme"), &AdoptOptions::default())
        .unwrap();
    assert!(!stale.exists());
    assert_eq!(
        std::fs::read_dir(root.path().join("books"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn settings_given_for_books_that_keep_their_own_are_reported_unused() {
    let home = tmp("adopt-ignored");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let old = tmp("adopt-ignored-src");
    let s = source_books(old.path(), true);
    let head = s.head();
    drop(s);
    let report = root
        .adopt(
            old.path(),
            &id("acme"),
            &AdoptOptions {
                settings: Some((vec![set("company.name", json!("Other"))], src("x"))),
                ..AdoptOptions::default()
            },
        )
        .unwrap();
    assert!(report.settings_ignored);
    assert_eq!((report.head, report.settings_imported), (head, 0));
}

#[test]
fn no_refusal_names_the_root() {
    let home = tmp("hidden");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let path = root.path().display().to_string();
    drop(root.create_book(&id("acme"), Currency::USD).unwrap());
    let open = Store::open(&root.book_dir(&id("acme"))).unwrap();
    let mut errors = vec![
        refused(root.create_book(&id("acme"), Currency::USD)).to_string(),
        root.adopt(
            &root.book_dir(&id("acme")),
            &id("x"),
            &AdoptOptions::default(),
        )
        .unwrap_err()
        .to_string(),
    ];
    drop(open);
    let books = root.path().join("books");
    std::fs::set_permissions(&books, std::fs::Permissions::from_mode(0o755)).unwrap();
    errors.push(root.list_books().unwrap_err().to_string());
    errors.push(refused(root.create_book(&id("beta"), Currency::USD)).to_string());
    std::fs::set_permissions(&books, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    errors.push(Root::open(root.path()).unwrap_err().to_string());
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    for e in errors {
        assert!(!e.contains(&path), "names the root: {e}");
        assert!(!e.is_empty());
    }
}

#[test]
fn setup_changes_nothing_under_a_folder_another_user_could_swap() {
    let home = tmp("order");
    let open = home.path().join("open");
    std::fs::create_dir(&open).unwrap();
    let existing = open.join("existing");
    std::fs::create_dir(&existing).unwrap();
    std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
    // An existing folder there: refused before it is tightened or marked.
    assert!(matches!(
        Root::setup(home.path(), Some(&existing), None),
        Err(RootError::Unsafe(_))
    ));
    assert_eq!(mode(&existing), 0o755);
    assert!(std::fs::read_dir(&existing).unwrap().next().is_none());
    // A new one there: refused before any folder is made.
    assert!(matches!(
        Root::setup(home.path(), Some(&open.join("new/books")), None),
        Err(RootError::Unsafe(_))
    ));
    assert!(!open.join("new").exists());
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn a_root_that_cant_be_opened_counts_as_holding_books() {
    let home = tmp("loosened");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    drop(root.create_book(&id("acme"), Currency::USD).unwrap());
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let elsewhere = home.path().join("elsewhere");
    match Root::setup(home.path(), Some(&elsewhere), None) {
        Err(RootError::Unsafe(why)) => assert!(why.contains("may hold books"), "{why}"),
        x => panic!("{x:?}"),
    }
    assert!(!elsewhere.exists());
    // Via XDG_DATA_HOME, the refusal says so.
    match Root::setup(home.path(), None, Some(&home.path().join("xdg"))) {
        Err(RootError::Unsafe(why)) => assert!(why.contains("XDG_DATA_HOME"), "{why}"),
        x => panic!("{x:?}"),
    }
    // Setup with nothing given makes it private again, books and all.
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn a_group_folder_of_this_account_s_own_private_group_is_fine() {
    use std::os::unix::fs::MetadataExt;
    let home = tmp("upg");
    let parent = home.path().join("share");
    std::fs::create_dir(&parent).unwrap();
    std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o775)).unwrap();
    let gid = std::fs::metadata(&parent).unwrap().gid();
    // SAFETY: getgid has no preconditions.
    let primary = unsafe { libc::getgid() };
    let result = Root::setup(home.path(), Some(&parent.join("books")), None);
    if gid == primary && private_group(primary) {
        assert!(result.is_ok(), "{:?}", result.err());
    } else {
        assert!(matches!(result, Err(RootError::Unsafe(_))));
    }
}

/// The test's own view of a private group: the primary group, named as
/// the login is, no members listed (from /etc/group and $USER, when there).
fn private_group(gid: u32) -> bool {
    let Ok(user) = std::env::var("USER") else {
        return false;
    };
    std::fs::read_to_string("/etc/group").is_ok_and(|groups| {
        groups.lines().any(|l| {
            let f: Vec<&str> = l.split(':').collect();
            f.len() >= 4 && f[0] == user && f[2] == gid.to_string() && f[3].is_empty()
        })
    })
}

#[test]
fn a_book_id_prints_as_itself() {
    assert_eq!(id("acme-2026").to_string(), "acme-2026");
    assert_eq!(id("acme-2026").as_str(), "acme-2026");
}

/// Something put under a source's `blobs/` for a test.
type Plant = Box<dyn Fn(&Path)>;

#[test]
fn only_stored_files_where_the_books_keep_them_are_adopted() {
    use sha2::{Digest, Sha256};
    let home = tmp("adopt-strays");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let bytes = b"a real stored file";
    let sha = hex::encode(Sha256::digest(bytes));
    // Each case: something under blobs/ that isn't where a stored file
    // sits. Every one is refused, with the same reason.
    let cases: Vec<(&str, Plant)> = vec![
        (
            "a folder not named by two hex digits",
            Box::new(|b: &Path| std::fs::create_dir(b.join("zz")).unwrap()),
        ),
        (
            "a folder named by three hex digits",
            Box::new(|b: &Path| std::fs::create_dir(b.join("abc")).unwrap()),
        ),
        (
            "a file where a folder belongs",
            Box::new(|b: &Path| std::fs::write(b.join("ab"), "x").unwrap()),
        ),
        (
            "a second-level folder not named by two hex digits",
            Box::new(|b: &Path| std::fs::create_dir_all(b.join("ab/zz")).unwrap()),
        ),
        (
            "a second-level file where a folder belongs",
            Box::new(|b: &Path| {
                std::fs::create_dir_all(b.join("ab")).unwrap();
                std::fs::write(b.join("ab/cd"), "x").unwrap()
            }),
        ),
        (
            "a stored file in the wrong folder",
            Box::new(move |b: &Path| {
                let wrong = if sha.starts_with("00") { "11" } else { "00" };
                std::fs::create_dir_all(b.join(wrong).join(&sha[2..4])).unwrap();
                std::fs::write(b.join(wrong).join(&sha[2..4]).join(&sha), bytes).unwrap()
            }),
        ),
        (
            "a file named by 64 characters that aren't hex",
            Box::new(|b: &Path| {
                std::fs::create_dir_all(b.join("ab/cd")).unwrap();
                std::fs::write(b.join("ab/cd").join(format!("ab{}", "z".repeat(62))), "x").unwrap()
            }),
        ),
    ];
    for (i, (what, plant)) in cases.iter().enumerate() {
        let src = tmp(&format!("adopt-stray-{i}"));
        drop(source_books(src.path(), true));
        plant(&src.path().join("blobs"));
        match root.adopt(src.path(), &id(&format!("s{i}")), &AdoptOptions::default()) {
            Err(RootError::Adopt(why)) => {
                assert!(why.contains("isn't a stored file"), "{what}: {why}")
            }
            x => panic!("{what}: {x:?}"),
        }
    }
    assert!(root.list_books().unwrap().is_empty());
}

#[test]
fn errors_other_than_not_found_are_never_taken_for_it() {
    let home = tmp("not-not-found");
    // Opening a root under a file: the error, not "no root yet".
    let file = home.path().join("file");
    std::fs::write(&file, "x").unwrap();
    assert!(matches!(
        Root::open(&file.join("root")),
        Err(RootError::Io(..))
    ));
    // The data folder is a file: finding the root fails, never falls back.
    let odd = tmp("not-not-found-home");
    std::fs::create_dir(odd.path().join(".local")).unwrap();
    std::fs::write(odd.path().join(".local/share"), "x").unwrap();
    assert!(matches!(
        Root::locate(odd.path(), None),
        Err(RootError::Io(..))
    ));
    // A source whose lock file can't be read: refused, never recreated.
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let src = tmp("not-not-found-src");
    drop(source_books(src.path(), true));
    let lock = src.path().join("wanderbooks.lock");
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o200)).unwrap();
    assert!(matches!(
        root.adopt(src.path(), &id("acme"), &AdoptOptions::default()),
        Err(RootError::Io(..))
    ));
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(root.list_books().unwrap().is_empty());
}

#[test]
fn an_adoption_preview_reads_the_source_and_writes_nothing() {
    let home = tmp("preview");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let old = tmp("preview-src");
    let s = source_books(old.path(), false);
    let head = s.head();
    drop(s);
    // No lock file beside the journal: the preview must not make one.
    let _ = std::fs::remove_file(old.path().join("wanderbooks.lock"));
    let before = snapshot_files(old.path());
    let found = root.adopt_preview(old.path()).unwrap();
    assert_eq!(found.head, head);
    assert_eq!(found.currency, Currency::USD);
    assert!(!found.keeps_settings);
    assert_eq!(
        snapshot_files(old.path()),
        before,
        "the preview wrote to the source"
    );
    assert!(root.list_books().unwrap().is_empty());
    // Books that keep their own settings say so.
    let kept = tmp("preview-kept");
    drop(source_books(kept.path(), true));
    assert!(root.adopt_preview(kept.path()).unwrap().keeps_settings);
    // The preview refuses what adoption refuses, naming no root.
    let book = root
        .create_book(&BookId::parse("acme").unwrap(), Currency::USD)
        .unwrap();
    drop(book);
    let root_text = root.path().display().to_string();
    for bad in [
        root.book_dir(&BookId::parse("acme").unwrap()),
        root.path().to_path_buf(),
        home.path().join("missing"),
        home.path().to_path_buf(),
    ] {
        let e = root.adopt_preview(&bad).unwrap_err();
        assert!(matches!(e, RootError::Adopt(_)), "{bad:?}: {e:?}");
        assert!(!e.to_string().contains(&root_text), "{e}");
    }
    // A cut-off last line is refused, as adoption refuses it.
    let cut = tmp("preview-cut");
    drop(source_books(cut.path(), false));
    let journal = cut.path().join(JOURNAL_FILE);
    let mut raw = std::fs::read(&journal).unwrap();
    raw.pop();
    std::fs::write(&journal, raw).unwrap();
    assert!(
        matches!(root.adopt_preview(cut.path()), Err(RootError::Adopt(why)) if why.contains("cut off"))
    );
}

#[test]
fn a_book_folder_is_made_private_and_a_shared_one_refused() {
    let home = tmp("bookfolder");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    drop(
        root.create_book(&BookId::parse("acme").unwrap(), Currency::USD)
            .unwrap(),
    );
    let book = root.book_dir(&BookId::parse("acme").unwrap());
    let keys = ledger_store::root::private_book_folder(&book, "keys").unwrap();
    assert_eq!(keys, book.join("keys"));
    assert_eq!(mode(&keys), 0o700);
    // Again: already there, fine.
    assert_eq!(
        ledger_store::root::private_book_folder(&book, "keys").unwrap(),
        keys
    );
    // Ours, but others could enter it: made private again.
    std::fs::set_permissions(&keys, std::fs::Permissions::from_mode(0o755)).unwrap();
    ledger_store::root::private_book_folder(&book, "keys").unwrap();
    assert_eq!(mode(&keys), 0o700);
    std::os::unix::fs::symlink(home.path(), book.join("vault")).unwrap();
    assert!(ledger_store::root::private_book_folder(&book, "vault").is_err());
}

#[test]
fn the_run_folder_is_private() {
    let home = tmp("run");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let run = root.run_dir().unwrap();
    assert_eq!(run, root.path().join("run"));
    assert_eq!(mode(&run), 0o700);
    std::fs::remove_dir(&run).unwrap();
    assert_eq!(mode(&root.run_dir().unwrap()), 0o700, "remade private");
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(root.run_dir(), Err(RootError::Unsafe(_))));
}

#[test]
fn inside_a_root_is_known_by_its_marker() {
    let home = tmp("inside");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    use ledger_store::root::inside_a_root;
    assert!(inside_a_root(root.path()));
    assert!(inside_a_root(&root.path().join("books/acme")));
    assert!(inside_a_root(&root.path().join("books/not/yet/made")));
    assert!(!inside_a_root(home.path()));
    // A path relative to where the process runs is taken from there.
    let here = std::env::current_dir().unwrap();
    let up = "../".repeat(here.components().count() - 1);
    let relative = PathBuf::from(format!(
        "{up}{}",
        root.path()
            .join("books/acme")
            .display()
            .to_string()
            .trim_start_matches('/')
    ));
    assert!(relative.is_relative());
    assert!(inside_a_root(&relative));
    let outside = PathBuf::from(format!(
        "{up}{}",
        home.path().display().to_string().trim_start_matches('/')
    ));
    assert!(!inside_a_root(&outside));
    assert!(!inside_a_root(&home.path().join("elsewhere")));
    // Through a link, too: it is where the path leads that counts.
    let link = home.path().join("link");
    std::os::unix::fs::symlink(root.path().join("books"), &link).unwrap();
    assert!(inside_a_root(&link));
    assert!(!root.path().join(MARKER_FILE).is_dir());
}

#[test]
fn books_that_changed_since_the_preview_are_not_adopted() {
    let home = tmp("expect");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let old = tmp("expect-src");
    drop(source_books(old.path(), true));
    let previewed = root.adopt_preview(old.path()).unwrap().head;
    // The old server writes one more event after the preview.
    let mut s = Store::open(old.path()).unwrap();
    s.add_account("6100", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let now = s.head();
    // While it holds them, a preview is refused as adoption would be.
    assert!(matches!(
        root.adopt_preview(old.path()),
        Err(RootError::Store(_, StoreError::Locked(_)))
    ));
    drop(s);
    let before = snapshot_files(old.path());
    let e = root
        .adopt(
            old.path(),
            &id("acme"),
            &AdoptOptions {
                expect_head: Some(previewed),
                ..AdoptOptions::default()
            },
        )
        .unwrap_err();
    assert!(
        matches!(&e, RootError::Adopt(why) if why.contains("changed since the preview")),
        "{e:?}"
    );
    assert!(root.list_books().unwrap().is_empty());
    assert_eq!(snapshot_files(old.path()), before);
    // Previewed again, it goes ahead.
    let report = root
        .adopt(
            old.path(),
            &id("acme"),
            &AdoptOptions {
                expect_head: Some(now.clone()),
                ..AdoptOptions::default()
            },
        )
        .unwrap();
    assert_eq!(report.source_head, now);
}

/// A backup of `store`'s books (in `dir`) in a folder target under
/// `home`: its address.
fn backed_up(home: &Path, dir: &Path, store: &Store) -> Option<String> {
    use ledger_store::backup::{self, Git, Target};
    let git = Git::find().ok()?;
    backup::commit(&git, dir, "acme", store.head()).unwrap();
    let folder = home.join("disk");
    std::fs::create_dir_all(&folder).unwrap();
    let target = Target::Folder(folder);
    backup::push(&git, dir, "acme", &target, true).unwrap();
    Some(target.source("acme"))
}

#[test]
fn a_restore_never_moves_books_something_holds() {
    use ledger_store::backup::Git;
    let home = tmp("restore-held");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let acme = id("acme");
    let held = root.create_book(&acme, Currency::USD).unwrap();
    let dir = root.book_dir(&acme);
    let source = backed_up(home.path(), &dir, &held).expect("git is installed");
    let git = Git::find().unwrap();
    let preview = root.restore_preview(&git, &source, &acme).unwrap();
    assert!(preview.exists);
    // Open (its writer lock held): refused, nothing moved.
    let e = root
        .restore(&git, &source, &acme, &preview.head, true)
        .unwrap_err();
    assert!(e.to_string().contains("are open"), "{e}");
    assert!(dir.join(JOURNAL_FILE).is_file());
    assert!(
        !root.path().join("legacy").exists()
            || std::fs::read_dir(root.path().join("legacy"))
                .unwrap()
                .count()
                == 0
    );
    // Not over books without --replace; nothing to replace that isn't.
    let e = root
        .restore(&git, &source, &acme, &preview.head, false)
        .unwrap_err();
    assert!(matches!(e, RootError::Exists(_)), "{e}");
    let e = root
        .restore(&git, &source, &id("nobody"), &preview.head, true)
        .unwrap_err();
    assert!(e.to_string().contains("no books called"), "{e}");
    // Let go: replaced, the old kept whole.
    drop(held);
    let r = root
        .restore(&git, &source, &acme, &preview.head, true)
        .unwrap();
    let kept = root.path().join("legacy").join(r.replaced.unwrap());
    assert!(kept.join(JOURNAL_FILE).is_file());
    assert_eq!(r.head, preview.head);
    // Nothing of the restore is left behind in books/.
    let left: Vec<_> = std::fs::read_dir(root.path().join("books"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}
