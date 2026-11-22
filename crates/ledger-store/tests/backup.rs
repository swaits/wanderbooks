//! Backups in plain folders (WB-backup-1 to WB-backup-9): what a backup
//! is, that it only grows, that a restore of any whole manifest is a
//! prefix of the books, that two writers do not corrupt one destination,
//! that a program that dies at any write leaves a destination that proves
//! whole at its previous manifest, that a flipped or cut piece is never
//! restored, and that a wrong passphrase never yields books.

use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::Store;
use ledger_store::backup::{
    BackupError, BackupKey, Ctx, Cut, Destination, find, save_file, verify_found, write_destination,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn usd(s: &str) -> Money {
    Money::new(s.parse().unwrap(), Currency::USD)
}

/// A book with a few entries and a stored file.
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

/// One more write to the books: an account and a stored file.
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

/// Every file under `root`, by relative path, with its bytes.
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

/// Prove the backup at `source` and give the books it holds, built in a
/// scratch folder next to `root`.
fn prove(
    source: &Path,
    pass: Option<&str>,
    root: &Path,
    name: &str,
) -> Result<ledger_store::backup::Verified, BackupError> {
    let found = find(source, pass, &Ctx::new())?;
    verify_found(&found, &root.join(name), &Ctx::new())
}

#[test]
fn a_backup_is_the_book_and_proves_whole() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();

    let w = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    assert!(w.new);
    assert_eq!(w.number, 1);
    assert_eq!(w.head, s.head());

    // The form on disk: a folder for the books, a read-me, one manifest,
    // the journal as a segment, the stored file under its hash.
    let t = tree(&disk);
    let names: Vec<&String> = t.keys().collect();
    assert!(names.iter().any(|n| *n == "acme/READ-ME.txt"), "{names:?}");
    assert!(names.iter().any(|n| *n == "acme/manifests/000001.json"));
    assert!(names.iter().any(|n| n.starts_with("acme/journal/000001-")));
    assert!(names.iter().any(|n| n.starts_with("acme/files/")));
    assert!(!names.iter().any(|n| n.contains(".git")));

    // Nothing new: no manifest.
    let again = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    assert!(!again.new);
    assert_eq!(again.number, 1);
    assert_eq!(tree(&disk), t);

    // Proven, and the books it holds are the book.
    let v = prove(&disk, None, dir.path(), "v1").unwrap();
    assert_eq!(v.head, s.head());
    assert_eq!(v.blobs, 1);
    assert_eq!(v.book.as_deref(), Some("acme"));
    assert_eq!(
        std::fs::read(v.tree.join("journal.jsonl")).unwrap(),
        std::fs::read(b.join("journal.jsonl")).unwrap()
    );

    // More writes: a new manifest and new pieces; the old ones untouched.
    more(&mut s, 1);
    let w2 = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    assert_eq!(w2.number, 2);
    let t2 = tree(&disk);
    for (name, bytes) in &t {
        assert_eq!(t2.get(name), Some(bytes), "{name} must not change");
    }
    assert!(t2.len() > t.len());
    let v2 = prove(&disk, None, dir.path(), "v2").unwrap();
    assert_eq!(v2.head, s.head());
    assert_eq!(v2.blobs, 2);
    // No scratch file of ours is left behind.
    assert!(!t2.keys().any(|n| n.contains(".wb-tmp-")));
}

#[test]
fn a_destination_of_other_books_or_another_history_is_never_written() {
    let dir = tempfile::tempdir().unwrap();
    let (b1, b2) = (dir.path().join("one"), dir.path().join("two"));
    let (mut s1, s2) = (book(&b1), book(&b2));
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b1, &s1), true, &Ctx::new()).unwrap();
    let before = tree(&disk);
    // The same id, another history (a different journal from the start).
    assert_eq!(
        write_destination(&plain(&disk), &cut(&b2, &s2), true, &Ctx::new()).unwrap_err(),
        BackupError::Elsewhere
    );
    assert_eq!(tree(&disk), before);
    // Books behind the destination: said, nothing written.
    more(&mut s1, 1);
    write_destination(&plain(&disk), &cut(&b1, &s1), true, &Ctx::new()).unwrap();
    let ahead = tree(&disk);
    let b1_old = dir.path().join("one-old");
    let _old = book(&b1_old);
    let err =
        write_destination(&plain(&disk), &cut(&b1_old, &_old), true, &Ctx::new()).unwrap_err();
    assert!(
        matches!(err, BackupError::Ahead(..) | BackupError::Elsewhere),
        "{err:?}"
    );
    assert_eq!(tree(&disk), ahead);
}

#[test]
fn a_disk_that_is_gone_is_never_made_again_where_it_should_be() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    let missing = dir.path().join("nodisk");
    assert_eq!(
        write_destination(&plain(&missing), &cut(&b, &s), true, &Ctx::new()).unwrap_err(),
        BackupError::NoFolder
    );
    // A mount point with nothing on it: the folder is there, the books'
    // backup is not, and the caller says it never made one here.
    let mount = dir.path().join("mount");
    std::fs::create_dir(&mount).unwrap();
    assert_eq!(
        write_destination(&plain(&mount), &cut(&b, &s), false, &Ctx::new()).unwrap_err(),
        BackupError::NoBackupThere
    );
    assert!(tree(&mount).is_empty());
}

#[test]
fn a_stopped_backup_leaves_what_proved_whole_before() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    more(&mut s, 1);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let err = write_destination(
        &plain(&disk),
        &cut(&b, &s),
        true,
        &Ctx::new().with_stop(stop),
    )
    .unwrap_err();
    assert!(matches!(err, BackupError::Cancelled(_)), "{err:?}");
    let v = prove(&disk, None, dir.path(), "v").unwrap();
    assert_eq!(v.number, 1);
}

// --- the passphrase --------------------------------------------------

const PASS: &str = "correct horse battery staple";

#[test]
fn a_sealed_backup_gives_away_counts_and_sizes_only() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let key = BackupKey::generate(PASS).unwrap();
    let dest = Destination {
        folder: disk.clone(),
        key: Some(key),
    };
    write_destination(&dest, &cut(&b, &s), true, &Ctx::new()).unwrap();
    for (name, bytes) in tree(&disk) {
        for word in [
            "acme", "journal", "access", "vault", "keys", "blobs", "files", "receipt", "Cash",
        ] {
            // The read-me is plain words and says nothing of the books;
            // nothing else carries a word of them, in its name or in it.
            if !name.ends_with("READ-ME.txt") {
                assert!(!name.contains(word), "{name} names {word}");
                assert!(
                    !String::from_utf8_lossy(&bytes).contains(word)
                        || name.ends_with("header.json"),
                    "{name} holds {word}"
                );
            }
        }
        assert!(!bytes.windows(8).any(|w| w == b"PDF-1.4 "), "{name}");
    }
    // Opened with the passphrase: the same books.
    let v = prove(&disk, Some(PASS), dir.path(), "v").unwrap();
    assert_eq!(v.head, s.head());
    assert!(v.key.is_some());
}

#[test]
fn a_wrong_passphrase_never_yields_books_and_is_told_from_damage() {
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
    assert!(matches!(
        find(&disk, None, &Ctx::new()),
        Err(BackupError::NeedsPassphrase)
    ));
    assert!(matches!(
        find(&disk, Some("not the right words at all"), &Ctx::new()),
        Err(BackupError::WrongPassphrase)
    ));
    // A damaged header is damage, not a wrong passphrase.
    let set = std::fs::read_dir(&disk)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("sealed-"))
        .unwrap()
        .path();
    let header = set.join("header.json");
    let mut bytes = std::fs::read(&header).unwrap();
    bytes[40] ^= 1;
    std::fs::write(&header, &bytes).unwrap();
    let err = find(&disk, Some(PASS), &Ctx::new()).err().unwrap();
    assert!(
        matches!(err, BackupError::Damaged(_) | BackupError::NotABackup),
        "{err:?}"
    );
    assert!(!dir.path().join("v").exists());
}

#[test]
fn a_changed_passphrase_is_a_new_set_beside_the_old_which_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let first = Destination {
        folder: disk.clone(),
        key: Some(BackupKey::generate(PASS).unwrap()),
    };
    write_destination(&first, &cut(&b, &s), true, &Ctx::new()).unwrap();
    let before = tree(&disk);
    more(&mut s, 1);
    // Another passphrase: found no set of its own, so (the caller says it
    // may) a new one is made; the first is not touched.
    let second = Destination {
        folder: disk.clone(),
        key: Some(BackupKey::generate("an entirely different phrase").unwrap()),
    };
    write_destination(&second, &cut(&b, &s), true, &Ctx::new()).unwrap();
    let after = tree(&disk);
    for (name, bytes) in &before {
        assert_eq!(after.get(name), Some(bytes), "{name}");
    }
    let sets: Vec<_> = std::fs::read_dir(&disk).unwrap().flatten().collect();
    assert_eq!(sets.len(), 2);
    // The old passphrase still opens the old set only when it is alone;
    // with two sets, a passphrase opens the one that is its own.
    let v = prove(&disk, Some("an entirely different phrase"), dir.path(), "v").unwrap();
    assert_eq!(v.head, s.head());
}

#[test]
fn the_key_is_kept_privately_and_only_there() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    let key = BackupKey::generate(PASS).unwrap();
    key.store(&b).unwrap();
    let loaded = BackupKey::load(&b).unwrap().expect("kept");
    assert!(key.same_as(&loaded));
    let file = b.join("local").join("backup-key.json");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains(PASS));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = std::fs::metadata(file.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
    }
    // Never in a backup: not the journal, not the folder of the backup.
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    let raw = text
        .lines()
        .find(|l| l.contains("master"))
        .unwrap()
        .to_string();
    let master = raw.split('"').nth(3).unwrap().to_string();
    for (name, bytes) in tree(&disk) {
        assert!(!String::from_utf8_lossy(&bytes).contains(&master), "{name}");
    }
    BackupKey::forget(&b).unwrap();
    assert!(BackupKey::load(&b).unwrap().is_none());
}

// --- the single file -------------------------------------------------

#[test]
fn a_single_file_holds_the_same_backup_plain_or_sealed() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    for (pass, name) in [(None, "plain.wbbackup"), (Some(PASS), "sealed.wbbackup")] {
        let key = pass.map(|p| BackupKey::generate(p).unwrap());
        let file = dir.path().join(name);
        let w = save_file(&file, &cut(&b, &s), key.as_ref(), &Ctx::new()).unwrap();
        assert_eq!(w.head, s.head());
        let v = prove(&file, pass, dir.path(), &format!("v-{name}")).unwrap();
        assert_eq!(v.head, s.head());
        assert_eq!(v.blobs, 1);
        // A file by that name is not written over.
        assert!(save_file(&file, &cut(&b, &s), key.as_ref(), &Ctx::new()).is_err());
    }
    assert!(matches!(
        find(&dir.path().join("sealed.wbbackup"), None, &Ctx::new()),
        Err(BackupError::NeedsPassphrase)
    ));
    assert!(matches!(
        find(
            &dir.path().join("sealed.wbbackup"),
            Some("a wrong passphrase here"),
            &Ctx::new()
        ),
        Err(BackupError::WrongPassphrase)
    ));
    // Not a backup at all.
    let junk = dir.path().join("junk.wbbackup");
    std::fs::write(&junk, b"not a zip").unwrap();
    assert!(matches!(
        find(&junk, None, &Ctx::new()),
        Err(BackupError::NotABackup)
    ));
}

// --- damage ------------------------------------------------------------

/// Every piece a manifest names, as paths under the set.
fn pieces(set: &Path) -> Vec<PathBuf> {
    tree(set)
        .keys()
        .filter(|n| {
            !n.starts_with("manifests/")
                && !n.ends_with("READ-ME.txt")
                && !n.ends_with("header.json")
        })
        .map(|n| set.join(n))
        .collect()
}

#[test]
fn a_flipped_or_cut_piece_is_found_and_never_restored() {
    for sealed in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let s = book(&b);
        let disk = dir.path().join("disk");
        std::fs::create_dir(&disk).unwrap();
        let key = sealed.then(|| BackupKey::generate(PASS).unwrap());
        let dest = Destination {
            folder: disk.clone(),
            key,
        };
        write_destination(&dest, &cut(&b, &s), true, &Ctx::new()).unwrap();
        let pass = sealed.then_some(PASS);
        // Found once (a sealed backup's key takes a moment to derive).
        let found = find(&disk, pass, &Ctx::new()).unwrap();
        let prove = |_: &Path, _: Option<&str>, root: &Path, name: &str| {
            verify_found(&found, &root.join(name), &Ctx::new())
        };
        let set = std::fs::read_dir(&disk)
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .path();
        for piece in pieces(&set) {
            let original = std::fs::read(&piece).unwrap();
            for damage in 0..3 {
                let mut bytes = original.clone();
                match damage {
                    0 => {
                        let at = bytes.len() / 2;
                        bytes[at] ^= 0x01;
                    }
                    1 => bytes.truncate(bytes.len().saturating_sub(1)),
                    _ => bytes.clear(),
                }
                std::fs::write(&piece, &bytes).unwrap();
                let scratch = format!("v-{damage}");
                let got = prove(&disk, pass, dir.path(), &scratch);
                assert!(
                    got.is_err(),
                    "{} damage {damage} of {piece:?} was restored",
                    if sealed { "sealed" } else { "plain" }
                );
                assert!(!dir.path().join(&scratch).exists(), "scratch left");
            }
            std::fs::write(&piece, &original).unwrap();
        }
        // Whole again once put back.
        prove(&disk, pass, dir.path(), "ok").unwrap();
        // A manifest cut short or changed is no manifest.
        let manifest = tree(&set)
            .keys()
            .find(|n| n.starts_with("manifests/"))
            .cloned()
            .unwrap();
        let mpath = set.join(&manifest);
        let original = std::fs::read(&mpath).unwrap();
        std::fs::write(&mpath, &original[..original.len() / 2]).unwrap();
        assert!(prove(&disk, pass, dir.path(), "cut").is_err());
        std::fs::write(&mpath, &original).unwrap();
    }
}

#[test]
fn a_damaged_newest_backup_falls_back_to_the_one_before_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    let first_head = s.head();
    more(&mut s, 1);
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    // Flip a byte in the second journal segment.
    let seg = tree(&disk.join("acme"))
        .keys()
        .filter(|n| n.starts_with("journal/"))
        .max()
        .cloned()
        .unwrap();
    let path = disk.join("acme").join(&seg);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[3] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    let v = prove(&disk, None, dir.path(), "v").unwrap();
    assert_eq!(v.number, 1);
    assert_eq!(v.passed_over, 1);
    assert_eq!(v.head, first_head);
}

// --- death at every write point --------------------------------------

#[test]
fn a_program_that_dies_at_any_write_leaves_what_proved_whole_before() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let base_disk = dir.path().join("base");
    std::fs::create_dir(&base_disk).unwrap();
    write_destination(&plain(&base_disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    let first_head = s.head();
    more(&mut s, 1);
    let c = cut(&b, &s);

    // How many write points a whole second backup has.
    let counted = Arc::new(AtomicUsize::new(0));
    let probe = {
        let disk = dir.path().join("probe");
        copy_tree(&base_disk, &disk);
        let n = counted.clone();
        let ctx = Ctx::new().with_faults(Arc::new(move |_, _| {
            n.fetch_add(1, Ordering::SeqCst);
            false
        }));
        write_destination(&plain(&disk), &c, true, &ctx).unwrap();
        counted.load(Ordering::SeqCst)
    };
    assert!(probe >= 6, "a second backup has several writes: {probe}");

    for die_at in 0..probe {
        let disk = dir.path().join(format!("died-{die_at}"));
        copy_tree(&base_disk, &disk);
        let n = Arc::new(AtomicUsize::new(0));
        let ctx = {
            let n = n.clone();
            Ctx::new().with_faults(Arc::new(move |_, _| {
                n.fetch_add(1, Ordering::SeqCst) == die_at
            }))
        };
        let err = write_destination(&plain(&disk), &c, true, &ctx).unwrap_err();
        assert_eq!(err, BackupError::Died, "point {die_at}");
        // What is left proves whole at the previous manifest, never a
        // half-written one (before the manifest was in place), or at the
        // new one (after).
        let v = prove(&disk, None, dir.path(), &format!("r-{die_at}")).unwrap();
        assert!(
            v.head == first_head || v.head == s.head(),
            "point {die_at}: head {:?}",
            v.head
        );
        // A restore from what is left is a prefix of the books.
        let restored = std::fs::read(v.tree.join("journal.jsonl")).unwrap();
        let live = std::fs::read(b.join("journal.jsonl")).unwrap();
        assert!(live.starts_with(&restored), "point {die_at}");
        // And the next run goes on from it (proven again at some points:
        // each proof replays the books).
        let w = write_destination(&plain(&disk), &c, true, &Ctx::new()).unwrap();
        assert_eq!(w.head, s.head());
        if die_at % 6 == 0 {
            let again = prove(&disk, None, dir.path(), &format!("again-{die_at}")).unwrap();
            assert_eq!(again.head, s.head());
        }
    }
}

fn copy_tree(from: &Path, to: &Path) {
    for (rel, bytes) in tree(from) {
        let p = to.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
    std::fs::create_dir_all(to).unwrap();
}

// --- two writers -------------------------------------------------------

#[test]
fn two_programs_writing_one_destination_do_not_corrupt_it() {
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
                    scope.spawn(move || write_destination(&plain(&disk), &c, true, &Ctx::new()))
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
        // However they interleaved: whole, at these books, and every
        // manifest there stands alone.
        let v = prove(&disk, None, dir.path(), &format!("v{round}")).unwrap();
        assert_eq!(v.head, s.head());
        let set = disk.join("acme");
        for n in std::fs::read_dir(set.join("manifests")).unwrap().flatten() {
            assert!(n.file_name().to_string_lossy().ends_with(".json"));
        }
    }
}

#[test]
fn a_loser_of_a_manifest_number_goes_on_from_the_winner() {
    // Another program takes the next number between this one's look and its
    // write: this one looks again and finds nothing left to write.
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let c = cut(&b, &s);
    let rival_disk = disk.clone();
    let rival_cut = c.clone();
    let done = Arc::new(AtomicUsize::new(0));
    let ctx = {
        let done = done.clone();
        Ctx::new().with_faults(Arc::new(move |kind, rel| {
            // Just before this writer's manifest, the rival writes its own.
            if kind == "create"
                && rel.starts_with("manifests/")
                && done.fetch_add(1, Ordering::SeqCst) == 0
            {
                write_destination(&plain(&rival_disk), &rival_cut, true, &Ctx::new()).unwrap();
            }
            false
        }))
    };
    let w = write_destination(&plain(&disk), &c, true, &ctx).unwrap();
    assert_eq!(w.head, s.head());
    assert!(!w.new, "the winner's manifest already says it");
    assert_eq!(
        std::fs::read_dir(disk.join("acme/manifests"))
            .unwrap()
            .count(),
        1
    );
    prove(&disk, None, dir.path(), "v").unwrap();
}

// --- what the store reads of a backup -----------------------------------

#[test]
fn the_text_read_from_a_stored_file_is_a_stored_file_too() {
    // The store's own verification reads the text an extraction recorded as
    // a stored file; a backup that left it out would not prove whole, so it
    // is carried and proven with the rest.
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let (attachment, _, _) = s
        .attach_receipt(
            None,
            "words.txt",
            "text/plain",
            b"a receipt with words",
            None,
        )
        .unwrap();
    s.extract_attachment(attachment).unwrap();
    let text_sha = s
        .extraction(attachment)
        .unwrap()
        .and_then(|x| x.text_sha256)
        .expect("the text is a stored file");
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    let v = prove(&disk, None, dir.path(), "v").unwrap();
    assert!(
        v.tree
            .join("blobs")
            .join(&text_sha[0..2])
            .join(&text_sha[2..4])
            .join(&text_sha)
            .is_file()
    );
    assert_eq!(v.head, s.head());
}

// --- what the removed tests of the old form protected ------------------------

#[test]
fn a_purged_vault_leaves_the_next_backup_and_not_the_earlier_ones() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    std::fs::create_dir_all(b.join("vault")).unwrap();
    std::fs::write(b.join("vault").join("tin-1.bin"), b"sealed record").unwrap();
    std::fs::create_dir_all(b.join("keys")).unwrap();
    std::fs::write(b.join("keys").join("legacy.key"), "ab".repeat(32)).unwrap();
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    // The vault is purged to nothing: the next backup holds none, the
    // earlier one still does (nothing in a destination is deleted).
    std::fs::remove_file(b.join("vault").join("tin-1.bin")).unwrap();
    std::fs::remove_dir(b.join("vault")).unwrap();
    let w = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    assert!(w.new && w.number == 2, "{w:?}");
    let now = prove(&disk, None, dir.path(), "now").unwrap();
    assert!(!now.tree.join("vault").exists());
    assert_eq!(
        std::fs::read_to_string(now.tree.join("keys").join("legacy.key")).unwrap(),
        "ab".repeat(32)
    );
    let then = dir.path().join("then");
    copy_tree(&disk, &then);
    std::fs::remove_file(then.join("acme/manifests/000002.json")).unwrap();
    let old = prove(&then, None, dir.path(), "old").unwrap();
    assert_eq!(
        std::fs::read(old.tree.join("vault").join("tin-1.bin")).unwrap(),
        b"sealed record"
    );
}

#[test]
fn a_scratch_file_left_by_a_dead_program_never_stops_the_next_backup() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    // What a program that died leaves: a scratch file, a short manifest at
    // the next number (its number is spent), a piece cut short.
    let set = disk.join("acme");
    std::fs::write(set.join("manifests").join(".wb-tmp-9-abcdef"), b"half").unwrap();
    std::fs::write(
        set.join("manifests").join("000002.json"),
        b"{\"format\":\"wan",
    )
    .unwrap();
    more(&mut s, 1);
    let w = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    assert!(w.new && w.number == 3, "{w:?}");
    let v = prove(&disk, None, dir.path(), "v").unwrap();
    assert_eq!(v.head, s.head());
}

#[test]
fn a_stored_file_that_does_not_match_its_name_is_refused_and_nothing_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    let sha = ledger_store::journal::sha256_hex(b"%PDF-1.4 r");
    let blob = b.join("blobs").join(&sha[0..2]).join(&sha[2..4]).join(&sha);
    std::fs::write(&blob, b"changed under its name").unwrap();
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let err = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap_err();
    assert!(matches!(err, BackupError::Damaged(_)), "{err:?}");
    // No manifest: nothing there is a backup.
    assert!(
        !tree(&disk)
            .keys()
            .any(|n| n.contains("manifests/") && n.ends_with(".json"))
    );
}

#[test]
fn a_drill_proves_this_books_history_only() {
    use ledger_store::backup::drill;
    let dir = tempfile::tempdir().unwrap();
    let (b1, b2) = (dir.path().join("one"), dir.path().join("two"));
    let (mut s1, s2) = (book(&b1), book(&b2));
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b1, &s1), true, &Ctx::new()).unwrap();
    let head = s1.head();
    // The books move on: the backup is the beginning of their journal.
    more(&mut s1, 1);
    let got = drill(
        &plain(&disk),
        "acme",
        &b1.join("journal.jsonl"),
        &dir.path().join("d1"),
        &Ctx::new(),
    )
    .unwrap();
    assert_eq!(got, head);
    assert!(!dir.path().join("d1").exists(), "the scratch is removed");
    // Another book's history under the same name: not this book's.
    let err = drill(
        &plain(&disk),
        "acme",
        &b2.join("journal.jsonl"),
        &dir.path().join("d2"),
        &Ctx::new(),
    )
    .unwrap_err();
    assert!(
        matches!(err, BackupError::Damaged(_)),
        "{err:?} {:?}",
        s2.head()
    );
    assert!(!dir.path().join("d2").exists());
}

#[test]
fn the_local_backup_proves_to_hold_the_book_as_it_is_keys_and_all() {
    use ledger_store::backup::{prove_local, write_local};
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    std::fs::create_dir_all(b.join("keys")).unwrap();
    std::fs::write(b.join("keys").join("legacy.key"), "ab".repeat(32)).unwrap();
    let scratch = dir.path().join("check");
    let c = cut(&b, &s);
    write_local(&b, &c, &Ctx::new()).unwrap();
    assert_eq!(
        prove_local(&b, "acme", &s.head(), &scratch, &Ctx::new()).unwrap(),
        1
    );
    assert!(!scratch.exists(), "the scratch folder is removed");
    // A changed key, a changed vault record, an extra one, books that moved
    // on, each refused.
    std::fs::write(b.join("keys").join("legacy.key"), "cd".repeat(32)).unwrap();
    assert!(prove_local(&b, "acme", &s.head(), &scratch, &Ctx::new()).is_err());
    std::fs::write(b.join("keys").join("legacy.key"), "ab".repeat(32)).unwrap();
    std::fs::create_dir_all(b.join("vault")).unwrap();
    std::fs::write(b.join("vault").join("tin-2.bin"), b"extra").unwrap();
    assert!(prove_local(&b, "acme", &s.head(), &scratch, &Ctx::new()).is_err());
    std::fs::remove_file(b.join("vault").join("tin-2.bin")).unwrap();
    more(&mut s, 1);
    assert!(prove_local(&b, "acme", &s.head(), &scratch, &Ctx::new()).is_err());
    // Written again, it proves the books as they are now.
    write_local(&b, &cut(&b, &s), &Ctx::new()).unwrap();
    prove_local(&b, "acme", &s.head(), &scratch, &Ctx::new()).unwrap();
    assert!(!scratch.exists());
}

#[cfg(unix)]
#[test]
fn a_link_in_a_backup_is_never_followed() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    // The journal's piece replaced by a link to a file elsewhere with the
    // same bytes: it is no piece of the backup.
    let set = disk.join("acme");
    let piece = std::fs::read_dir(set.join("journal"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let elsewhere = dir.path().join("elsewhere");
    std::fs::copy(&piece, &elsewhere).unwrap();
    std::fs::remove_file(&piece).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &piece).unwrap();
    assert!(prove(&disk, None, dir.path(), "v").is_err());
}

#[test]
fn a_drill_stops_inside_the_phase_it_is_told_to_stop_in() {
    use ledger_store::backup::drill;
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    // The phases a drill goes through; a stop asked as each begins ends the
    // drill there (the observer sees no later phase), scratch removed.
    let phases = ["fetch", "replay", "verify", "blobs", "vault", "prefix"];
    for stop_at in phases {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let ctx = {
            let (stop, seen) = (stop.clone(), seen.clone());
            Ctx::new()
                .with_stop(stop.clone())
                .with_phase_watch(Arc::new(move |p: &str| {
                    seen.lock().unwrap().push(p.to_string());
                    if p == stop_at {
                        stop.store(true, Ordering::SeqCst);
                    }
                }))
        };
        let scratch = dir.path().join(format!("d-{stop_at}"));
        let got = drill(
            &plain(&disk),
            "acme",
            &b.join("journal.jsonl"),
            &scratch,
            &ctx,
        );
        assert!(
            matches!(got, Err(BackupError::Cancelled(_))),
            "{stop_at}: {got:?}"
        );
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.last().map(String::as_str), Some(stop_at), "{seen:?}");
        assert!(!scratch.exists(), "{stop_at}: scratch left");
    }
}
