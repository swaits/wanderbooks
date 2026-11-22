//! Backups as properties (WB-backup-1 to WB-backup-7): random books and
//! random sequences of backups, interleavings of two writers, damage to any
//! piece, and passphrases that are not the right one. Each case builds a
//! book, which is the cost; the cases are few and the oracles are the laws.

use ledger_core::{AccountType, Currency};
use ledger_store::Store;
use ledger_store::backup::{
    BackupError, BackupKey, Ctx, Cut, Destination, find, verify_found, write_destination,
};
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// One thing that happens to the books between backups.
#[derive(Clone, Debug)]
enum Op {
    /// A new account.
    Account,
    /// A stored file of these bytes (the same bytes twice are one file).
    File(Vec<u8>),
    /// A backup.
    Backup,
}

fn ops() -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(
        prop_oneof![
            2 => Just(Op::Account),
            2 => prop::collection::vec(any::<u8>(), 1..40).prop_map(Op::File),
            3 => Just(Op::Backup),
        ],
        2..9,
    )
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

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for (rel, bytes) in tree(from) {
        let p = to.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
}

/// A book with one account, and a way to apply [`Op`]s to it.
struct Books {
    store: Store,
    dir: std::path::PathBuf,
    n: usize,
}

impl Books {
    fn new(dir: &Path) -> Self {
        let mut store = Store::create(dir, Currency::USD).unwrap();
        store
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        Self {
            store,
            dir: dir.to_path_buf(),
            n: 0,
        }
    }

    fn apply(&mut self, op: &Op) {
        match op {
            Op::Account => {
                self.n += 1;
                self.store
                    .add_account(
                        &format!("{}", 6000 + self.n),
                        &format!("Expense {}", self.n),
                        AccountType::Expense,
                        None,
                        false,
                    )
                    .unwrap();
            }
            Op::File(bytes) => {
                self.store
                    .attach_receipt(None, "f.bin", "application/octet-stream", bytes, None)
                    .unwrap();
            }
            Op::Backup => {}
        }
    }

    fn cut(&self) -> Cut {
        Cut::take("acme", &self.dir, self.store.head()).unwrap()
    }
}

fn prove(
    folder: &Path,
    pass: Option<&str>,
    scratch: &Path,
) -> Result<ledger_store::backup::Verified, BackupError> {
    let found = find(folder, pass, &Ctx::new())?;
    verify_found(&found, scratch, &Ctx::new())
}

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 5,
        max_shrink_iters: 4,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    /// WB-backup-1: whatever the books do and whenever they are backed up,
    /// every file the destination held is still there, byte for byte.
    #[test]
    fn a_destination_only_grows(ops in ops()) {
        let dir = tempfile::tempdir().unwrap();
        let mut books = Books::new(&dir.path().join("book"));
        let disk = dir.path().join("disk");
        std::fs::create_dir(&disk).unwrap();
        let mut before = tree(&disk);
        for op in &ops {
            books.apply(op);
            if matches!(op, Op::Backup) {
                write_destination(&plain(&disk), &books.cut(), true, &Ctx::new()).unwrap();
                let after = tree(&disk);
                for (name, bytes) in &before {
                    prop_assert_eq!(after.get(name), Some(bytes), "{} changed or went", name);
                }
                prop_assert!(!after.keys().any(|n| n.contains(".wb-tmp-")));
                before = after;
            }
        }
    }

    /// WB-backup-3: the destination as of any of its manifests restores to
    /// books whose journal is byte-identical to the beginning of the
    /// books' journal now, and whose head is the one that manifest names.
    #[test]
    fn any_whole_manifest_restores_a_prefix_of_the_books(ops in ops()) {
        let dir = tempfile::tempdir().unwrap();
        let mut books = Books::new(&dir.path().join("book"));
        let disk = dir.path().join("disk");
        std::fs::create_dir(&disk).unwrap();
        let mut heads = Vec::new();
        for op in &ops {
            books.apply(op);
            if matches!(op, Op::Backup) {
                let w = write_destination(&plain(&disk), &books.cut(), true, &Ctx::new()).unwrap();
                if w.new {
                    heads.push((w.number, w.head));
                }
            }
        }
        // (A last backup, so there is at least one.)
        let w = write_destination(&plain(&disk), &books.cut(), true, &Ctx::new()).unwrap();
        if w.new {
            heads.push((w.number, w.head));
        }
        let live = std::fs::read(dir.path().join("book").join("journal.jsonl")).unwrap();
        for (number, head) in &heads {
            // The destination as it was when that manifest was the newest.
            let then = dir.path().join(format!("then-{number}"));
            copy_tree(&disk, &then);
            for (name, _) in tree(&then) {
                if let Some(n) = name
                    .strip_prefix("acme/manifests/")
                    .and_then(|r| r.strip_suffix(".json"))
                    .and_then(|r| r.parse::<u64>().ok())
                    && n > *number
                {
                    std::fs::remove_file(then.join(&name)).unwrap();
                }
            }
            let v = prove(&then, None, &dir.path().join(format!("v-{number}"))).unwrap();
            prop_assert_eq!(&v.head, head);
            let restored = std::fs::read(v.tree.join("journal.jsonl")).unwrap();
            prop_assert!(live.starts_with(&restored));
        }
    }

    /// WB-backup-5: a writer that finds another writer's backup appear at
    /// any of its write points (the other with books a little behind or a
    /// little ahead) leaves a destination that proves whole, at the head
    /// of one of them, restoring to a prefix of the longer books.
    #[test]
    fn random_interleavings_of_two_writers_leave_a_whole_backup(
        files in prop::collection::vec(prop::collection::vec(any::<u8>(), 1..20), 1..3),
        rival_after in 0usize..3,
        at in 0usize..14,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut books = Books::new(&dir.path().join("book"));
        for f in &files {
            books.apply(&Op::File(f.clone()));
        }
        let first = books.cut();
        for _ in 0..rival_after {
            books.apply(&Op::Account);
        }
        let second = books.cut();
        let disk = dir.path().join("disk");
        std::fs::create_dir(&disk).unwrap();
        // The writer of `first` is met, at its `at`th write point, by a
        // complete write of `second`.
        let count = Arc::new(AtomicUsize::new(0));
        let rival = {
            let (disk, second, count) = (disk.clone(), second.clone(), count.clone());
            Ctx::new().with_faults(Arc::new(move |_, _| {
                if count.fetch_add(1, Ordering::SeqCst) == at {
                    write_destination(&plain(&disk), &second, true, &Ctx::new()).unwrap();
                }
                false
            }))
        };
        let mine = write_destination(&plain(&disk), &first, true, &rival);
        match mine {
            Ok(_) | Err(BackupError::Ahead(..)) | Err(BackupError::Elsewhere) => {}
            Err(e) => prop_assert!(false, "{e:?}"),
        }
        let v = prove(&disk, None, &dir.path().join("v")).unwrap();
        prop_assert!(v.head == *first.head() || v.head == *second.head());
        let live = std::fs::read(dir.path().join("book").join("journal.jsonl")).unwrap();
        prop_assert!(live.starts_with(&std::fs::read(v.tree.join("journal.jsonl")).unwrap()));
    }

    /// WB-backup-6: a piece with one byte flipped, or cut anywhere, is
    /// found: the backup that names it is not restored.
    #[test]
    fn any_flipped_or_cut_piece_is_found_never_restored(
        ops in ops(),
        pick in any::<prop::sample::Index>(),
        at in any::<prop::sample::Index>(),
        cut in any::<bool>(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut books = Books::new(&dir.path().join("book"));
        books.apply(&Op::File(vec![1, 2, 3, 4]));
        let disk = dir.path().join("disk");
        std::fs::create_dir(&disk).unwrap();
        for op in &ops {
            books.apply(op);
        }
        write_destination(&plain(&disk), &books.cut(), true, &Ctx::new()).unwrap();
        let set = disk.join("acme");
        let pieces: Vec<String> = tree(&set)
            .into_keys()
            .filter(|n| !n.starts_with("manifests/") && n != "READ-ME.txt")
            .collect();
        let name = &pieces[pick.index(pieces.len())];
        let path = set.join(name);
        let mut bytes = std::fs::read(&path).unwrap();
        prop_assume!(!bytes.is_empty());
        let i = at.index(bytes.len());
        if cut {
            bytes.truncate(i);
        } else {
            bytes[i] ^= 0x01;
        }
        std::fs::write(&path, &bytes).unwrap();
        let scratch = dir.path().join("v");
        prop_assert!(prove(&disk, None, &scratch).is_err(), "{name} damaged was restored");
        prop_assert!(!scratch.exists());
    }

    /// WB-backup-7: no passphrase but the right one gives books.
    #[test]
    fn a_wrong_passphrase_never_yields_books(wrong in "[a-zA-Z0-9 ]{1,30}") {
        prop_assume!(wrong != "the right words, exactly");
        let dir = tempfile::tempdir().unwrap();
        let books = Books::new(&dir.path().join("book"));
        let disk = dir.path().join("disk");
        std::fs::create_dir(&disk).unwrap();
        let dest = Destination {
            folder: disk.clone(),
            key: Some(BackupKey::generate("the right words, exactly").unwrap()),
        };
        write_destination(&dest, &books.cut(), true, &Ctx::new()).unwrap();
        let scratch = dir.path().join("v");
        let got = prove(&disk, Some(&wrong), &scratch);
        prop_assert!(matches!(got, Err(BackupError::WrongPassphrase)), "{got:?}");
        prop_assert!(!scratch.exists());
        let none = prove(&disk, None, &scratch);
        prop_assert!(matches!(none, Err(BackupError::NeedsPassphrase)));
    }
}
