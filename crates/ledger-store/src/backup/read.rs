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

/// Where the contents of a piece go as they are read and proven. A piece
/// that proves damaged after some of it was handed over is read again from
/// another generation, so a sink can be taken back to where the piece
/// began.
pub(crate) trait Sink {
    /// The next chunk of contents.
    fn put(&mut self, chunk: &[u8]) -> std::io::Result<()>;
    /// A piece begins here: [`Sink::rewind`] comes back to this.
    fn mark(&mut self);
    /// Forget what was handed over since the last [`Sink::mark`].
    fn rewind(&mut self) -> std::io::Result<()>;
}

/// Contents thrown away (the piece is only proven).
pub(crate) struct Discard;

impl Sink for Discard {
    fn put(&mut self, _: &[u8]) -> std::io::Result<()> {
        Ok(())
    }
    fn mark(&mut self) {}
    fn rewind(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Contents gathered in memory.
#[derive(Default)]
pub(crate) struct VecSink {
    pub(crate) bytes: Vec<u8>,
    at: usize,
}

impl Sink for VecSink {
    fn put(&mut self, chunk: &[u8]) -> std::io::Result<()> {
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }
    fn mark(&mut self) {
        self.at = self.bytes.len();
    }
    fn rewind(&mut self) -> std::io::Result<()> {
        self.bytes.truncate(self.at);
        Ok(())
    }
}

/// Contents written to a file of this program's own making.
pub(crate) struct FileSink {
    file: std::fs::File,
    at: u64,
    len: u64,
}

impl FileSink {
    pub(crate) fn new(file: std::fs::File) -> Self {
        Self {
            file,
            at: 0,
            len: 0,
        }
    }

    pub(crate) fn finish(self) -> std::io::Result<()> {
        self.file.sync_all()
    }
}

impl Sink for FileSink {
    fn put(&mut self, chunk: &[u8]) -> std::io::Result<()> {
        self.len += chunk.len() as u64;
        self.file.write_all(chunk)
    }
    fn mark(&mut self) {
        self.at = self.len;
    }
    fn rewind(&mut self) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom};
        self.file.set_len(self.at)?;
        self.file.seek(SeekFrom::Start(self.at))?;
        self.len = self.at;
        Ok(())
    }
}

/// What a series of segments hashes to as its segments are read through
/// it, in front of another sink.
struct SeriesSink<'a> {
    inner: &'a mut dyn Sink,
    total: Sha256,
    bytes: u64,
    at: (Sha256, u64),
}

impl Sink for SeriesSink<'_> {
    fn put(&mut self, chunk: &[u8]) -> std::io::Result<()> {
        self.total.update(chunk);
        self.bytes += chunk.len() as u64;
        self.inner.put(chunk)
    }
    fn mark(&mut self) {
        self.at = (self.total.clone(), self.bytes);
        self.inner.mark();
    }
    fn rewind(&mut self) -> std::io::Result<()> {
        self.total = self.at.0.clone();
        self.bytes = self.at.1;
        self.inner.rewind()
    }
}

/// A piece that proved damaged: where the manifest names it, and what it
/// should hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Damage {
    pub kind: Kind,
    /// The path the manifest names it at (a stored file's, its first
    /// generation's).
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
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

    /// The manifest numbered `n` as it is kept: read, opened, checked by
    /// its own line (a delta manifest names only what is new in its
    /// series: [`Set::manifest`] gives it whole).
    fn manifest_stored(&self, n: u64) -> Result<Manifest, BackupError> {
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
        Ok(m)
    }

    /// `stored` whole: a manifest that names its base is joined to it. The
    /// base is read and must hash to what `stored` says (so a changed,
    /// swapped or missing base makes this manifest unreadable, never a
    /// manifest that names something else), and must be one link shorter
    /// in the chain, so a chain can't loop or run past [`DELTA_CHAIN`].
    fn resolve(&self, mut stored: Manifest) -> Result<Manifest, BackupError> {
        let bad = |why: &str| BackupError::Damaged(format!("a manifest of the backup {why}"));
        let Some(base) = stored.base.clone() else {
            if stored.depth != 0
                || [&stored.journal, &stored.access, &stored.files]
                    .iter()
                    .any(|s| s.keep != 0)
            {
                return Err(bad("names a base it doesn't have"));
            }
            return Ok(stored);
        };
        if stored.depth == 0 || stored.depth > DELTA_CHAIN || base.number >= stored.number {
            return Err(bad("is chained wrongly"));
        }
        let below = self.manifest_stored(base.number)?;
        if below.depth + 1 != stored.depth || below.sha != base.sha256 {
            return Err(bad("is built on a manifest that is not the one it names"));
        }
        let below = self.resolve(below)?;
        for (mine, theirs) in [
            (&mut stored.journal, &below.journal),
            (&mut stored.access, &below.access),
            (&mut stored.files, &below.files),
        ] {
            if mine.keep > theirs.segments.len() {
                return Err(bad("keeps more of its base than there is"));
            }
            let mut segments = theirs.segments[..mine.keep].to_vec();
            segments.append(&mut mine.segments);
            mine.segments = segments;
            mine.keep = 0;
        }
        Ok(stored)
    }

    /// The manifest numbered `n`: read, opened, whole, checked.
    pub(crate) fn manifest(&self, n: u64) -> Result<Manifest, BackupError> {
        let m = self.resolve(self.manifest_stored(n)?)?;
        m.check_names(self.sealed())?;
        Ok(m)
    }

    /// The paths a piece is kept at, as found: `rel` and its other
    /// generations (`.g1`, `.g2`, ...), in the order of their generation.
    pub(crate) fn generations(&self, rel: &str) -> Result<Vec<String>, BackupError> {
        let (stem, _) = split_generation(rel);
        let (dir, name) = stem.rsplit_once('/').unwrap_or(("", stem));
        let mut found: Vec<(u32, String)> = Vec::new();
        for entry in self.io.list(dir)? {
            let (entry_stem, g) = split_generation(&entry);
            if entry_stem == name {
                let path = if dir.is_empty() {
                    entry.clone()
                } else {
                    format!("{dir}/{entry}")
                };
                found.push((g, path));
            }
        }
        found.sort();
        Ok(found.into_iter().map(|(_, p)| p).collect())
    }

    /// Whether the piece `p` is there at the length it names, at the path
    /// the manifest names or, when that is not so, at another generation.
    fn piece_present(&self, p: &Piece) -> Result<bool, BackupError> {
        if self.io.len(&p.file)? == Some(p.stored) {
            return Ok(true);
        }
        for other in self.generations(&p.file)? {
            if other != p.file && self.io.len(&other)? == Some(p.stored) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether every piece `m` names, apart from the stored files, is
    /// there at the length it names: the cheap look that decides which
    /// manifest is worth proving. The proof re-hashes.
    pub(crate) fn looks_whole(&self, m: &Manifest) -> Result<bool, BackupError> {
        let pieces = m
            .journal
            .segments
            .iter()
            .chain(&m.access.segments)
            .chain(&m.files.segments)
            .chain(m.vault.iter().map(|n| &n.piece))
            .chain(m.keys.iter().map(|n| &n.piece));
        for p in pieces {
            if !self.piece_present(p)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The newest manifest that is whole to the eye (INVARIANT 3), and
    /// how many newer ones were passed over. Newest first, and only as
    /// far as the first.
    pub(crate) fn newest_whole(&self) -> Result<Option<(Manifest, usize)>, BackupError> {
        let numbers = self.numbers()?;
        for (skipped, n) in numbers.iter().rev().enumerate() {
            self.ctx.check("reading the backup")?;
            match self.manifest(*n) {
                Ok(m) if self.looks_whole(&m)? => return Ok(Some((m, skipped))),
                Ok(_) => {}
                // A manifest that is not whole is passed over; a stop, a
                // file that is not downloaded, a disk's own refusal is not.
                Err(BackupError::Damaged(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    /// Read the piece named by `p` (of `kind`) into `sink`, proven as it
    /// goes. Ok(true): it is what the manifest says. Ok(false): it is not
    /// there, or is not what it should be, at the path named nor at any
    /// other generation; `sink` is as it was. Err: a stop, a file not
    /// downloaded, a disk's refusal: nothing is said of the piece.
    pub(crate) fn read_piece_into(
        &self,
        rel: &str,
        kind: Kind,
        bytes: u64,
        sha: &str,
        sink: &mut dyn Sink,
    ) -> Result<bool, BackupError> {
        sink.mark();
        if self.attempt(rel, kind, bytes, sha, sink)? {
            return Ok(true);
        }
        sink.rewind()?;
        // The piece was written again beside a damaged one: any
        // generation of it that hashes right is the piece.
        for other in self.generations(rel)? {
            if other == rel {
                continue;
            }
            sink.mark();
            if self.attempt(&other, kind, bytes, sha, sink)? {
                return Ok(true);
            }
            sink.rewind()?;
        }
        Ok(false)
    }

    /// One reading of the piece at `rel`.
    fn attempt(
        &self,
        rel: &str,
        kind: Kind,
        bytes: u64,
        sha: &str,
        sink: &mut dyn Sink,
    ) -> Result<bool, BackupError> {
        let mut hasher = Sha256::new();
        let mut count = 0u64;
        let ctx = self.ctx.clone();
        let mut feed = |chunk: &[u8]| -> std::io::Result<()> {
            hasher.update(chunk);
            count += chunk.len() as u64;
            sink.put(chunk)
        };
        let opened: Result<(), BackupError> = match &self.keys {
            None => {
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
                io_res.map_err(uncarry)
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
            // Not there, or not a file of the form: damage to the piece.
            Err(BackupError::Io(
                std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidData,
            ))
            | Err(BackupError::Damaged(_)) => return Ok(false),
            Err(e) => return Err(e),
        }
        Ok(count == bytes && hex::encode(hasher.finalize()) == sha)
    }

    /// A series read whole through `sink`: its segments, each proven, and
    /// the whole proven against the manifest. A segment that is damaged
    /// is listed and the reading goes on with the next, so that all the
    /// damage is found at once; the whole is then not proven (there is a
    /// gap), and it is the caller's to say.
    pub(crate) fn read_series_lenient(
        &self,
        series: &Series,
        kind: Kind,
        sink: &mut dyn Sink,
    ) -> Result<Vec<Damage>, BackupError> {
        let mut series_sink = SeriesSink {
            inner: sink,
            total: Sha256::new(),
            bytes: 0,
            at: (Sha256::new(), 0),
        };
        let mut damaged = Vec::new();
        for seg in &series.segments {
            self.ctx.check("reading the backup")?;
            if !self.read_piece_into(&seg.file, kind, seg.bytes, &seg.sha256, &mut series_sink)? {
                damaged.push(Damage {
                    kind,
                    file: seg.file.clone(),
                    bytes: seg.bytes,
                    sha256: seg.sha256.clone(),
                });
            }
        }
        if damaged.is_empty()
            && (series_sink.bytes != series.bytes
                || (series_sink.bytes > 0
                    && hex::encode(series_sink.total.finalize()) != series.sha256))
        {
            return Err(BackupError::Damaged(
                "the backup's pieces don't add up to what its manifest says".into(),
            ));
        }
        Ok(damaged)
    }

    /// A series read whole through `sink`, its segments and the whole
    /// proven against the manifest.
    pub(crate) fn read_series(
        &self,
        series: &Series,
        kind: Kind,
        sink: &mut dyn Sink,
    ) -> Result<(), BackupError> {
        let damaged = self.read_series_lenient(series, kind, sink)?;
        match damaged.first() {
            None => Ok(()),
            Some(d) => Err(BackupError::Damaged(format!(
                "a piece of the backup ({}) is damaged or missing",
                short(&d.sha256)
            ))),
        }
    }
}

/// The index of stored files in `text` (one `{"sha256","bytes"}` per
/// line), each file once.
pub(crate) fn parse_index(text: &[u8]) -> Result<Vec<IndexLine>, BackupError> {
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
    /// The journal event its newest backup reaches, if it could be read.
    pub through_seq: Option<u64>,
    /// When its newest backup was made, if it could be read.
    pub written: Option<String>,
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
    cache: &mut KeyCache,
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
    let key = BackupKey::from_header_cached(pass, &header, cache)?;
    let set = Set {
        io: probe.io,
        keys: Some(key.keys()),
        ctx: ctx.clone(),
    };
    Ok((set, Some(key)))
}

/// Keys derived while a folder is looked through, one per distinct setting.
type KeyCache = Vec<(super::seal::Kdf, Result<BackupKey, BackupError>)>;

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
        let (set, key) = open_set(
            super::single::open(source, ctx)?,
            passphrase,
            ctx,
            &mut Vec::new(),
        )?;
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
            &mut Vec::new(),
        )?;
        return found_from(set, key);
    }
    let folders = set_folders(source);
    let mut found: Vec<(String, Found)> = Vec::new();
    let mut wrong = false;
    let mut needs = false;
    // One key derived for each distinct setting, however many sealed sets
    // the folder holds (what a header asks is not paid for per set).
    let mut cache: KeyCache = Vec::new();
    for name in &folders {
        match open_set(
            Box::new(DirSet::new(source.join(name), ctx.clone())),
            passphrase,
            ctx,
            &mut cache,
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
        let newest = (!sealed)
            .then(|| set.newest_whole().ok().flatten().map(|(m, _)| m))
            .flatten();
        Candidate {
            folder: name.to_string(),
            book: newest.as_ref().map(|m| m.book.clone()),
            sealed,
            through_seq: newest.as_ref().map(|m| m.head.seq),
            written: newest.map(|m| m.written),
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
    /// When it was made (UTC), as its manifest says.
    pub written: String,
    /// How many newer backups there are that were passed over: not whole,
    /// or damaged. Not zero means this is not the newest backup there is.
    pub passed_over: usize,
    /// When the newest backup there is was made, if it can be read.
    pub newest_written: Option<String>,
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

/// Which stored files a pass over a manifest reads.
pub(crate) enum Files<'a> {
    /// Every one, built into the scratch folder (a restore, a drill).
    All,
    /// Only these (the proof after a backup): read and proven, not kept.
    Only(&'a BTreeSet<String>),
}

/// What reading a manifest's pieces found.
pub(crate) struct Audit {
    /// Pieces that are damaged or missing, every one of them.
    pub damaged: Vec<Damage>,
    /// Stored files the manifest names, by hash.
    pub index: BTreeSet<String>,
    /// How many stored files were read and proven.
    pub files_read: u64,
}

/// Build the books of `m` in `scratch` (a new folder): the journal, the
/// access log, the vault, the keys and (with [`Files::All`]) every stored
/// file; every piece proven as it is read, and every piece that is not
/// whole listed, not only the first. (The store reads some stored files
/// itself when it verifies the books, so a drill keeps them too, as a
/// restore does.)
fn materialize(
    set: &Set,
    m: &Manifest,
    scratch: &Path,
    files: &Files<'_>,
) -> Result<Audit, BackupError> {
    let ctx = &set.ctx;
    std::fs::create_dir_all(scratch)?;
    let mut damaged = Vec::new();
    for (series, kind, name) in [
        (&m.journal, Kind::Journal, crate::JOURNAL_FILE),
        (&m.access, Kind::Access, crate::access::ACCESS_FILE),
    ] {
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(scratch.join(name))?;
        let mut sink = FileSink::new(f);
        damaged.extend(set.read_series_lenient(series, kind, &mut sink)?);
        sink.finish()?;
    }
    for (list, kind, dir) in [
        (&m.vault, Kind::Vault, crate::root::VAULT_DIR),
        (&m.keys, Kind::Key, crate::root::KEYS_DIR),
    ] {
        for n in list {
            let mut bytes = VecSink::default();
            if set.read_piece_into(
                &n.piece.file,
                kind,
                n.piece.bytes,
                &n.piece.sha256,
                &mut bytes,
            )? {
                put_scratch(&scratch.join(dir), &n.name, &bytes.bytes)?;
            } else {
                damaged.push(Damage {
                    kind,
                    file: n.piece.file.clone(),
                    bytes: n.piece.bytes,
                    sha256: n.piece.sha256.clone(),
                });
            }
        }
    }
    // The index (itself pieces), then the files it names.
    let mut text = VecSink::default();
    let index_damage = set.read_series_lenient(&m.files, Kind::Index, &mut text)?;
    let mut index = BTreeSet::new();
    let mut lines = Vec::new();
    if index_damage.is_empty() {
        for l in parse_index(&text.bytes)? {
            index.insert(l.sha256.clone());
            lines.push(l);
        }
    }
    damaged.extend(index_damage);
    let mut files_read = 0u64;
    for line in lines {
        ctx.check("reading the backup")?;
        let wanted = match files {
            Files::All => true,
            Files::Only(only) => only.contains(&line.sha256),
        };
        if !wanted {
            continue;
        }
        let rel = piece_path(set.keys.as_ref(), Kind::File, 0, &line.sha256);
        let ok = match files {
            Files::All => {
                let blob = scratch
                    .join(crate::store::BLOBS_DIR)
                    .join(&line.sha256[0..2])
                    .join(&line.sha256[2..4]);
                std::fs::create_dir_all(&blob)?;
                let f = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(blob.join(&line.sha256))?;
                let mut sink = FileSink::new(f);
                let ok =
                    set.read_piece_into(&rel, Kind::File, line.bytes, &line.sha256, &mut sink)?;
                sink.finish()?;
                ok
            }
            Files::Only(_) => {
                set.read_piece_into(&rel, Kind::File, line.bytes, &line.sha256, &mut Discard)?
            }
        };
        if ok {
            files_read += 1;
        } else {
            damaged.push(Damage {
                kind: Kind::File,
                file: rel,
                bytes: line.bytes,
                sha256: line.sha256.clone(),
            });
        }
    }
    Ok(Audit {
        damaged,
        index,
        files_read,
    })
}

/// The words for damage found: the first piece, and how many there are.
fn damaged_words(damaged: &[Damage]) -> String {
    match damaged {
        [] => "nothing is damaged".to_string(),
        [one] => format!(
            "a piece of the backup ({}) is damaged or missing",
            short(&one.sha256)
        ),
        [first, rest @ ..] => format!(
            "{} pieces of the backup are damaged or missing (the first is {})",
            rest.len() + 1,
            short(&first.sha256)
        ),
    }
}

/// How thoroughly the books built from a manifest are proven.
pub(crate) enum Depth {
    /// Everything: the chain replays, the store verifies and re-hashes
    /// every stored file it names.
    Whole,
    /// What a backup proves of itself when it is made: the chain
    /// replays, the books open, the vault and its key are there, the head
    /// is the one recorded, and every stored file the books name is in
    /// the manifest's index. Stored files are proven by what was read.
    Chain,
}

/// Prove the books built in `scratch` from `m` (INVARIANT 3): the chain
/// replays, every stored file the books name is in the index (and, for
/// [`Depth::Whole`], among those proven), every vault record the books name
/// and the key that opens them are there, the head is the one the
/// manifest records.
fn prove_books(
    m: &Manifest,
    scratch: &Path,
    audit: &Audit,
    depth: &Depth,
    ctx: &Ctx,
) -> Result<(u64, (u64, String)), BackupError> {
    ctx.phase("replay");
    let _stoppable = ctx.stop_flag().map(crate::cancel::scope);
    if matches!(depth, Depth::Chain) {
        // The hash chain, up to the head, before the books are opened.
        let lines = crate::journal::read_and_verify_journal(&scratch.join(crate::JOURNAL_FILE))?;
        let last = lines.last().map(|l| l.seq).unwrap_or(0);
        if last != m.head.seq {
            return Err(BackupError::Damaged(format!(
                "its journal ends at event {last}, but the backup records another head"
            )));
        }
    }
    let mut store = crate::Store::open(scratch)?;
    if matches!(depth, Depth::Whole) {
        ctx.phase("verify");
        store.verify_and_heal()?;
    }
    ctx.phase("blobs");
    let named: BTreeSet<String> = store.stored_blob_shas()?.into_iter().collect();
    if let Some(missing) = named.iter().find(|s| !audit.index.contains(*s)) {
        return Err(BackupError::Damaged(format!(
            "a stored file the books name isn't in it ({})",
            short(missing)
        )));
    }
    let blobs = match depth {
        Depth::Whole => store.verify_blobs()?,
        Depth::Chain => audit.files_read,
    };
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
    ctx.check("reading its vault")?;
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

/// The books built from manifest `m` into `scratch` and proven, whole.
fn prove_manifest(
    set: &Set,
    m: &Manifest,
    scratch: &Path,
) -> Result<(u64, (u64, String)), BackupError> {
    let audit = materialize(set, m, scratch, &Files::All)?;
    if !audit.damaged.is_empty() {
        return Err(BackupError::Damaged(damaged_words(&audit.damaged)));
    }
    prove_books(m, scratch, &audit, &Depth::Whole, &set.ctx)
}

/// What the proof after a backup found.
pub(crate) struct Proof {
    /// Pieces damaged or missing: none when the backup is whole as far as
    /// the proof goes.
    pub damaged: Vec<Damage>,
    /// Stored files re-hashed by it.
    pub files_read: u64,
}

/// How many backups it takes for every stored file to be re-read: the
/// proof after a backup reads the stored files that backup added and the
/// share of the others whose hash begins with a digit group equal to its
/// number modulo this.
pub(crate) const ROTATION: u64 = 16;

/// Which share of the stored files manifest number `number` re-reads: a
/// file belongs to the share `bucket(sha)`, and a backup reads the files
/// of the share equal to its number modulo [`ROTATION`]. Any `ROTATION`
/// manifest numbers in a row between them read every file once.
pub(crate) fn in_rotation(sha: &str, number: u64) -> bool {
    u64::from_str_radix(&sha[..4], 16).unwrap_or(0) % ROTATION == number % ROTATION
}

/// The proof after a backup, of the manifest numbered `number`, read back
/// from the set as a reader would read it: every journal and access
/// segment re-hashed and the chain replayed to its head, the vault and the
/// keys and their presence, the index, and of the stored files those in
/// `added` (this backup's) and the share [`in_rotation`] of the rest.
/// Builds the books in `scratch` (a new folder, removed after).
pub(crate) fn prove_written(
    set: &Set,
    number: u64,
    added: &BTreeSet<String>,
    scratch: &Path,
) -> Result<Proof, BackupError> {
    let m = set.manifest(number)?;
    // The share depends on the index, which is read inside the audit:
    // those files of it that are in the share or added are read.
    let mut index = VecSink::default();
    let _ = set.read_series_lenient(&m.files, Kind::Index, &mut index)?;
    let mut want: BTreeSet<String> = added.clone();
    for line in index.bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        if let Ok(l) = serde_json::from_slice::<IndexLine>(line)
            && is_sha(&l.sha256)
            && in_rotation(&l.sha256, number)
        {
            want.insert(l.sha256);
        }
    }
    let result = (|| {
        let audit = materialize(set, &m, scratch, &Files::Only(&want))?;
        if !audit.damaged.is_empty() {
            return Ok(Proof {
                damaged: audit.damaged,
                files_read: audit.files_read,
            });
        }
        prove_books(&m, scratch, &audit, &Depth::Chain, &set.ctx)?;
        Ok(Proof {
            damaged: Vec::new(),
            files_read: audit.files_read,
        })
    })();
    let _ = std::fs::remove_dir_all(scratch);
    result
}

/// The manifest numbered `n` proven whole, its books built in `scratch`:
/// the pieces of the attempt are removed when it does not prove.
fn verify_one(
    found: &Found,
    n: u64,
    scratch: &Path,
    passed: usize,
    newest: u64,
) -> Result<Result<Verified, BackupError>, BackupError> {
    let set = &found.set;
    let ctx = &set.ctx;
    let m = match set.manifest(n) {
        Ok(m) => m,
        Err(e @ BackupError::Damaged(_)) => return Ok(Err(e)),
        Err(e) => return Err(e),
    };
    if !set.looks_whole(&m)? {
        return Ok(Err(BackupError::Damaged(format!(
            "backup number {n} is missing some of its pieces"
        ))));
    }
    ctx.phase("fetch");
    ctx.check("verifying")?;
    match prove_manifest(set, &m, scratch) {
        Ok((blobs, head)) => Ok(Ok(Verified {
            head,
            blobs,
            tree: scratch.to_path_buf(),
            book: Some(m.book.clone()),
            number: m.number,
            written: m.written.clone(),
            passed_over: passed,
            newest_written: if n == newest {
                Some(m.written.clone())
            } else {
                set.manifest(newest).ok().map(|x| x.written)
            },
            key: found.key.clone(),
        })),
        Err(e @ (BackupError::Cancelled(_) | BackupError::Died)) => {
            let _ = std::fs::remove_dir_all(scratch);
            Err(e)
        }
        Err(e @ BackupError::Damaged(_)) => {
            let _ = std::fs::remove_dir_all(scratch);
            Ok(Err(e))
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(scratch);
            Err(e)
        }
    }
}

/// Prove the backup found, building its books in `scratch` (a new folder):
/// the newest manifest that proves whole, tried newest first and no
/// further than the first that proves; a newer one that does not is
/// passed over, and counted. Every stored file is re-hashed and kept in
/// the scratch folder (a restore moves them from it; a drill removes it).
pub fn verify_found(found: &Found, scratch: &Path, ctx: &Ctx) -> Result<Verified, BackupError> {
    if scratch.exists() {
        return Err(BackupError::Damaged("its scratch folder is in use".into()));
    }
    let set = &found.set;
    let numbers = set.numbers()?;
    let Some(newest) = numbers.last().copied() else {
        return Err(BackupError::NotABackup);
    };
    let mut last: Option<BackupError> = None;
    for (passed, n) in numbers.iter().rev().enumerate() {
        ctx.check("verifying")?;
        match verify_one(found, *n, scratch, passed, newest)? {
            Ok(v) => return Ok(v),
            // Not this one: another, older, may be whole.
            Err(e) => last = Some(e),
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

/// How many older backups a drill tries, when the newest is damaged, to
/// say whether one of them is whole.
const OLDER_TRIES: usize = 5;

/// A restore drill (INVARIANT 3): the backup of the book `book` in the
/// destination, its books built in `scratch` (a new folder, removed
/// after), proven whole, and its journal a byte prefix of `live` (the
/// book's journal now). Its head.
///
/// A drill passes only on the **newest** manifest, never on an older
/// one: an older one that proves whole while the newest is damaged is the
/// state in which a restore would hand back old books, so it is a failure
/// ([`BackupError::NewestDamaged`], which says when the older one is
/// from). And the newest must reach `expect`, the head of the last backup
/// recorded good: a newest that stops short of it means a backup that was
/// recorded is not there.
pub fn drill(
    dest: &Destination,
    book: &str,
    live: &Path,
    scratch: &Path,
    expect: Option<&(u64, String)>,
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
    let _ = std::fs::remove_dir_all(scratch);
    let numbers = found.set.numbers()?;
    let newest = numbers.last().copied().ok_or(BackupError::NoBackupThere)?;
    let checked = match verify_one(&found, newest, scratch, 0, newest) {
        Err(e) => Err(e),
        Ok(Ok(v)) => {
            ctx.phase("prefix");
            if v.book.as_deref() != Some(book) {
                Err(BackupError::Elsewhere)
            } else if let Some(e) = expect
                && (v.head.0 < e.0 || (v.head.0 == e.0 && v.head.1 != e.1))
            {
                Err(BackupError::Damaged(format!(
                    "the newest backup in the folder reaches event {}, but the last backup recorded as good reached event {}: a backup that was made is not there",
                    v.head.0, e.0
                )))
            } else {
                match is_prefix(live, &v.tree.join(crate::JOURNAL_FILE), ctx) {
                    Ok(true) => Ok(v.head),
                    Ok(false) => Err(BackupError::Damaged(
                        "its journal isn't this book's history".into(),
                    )),
                    Err(e) => Err(e),
                }
            }
        }
        Ok(Err(_)) => {
            // The newest is damaged: say whether an older one is whole.
            let newest_from = found.set.manifest(newest).ok().map(|m| m.written);
            let mut older_from = None;
            for n in numbers.iter().rev().skip(1).take(OLDER_TRIES) {
                let _ = std::fs::remove_dir_all(scratch);
                match verify_one(&found, *n, scratch, 1, newest) {
                    Ok(Ok(v)) => {
                        older_from = Some(v.written);
                        break;
                    }
                    Ok(Err(_)) => {}
                    Err(e) => {
                        let _ = std::fs::remove_dir_all(scratch);
                        return Err(e);
                    }
                }
            }
            Err(BackupError::NewestDamaged {
                number: newest,
                newest_from,
                older_from,
            })
        }
    };
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
