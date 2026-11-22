//! Backups onto the disks people really have (WB-backup-18 to WB-backup-22):
//! a disk that makes no hard links (FAT or exFAT on a stick), sealed pieces
//! in folders a small disk can hold, a header that asks for more than it
//! should, and a folder a sync service has not finished putting on this
//! computer.

use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::Store;
use ledger_store::backup::{
    BackupError, BackupKey, Ctx, Cut, Destination, Options, find, key_derivations, verify_found,
    write_destination, write_destination_with,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn usd(s: &str) -> Money {
    Money::new(s.parse().unwrap(), Currency::USD)
}

fn book(dir: &Path) -> Store {
    let mut s = Store::create(dir, Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let cap = s
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let e = s
        .create_entry(
            chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            "Start",
        )
        .unwrap();
    s.add_line(e, cash, Side::Debit, usd("100.00")).unwrap();
    s.add_line(e, cap, Side::Credit, usd("100.00")).unwrap();
    s.post_entry(e).unwrap();
    s.attach_receipt(Some(e), "r.pdf", "application/pdf", b"%PDF-1.4 r", None)
        .unwrap();
    s
}

fn more(s: &mut Store, n: usize) {
    s.add_account(
        &format!("{}", 6000 + n),
        &format!("Expense {n}"),
        AccountType::Expense,
        None,
        false,
    )
    .unwrap();
    s.attach_receipt(
        None,
        &format!("f{n}.txt"),
        "text/plain",
        format!("receipt number {n}").as_bytes(),
        None,
    )
    .unwrap();
}

fn cut(dir: &Path, s: &Store) -> Cut {
    Cut::take("acme", dir, s.head()).unwrap()
}

fn plain(folder: &Path) -> Destination {
    Destination {
        folder: folder.to_path_buf(),
        key: None,
    }
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    if root.is_dir() {
        walk(root, root, &mut out);
    }
    out
}

fn prove(
    source: &Path,
    pass: Option<&str>,
    root: &Path,
    name: &str,
) -> ledger_store::backup::Verified {
    let found = find(source, pass, &Ctx::new()).unwrap();
    verify_found(&found, &root.join(name), &Ctx::new()).unwrap()
}

// --- a disk that makes no hard links ---------------------------------------

#[test]
fn a_disk_that_makes_no_links_takes_whole_backups_and_only_grows() {
    for ctx in [
        Ctx::new().with_no_hard_links(),
        // And one whose move cannot refuse to replace either: exclusive
        // create and copy.
        Ctx::new().with_no_exclusive_move(),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let mut s = book(&b);
        let disk = dir.path().join("disk");
        std::fs::create_dir(&disk).unwrap();
        let mut before = BTreeMap::new();
        for n in 0..3 {
            more(&mut s, n);
            let w = write_destination(&plain(&disk), &cut(&b, &s), true, &ctx).unwrap();
            assert!(w.new && w.repaired.is_empty(), "{w:?}");
            let now = tree(&disk);
            for (name, bytes) in &before {
                assert_eq!(now.get(name), Some(bytes), "{name} must not change");
            }
            assert!(!now.keys().any(|n| n.contains(".wb-tmp-")), "scratch left");
            before = now;
        }
        let v = prove(&disk, None, dir.path(), "v");
        assert_eq!(v.head, s.head());
        assert_eq!(v.passed_over, 0);
    }
}

#[test]
fn two_writers_racing_on_a_disk_that_makes_no_links_do_not_corrupt_it() {
    for exclusive_move_missing in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let mut s = book(&b);
        let disk = dir.path().join("disk");
        std::fs::create_dir(&disk).unwrap();
        for round in 0..3 {
            more(&mut s, round);
            let c = cut(&b, &s);
            let results: Vec<_> = std::thread::scope(|scope| {
                let handles: Vec<_> = (0..3)
                    .map(|_| {
                        let (c, disk) = (c.clone(), disk.clone());
                        scope.spawn(move || {
                            let ctx = if exclusive_move_missing {
                                Ctx::new().with_no_exclusive_move()
                            } else {
                                Ctx::new().with_no_hard_links()
                            };
                            write_destination(&plain(&disk), &c, true, &ctx)
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
            for r in &results {
                let w = r
                    .as_ref()
                    .unwrap_or_else(|e| panic!("round {round}: {e:?}"));
                assert_eq!(w.head, s.head());
            }
            let v = prove(&disk, None, dir.path(), &format!("v{round}"));
            assert_eq!(v.head, s.head());
            assert_eq!(v.passed_over, 0, "round {round}");
            assert!(
                !tree(&disk).keys().any(|n| n.contains(".wb-tmp-")),
                "scratch left"
            );
        }
    }
}

#[test]
fn a_taken_number_or_name_is_never_replaced_on_a_disk_that_makes_no_links() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let ctx = Ctx::new().with_no_hard_links();
    write_destination(&plain(&disk), &cut(&b, &s), true, &ctx).unwrap();
    let set = disk.join("wanderbooks-acme");
    // A rival's manifest 2 is already in place when this one comes to put
    // its own as number 2.
    more(&mut s, 1);
    let rival = disk.join("rival");
    std::fs::create_dir(&rival).unwrap();
    let c = cut(&b, &s);
    let taken = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hook = {
        let (set, taken) = (set.clone(), taken.clone());
        Ctx::new()
            .with_no_hard_links()
            .with_faults(std::sync::Arc::new(move |kind, rel| {
                if kind == "create"
                    && rel.starts_with("manifests/")
                    && taken.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
                {
                    std::fs::write(set.join("manifests/000002.json"), b"a rival's, whole").unwrap();
                }
                false
            }))
    };
    let _ = write_destination(&plain(&disk), &c, true, &hook);
    assert_eq!(
        std::fs::read(set.join("manifests/000002.json")).unwrap(),
        b"a rival's, whole",
        "the manifest that was there is never replaced"
    );
}

// --- sealed pieces in folders a small disk can hold --------------------------

const PASS: &str = "correct horse battery staple";

/// The count of derived keys is the process's: the tests that read it, and
/// every test that derives one, take turns.
static KEYS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    KEYS.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn sealed_pieces_are_spread_over_folders_as_the_plain_form_does() {
    let _turn = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    for n in 0..30 {
        more(&mut s, n);
    }
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let dest = Destination {
        folder: disk.clone(),
        key: Some(BackupKey::generate(PASS).unwrap()),
    };
    write_destination(&dest, &cut(&b, &s), true, &Ctx::new()).unwrap();
    let set = std::fs::read_dir(&disk)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("sealed-"))
        .unwrap()
        .path();
    let mut in_folders = 0usize;
    for entry in std::fs::read_dir(set.join("pieces")).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(
            entry.path().is_dir(),
            "{name}: a piece sits in a flat folder"
        );
        assert_eq!(name.len(), 2, "{name}");
        for piece in std::fs::read_dir(entry.path()).unwrap().flatten() {
            assert!(
                piece.file_name().to_string_lossy().starts_with(&name),
                "a piece is in the folder of its name's first two digits"
            );
            in_folders += 1;
        }
    }
    assert!(in_folders > 30, "{in_folders}");
    let v = prove(&disk, Some(PASS), dir.path(), "v");
    assert_eq!(v.head, s.head());
}

// --- a header that is read from a folder anyone may have written -------------

/// The sealed set's folder in `disk`.
fn sealed_set(disk: &Path) -> PathBuf {
    std::fs::read_dir(disk)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("sealed-"))
        .unwrap()
        .path()
}

/// Rewrite the header's key settings and make its own check line right
/// again (what a person who meant to would do).
fn set_header(header: &Path, edit: impl Fn(&mut serde_json::Value)) {
    let text = std::fs::read_to_string(header).unwrap();
    let body = text.lines().next().unwrap();
    let mut v: serde_json::Value = serde_json::from_str(body).unwrap();
    edit(&mut v);
    let body = serde_json::to_string(&v).unwrap();
    let check = hex::encode(Sha256::digest(body.as_bytes()));
    std::fs::write(
        header,
        format!("{body}\nwanderbooks-header-sha256 {check}\n"),
    )
    .unwrap();
}

#[test]
fn a_header_cannot_ask_for_more_than_a_few_times_what_this_program_spends() {
    let _turn = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let dest = Destination {
        folder: disk.clone(),
        key: Some(BackupKey::generate(PASS).unwrap()),
    };
    write_destination(&dest, &cut(&b, &s), true, &Ctx::new()).unwrap();
    let header = sealed_set(&disk).join("header.json");
    let original = std::fs::read(&header).unwrap();
    for (what, value) in [
        ("m", serde_json::json!(4_000_000u32)),
        ("m", serde_json::json!(64 * 1024 * 5)),
        ("t", serde_json::json!(13)),
        ("p", serde_json::json!(5)),
    ] {
        set_header(&header, |v| v["kdf"][what] = value.clone());
        let derived = key_derivations();
        let err = find(&disk, Some(PASS), &Ctx::new()).err().unwrap();
        // Refused as damage before any key is derived: no memory spent.
        assert!(
            matches!(err, BackupError::Damaged(_) | BackupError::NotABackup),
            "{what}: {err:?}"
        );
        assert_eq!(key_derivations(), derived, "{what}: a key was derived");
    }
    std::fs::write(&header, &original).unwrap();
    let v = prove(&disk, Some(PASS), dir.path(), "v");
    assert_eq!(v.head, s.head());
}

#[test]
fn a_header_that_was_changed_reads_as_damage_and_not_as_a_wrong_passphrase() {
    let _turn = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let dest = Destination {
        folder: disk.clone(),
        key: Some(BackupKey::generate(PASS).unwrap()),
    };
    write_destination(&dest, &cut(&b, &s), true, &Ctx::new()).unwrap();
    let header = sealed_set(&disk).join("header.json");
    // A setting changed in the file as a bad sector or a bad sync would:
    // the header's own check line no longer matches.
    let text = std::fs::read_to_string(&header).unwrap();
    let changed = text.replacen("\"t\":3", "\"t\":4", 1);
    assert_ne!(text, changed, "the header names its passes");
    std::fs::write(&header, changed).unwrap();
    let err = find(&disk, Some(PASS), &Ctx::new()).err().unwrap();
    assert!(
        matches!(err, BackupError::Damaged(_) | BackupError::NotABackup),
        "{err:?}"
    );
    // Changed on purpose and made consistent again: the key comes out
    // different and opens nothing (the safe way to fail), and no
    // passphrase is guessed any faster.
    std::fs::write(&header, &text).unwrap();
    set_header(&header, |v| v["kdf"]["t"] = serde_json::json!(4));
    assert!(matches!(
        find(&disk, Some(PASS), &Ctx::new()),
        Err(BackupError::WrongPassphrase)
    ));
}

#[test]
fn a_folder_of_sealed_sets_costs_one_key_derivation_per_setting_not_one_per_set() {
    let _turn = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let dest = Destination {
        folder: disk.clone(),
        key: Some(BackupKey::generate(PASS).unwrap()),
    };
    let c = cut(&b, &s);
    write_destination(&dest, &c, true, &Ctx::new()).unwrap();
    for _ in 0..2 {
        write_destination_with(
            &dest,
            &c,
            Options {
                may_create: true,
                new_set: true,
            },
            &Ctx::new(),
        )
        .unwrap();
    }
    let sets = std::fs::read_dir(&disk)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("sealed-"))
        .count();
    assert_eq!(sets, 3);
    let derived = key_derivations();
    // Three sets of one book: it asks which, after one derivation.
    let err = find(&disk, Some(PASS), &Ctx::new()).err().unwrap();
    assert!(matches!(err, BackupError::Several(_)), "{err:?}");
    assert_eq!(key_derivations() - derived, 1);
}

// --- files a sync service has not downloaded ---------------------------------

#[test]
fn a_file_that_is_not_downloaded_yet_reads_as_such_and_never_as_damage_or_absence() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    let set = disk.join("wanderbooks-acme");
    let before = tree(&disk);

    // A piece of the newest backup that iCloud has left as a stand-in
    // (`.name.icloud`) in its place.
    let manifest = std::fs::read_to_string(set.join("manifests/000001.json")).unwrap();
    let m: serde_json::Value = serde_json::from_str(manifest.lines().next().unwrap()).unwrap();
    let seg = set.join(m["journal"]["segments"][0]["file"].as_str().unwrap());
    let stand_in = seg.with_file_name(format!(
        ".{}.icloud",
        seg.file_name().unwrap().to_string_lossy()
    ));
    let held = std::fs::read(&seg).unwrap();
    std::fs::rename(&seg, &stand_in).unwrap();
    let found = find(&disk, None, &Ctx::new());
    // Nothing says "damaged", nothing says "not a backup": it is waiting.
    assert_eq!(found.err(), Some(BackupError::NotDownloaded));
    more(&mut s, 1);
    assert_eq!(
        write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap_err(),
        BackupError::NotDownloaded
    );
    assert!(
        BackupError::NotDownloaded
            .to_string()
            .contains("not downloaded to this computer yet")
    );
    // Downloaded: all as it was.
    std::fs::rename(&stand_in, &seg).unwrap();
    assert_eq!(std::fs::read(&seg).unwrap(), held);
    assert_eq!(tree(&disk), before);
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();

    // The newest manifest as a stand-in: the folder is listed with it, and
    // the writer will not make a manifest under its number.
    let newest = set.join("manifests/000002.json");
    let stand = set.join("manifests/.000002.json.icloud");
    std::fs::rename(&newest, &stand).unwrap();
    more(&mut s, 2);
    assert_eq!(
        write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap_err(),
        BackupError::NotDownloaded
    );
    assert_eq!(
        find(&disk, None, &Ctx::new()).err(),
        Some(BackupError::NotDownloaded)
    );
}
