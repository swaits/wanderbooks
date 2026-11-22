//! Writing a backup (INVARIANTS 1, 2, 4, 5 of the module): what is new
//! since the destination's newest whole manifest, as pieces, then the
//! manifest.

use super::cut::Cut;
use super::form::*;
use super::io::{Fill, Put, PutKind, carry, uncarry};
use super::read::{Hashed, Set, Sink};
use super::seal::{BackupKey, Keys, piece_aad};
use super::{BackupError, Ctx, READ_ME_TEXT};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// How many times a backup looks again after another program took the
/// manifest number it was about to take.
const ATTEMPTS: usize = 8;

/// A sealed set's folder begins with this.
pub(crate) const SEALED_PREFIX: &str = "sealed-";

/// Where a backup goes: a folder the person chose, and the key if the
/// backups are protected by a passphrase.
#[derive(Debug, Clone)]
pub struct Destination {
    /// The folder.
    pub folder: PathBuf,
    /// The key, when sealed.
    pub key: Option<BackupKey>,
}

/// A backup written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The manifest that is the backup: its number.
    pub number: u64,
    /// The head it reaches.
    pub head: (u64, String),
    /// False when the destination already held exactly these books: no
    /// manifest was written.
    pub new: bool,
    /// Pieces the proof after the backup found damaged or missing, which
    /// were written again under a new generation (named as they are in the
    /// set, never as a path of the data root). The damaged files stay
    /// where they are.
    pub repaired: Vec<String>,
    /// Stored files the proof after the backup re-read.
    pub files_proven: u64,
}

/// What a write is asked to do about the destination's sets.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// The destination may be begun (it never took these books' backup).
    pub may_create: bool,
    /// Begin a new set beside the one there (the person's own request,
    /// when the destination holds a longer history of these books than
    /// the books now have): the old set is never touched.
    pub new_set: bool,
}

/// The folder the first set of `book` has in a destination when it is not
/// sealed: a fixed prefix and the id, a legal name on every system.
pub fn set_name_for(book: &str) -> String {
    plain_set_name(book, 1)
}

/// The number a set folder name ends in (`.2`, `.3`), 1 for none, and the
/// name without it.
fn split_set_suffix(name: &str) -> (&str, u32) {
    match name.rsplit_once('.') {
        Some((base, n)) => match n.parse::<u32>() {
            Ok(n) if n >= 2 => (base, n),
            _ => (name, 1),
        },
        None => (name, 1),
    }
}

/// Whether the folder holds sets that are sealed and none of the plain
/// kind for `book`: backups were made protected by a passphrase there.
/// (A destination looked at when this computer has no key for it.)
pub fn holds_only_sealed(folder: &Path, book: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return false;
    };
    let (mut sealed, mut plain) = (false, false);
    for e in entries.flatten() {
        let Some(name) = e.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.starts_with(SEALED_PREFIX) && e.path().join(HEADER).is_file() {
            sealed = true;
        }
        if parse_plain_set_name(&name).is_some_and(|(b, _)| b == book) {
            plain = true;
        }
    }
    sealed && !plain
}

/// The sets of `book` in the destination, oldest first, each with the
/// folder name it has: for a plain one, the folders named for the books
/// (`wanderbooks-<id>`, `wanderbooks-<id>.2`); for a sealed one, the
/// folders whose header this key opens and whose backups are these books'
/// (or none yet).
fn sets_of(dest: &Destination, book: &str, ctx: &Ctx) -> Result<Vec<(String, Set)>, BackupError> {
    let mut found: Vec<(String, Set)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dest.folder) else {
        return Ok(found);
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    names.sort();
    match &dest.key {
        None => {
            let mut plain: Vec<(u32, String)> = names
                .into_iter()
                .filter_map(|n| match parse_plain_set_name(&n) {
                    Some((b, number)) if b == book => Some((number, n.clone())),
                    _ => None,
                })
                .collect();
            plain.sort();
            for (_, name) in plain {
                let dir = dest.folder.join(&name);
                if dir.is_dir() {
                    found.push((name, Set::dir(dir, None, ctx)));
                }
            }
        }
        Some(key) => {
            let keys = key.keys();
            let mut sealed: Vec<(String, u32, String)> = names
                .into_iter()
                .filter(|n| n.starts_with(SEALED_PREFIX))
                .map(|n| {
                    let (base, number) = split_set_suffix(&n);
                    (base.to_string(), number, n.clone())
                })
                .collect();
            sealed.sort();
            for (_, _, name) in sealed {
                let dir = dest.folder.join(&name);
                let Ok(raw) = std::fs::read(dir.join(HEADER)) else {
                    continue;
                };
                let Ok(header) = super::seal::Header::decode(&raw) else {
                    continue;
                };
                if keys.check(&header).is_err() {
                    continue;
                }
                let set = Set::dir(dir, Some(key.keys()), ctx);
                match set.newest_whole()? {
                    Some((m, _)) if m.book != book => continue,
                    _ => found.push((name, set)),
                }
            }
        }
    }
    Ok(found)
}

/// The set of `book` in the destination that its backups go to: the
/// newest of its sets. None when it is not there.
pub(crate) fn locate(
    dest: &Destination,
    book: &str,
    ctx: &Ctx,
) -> Result<Option<Set>, BackupError> {
    Ok(current_of(sets_of(dest, book, ctx)?).map(|(_, set)| set))
}

/// The set backups go to among `sets` (oldest first): of those that share
/// the first one's name, the newest.
fn current_of(sets: Vec<(String, Set)>) -> Option<(String, Set)> {
    let base = sets
        .first()
        .map(|(n, _)| split_set_suffix(n).0.to_string())?;
    sets.into_iter()
        .rfind(|(n, _)| split_set_suffix(n).0 == base)
}

/// One thing to put before the manifest.
enum Plan {
    /// A range of a live file: a segment of the journal or access log.
    Range {
        path: PathBuf,
        from: u64,
        to: u64,
        kind: Kind,
        piece: Piece,
    },
    /// Bytes in hand: an index segment, a vault record, a key.
    Bytes {
        kind: Kind,
        piece: Piece,
        bytes: Vec<u8>,
    },
    /// A stored file, in the book.
    File { path: PathBuf, piece: Piece },
}

fn stored_len(keys: Option<&Keys>, plain: u64) -> u64 {
    match keys {
        Some(_) => Keys::sealed_len(plain),
        None => plain,
    }
}

/// Where a piece to be written goes: `path` itself, unless that file is
/// one a proof found damaged, in which case the next generation of it (so
/// the damaged file stays where it is and the piece is written beside it).
/// A generation that is there already and not known damaged is taken: it
/// is a repair that a run before made.
fn fresh_path(path: &str, distrust: &BTreeSet<String>) -> String {
    if !distrust.contains(path) {
        return path.to_string();
    }
    let (stem, _) = split_generation(path);
    (1u32..)
        .map(|g| generation_path(stem, g))
        .find(|c| !distrust.contains(c))
        .expect("an unbounded count has one")
}

/// Hash `path` from 0 to `len`: the whole, the part before `mid`, and the
/// part from `mid`.
fn hash_range(
    path: &Path,
    len: u64,
    mid: u64,
    ctx: &Ctx,
) -> Result<(String, String, String), BackupError> {
    let (mut all, mut before, mut after) = (Sha256::new(), Sha256::new(), Sha256::new());
    if len > 0 {
        let mut f = std::fs::File::open(path)?;
        let mut done = 0u64;
        let mut buf = vec![0u8; 1 << 16];
        while done < len {
            ctx.check("reading the books")?;
            let want = (len - done).min(buf.len() as u64) as usize;
            let n = f.read(&mut buf[..want])?;
            if n == 0 {
                return Err(BackupError::Damaged(
                    "the books' file is shorter than it was".into(),
                ));
            }
            let chunk = &buf[..n];
            all.update(chunk);
            let split = mid.saturating_sub(done).min(n as u64) as usize;
            before.update(&chunk[..split]);
            after.update(&chunk[split..]);
            done += n as u64;
        }
    }
    let h = |d: Sha256| hex::encode(d.finalize());
    Ok((h(all), h(before), h(after)))
}

/// A hash of the first `limit` bytes handed to it.
struct PrefixHash {
    hasher: Sha256,
    taken: u64,
    limit: u64,
    at: (Sha256, u64),
}

impl Sink for PrefixHash {
    fn put(&mut self, chunk: &[u8]) -> std::io::Result<()> {
        let room = self
            .limit
            .saturating_sub(self.taken)
            .min(chunk.len() as u64) as usize;
        self.hasher.update(&chunk[..room]);
        self.taken += chunk.len() as u64;
        Ok(())
    }
    fn mark(&mut self) {
        self.at = (self.hasher.clone(), self.taken);
    }
    fn rewind(&mut self) -> std::io::Result<()> {
        self.hasher = self.at.0.clone();
        self.taken = self.at.1;
        Ok(())
    }
}

/// Whether the live journal's first `len` bytes (hashing to `live`) are
/// the first `len` bytes of the destination's longer journal `series`.
fn live_is_prefix_of(
    set: &Set,
    series: &Series,
    len: u64,
    live: &str,
) -> Result<bool, BackupError> {
    let mut sink = PrefixHash {
        hasher: Sha256::new(),
        taken: 0,
        limit: len,
        at: (Sha256::new(), 0),
    };
    set.read_series(series, Kind::Journal, &mut sink)?;
    Ok(hex::encode(sink.hasher.finalize()) == live)
}

/// Grow the series of the live file `path` (`len` bytes of whole lines)
/// from `base`, the destination's: a new segment of the bytes it lacks;
/// and a segment of the base that is in `distrust` (a proof found it
/// damaged) written again from the live file under a new generation. The
/// journal's must continue the destination's exactly (`strict`: otherwise
/// the books are not the same); an access log that does not is begun
/// again whole.
#[allow(clippy::too_many_arguments)]
fn grow(
    path: &Path,
    len: u64,
    base: Option<&Series>,
    kind: Kind,
    strict: bool,
    keys: Option<&Keys>,
    distrust: &BTreeSet<String>,
    ctx: &Ctx,
) -> Result<(Series, Vec<Plan>), BackupError> {
    let empty = Series::default();
    let mut base = base.unwrap_or(&empty);
    if base.bytes > len {
        // Only an access log can be shorter than it was (begun again).
        if strict {
            return Err(BackupError::Elsewhere);
        }
        base = &empty;
    }
    let (all, before, after) = hash_range(path, len, base.bytes, ctx)?;
    if base.bytes > 0 && before != base.sha256 {
        if strict {
            return Err(BackupError::Elsewhere);
        }
        base = &empty;
    }
    let mut plans = Vec::new();
    let mut series = base.clone();
    series.keep = 0;
    // Segments of the base that were found damaged: written again from the
    // live file (their bytes are the live file's, at the offsets the
    // segments before them add up to).
    let mut offset = 0u64;
    for seg in &mut series.segments {
        if distrust.contains(&seg.file) {
            let file = fresh_path(&seg.file, distrust);
            let piece = Piece {
                file: file.clone(),
                ..seg.clone()
            };
            plans.push(Plan::Range {
                path: path.to_path_buf(),
                from: offset,
                to: offset + seg.bytes,
                kind,
                piece,
            });
            seg.file = file;
        }
        offset += seg.bytes;
    }
    if base.bytes == len {
        return Ok((series, plans));
    }
    // `after` was hashed from the base's end: with a different base, from
    // the whole.
    let (from, seg_sha) = if base.bytes == 0 {
        (0, all.clone())
    } else {
        (base.bytes, after)
    };
    let bytes = len - from;
    let ordinal = series.segments.len();
    let piece = Piece {
        file: fresh_path(&piece_path(keys, kind, ordinal, &seg_sha), distrust),
        stored: stored_len(keys, bytes),
        bytes,
        sha256: seg_sha,
    };
    series.segments.push(piece.clone());
    series.bytes = len;
    series.sha256 = all;
    plans.push(Plan::Range {
        path: path.to_path_buf(),
        from,
        to: len,
        kind,
        piece,
    });
    Ok((series, plans))
}

/// `&mut dyn Write` as a `Write` that `io::copy` takes.
struct WriteAdapter<'a>(&'a mut dyn Write);

impl Write for WriteAdapter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// A piece put in the set: its contents from `plain`, sealed when the set
/// is, proven against `piece` as it is written.
fn put_piece(
    set: &Set,
    kind: Kind,
    piece: &Piece,
    plain: &mut dyn Read,
    ctx: &Ctx,
) -> Result<Put, BackupError> {
    ctx.check("backing up")?;
    let mut hashed = Hashed::new(plain, ctx);
    let keys = set.keys.as_ref();
    let aad = piece_aad(kind.dir(), &piece.sha256);
    let mut fill = |w: &mut dyn Write| -> std::io::Result<()> {
        match keys {
            Some(k) => k.seal_stream(&mut hashed, w, &aad)?,
            None => {
                std::io::copy(&mut hashed, &mut WriteAdapter(w))?;
            }
        }
        if hashed.count != piece.bytes
            || hex::encode(hashed.hasher.clone().finalize()) != piece.sha256
        {
            return Err(carry(BackupError::Damaged(
                "something of the books changed or was damaged while it was being backed up: try again"
                    .into(),
            )));
        }
        Ok(())
    };
    let fill_ref: Fill<'_> = &mut fill;
    set.io
        .put(&piece.file, PutKind::Piece, fill_ref)
        .map_err(uncarry)
}

fn execute(set: &Set, plan: &Plan, ctx: &Ctx) -> Result<Put, BackupError> {
    match plan {
        Plan::Range {
            path,
            from,
            to,
            kind,
            piece,
        } => {
            let mut f = std::fs::File::open(path)?;
            f.seek(SeekFrom::Start(*from))?;
            put_piece(set, *kind, piece, &mut f.take(to - from), ctx)
        }
        Plan::Bytes { kind, piece, bytes } => put_piece(set, *kind, piece, &mut &bytes[..], ctx),
        Plan::File { path, piece } => {
            let mut f = std::fs::File::open(path)?;
            put_piece(set, Kind::File, piece, &mut f, ctx)
        }
    }
}

/// The index segment's text for `lines`.
fn index_text(lines: &[IndexLine]) -> Vec<u8> {
    let mut out = Vec::new();
    for l in lines {
        out.extend_from_slice(&serde_json::to_vec(l).expect("an index line serializes"));
        out.push(b'\n');
    }
    out
}

fn named_pieces(
    files: &[(String, Vec<u8>)],
    kind: Kind,
    keys: Option<&Keys>,
    distrust: &BTreeSet<String>,
    plans: &mut Vec<Plan>,
) -> Vec<Named> {
    files
        .iter()
        .map(|(name, bytes)| {
            let sha = sha_hex(bytes);
            let piece = Piece {
                file: fresh_path(&piece_path(keys, kind, 0, &sha), distrust),
                stored: stored_len(keys, bytes.len() as u64),
                bytes: bytes.len() as u64,
                sha256: sha,
            };
            plans.push(Plan::Bytes {
                kind,
                piece: piece.clone(),
                bytes: bytes.clone(),
            });
            Named {
                name: name.clone(),
                piece,
            }
        })
        .collect()
}

/// What to put, and the manifest that says it is whole once it is there
/// (its number and time are the writer's, last), and the stored files the
/// plan puts (for the proof after the backup to read).
///
/// `distrust` holds the files a proof found damaged: each is written
/// again under a new generation and named so.
fn plan(
    set: &Set,
    prev: Option<&Manifest>,
    cut: &Cut,
    distrust: &BTreeSet<String>,
    ctx: &Ctx,
) -> Result<(Manifest, Vec<Plan>), BackupError> {
    let keys = set.keys.as_ref();
    let mut plans = Vec::new();
    if let Some(p) = prev {
        if p.book != cut.book() {
            return Err(BackupError::Elsewhere);
        }
        if p.journal.bytes > cut.journal_len {
            // A longer history: of these books (the live journal is the
            // beginning of it), or of others.
            let (live, _, _) = hash_range(&cut.journal_path(), cut.journal_len, 0, ctx)?;
            return Err(
                if live_is_prefix_of(set, &p.journal, cut.journal_len, &live)? {
                    BackupError::Ahead(p.head.seq, cut.head().0)
                } else {
                    BackupError::Elsewhere
                },
            );
        }
    }
    let (journal, put) = grow(
        &cut.journal_path(),
        cut.journal_len,
        prev.map(|p| &p.journal),
        Kind::Journal,
        true,
        keys,
        distrust,
        ctx,
    )?;
    plans.extend(put);
    let (access, put) = grow(
        &cut.access_path(),
        cut.access_len,
        prev.map(|p| &p.access),
        Kind::Access,
        false,
        keys,
        distrust,
        ctx,
    )?;
    plans.extend(put);

    // The index of stored files: the new ones, as a segment. An index that
    // can't be read whole (a segment of it is damaged) is begun again from
    // the stored files the books have now.
    ctx.phase("files");
    let live = cut.stored_files(ctx)?;
    let mut restart = false;
    let known: Vec<IndexLine> = match prev {
        Some(p) => {
            if p.files.segments.iter().any(|s| distrust.contains(&s.file)) {
                restart = true;
                Vec::new()
            } else {
                match set.index(p) {
                    Ok(k) => k,
                    Err(BackupError::Damaged(_)) => {
                        restart = true;
                        Vec::new()
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        None => Vec::new(),
    };
    let known_set: BTreeMap<&str, u64> =
        known.iter().map(|l| (l.sha256.as_str(), l.bytes)).collect();
    let new: Vec<IndexLine> = live
        .iter()
        .filter(|l| !known_set.contains_key(l.sha256.as_str()))
        .cloned()
        .collect();
    // Stored files found damaged: written again, named by their hash and
    // generation (the index doesn't say where; the reader looks).
    for l in &live {
        let path = piece_path(keys, Kind::File, 0, &l.sha256);
        if distrust.contains(&path) && known_set.contains_key(l.sha256.as_str()) {
            plans.push(Plan::File {
                path: cut.blob_path(&l.sha256),
                piece: Piece {
                    file: fresh_path(&path, distrust),
                    stored: stored_len(keys, l.bytes),
                    bytes: l.bytes,
                    sha256: l.sha256.clone(),
                },
            });
        }
    }
    let (files, file_count, file_bytes) = match (prev, new.is_empty() && !restart) {
        (_, true) => prev
            .map(|p| (p.files.clone(), p.file_count, p.file_bytes))
            .unwrap_or_default(),
        _ => {
            let mut base = if restart {
                Series::default()
            } else {
                prev.map(|p| p.files.clone()).unwrap_or_default()
            };
            base.keep = 0;
            let mut union: Vec<IndexLine> = known.clone();
            union.extend(new.iter().cloned());
            let count = union.len() as u64;
            let total = union.iter().fold(0u64, |a, l| a.saturating_add(l.bytes));
            let text = index_text(&new);
            let seg_sha = sha_hex(&text);
            let piece = Piece {
                file: fresh_path(
                    &piece_path(keys, Kind::Index, base.segments.len(), &seg_sha),
                    distrust,
                ),
                stored: stored_len(keys, text.len() as u64),
                bytes: text.len() as u64,
                sha256: seg_sha,
            };
            let mut segments = base.segments.clone();
            segments.push(piece.clone());
            // The whole index's hash: the segments one after another.
            let mut all_text = VecSinkText::default();
            if !base.segments.is_empty() {
                set.read_series(&base, Kind::Index, &mut all_text)?;
            }
            all_text.0.extend_from_slice(&text);
            plans.push(Plan::Bytes {
                kind: Kind::Index,
                piece,
                bytes: text,
            });
            for l in &new {
                plans.push(Plan::File {
                    path: cut.blob_path(&l.sha256),
                    piece: Piece {
                        file: fresh_path(&piece_path(keys, Kind::File, 0, &l.sha256), distrust),
                        stored: stored_len(keys, l.bytes),
                        bytes: l.bytes,
                        sha256: l.sha256.clone(),
                    },
                });
            }
            (
                Series {
                    bytes: all_text.0.len() as u64,
                    sha256: sha_hex(&all_text.0),
                    segments,
                    keep: 0,
                },
                count,
                total,
            )
        }
    };
    let vault = named_pieces(&cut.vault, Kind::Vault, keys, distrust, &mut plans);
    let keyed = named_pieces(&cut.keys, Kind::Key, keys, distrust, &mut plans);
    let manifest = Manifest {
        format: FORMAT.into(),
        version: VERSION,
        book: cut.book().to_string(),
        number: 0,
        written: String::new(),
        head: Head {
            seq: cut.head().0,
            hash: cut.head().1.clone(),
        },
        journal,
        access,
        files,
        file_count,
        file_bytes,
        vault,
        keys: keyed,
        base: None,
        depth: 0,
        sha: String::new(),
    };
    Ok((manifest, plans))
}

/// Text gathered as a series is read.
#[derive(Default)]
struct VecSinkText(Vec<u8>);

impl Sink for VecSinkText {
    fn put(&mut self, chunk: &[u8]) -> std::io::Result<()> {
        self.0.extend_from_slice(chunk);
        Ok(())
    }
    fn mark(&mut self) {}
    fn rewind(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The manifest as it is kept: when `prev` (whole, as read) is a link in a
/// chain that is not yet [`DELTA_CHAIN`] long, built on it, naming only
/// what each series adds to its segments. Otherwise it names everything.
fn kept_form(full: &Manifest, prev: Option<&Manifest>) -> Manifest {
    let mut m = full.clone();
    let Some(p) = prev.filter(|p| p.depth + 1 < DELTA_CHAIN && !p.sha.is_empty()) else {
        return m;
    };
    m.base = Some(BaseRef {
        number: p.number,
        sha256: p.sha.clone(),
    });
    m.depth = p.depth + 1;
    for (mine, theirs) in [
        (&mut m.journal, &p.journal),
        (&mut m.access, &p.access),
        (&mut m.files, &p.files),
    ] {
        let keep = mine
            .segments
            .iter()
            .zip(&theirs.segments)
            .take_while(|(a, b)| a == b)
            .count();
        mine.segments = mine.segments.split_off(keep);
        mine.keep = keep;
    }
    m
}

/// The read-me, and for a sealed set its header: put once.
fn ensure_markers(set: &Set) -> Result<(), BackupError> {
    set.io.put(READ_ME, PutKind::Once, &mut |w| {
        w.write_all(READ_ME_TEXT.as_bytes())
    })?;
    if let Some(keys) = &set.keys {
        let header = keys.header();
        let bytes = header.encode();
        let put = set
            .io
            .put(HEADER, PutKind::Once, &mut |w| w.write_all(&bytes))?;
        if put == Put::Existed {
            // Another's header there already: ours only if it is the same
            // key (the salt it was derived with).
            let mut raw = Vec::new();
            set.io
                .read_with(HEADER, &mut |r| r.read_to_end(&mut raw).map(|_| ()))?;
            let theirs = super::seal::Header::decode(&raw)?;
            if theirs.kdf != header.kdf {
                return Err(BackupError::Elsewhere);
            }
        }
    }
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Put the manifest `full` (its number and time are filled in here) as the
/// next number in the set, built on `prev`. The number it took, or None
/// when another program took the number first.
fn put_manifest(
    set: &Set,
    full: &mut Manifest,
    prev: Option<&Manifest>,
    next: u64,
    ctx: &Ctx,
) -> Result<Option<u64>, BackupError> {
    full.number = next;
    full.written = now();
    let kept = kept_form(full, prev);
    let mut bytes = encode_manifest(&kept);
    if let Some(k) = &set.keys {
        bytes = k.seal_bytes(&bytes, &super::seal::manifest_aad(next));
    }
    ctx.phase("manifest");
    ctx.check("backing up")?;
    let put = set
        .io
        .put(
            &manifest_path(next, set.sealed()),
            PutKind::Manifest,
            &mut |w| w.write_all(&bytes),
        )
        .map_err(uncarry)?;
    Ok((put == Put::Created).then_some(next))
}

/// A folder for the proof's scratch books: private, in the book's own
/// `ops/`, named for this call.
fn proof_scratch(cut: &Cut) -> Result<PathBuf, BackupError> {
    let ops = crate::root::private_book_folder(cut.dir(), crate::ops::OPS_DIR)
        .map_err(|e| BackupError::Damaged(e.to_string()))?;
    let mut r = [0u8; 6];
    let _ = getrandom::getrandom(&mut r);
    Ok(ops.join(format!("proof-{}", hex::encode(r))))
}

/// The most times a backup is written again to repair what its own proof
/// found damaged.
const REPAIR_ROUNDS: usize = 2;

/// Prove the manifest `number` just put, and repair what the proof finds
/// damaged (a piece already there under the name, taken as the piece, that
/// is not): written again under a new generation, a new manifest naming
/// it, proven again. What the proof found is in `repaired`.
fn prove_and_repair(
    set: &Set,
    cut: &Cut,
    mut number: u64,
    mut added: BTreeSet<String>,
    ctx: &Ctx,
) -> Result<Written, BackupError> {
    let mut distrust: BTreeSet<String> = BTreeSet::new();
    let mut repaired: Vec<String> = Vec::new();
    for round in 0..=REPAIR_ROUNDS {
        ctx.phase("proof");
        ctx.check("proving the backup")?;
        let proof = super::read::prove_written(set, number, &added, &proof_scratch(cut)?)?;
        if proof.damaged.is_empty() {
            return Ok(Written {
                number,
                head: cut.head().clone(),
                new: true,
                repaired,
                files_proven: proof.files_read,
            });
        }
        for d in &proof.damaged {
            if !repaired.contains(&d.file) {
                repaired.push(d.file.clone());
            }
            distrust.insert(d.file.clone());
        }
        if round == REPAIR_ROUNDS {
            break;
        }
        // Written again beside the damaged ones, and a manifest naming
        // them (the stored files are found by their hash and generation).
        ctx.phase("repair");
        let prev = set.manifest(number)?;
        let (mut manifest, plans) = plan(set, Some(&prev), cut, &distrust, ctx)?;
        added = BTreeSet::new();
        for p in &plans {
            ctx.check("backing up")?;
            if execute(set, p, ctx)? == Put::Created
                && let Plan::File { piece, .. } = p
            {
                added.insert(piece.sha256.clone());
            }
        }
        if !manifest.same_content(&prev) {
            let mut taken = None;
            for _ in 0..ATTEMPTS {
                let next = set.numbers()?.last().map_or(1, |n| n + 1);
                taken = put_manifest(set, &mut manifest, Some(&prev), next, ctx)?;
                if taken.is_some() {
                    break;
                }
            }
            number = taken.ok_or(BackupError::Busy)?;
        }
    }
    Err(BackupError::Damaged(
        "a piece of the backup was found damaged and could not be made whole again from the books"
            .into(),
    ))
}

/// Write the cut into the set (INVARIANTS 2, 4, 5); with `prove`, then
/// read the manifest back and prove it, repairing what is damaged.
pub(crate) fn write_into_set(
    set: &Set,
    cut: &Cut,
    ctx: &Ctx,
    prove: bool,
) -> Result<Written, BackupError> {
    ensure_markers(set)?;
    for _ in 0..ATTEMPTS {
        ctx.check("backing up")?;
        let numbers = set.numbers()?;
        let prev = set.newest_whole()?.map(|(m, _)| m);
        let next = numbers.last().map_or(1, |n| n + 1);
        ctx.phase("plan");
        let (mut manifest, plans) = plan(set, prev.as_ref(), cut, &BTreeSet::new(), ctx)?;
        if let Some(p) = &prev
            && manifest.same_content(p)
        {
            return Ok(Written {
                number: p.number,
                head: cut.head().clone(),
                new: false,
                repaired: Vec::new(),
                files_proven: 0,
            });
        }
        ctx.phase("pieces");
        let mut added = BTreeSet::new();
        for p in &plans {
            ctx.check("backing up")?;
            if execute(set, p, ctx)? == Put::Created
                && let Plan::File { piece, .. } = p
            {
                added.insert(piece.sha256.clone());
            }
        }
        match put_manifest(set, &mut manifest, prev.as_ref(), next, ctx)? {
            Some(number) if prove => return prove_and_repair(set, cut, number, added, ctx),
            Some(number) => {
                return Ok(Written {
                    number,
                    head: cut.head().clone(),
                    new: true,
                    repaired: Vec::new(),
                    files_proven: 0,
                });
            }
            // Another program took the number: look again.
            None => continue,
        }
    }
    Err(BackupError::Busy)
}

/// Back the cut up to the destination (INVARIANTS 1 to 5). The destination
/// is made only when `may_create` (it never took a backup of these
/// books): one that took backups and is gone is a disk not connected,
/// never made again where the disk should be.
pub fn write_destination(
    dest: &Destination,
    cut: &Cut,
    may_create: bool,
    ctx: &Ctx,
) -> Result<Written, BackupError> {
    write_destination_with(
        dest,
        cut,
        Options {
            may_create,
            new_set: false,
        },
        ctx,
    )
}

/// [`write_destination`], with what is asked of the destination's sets
/// ([`Options`]).
pub fn write_destination_with(
    dest: &Destination,
    cut: &Cut,
    opts: Options,
    ctx: &Ctx,
) -> Result<Written, BackupError> {
    if !dest.folder.is_dir() {
        return Err(BackupError::NoFolder);
    }
    let sets = sets_of(dest, cut.book(), ctx)?;
    let set = if opts.new_set && !sets.is_empty() {
        // A new set beside the old one, which is never touched: the old
        // folder's name and the next number.
        let (name, _) = current_of(sets).expect("there is one");
        let (base, number) = split_set_suffix(&name);
        let next = format!("{base}.{}", number + 1);
        Set::dir(
            dest.folder.join(next),
            dest.key.as_ref().map(|k| k.keys()),
            ctx,
        )
    } else {
        match current_of(sets) {
            Some((_, set)) => set,
            None if !opts.may_create => return Err(BackupError::NoBackupThere),
            None => match &dest.key {
                None => Set::dir(dest.folder.join(set_name_for(cut.book())), None, ctx),
                Some(key) => {
                    let mut id = [0u8; 8];
                    getrandom::getrandom(&mut id)
                        .map_err(|e| BackupError::Io(std::io::Error::from(e).kind()))?;
                    let name = format!("{SEALED_PREFIX}{}", hex::encode(id));
                    Set::dir(dest.folder.join(name), Some(key.keys()), ctx)
                }
            },
        }
    };
    write_into_set(&set, cut, ctx, true)
}

/// Write the cut to the book's own local backup (`local-backup/` in its
/// folder, the same form, plain): what a format upgrade proves before it
/// begins. The caller holds the store's lock.
pub fn write_local(book_dir: &Path, cut: &Cut, ctx: &Ctx) -> Result<Written, BackupError> {
    let folder = crate::root::private_book_folder(book_dir, super::LOCAL_DIR)
        .map_err(|e| BackupError::Damaged(e.to_string()))?;
    write_destination(&Destination { folder, key: None }, cut, true, ctx)
}
