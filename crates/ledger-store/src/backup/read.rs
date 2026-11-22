//! Reading a backup: finding the set, the newest manifest that is whole,
//! the proof (INVARIANT 3), and the books built from it in a scratch
//! folder, for a restore, a drill and the proof before a format upgrade.

use super::form::*;
use super::io::{DirSet, SetIo, carry, uncarry};
use super::seal::{BackupKey, Header, Keys, manifest_aad, piece_aad};
use super::{BackupError, Ctx, Destination};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The most a manifest or header may be before it is refused as no such
/// file: nothing this program writes comes near it.
const SMALL_MAX: u64 = 256 << 20;

/// A set of backups of one set of books, open for reading.
pub(crate) struct Set {
    pub(crate) io: Box<dyn SetIo>,
    pub(crate) keys: Option<Keys>,
    pub(crate) ctx: Ctx,
}

/// A reader that looks for the stop before every read and hashes what it
/// gives.
pub(crate) struct Hashed<'a, R: Read> {
    inner: R,
    ctx: &'a Ctx,
    pub(crate) hasher: Sha256,
    pub(crate) count: u64,
}

impl<'a, R: Read> Hashed<'a, R> {
    pub(crate) fn new(inner: R, ctx: &'a Ctx) -> Self {
        Self {
            inner,
            ctx,
            hasher: Sha256::new(),
            count: 0,
        }
    }
}

impl<R: Read> Read for Hashed<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.ctx.check("backing up").map_err(carry)?;
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.count += n as u64;
        Ok(n)
    }
}

impl Set {
    pub(crate) fn dir(path: PathBuf, keys: Option<Keys>, ctx: &Ctx) -> Set {
        Set {
            io: Box::new(DirSet::new(path, ctx.clone())),
            keys,
            ctx: ctx.clone(),
        }
    }

    pub(crate) fn sealed(&self) -> bool {
        self.keys.is_some()
    }

    /// The numbers of the manifests there, ascending.
    pub(crate) fn numbers(&self) -> Result<Vec<u64>, BackupError> {
        let mut n: Vec<u64> = self
            .io
            .list(MANIFESTS)?
            .iter()
            .filter_map(|name| manifest_number(name, self.sealed()))
            .collect();
        n.sort_unstable();
        Ok(n)
    }

    fn read_small(&self, rel: &str) -> Result<Vec<u8>, BackupError> {
        match self.io.len(rel)? {
            Some(len) if len <= SMALL_MAX => {}
            Some(_) => {
                return Err(BackupError::Damaged(format!(
                    "{rel} is not a file of a backup"
                )));
            }
            None => return Err(BackupError::Damaged(format!("{rel} is missing"))),
        }
        let mut out = Vec::new();
        self.io.read_with(rel, &mut |r| {
            r.take(SMALL_MAX).read_to_end(&mut out).map(|_| ())
        })?;
        Ok(out)
    }

    /// The manifest numbered `n`: read, opened, checked.
    pub(crate) fn manifest(&self, n: u64) -> Result<Manifest, BackupError> {
        let raw = self.read_small(&manifest_path(n, self.sealed()))?;
        let bytes = match &self.keys {
            Some(k) => k.open_bytes(&raw, &manifest_aad(n))?,
            None => raw,
        };
        let m = decode_manifest(&bytes)?;
        if m.number != n {
            return Err(BackupError::Damaged(
                "a manifest of the backup is under another number".into(),
            ));
        }
        m.check_names(self.sealed())?;
        Ok(m)
    }

    /// Whether every piece `m` names, apart from the stored files, is
    /// there at the length it names: the cheap look that decides which
    /// manifest is the newest whole one. The proof re-hashes.
    fn looks_whole(&self, m: &Manifest) -> Result<bool, BackupError> {
        let pieces = m
            .journal
            .segments
            .iter()
            .chain(&m.access.segments)
            .chain(&m.files.segments)
            .chain(m.vault.iter().map(|n| &n.piece))
            .chain(m.keys.iter().map(|n| &n.piece));
        for p in pieces {
            if self.io.len(&p.file)? != Some(p.stored) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The newest manifest that is whole to the eye (INVARIANT 3), and
    /// how many newer ones were passed over.
    pub(crate) fn newest_whole(&self) -> Result<Option<(Manifest, usize)>, BackupError> {
        let numbers = self.numbers()?;
        for (skipped, n) in numbers.iter().rev().enumerate() {
            self.ctx.check("reading the backup")?;
            if let Ok(m) = self.manifest(*n)
                && self.looks_whole(&m)?
            {
                return Ok(Some((m, skipped)));
            }
        }
        Ok(None)
    }

    /// Every manifest that is whole to the eye, newest first.
    pub(crate) fn whole_ones(&self) -> Result<Vec<Manifest>, BackupError> {
        let mut out = Vec::new();
        for n in self.numbers()?.iter().rev() {
            self.ctx.check("reading the backup")?;
            if let Ok(m) = self.manifest(*n)
                && self.looks_whole(&m)?
            {
                out.push(m);
            }
        }
        Ok(out)
    }

    /// The contents of the piece at `rel` (of `kind`, `bytes` long, hashing
    /// to `sha`) handed to `sink`, proven as they go: a piece that is not
    /// what its manifest says is an error. A sealed piece is opened first.
    pub(crate) fn read_piece(
        &self,
        rel: &str,
        kind: Kind,
        bytes: u64,
        sha: &str,
        sink: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
    ) -> Result<(), BackupError> {
        let damaged =
            || BackupError::Damaged(format!("a piece of the backup ({}) is damaged", short(sha)));
        let mut hasher = Sha256::new();
        let mut count = 0u64;
        let mut feed = |chunk: &[u8]| -> std::io::Result<()> {
            hasher.update(chunk);
            count += chunk.len() as u64;
            sink(chunk)
        };
        let ctx = self.ctx.clone();
        let opened: Result<(), BackupError> = match &self.keys {
            None => {
                let mut res: Result<(), BackupError> = Ok(());
                let io_res = self.io.read_with(rel, &mut |r| {
                    let mut r = r.take(bytes.saturating_add(1));
                    let mut buf = vec![0u8; 1 << 16];
                    loop {
                        ctx.check("reading the backup").map_err(carry)?;
                        let n = r.read(&mut buf)?;
                        if n == 0 {
                            return Ok(());
                        }
                        feed(&buf[..n])?;
                    }
                });
                if let Err(e) = io_res {
                    res = Err(uncarry(e));
                }
                res
            }
            Some(keys) => {
                let mut res: Result<(), BackupError> = Ok(());
                let aad = piece_aad(kind.dir(), sha);
                let limit = Keys::sealed_len(bytes).saturating_add(1);
                let io_res = self.io.read_with(rel, &mut |r| {
                    res = keys.open_stream(&mut r.take(limit), &aad, &mut |c| {
                        ctx.check("reading the backup").map_err(carry)?;
                        feed(c)
                    });
                    Ok(())
                });
                match io_res {
                    Err(e) => Err(uncarry(e)),
                    Ok(()) => res,
                }
            }
        };
        match opened {
            Ok(()) => {}
            Err(BackupError::Io(std::io::ErrorKind::NotFound)) => {
                return Err(BackupError::Damaged(format!(
                    "a piece of the backup ({}) is missing",
                    short(sha)
                )));
            }
            Err(e) => return Err(e),
        }
        if count != bytes || hex::encode(hasher.finalize()) != sha {
            return Err(damaged());
        }
        Ok(())
    }

    /// A series read whole through `sink`, its segments and the whole
    /// proven against the manifest.
    pub(crate) fn read_series(
        &self,
        series: &Series,
        kind: Kind,
        sink: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
    ) -> Result<(), BackupError> {
        let mut total = Sha256::new();
        let mut bytes = 0u64;
        for seg in &series.segments {
            self.ctx.check("reading the backup")?;
            self.read_piece(&seg.file, kind, seg.bytes, &seg.sha256, &mut |c| {
                total.update(c);
                bytes += c.len() as u64;
                sink(c)
            })?;
        }
        if bytes != series.bytes || (bytes > 0 && hex::encode(total.finalize()) != series.sha256) {
            return Err(BackupError::Damaged(
                "the backup's pieces don't add up to what its manifest says".into(),
            ));
        }
        Ok(())
    }

    /// The index of stored files a manifest names.
    pub(crate) fn index(&self, m: &Manifest) -> Result<Vec<IndexLine>, BackupError> {
        let mut text = Vec::new();
        self.read_series(&m.files, Kind::Index, &mut |c| {
            text.extend_from_slice(c);
            Ok(())
        })?;
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        for line in text.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            let l: IndexLine = serde_json::from_slice(line).map_err(|_| {
                BackupError::Damaged("the backup's index of stored files is damaged".into())
            })?;
            if !is_sha(&l.sha256) {
                return Err(BackupError::Damaged(
                    "the backup's index of stored files names a file by no hash".into(),
                ));
            }
            if seen.insert(l.sha256.clone()) {
                out.push(l);
            }
        }
        Ok(out)
    }
}

fn short(sha: &str) -> &str {
    &sha[..12.min(sha.len())]
}

/// A set found at a source, ready to be proven.
pub struct Found {
    set: Set,
    key: Option<BackupKey>,
    /// The books' id as the newest manifest says it.
    pub book: Option<String>,
    /// Whether it is sealed.
    pub sealed: bool,
}

/// One set of books in a folder, as listed for a person to choose from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The folder's name inside the destination.
    pub folder: String,
    /// The books' id, if it could be read (a sealed one needs the
    /// passphrase first).
    pub book: Option<String>,
    /// Sealed.
    pub sealed: bool,
}

fn open_header(set: &Set) -> Result<Option<Header>, BackupError> {
    match set.io.len(HEADER)? {
        None => Ok(None),
        Some(_) => Header::decode(&set.read_small(HEADER)?).map(Some),
    }
}

/// Open the set at `io`, plain or sealed, with `passphrase` for a sealed one.
fn open_set(
    io: Box<dyn SetIo>,
    passphrase: Option<&str>,
    ctx: &Ctx,
) -> Result<(Set, Option<BackupKey>), BackupError> {
    let probe = Set {
        io,
        keys: None,
        ctx: ctx.clone(),
    };
    let Some(header) = open_header(&probe)? else {
        return Ok((probe, None));
    };
    let pass = passphrase.ok_or(BackupError::NeedsPassphrase)?;
    let key = BackupKey::from_header(pass, &header)?;
    let set = Set {
        io: probe.io,
        keys: Some(key.keys()),
        ctx: ctx.clone(),
    };
    Ok((set, Some(key)))
}

fn found_from(set: Set, key: Option<BackupKey>) -> Result<Found, BackupError> {
    let book = set.newest_whole()?.map(|(m, _)| m.book);
    Ok(Found {
        sealed: set.sealed(),
        set,
        key,
        book,
    })
}

/// The folders of `dir` that hold a set (a `manifests` folder inside).
fn set_folders(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.join(MANIFESTS).is_dir()
                && let Some(n) = p.file_name().and_then(|n| n.to_str())
            {
                out.push(n.to_string());
            }
        }
    }
    out.sort();
    out
}

/// Find the backup at `source`: a destination folder, a set's own folder,
/// or a single backup file. A sealed one needs `passphrase`.
pub fn find(source: &Path, passphrase: Option<&str>, ctx: &Ctx) -> Result<Found, BackupError> {
    let meta = std::fs::symlink_metadata(source).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => BackupError::NotABackup,
        _ => BackupError::from(e),
    })?;
    if meta.is_file() {
        let (set, key) = open_set(super::single::open(source, ctx)?, passphrase, ctx)?;
        return found_from(set, key);
    }
    if !meta.is_dir() {
        return Err(BackupError::NotABackup);
    }
    if source.join(MANIFESTS).is_dir() {
        let (set, key) = open_set(
            Box::new(DirSet::new(source.to_path_buf(), ctx.clone())),
            passphrase,
            ctx,
        )?;
        return found_from(set, key);
    }
    let folders = set_folders(source);
    let mut found: Vec<(String, Found)> = Vec::new();
    let mut wrong = false;
    let mut needs = false;
    for name in &folders {
        match open_set(
            Box::new(DirSet::new(source.join(name), ctx.clone())),
            passphrase,
            ctx,
        ) {
            Ok((set, key)) => found.push((name.clone(), found_from(set, key)?)),
            Err(BackupError::WrongPassphrase) => wrong = true,
            Err(BackupError::NeedsPassphrase) => needs = true,
            // A folder that is not a whole set is not a candidate.
            Err(BackupError::Damaged(_)) => {}
            Err(e) => return Err(e),
        }
    }
    // Only sets that hold a backup count.
    found.retain(|(_, f)| f.book.is_some());
    match found.len() {
        1 => Ok(found.pop().expect("one").1),
        0 if wrong => Err(BackupError::WrongPassphrase),
        0 if needs => Err(BackupError::NeedsPassphrase),
        0 => Err(BackupError::NotABackup),
        _ => Err(BackupError::Several(
            found
                .iter()
                .map(|(n, f)| format!("{} ({n})", f.book.as_deref().unwrap_or("?")))
                .collect::<Vec<_>>()
                .join(", "),
        )),
    }
}

/// Every set in a destination folder, for a person to choose from.
pub fn list(source: &Path) -> Vec<Candidate> {
    let probe = |name: &str| -> Candidate {
        let ctx = Ctx::new();
        let set = Set::dir(source.join(name), None, &ctx);
        let sealed = set.io.len(HEADER).ok().flatten().is_some();
        let book = (!sealed)
            .then(|| set.newest_whole().ok().flatten().map(|(m, _)| m.book))
            .flatten();
        Candidate {
            folder: name.to_string(),
            book,
            sealed,
        }
    };
    set_folders(source).iter().map(|n| probe(n)).collect()
}

/// A backup proven whole (INVARIANT 3).
#[derive(Debug, Clone)]
pub struct Verified {
    /// Its journal's head: seq and hash.
    pub head: (u64, String),
    /// Stored files re-hashed.
    pub blobs: u64,
    /// The scratch folder the books were built in.
    pub tree: PathBuf,
    /// The id of the books it holds, as they were backed up.
    pub book: Option<String>,
    /// Its manifest's number.
    pub number: u64,
    /// Newer manifests that were not whole and were passed over.
    pub passed_over: usize,
    /// The key, if it was sealed: for the restored books to carry on
    /// backing up sealed.
    pub key: Option<BackupKey>,
}

/// Write `bytes` as a new file in `dir` (a scratch folder of this program's
/// own: plain files, nothing removed).
fn put_scratch(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(name))?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Build the books of `m` in `scratch` (a new folder): the journal, the
/// access log, the vault, the keys and every stored file; every piece
/// proven as it is read. (The store reads some stored files itself when
/// it verifies the books, so a drill keeps them too, as a restore does.)
/// Returns the hashes of the stored files proven.
fn materialize(set: &Set, m: &Manifest, scratch: &Path) -> Result<BTreeSet<String>, BackupError> {
    let ctx = &set.ctx;
    std::fs::create_dir_all(scratch)?;
    for (series, kind, name) in [
        (&m.journal, Kind::Journal, crate::JOURNAL_FILE),
        (&m.access, Kind::Access, crate::access::ACCESS_FILE),
    ] {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(scratch.join(name))?;
        set.read_series(series, kind, &mut |c| f.write_all(c))?;
        f.sync_all()?;
    }
    for (list, kind, dir) in [
        (&m.vault, Kind::Vault, crate::root::VAULT_DIR),
        (&m.keys, Kind::Key, crate::root::KEYS_DIR),
    ] {
        for n in list {
            let mut bytes = Vec::new();
            set.read_piece(
                &n.piece.file,
                kind,
                n.piece.bytes,
                &n.piece.sha256,
                &mut |c| {
                    bytes.extend_from_slice(c);
                    Ok(())
                },
            )?;
            put_scratch(&scratch.join(dir), &n.name, &bytes)?;
        }
    }
    let mut proven = BTreeSet::new();
    for line in set.index(m)? {
        ctx.check("reading the backup")?;
        let rel = piece_path(set.keys.as_ref(), Kind::File, 0, &line.sha256);
        let blob = scratch
            .join(crate::store::BLOBS_DIR)
            .join(&line.sha256[0..2])
            .join(&line.sha256[2..4]);
        std::fs::create_dir_all(&blob)?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(blob.join(&line.sha256))?;
        set.read_piece(&rel, Kind::File, line.bytes, &line.sha256, &mut |c| {
            f.write_all(c)
        })?;
        f.sync_all()?;
        proven.insert(line.sha256);
    }
    Ok(proven)
}

/// Prove the books built in `scratch` from `m` (INVARIANT 3): the chain
/// replays, every stored file the books name is among those proven, every
/// vault record the books name and the key that opens them are there, the
/// head is the one the manifest records.
fn prove_books(
    m: &Manifest,
    scratch: &Path,
    proven_files: &BTreeSet<String>,
    ctx: &Ctx,
) -> Result<(u64, (u64, String)), BackupError> {
    ctx.phase("replay");
    let _stoppable = ctx.stop_flag().map(crate::cancel::scope);
    let mut store = crate::Store::open(scratch)?;
    ctx.phase("verify");
    store.verify_and_heal()?;
    ctx.phase("blobs");
    let named: BTreeSet<String> = store.stored_blob_shas()?.into_iter().collect();
    if let Some(missing) = named.iter().find(|s| !proven_files.contains(*s)) {
        return Err(BackupError::Damaged(format!(
            "a stored file the books name isn't in it ({})",
            short(missing)
        )));
    }
    let blobs = store.verify_blobs()?;
    let head = store.head();
    let tins: Vec<String> = store
        .ledger()
        .parties()
        .flat_map(|p| {
            [
                p.tin.as_ref().and_then(|t| t.vault_record.clone()),
                p.w9.as_ref().and_then(|w| w.vault_record.clone()),
            ]
        })
        .flatten()
        .collect();
    drop(store);
    ctx.phase("vault");
    if !tins.is_empty() {
        let held: BTreeSet<&str> = m.vault.iter().map(|n| n.piece.sha256.as_str()).collect();
        if let Some(missing) = tins.iter().find(|h| !held.contains(h.as_str())) {
            return Err(BackupError::Damaged(format!(
                "a TIN record the books name isn't in it ({})",
                short(missing)
            )));
        }
        if !m
            .keys
            .iter()
            .any(|n| n.name == crate::root::LEGACY_KEY_FILE)
        {
            return Err(BackupError::Damaged(
                "its TIN records are in it, but not the key that opens them".into(),
            ));
        }
    }
    if head.0 != m.head.seq || head.1 != m.head.hash {
        return Err(BackupError::Damaged(format!(
            "its journal ends at event {}, but the backup records another head",
            head.0
        )));
    }
    Ok((blobs, head))
}

/// Prove the backup found, building its books in `scratch` (a new folder):
/// the newest manifest that proves whole; a newer one that does not is
/// passed over and said. Every stored file is re-hashed and kept in the
/// scratch folder (a restore moves them from it; a drill removes it).
pub fn verify_found(found: &Found, scratch: &Path, ctx: &Ctx) -> Result<Verified, BackupError> {
    if scratch.exists() {
        return Err(BackupError::Damaged("its scratch folder is in use".into()));
    }
    let set = &found.set;
    let mut passed = 0usize;
    let mut last: Option<BackupError> = None;
    for m in set.whole_ones()? {
        ctx.phase("fetch");
        ctx.check("verifying")?;
        let attempt =
            materialize(set, &m, scratch).and_then(|files| prove_books(&m, scratch, &files, ctx));
        match attempt {
            Ok((blobs, head)) => {
                return Ok(Verified {
                    head,
                    blobs,
                    tree: scratch.to_path_buf(),
                    book: Some(m.book.clone()),
                    number: m.number,
                    passed_over: passed,
                    key: found.key.clone(),
                });
            }
            Err(e @ (BackupError::Cancelled(_) | BackupError::Died)) => {
                let _ = std::fs::remove_dir_all(scratch);
                return Err(e);
            }
            Err(e) => {
                // Not this one: another, older, may be whole.
                let _ = std::fs::remove_dir_all(scratch);
                passed += 1;
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or(BackupError::NotABackup))
}

/// Whether the journal at `backup` is a byte prefix of the one at `live`
/// (a drill: the backup holds the book's history, nothing else).
pub fn is_prefix(live: &Path, backup: &Path, ctx: &Ctx) -> Result<bool, BackupError> {
    let mut a = std::fs::File::open(live)?;
    let mut b = std::fs::File::open(backup)?;
    let (mut x, mut y) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        ctx.check("comparing journals")?;
        let n = b.read(&mut y)?;
        if n == 0 {
            return Ok(true);
        }
        let mut got = 0;
        while got < n {
            let m = a.read(&mut x[got..n])?;
            if m == 0 {
                return Ok(false);
            }
            got += m;
        }
        if x[..n] != y[..n] {
            return Ok(false);
        }
    }
}

/// A restore drill (INVARIANT 3): the backup of the book `book` in the
/// destination, its books built in `scratch` (a new folder, removed
/// after), proven whole, and its journal a byte prefix of `live` (the
/// book's journal now). Its head.
pub fn drill(
    dest: &Destination,
    book: &str,
    live: &Path,
    scratch: &Path,
    ctx: &Ctx,
) -> Result<(u64, String), BackupError> {
    if !dest.folder.is_dir() {
        return Err(BackupError::NoFolder);
    }
    let set = super::write::locate(dest, book, ctx)?.ok_or(BackupError::NoBackupThere)?;
    let found = Found {
        sealed: set.sealed(),
        book: Some(book.to_string()),
        key: dest.key.clone(),
        set,
    };
    let checked = verify_found(&found, scratch, ctx).and_then(|v| {
        ctx.phase("prefix");
        if v.book.as_deref() != Some(book) {
            return Err(BackupError::Elsewhere);
        }
        match is_prefix(live, &v.tree.join(crate::JOURNAL_FILE), ctx) {
            Ok(true) => Ok(v.head),
            Ok(false) => Err(BackupError::Damaged(
                "its journal isn't this book's history".into(),
            )),
            Err(e) => Err(e),
        }
    });
    let _ = std::fs::remove_dir_all(scratch);
    checked
}

/// Prove the book's own local backup (`local-backup/`, in its folder)
/// holds the book in `book_dir` exactly as it is now, its journal's head
/// `head` (INVARIANT 3, and more): proven as a restore proves a backup,
/// its head `head`, its journal the book's byte for byte, and its vault
/// and keys the book's, file for file and byte for byte. What the format
/// upgrade asks of the backup it makes first (plan/served.md 3.9: "a
/// verified local backup that includes the keys"). The caller holds the
/// store's lock, so the book can't move while it is compared.
pub fn prove_local(
    book_dir: &Path,
    book: &str,
    head: &(u64, String),
    scratch: &Path,
    ctx: &Ctx,
) -> Result<u64, BackupError> {
    let dest = Destination {
        folder: book_dir.join(super::LOCAL_DIR),
        key: None,
    };
    let set = super::write::locate(&dest, book, ctx)?.ok_or(BackupError::NoBackupThere)?;
    let found = Found {
        sealed: false,
        book: Some(book.to_string()),
        key: None,
        set,
    };
    let proven = verify_found(&found, scratch, ctx).and_then(|v| {
        if v.head != *head {
            return Err(BackupError::Damaged(format!(
                "it holds the books through event {}, and they are at event {}",
                v.head.0, head.0
            )));
        }
        ctx.phase("compare");
        ctx.check("comparing")?;
        if std::fs::read(book_dir.join(crate::JOURNAL_FILE)).unwrap_or_default()
            != std::fs::read(v.tree.join(crate::JOURNAL_FILE))?
        {
            return Err(BackupError::Damaged(
                "its journal isn't the books' own, byte for byte".into(),
            ));
        }
        for kept in [crate::root::VAULT_DIR, crate::root::KEYS_DIR] {
            if flat(&book_dir.join(kept))? != flat(&v.tree.join(kept))? {
                return Err(BackupError::Damaged(format!(
                    "its {} isn't the books' own, file for file",
                    if kept == crate::root::KEYS_DIR {
                        "keys folder"
                    } else {
                        "TIN vault"
                    }
                )));
            }
        }
        Ok(v.blobs)
    });
    let _ = std::fs::remove_dir_all(scratch);
    proven
}

fn flat(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, BackupError> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries {
            let e = e?;
            out.push((
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path())?,
            ));
        }
    }
    out.sort();
    Ok(out)
}
