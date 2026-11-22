//! Tests of the parts of a backup that need what the crate keeps to itself:
//! how a failed link is told from a disk that makes none, what a sync
//! service's stand-ins look like, the share of stored files each backup
//! re-reads, and a destination with thousands of backups.

use super::Ctx;
use super::cut::Cut;
use super::form::*;
use super::io::{dataless_flags, is_stand_in_name, link_unsupported, placeholder_attributes};
use super::read::{Set, in_rotation};
use super::write::{kept_form, write_into_set};
use std::io::Write;
use std::time::Instant;

#[test]
fn only_an_error_that_means_no_links_is_taken_to_mean_it() {
    use std::io::{Error, ErrorKind};
    assert!(link_unsupported(&Error::from(ErrorKind::Unsupported)));
    // The name being taken, a missing folder, a refused permission and a
    // full disk are what they say: never a reason to try a rename, which
    // replaces.
    for kind in [
        ErrorKind::AlreadyExists,
        ErrorKind::NotFound,
        ErrorKind::PermissionDenied,
        ErrorKind::StorageFull,
        ErrorKind::Other,
    ] {
        assert!(!link_unsupported(&Error::from(kind)), "{kind:?}");
    }
    #[cfg(unix)]
    {
        for n in [libc::EPERM, libc::ENOSYS, libc::EOPNOTSUPP, libc::ENOTSUP] {
            assert!(link_unsupported(&Error::from_raw_os_error(n)), "errno {n}");
        }
        for n in [
            libc::EACCES,
            libc::ENOENT,
            libc::EEXIST,
            libc::ENOSPC,
            libc::EIO,
        ] {
            assert!(!link_unsupported(&Error::from_raw_os_error(n)), "errno {n}");
        }
    }
}

#[test]
fn what_a_sync_service_leaves_for_a_file_it_has_not_downloaded_is_known() {
    // iCloud, before macOS 14: `.name.icloud` in the folder.
    assert!(is_stand_in_name(".000002.json.icloud"));
    assert!(is_stand_in_name(".a.icloud"));
    assert!(!is_stand_in_name("000002.json"));
    assert!(!is_stand_in_name(".wb-tmp-1-abc"));
    assert!(!is_stand_in_name(".icloud"));
    // Windows: offline (0x1000), recall on open (0x40000), recall on data
    // access (0x400000); a plain file (archive, 0x20) and a read-only one
    // (0x1) are not.
    for attrs in [0x1000, 0x40000, 0x400000, 0x20 | 0x400000] {
        assert!(placeholder_attributes(attrs), "{attrs:#x}");
    }
    for attrs in [0x20, 0x1, 0x80, 0x100] {
        assert!(!placeholder_attributes(attrs), "{attrs:#x}");
    }
    // macOS: SF_DATALESS.
    assert!(dataless_flags(0x4000_0000));
    assert!(!dataless_flags(0x0004_0000));
}

#[test]
fn a_piece_belongs_to_one_share_and_every_share_is_read_within_sixteen_backups() {
    use super::read::ROTATION;
    let mut x = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..2000 {
        // A pseudo-random hash.
        let mut sha = String::new();
        while sha.len() < 64 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            sha.push_str(&format!("{x:016x}"));
        }
        sha.truncate(64);
        for start in [1u64, 2, 17, 255, 3000] {
            let reads = (start..start + ROTATION)
                .filter(|n| in_rotation(&sha, *n))
                .count();
            assert_eq!(reads, 1, "{sha} from backup {start}");
        }
    }
}

#[test]
fn generations_and_set_names_round_trip() {
    assert_eq!(generation_path("files/ab/xyz", 0), "files/ab/xyz");
    assert_eq!(generation_path("files/ab/xyz", 2), "files/ab/xyz.g2");
    assert_eq!(split_generation("files/ab/xyz.g2"), ("files/ab/xyz", 2));
    assert_eq!(split_generation("files/ab/xyz"), ("files/ab/xyz", 0));
    assert_eq!(
        split_generation("journal/000001-abcd.g"),
        ("journal/000001-abcd.g", 0)
    );
    assert_eq!(plain_set_name("con", 1), "wanderbooks-con");
    assert_eq!(plain_set_name("acme", 3), "wanderbooks-acme.3");
    assert_eq!(parse_plain_set_name("wanderbooks-acme"), Some(("acme", 1)));
    assert_eq!(
        parse_plain_set_name("wanderbooks-acme.3"),
        Some(("acme", 3))
    );
    // No book id holds a dot, so a later set is never another book's.
    assert_eq!(parse_plain_set_name("wanderbooks-acme.1"), None);
    assert_eq!(parse_plain_set_name("acme"), None);
}

/// A `Set` over a fresh folder, nothing synced.
fn fresh_set(root: &std::path::Path) -> (Set, std::path::PathBuf) {
    let at = root.join("disk/wanderbooks-acme");
    (Set::dir(at.clone(), None, &Ctx::new().with_no_sync()), at)
}

/// What a person's backups look like after a while, made by the real
/// writer: every manifest names every segment once read, but is kept as
/// the one before it and what is new, and a chain is cut at 64.
#[test]
fn the_writers_manifests_chain_to_a_bounded_depth_and_are_proven_link_by_link() {
    const N: u64 = 150;
    let dir = tempfile::tempdir().unwrap();
    let book = dir.path().join("book");
    std::fs::create_dir(&book).unwrap();
    let ctx = Ctx::new().with_no_sync();
    let (set, at) = fresh_set(dir.path());
    // A journal of made-up lines: the writer never reads one, it hashes the
    // bytes and takes the head from the caller. Each backup adds a line.
    let mut journal = std::fs::File::create(book.join(crate::JOURNAL_FILE)).unwrap();
    for i in 1..=N {
        let line = format!("{{\"seq\":{i},\"pad\":\"{}\"}}", "x".repeat(60));
        writeln!(journal, "{line}").unwrap();
        journal.flush().unwrap();
        let cut = Cut::take("acme", &book, (i, sha_hex(line.as_bytes()))).unwrap();
        let w = write_into_set(&set, &cut, &ctx, false).unwrap();
        assert!(w.new && w.number == i);
    }
    let mut depths = Vec::new();
    for n in 1..=N {
        let m = set.manifest(n).unwrap();
        depths.push(m.depth);
        assert_eq!(m.journal.segments.len() as u64, n, "manifest {n}");
        let raw = std::fs::read_to_string(at.join(manifest_path(n, false))).unwrap();
        let stored: Manifest = serde_json::from_str(raw.lines().next().unwrap()).unwrap();
        assert_eq!(stored.depth == 0, stored.base.is_none());
        // A stored manifest that chains names only what is new.
        if stored.base.is_some() {
            assert_eq!(stored.journal.segments.len(), 1, "manifest {n}");
            assert_eq!(stored.journal.keep as u64, n - 1);
        }
    }
    // Chains of 64: a manifest that names everything at 1, 65, 129.
    assert_eq!(depths[0], 0);
    assert_eq!(depths[63], 63);
    assert_eq!(depths[64], 0);
    assert_eq!(depths[128], 0);
    assert!(depths.iter().all(|d| *d < DELTA_CHAIN));

    // Each link is proven by the hash its successor holds: a manifest in
    // the chain that was changed makes the ones built on it unreadable, and
    // the reader takes the newest that still reads, saying how many it
    // passed.
    let k = N - 5;
    let bad = at.join(manifest_path(k, false));
    let mut bytes = std::fs::read(&bad).unwrap();
    let i = bytes.len() / 3;
    bytes[i] = if bytes[i] == b'0' { b'1' } else { b'0' };
    std::fs::write(&bad, bytes).unwrap();
    assert!(matches!(
        set.manifest(N),
        Err(super::BackupError::Damaged(_))
    ));
    let (m, passed) = set.newest_whole().unwrap().unwrap();
    assert_eq!((m.number, passed as u64), (k - 1, N - k + 1));
    assert_eq!(m.journal.segments.len() as u64, k - 1);
}

/// A destination of thousands of backups, made cheaply: the real forms
/// (`kept_form`, `encode_manifest`) written straight into a folder with a
/// one-byte file for each segment, no books and no syncing, then read as
/// a restore reads it. The time and size are printed (the numbers the
/// docs quote) and bounded.
#[test]
fn a_destination_of_thousands_of_backups_stays_small_and_quick_to_read() {
    const N: u64 = 3000;
    let dir = tempfile::tempdir().unwrap();
    let (set, at) = fresh_set(dir.path());
    std::fs::create_dir_all(at.join("journal")).unwrap();
    std::fs::create_dir_all(at.join(MANIFESTS)).unwrap();
    let made = Instant::now();
    let mut full = Manifest {
        format: FORMAT.into(),
        version: VERSION,
        book: "acme".into(),
        number: 0,
        written: "2026-11-21T00:00:00Z".into(),
        head: Head {
            seq: 0,
            hash: String::new(),
        },
        journal: Series::default(),
        access: Series::default(),
        files: Series::default(),
        file_count: 0,
        file_bytes: 0,
        vault: Vec::new(),
        keys: Vec::new(),
        base: None,
        depth: 0,
        sha: String::new(),
    };
    let mut prev: Option<Manifest> = None;
    for i in 1..=N {
        let name = format!("journal/{i:06}-{i:016x}");
        std::fs::write(at.join(&name), b"x").unwrap();
        full.journal.segments.push(Piece {
            file: name,
            stored: 1,
            bytes: 1,
            sha256: format!("{i:064x}"),
        });
        full.journal.bytes = i;
        full.journal.sha256 = format!("{i:064x}");
        full.number = i;
        full.head = Head {
            seq: i,
            hash: format!("{i:064x}"),
        };
        let kept = kept_form(&full, prev.as_ref());
        let bytes = encode_manifest(&kept);
        std::fs::write(at.join(manifest_path(i, false)), &bytes).unwrap();
        let mut view = full.clone();
        view.depth = kept.depth;
        view.sha = decode_manifest(&bytes).unwrap().sha;
        prev = Some(view);
    }
    let made = made.elapsed();

    let sizes: Vec<u64> = (1..=N)
        .map(|n| {
            std::fs::metadata(at.join(manifest_path(n, false)))
                .unwrap()
                .len()
        })
        .collect();
    let total: u64 = sizes.iter().sum();
    let largest = *sizes.iter().max().unwrap();
    let newest = *sizes.last().unwrap();
    // What it would have been if every manifest named everything: a
    // journal segment is ~170 bytes of a manifest.
    let everything: u64 = (1..=N).map(|n| 170 * n).sum();

    // Read as a restore reads it: the newest found, and whole.
    let began = Instant::now();
    let (m, passed) = set.newest_whole().unwrap().unwrap();
    let found_in = began.elapsed();
    assert_eq!((m.number, passed), (N, 0));
    assert_eq!(m.journal.segments.len() as u64, N);
    let began = Instant::now();
    let numbers = set.numbers().unwrap().len();
    let listed_in = began.elapsed();
    assert_eq!(numbers as u64, N);
    println!(
        "{N} backups: manifests {total} bytes in all (named in full each time: ~{everything}), largest {largest}, newest {newest}; made in {made:?}; newest found and whole in {found_in:?}; listed in {listed_in:?}"
    );
    assert!(newest < 6 * 1024, "the newest manifest is {newest} bytes");
    assert!(total < everything / 20, "{total} of ~{everything}");
    assert!(found_in.as_secs() < 10, "{found_in:?}");
    // Any manifest in it reads whole (a sample: each read walks a chain).
    for n in (1..=N).step_by(131).chain([N - 1]) {
        let m = set.manifest(n).unwrap();
        assert_eq!(m.journal.segments.len() as u64, n);
    }
}
