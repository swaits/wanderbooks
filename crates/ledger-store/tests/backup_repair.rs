//! A backup that restores old or wrong books while reporting good is the
//! worst defect this product can have (WB-backup-12 to WB-backup-17): a
//! damaged piece must not be reused by later backups, the drill must not
//! prove an older backup and say ok, a damaged piece must be found by the
//! proof after the backup and written again beside the damaged file, books
//! that are behind a destination must be told from books that have
//! diverged, and a restore must say when it is not of the newest backup.

use ledger_core::{AccountType, Currency, Money, Role, Side, TinFacts, TinType};
use ledger_store::Store;
use ledger_store::backup::{
    BackupError, Ctx, Cut, Destination, Options, drill, find, verify_found, write_destination,
    write_destination_with,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

fn set_of(disk: &Path) -> PathBuf {
    disk.join("wanderbooks-acme")
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

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_tree(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

/// Flip one byte in the middle of a file, as a bad sector or a bad sync
/// would; the file keeps its length.
fn flip(path: &Path) {
    let mut bytes = std::fs::read(path).unwrap();
    let at = bytes.len() / 2;
    bytes[at] ^= 0x55;
    std::fs::write(path, bytes).unwrap();
}

/// The manifest `n` of the set, as JSON.
fn manifest(set: &Path, n: u64) -> serde_json::Value {
    let text = std::fs::read_to_string(set.join(format!("manifests/{n:06}.json"))).unwrap();
    serde_json::from_str(text.lines().next().unwrap()).unwrap()
}

/// The newest journal segment a manifest names (the one its backup added).
fn newest_journal_segment(set: &Path, n: u64) -> String {
    manifest(set, n)["journal"]["segments"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["file"]
        .as_str()
        .unwrap()
        .to_string()
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn one_damaged_segment_is_found_by_the_next_backup_and_the_drill_does_not_hide_it() {
    // The reviewer's sequence: a byte flips in the segment backup 2 wrote;
    // backups go on; a drill runs; a restore is made. Before: every later
    // backup reused the damaged segment and was recorded good, the drill
    // proved backup 1 and said ok, and a restore returned books from long
    // ago.
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let first_head = s.head();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    more(&mut s, 1);
    let w2 = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    assert_eq!(w2.number, 2);
    assert!(w2.repaired.is_empty());
    let seg = newest_journal_segment(&set_of(&disk), 2);
    let damaged = set_of(&disk).join(&seg);
    flip(&damaged);
    let flipped = std::fs::read(&damaged).unwrap();

    // The very next backup finds it (its proof re-hashes every journal
    // segment its manifest names) and writes the piece again beside it.
    more(&mut s, 2);
    let w3 = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    assert!(w3.new);
    assert_eq!(w3.repaired, vec![seg.clone()], "{w3:?}");
    assert_eq!(
        std::fs::read(&damaged).unwrap(),
        flipped,
        "the damaged file is left exactly where it is"
    );
    let again = set_of(&disk).join(format!("{seg}.g1"));
    assert!(
        again.is_file(),
        "the piece is written again as generation 1"
    );
    assert_eq!(sha(&std::fs::read(&again).unwrap()), {
        let m = manifest(&set_of(&disk), 2);
        m["journal"]["segments"].as_array().unwrap().last().unwrap()["sha256"]
            .as_str()
            .unwrap()
            .to_string()
    });

    // Many more backups: none reuses the damaged file, none finds more.
    for n in 3..8 {
        more(&mut s, n);
        let w = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
        assert!(w.new && w.repaired.is_empty(), "{n}: {w:?}");
    }
    let last = s.head();
    // The drill proves the newest backup, which reaches the live head.
    let got = drill(
        &plain(&disk),
        "acme",
        &b.join("journal.jsonl"),
        &dir.path().join("drill"),
        Some(&last),
        &Ctx::new(),
    )
    .unwrap();
    assert_eq!(got, last);
    // A restore gives the newest books, not backup 1's.
    let found = find(&disk, None, &Ctx::new()).unwrap();
    let v = verify_found(&found, &dir.path().join("restore"), &Ctx::new()).unwrap();
    assert_eq!(v.head, last);
    assert_ne!(v.head, first_head);
    assert_eq!(v.passed_over, 0);
    // Manifests 1, 2, 3, the repair's (4) and the five after it.
    assert_eq!(v.number, 9);
}

#[test]
fn a_drill_never_passes_on_an_older_backup() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    more(&mut s, 1);
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    let second_head = s.head();
    more(&mut s, 2);
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    let newest_head = s.head();
    // The newest backup's own segment is damaged after it was made, and no
    // backup comes after it to find that.
    flip(&set_of(&disk).join(newest_journal_segment(&set_of(&disk), 3)));

    // The drill is a failure that says so, with the date of the older one.
    let err = drill(
        &plain(&disk),
        "acme",
        &b.join("journal.jsonl"),
        &dir.path().join("drill"),
        Some(&newest_head),
        &Ctx::new(),
    )
    .unwrap_err();
    let words = err.to_string();
    match &err {
        BackupError::NewestDamaged {
            number,
            older_from,
            newest_from,
        } => {
            assert_eq!(*number, 3);
            assert!(older_from.is_some() && newest_from.is_some(), "{err:?}");
        }
        other => panic!("{other:?}"),
    }
    assert!(words.contains("the newest backup (number 3"), "{words}");
    assert!(words.contains("is damaged; an older one from "), "{words}");
    assert!(!dir.path().join("drill").exists());

    // A restore passes over it, and the preview can say so: both dates and
    // how many it passes over.
    let found = find(&disk, None, &Ctx::new()).unwrap();
    let v = verify_found(&found, &dir.path().join("restore"), &Ctx::new()).unwrap();
    assert_eq!(v.number, 2);
    assert_eq!(v.head, second_head);
    assert_eq!(v.passed_over, 1);
    assert!(v.newest_written.is_some() && !v.written.is_empty());

    // Everything damaged: no older one proves whole either, and it says so.
    for n in [1u64, 2] {
        flip(&set_of(&disk).join(newest_journal_segment(&set_of(&disk), n)));
    }
    let err = drill(
        &plain(&disk),
        "acme",
        &b.join("journal.jsonl"),
        &dir.path().join("drill2"),
        None,
        &Ctx::new(),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("no older one proves whole either"),
        "{err}"
    );
}

#[test]
fn a_drill_needs_the_newest_backup_to_reach_the_last_one_recorded_good() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    let have = s.head();
    more(&mut s, 1);
    let recorded = s.head();
    // A backup was recorded good through `recorded`, but the folder's
    // newest stops at `have` (a manifest lost to a sync conflict, say).
    let err = drill(
        &plain(&disk),
        "acme",
        &b.join("journal.jsonl"),
        &dir.path().join("d"),
        Some(&recorded),
        &Ctx::new(),
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("a backup that was made is not there"),
        "{err}"
    );
    // The same head with another hash is no better.
    let other = (have.0, "0".repeat(64));
    assert!(
        drill(
            &plain(&disk),
            "acme",
            &b.join("journal.jsonl"),
            &dir.path().join("d"),
            Some(&other),
            &Ctx::new(),
        )
        .is_err()
    );
    // And it passes when it reaches what was recorded.
    assert_eq!(
        drill(
            &plain(&disk),
            "acme",
            &b.join("journal.jsonl"),
            &dir.path().join("d"),
            Some(&have),
            &Ctx::new(),
        )
        .unwrap(),
        have
    );
}

#[test]
fn a_damaged_stored_file_is_found_by_the_rotation_and_the_next_backup_repairs_it() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    // Stored files, until one is in the share backup number 2 re-reads
    // (the share a file belongs to is its hash's first four digits modulo
    // 16: every stored file is re-read within 16 backups).
    let mut chosen: Option<(String, Vec<u8>)> = None;
    for n in 0..400 {
        let bytes = format!("a stored file, number {n}").into_bytes();
        s.attach_receipt(None, &format!("s{n}.txt"), "text/plain", &bytes, None)
            .unwrap();
        let h = sha(&bytes);
        if chosen.is_none() && u64::from_str_radix(&h[..4], 16).unwrap() % 16 == 2 {
            chosen = Some((h, bytes));
        }
        if chosen.is_some() && n > 5 {
            break;
        }
    }
    let (h, bytes) = chosen.expect("some file is in the share");
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    let rel = format!("files/{}/{h}", &h[..2]);
    let piece = set_of(&disk).join(&rel);
    flip(&piece);
    let flipped = std::fs::read(&piece).unwrap();
    more(&mut s, 1);
    let w = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    assert_eq!(w.number, 2);
    assert_eq!(w.repaired, vec![rel.clone()], "{w:?}");
    assert_eq!(std::fs::read(&piece).unwrap(), flipped, "left where it is");
    let again = set_of(&disk).join(format!("{rel}.g1"));
    assert_eq!(std::fs::read(&again).unwrap(), bytes);
    // Every manifest of the set restores with the file whole (the reader
    // takes the generation that hashes right).
    let found = find(&disk, None, &Ctx::new()).unwrap();
    let v = verify_found(&found, &dir.path().join("v"), &Ctx::new()).unwrap();
    assert_eq!(v.passed_over, 0);
    assert_eq!(v.head, s.head());
    let restored = v.tree.join("blobs").join(&h[..2]).join(&h[2..4]).join(&h);
    assert_eq!(std::fs::read(restored).unwrap(), bytes);
    // And the next backup carries on without finding it again.
    more(&mut s, 2);
    let w = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    assert!(w.new && w.repaired.is_empty(), "{w:?}");
}

#[test]
fn a_piece_damaged_as_it_is_written_is_repaired_in_the_same_backup() {
    // A piece that is there under its name but is not what its name says
    // (a copy that died half way on a disk that takes no links, a bad
    // sector) is taken as the piece by a put; the proof finds it.
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let set = set_of(&disk);
    let flipped_once = Arc::new(Mutex::new(false));
    let ctx = {
        let (set, done) = (set.clone(), flipped_once.clone());
        Ctx::new().with_faults(Arc::new(move |kind, rel| {
            if kind == "published" && rel.starts_with("files/") {
                let mut d = done.lock().unwrap();
                if !*d {
                    flip(&set.join(rel));
                    *d = true;
                }
            }
            false
        }))
    };
    let w = write_destination(&plain(&disk), &cut(&b, &s), true, &ctx).unwrap();
    assert!(w.new);
    assert_eq!(w.repaired.len(), 1, "{w:?}");
    assert!(set.join(format!("{}.g1", w.repaired[0])).is_file());
    let found = find(&disk, None, &Ctx::new()).unwrap();
    let v = verify_found(&found, &dir.path().join("v"), &Ctx::new()).unwrap();
    assert_eq!(v.head, s.head());
}

#[test]
fn a_damaged_vault_record_and_an_index_segment_are_written_again_too() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let vault_bytes = b"a sealed vault record".to_vec();
    std::fs::create_dir_all(b.join("vault")).unwrap();
    std::fs::write(b.join("vault").join("tin-1.bin"), &vault_bytes).unwrap();
    std::fs::create_dir_all(b.join("keys")).unwrap();
    std::fs::write(b.join("keys").join("legacy.key"), "ab".repeat(32)).unwrap();
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    let set = set_of(&disk);
    flip(&set.join(format!("vault/{}", sha(&vault_bytes))));
    let index = manifest(&set, 1)["files"]["segments"][0]["file"]
        .as_str()
        .unwrap()
        .to_string();
    flip(&set.join(&index));
    more(&mut s, 1);
    let w = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    assert!(w.new, "{w:?}");
    assert_eq!(w.repaired.len(), 2, "{w:?}");
    // The newest manifest names the new generations; the restore proves.
    let found = find(&disk, None, &Ctx::new()).unwrap();
    let v = verify_found(&found, &dir.path().join("v"), &Ctx::new()).unwrap();
    assert_eq!(v.head, s.head());
    assert_eq!(v.passed_over, 0);
    assert_eq!(
        std::fs::read(v.tree.join("vault").join("tin-1.bin")).unwrap(),
        vault_bytes
    );
}

#[test]
fn books_behind_the_destination_are_ahead_and_other_books_are_diverged() {
    let dir = tempfile::tempdir().unwrap();
    let (b, old, other) = (
        dir.path().join("book"),
        dir.path().join("old"),
        dir.path().join("other"),
    );
    let mut s = book(&b);
    // The books as they were: a copy of the folder at this point.
    copy_tree(&b, &old);
    more(&mut s, 1);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    let before = tree(&disk);

    // Restored from an older backup: the live journal is the beginning of
    // the destination's. That is ahead, and the words say what to do.
    let old_store = Store::open(&old).unwrap();
    let err = write_destination(
        &plain(&disk),
        &Cut::take("acme", &old, old_store.head()).unwrap(),
        true,
        &Ctx::new(),
    )
    .unwrap_err();
    assert!(matches!(err, BackupError::Ahead(..)), "{err:?}");
    let words = err.to_string();
    assert!(words.contains("a longer history of these books"), "{words}");
    assert!(words.contains("start a new backup set"), "{words}");
    assert_eq!(tree(&disk), before, "nothing was written");

    // Another history under the same id is diverged, said as such (not
    // ahead, whichever is longer).
    let other_store = book(&other);
    let err = write_destination(
        &plain(&disk),
        &Cut::take("acme", &other, other_store.head()).unwrap(),
        true,
        &Ctx::new(),
    )
    .unwrap_err();
    assert_eq!(err, BackupError::Elsewhere);
    assert!(err.to_string().contains("diverged"), "{err}");
    let mut longer = other_store;
    for n in 0..4 {
        more(&mut longer, n);
    }
    let err = write_destination(
        &plain(&disk),
        &Cut::take("acme", &other, longer.head()).unwrap(),
        true,
        &Ctx::new(),
    )
    .unwrap_err();
    assert_eq!(err, BackupError::Elsewhere);
    assert_eq!(tree(&disk), before);
}

#[test]
fn a_new_set_begins_beside_the_old_at_the_persons_request_and_the_old_is_never_touched() {
    let dir = tempfile::tempdir().unwrap();
    let (b, old) = (dir.path().join("book"), dir.path().join("old"));
    let mut s = book(&b);
    copy_tree(&b, &old);
    more(&mut s, 1);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    let before = tree(&disk);
    let mut old_store = Store::open(&old).unwrap();
    let old_cut = Cut::take("acme", &old, old_store.head()).unwrap();
    assert!(matches!(
        write_destination(&plain(&disk), &old_cut, true, &Ctx::new()),
        Err(BackupError::Ahead(..))
    ));

    // At the person's request: a new set, `.2`, beside it.
    let w = write_destination_with(
        &plain(&disk),
        &old_cut,
        Options {
            may_create: true,
            new_set: true,
        },
        &Ctx::new(),
    )
    .unwrap();
    assert_eq!((w.number, w.new), (1, true));
    assert!(
        disk.join("wanderbooks-acme.2/manifests/000001.json")
            .is_file()
    );
    let after = tree(&disk);
    for (name, bytes) in &before {
        assert_eq!(
            after.get(name),
            Some(bytes),
            "{name}: the old set is untouched"
        );
    }

    // The next ordinary backup goes to the new set, which grows; the old
    // one stays as it was.
    more(&mut old_store, 9);
    let w = write_destination(
        &plain(&disk),
        &Cut::take("acme", &old, old_store.head()).unwrap(),
        true,
        &Ctx::new(),
    )
    .unwrap();
    assert_eq!(w.number, 2);
    assert!(
        disk.join("wanderbooks-acme.2/manifests/000002.json")
            .is_file()
    );
    let after = tree(&disk);
    for (name, bytes) in &before {
        assert_eq!(after.get(name), Some(bytes), "{name}");
    }
    // A drill takes the newest set too.
    assert_eq!(
        drill(
            &plain(&disk),
            "acme",
            &old.join("journal.jsonl"),
            &dir.path().join("d"),
            Some(&old_store.head()),
            &Ctx::new(),
        )
        .unwrap(),
        old_store.head()
    );
    // The folder now holds two sets of the same books: a restore from it
    // asks which (the sets are listed with their dates), never guesses.
    assert!(matches!(
        find(&disk, None, &Ctx::new()),
        Err(BackupError::Several(_))
    ));
    let listed = ledger_store::backup::list(&disk);
    assert_eq!(listed.len(), 2);
    assert!(
        listed
            .iter()
            .all(|c| c.through_seq.is_some() && c.written.is_some())
    );
}

#[test]
fn a_set_folder_is_a_legal_name_whatever_the_books_are_called() {
    let dir = tempfile::tempdir().unwrap();
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    for id in ["con", "nul", "aux", "prn", "com1", "lpt9"] {
        let b = dir.path().join(format!("book-{id}"));
        let s = book(&b);
        write_destination(
            &plain(&disk),
            &Cut::take(id, &b, s.head()).unwrap(),
            true,
            &Ctx::new(),
        )
        .unwrap();
        assert!(
            disk.join(format!("wanderbooks-{id}/manifests/000001.json"))
                .is_file(),
            "{id}"
        );
        assert!(!disk.join(id).exists(), "{id}: no reserved name is made");
    }
}

#[test]
fn only_the_newest_backup_is_tried_in_the_good_case() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    for n in 0..6 {
        more(&mut s, n);
        write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    }
    // Each backup added one segment, and each manifest names only its own.
    let segments: usize = (1..=6)
        .map(|n| {
            manifest(&set_of(&disk), n)["journal"]["segments"]
                .as_array()
                .unwrap()
                .len()
        })
        .sum();
    assert_eq!(segments, 6);
    let probes = Arc::new(Mutex::new(0usize));
    let ctx = {
        let probes = probes.clone();
        Ctx::new().with_trace(Arc::new(move |kind, rel| {
            if kind == "len" && rel.starts_with("journal/") {
                *probes.lock().unwrap() += 1;
            }
        }))
    };
    let found = find(&disk, None, &ctx).unwrap();
    let v = verify_found(&found, &dir.path().join("v"), &ctx).unwrap();
    assert_eq!(v.number, 6);
    // One look at each journal segment of the newest manifest to find the
    // set's book, one to prove it: never the segments of the five older
    // manifests, which reading every manifest first would have measured
    // too (1 + 2 + ... + 6 more).
    assert_eq!(*probes.lock().unwrap(), 2 * segments);
}

#[test]
fn a_backup_cut_while_the_vault_is_being_written_restores_a_vault_that_matches_the_journal() {
    // ledger-vault stages a record as `NAME.staged-TAG` and sets the old one
    // aside as `NAME.purging-TAG` while a call is between its first step and
    // its last. A backup taken then must hold the vault as the journal says
    // it is: the staged record is not the books' yet, and the set-aside one
    // still is.
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let mut s = book(&b);
    let record = b"sealed tin record, party one".to_vec();
    let party = s.add_party("Mo", &[Role::Member]).unwrap();
    s.set_party_tin(
        party,
        Some(TinFacts {
            tin_type: TinType::Ssn,
            last4: "4915".into(),
            vault_record: Some(sha(&record)),
            purged: None,
        }),
    )
    .unwrap();
    std::fs::create_dir_all(b.join("vault")).unwrap();
    std::fs::create_dir_all(b.join("keys")).unwrap();
    std::fs::write(b.join("keys").join("legacy.key"), "ab".repeat(32)).unwrap();
    // The purge of party 1's record is staged: set aside, not yet journaled.
    let tag = "0123456789abcdef";
    std::fs::write(
        b.join("vault").join(format!("tin-1.purging-{tag}")),
        &record,
    )
    .unwrap();
    // And a new record for party 2 is staged, not the books'.
    std::fs::write(
        b.join("vault").join(format!("tin-2.staged-{tag}")),
        b"staged, not committed",
    )
    .unwrap();
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let w = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    assert!(w.new && w.repaired.is_empty(), "{w:?}");
    let found = find(&disk, None, &Ctx::new()).unwrap();
    let v = verify_found(&found, &dir.path().join("v"), &Ctx::new()).unwrap();
    let held: Vec<String> = std::fs::read_dir(v.tree.join("vault"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(held, vec!["tin-1.bin".to_string()], "{held:?}");
    assert_eq!(
        std::fs::read(v.tree.join("vault").join("tin-1.bin")).unwrap(),
        record
    );
    // No scratch name of the vault is anywhere in the destination.
    assert!(
        !tree(&disk)
            .keys()
            .any(|n| n.contains(".staged-") || n.contains(".purging-"))
    );
}

#[test]
fn a_proof_cut_short_by_a_close_ends_cleanly_and_is_made_again_by_the_next_backup() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("book");
    let s = book(&b);
    let disk = dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    // The books close while the backup's proof is under way (here: as it
    // replays the chain).
    let stop = Arc::new(AtomicBool::new(false));
    let ctx = {
        let stop = stop.clone();
        Ctx::new()
            .with_stop(stop.clone())
            .with_phase_watch(Arc::new(move |p: &str| {
                if p == "replay" {
                    stop.store(true, Ordering::SeqCst);
                }
            }))
    };
    let err = write_destination(&plain(&disk), &cut(&b, &s), true, &ctx).unwrap_err();
    assert!(matches!(err, BackupError::Cancelled(_)), "{err:?}");
    // Nothing of the proof is left behind in the books' folder, and the
    // manifest that was written is there but is not marked proven.
    let left: Vec<_> = std::fs::read_dir(b.join("ops"))
        .map(|d| {
            d.flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("proof-"))
                .collect()
        })
        .unwrap_or_default();
    assert!(left.is_empty(), "scratch books left: {left:?}");
    let set = set_of(&disk);
    assert!(set.join("manifests/000001.json").is_file());
    assert!(!set.join("proofs/000001").exists());
    // The next backup has nothing new to write, and still proves it: it
    // never says good of a backup that was not proven.
    let w = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    assert_eq!(w.number, 1);
    assert!(!w.new, "{w:?}");
    assert!(set.join("proofs/000001").is_file());
    // Once proven, a backup with nothing new is only looked at.
    let before = tree(&disk);
    let w = write_destination(&plain(&disk), &cut(&b, &s), true, &Ctx::new()).unwrap();
    assert!(!w.new);
    assert_eq!(tree(&disk), before);
}
