//! Writing a backup (INVARIANTS 1, 2, 4, 5 of the module): what is new
//! since the destination's newest whole manifest, as pieces, then the
//! manifest.

use super::cut::Cut;
use super::form::*;
use super::io::{Fill, Put, PutKind, carry, uncarry};
use super::read::{Hashed, Set};
use super::seal::{BackupKey, Keys, piece_aad};
use super::{BackupError, Ctx, READ_ME_TEXT};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
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
}

/// The folder a set of books has in a destination when it is not sealed:
/// named for the books.
pub fn set_name_for(book: &str) -> String {
    book.to_string()
}

/// The set of `book` in the destination: for a plain one, the folder named
/// for the books; for a sealed one, the folder whose header this key opens
/// and whose backups are these books' (or none yet). None when it is not
/// there.
pub(crate) fn locate(
    dest: &Destination,
    book: &str,
    ctx: &Ctx,
) -> Result<Option<Set>, BackupError> {
    match &dest.key {
        None => {
            let dir = dest.folder.join(set_name_for(book));
            Ok(dir.is_dir().then(|| Set::dir(dir, None, ctx)))
        }
        Some(key) => {
            let keys = key.keys();
            let Ok(entries) = std::fs::read_dir(&dest.folder) else {
                return Ok(None);
            };
            let mut names: Vec<String> = entries
                .flatten()
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|n| n.starts_with(SEALED_PREFIX))
                .collect();
            names.sort();
            for name in names {
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
                    _ => return Ok(Some(set)),
                }
            }
            Ok(None)
        }
    }
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

/// Grow the series of the live file `path` (`len` bytes of whole lines)
/// from `base`, the destination's: a new segment of the bytes it lacks.
/// The journal's must continue the destination's exactly (`strict`:
/// otherwise the books are not the same); an access log that does not is
/// begun again whole.
fn grow(
    path: &Path,
    len: u64,
    base: Option<&Series>,
    kind: Kind,
    strict: bool,
    keys: Option<&Keys>,
    ctx: &Ctx,
) -> Result<(Series, Option<Plan>), BackupError> {
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
    if base.bytes == len {
        return Ok((base.clone(), None));
    }
    let consolidate = base.segments.len() >= CONSOLIDATE_AT;
    // `after` was hashed from the base's end: with a different base, from
    // the whole.
    let (from, seg_sha, ordinal, mut segments) = if consolidate || base.bytes == 0 {
        (0, all.clone(), 0, Vec::new())
    } else {
        (
            base.bytes,
            after,
            base.segments.len(),
            base.segments.clone(),
        )
    };
    let bytes = len - from;
    let piece = Piece {
        file: piece_path(keys, kind, ordinal, &seg_sha),
        stored: stored_len(keys, bytes),
        bytes,
        sha256: seg_sha,
    };
    segments.push(piece.clone());
    Ok((
        Series {
            bytes: len,
            sha256: all,
            segments,
        },
        Some(Plan::Range {
            path: path.to_path_buf(),
            from,
            to: len,
            kind,
            piece,
        }),
    ))
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
) -> Result<(), BackupError> {
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
        .map_err(uncarry)?;
    Ok(())
}

fn execute(set: &Set, plan: &Plan, ctx: &Ctx) -> Result<(), BackupError> {
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
    plans: &mut Vec<Plan>,
) -> Vec<Named> {
    files
        .iter()
        .map(|(name, bytes)| {
            let sha = sha_hex(bytes);
            let piece = Piece {
                file: piece_path(keys, kind, 0, &sha),
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
/// (its number and time are the writer's, last).
fn plan(
    set: &Set,
    prev: Option<&Manifest>,
    cut: &Cut,
    ctx: &Ctx,
) -> Result<(Manifest, Vec<Plan>), BackupError> {
    let keys = set.keys.as_ref();
    let mut plans = Vec::new();
    if let Some(p) = prev {
        if p.book != cut.book() {
            return Err(BackupError::Elsewhere);
        }
        if p.journal.bytes > cut.journal_len {
            return Err(BackupError::Ahead(p.head.seq, cut.head().0));
        }
    }
    let (journal, put) = grow(
        &cut.journal_path(),
        cut.journal_len,
        prev.map(|p| &p.journal),
        Kind::Journal,
        true,
        keys,
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
        ctx,
    )?;
    plans.extend(put);

    // The index of stored files: the new ones, as a segment.
    ctx.phase("files");
    let live = cut.stored_files(ctx)?;
    let known: Vec<IndexLine> = match prev {
        Some(p) => set.index(p)?,
        None => Vec::new(),
    };
    let known_set: BTreeMap<&str, u64> =
        known.iter().map(|l| (l.sha256.as_str(), l.bytes)).collect();
    let new: Vec<IndexLine> = live
        .into_iter()
        .filter(|l| !known_set.contains_key(l.sha256.as_str()))
        .collect();
    let (files, file_count, file_bytes) = match (prev, new.is_empty()) {
        (_, true) => prev
            .map(|p| (p.files.clone(), p.file_count, p.file_bytes))
            .unwrap_or_default(),
        _ => {
            let base = prev.map(|p| &p.files).cloned().unwrap_or_default();
            let consolidate = base.segments.len() >= CONSOLIDATE_AT;
            let mut union: Vec<IndexLine> = known.clone();
            union.extend(new.iter().cloned());
            let count = union.len() as u64;
            let total = union.iter().fold(0u64, |a, l| a.saturating_add(l.bytes));
            let seg_lines = if consolidate { &union } else { &new };
            let text = index_text(seg_lines);
            let seg_sha = sha_hex(&text);
            let ordinal = if consolidate { 0 } else { base.segments.len() };
            let piece = Piece {
                file: piece_path(keys, Kind::Index, ordinal, &seg_sha),
                stored: stored_len(keys, text.len() as u64),
                bytes: text.len() as u64,
                sha256: seg_sha,
            };
            let mut segments = if consolidate {
                Vec::new()
            } else {
                base.segments.clone()
            };
            segments.push(piece.clone());
            // The whole index's hash: the segments one after another.
            let mut all_text = Vec::new();
            if !consolidate {
                set.read_series(&base, Kind::Index, &mut |c| {
                    all_text.extend_from_slice(c);
                    Ok(())
                })?;
            }
            all_text.extend_from_slice(&text);
            plans.push(Plan::Bytes {
                kind: Kind::Index,
                piece,
                bytes: text,
            });
            for l in &new {
                plans.push(Plan::File {
                    path: cut.blob_path(&l.sha256),
                    piece: Piece {
                        file: piece_path(keys, Kind::File, 0, &l.sha256),
                        stored: stored_len(keys, l.bytes),
                        bytes: l.bytes,
                        sha256: l.sha256.clone(),
                    },
                });
            }
            (
                Series {
                    bytes: all_text.len() as u64,
                    sha256: sha_hex(&all_text),
                    segments,
                },
                count,
                total,
            )
        }
    };
    let vault = named_pieces(&cut.vault, Kind::Vault, keys, &mut plans);
    let keyed = named_pieces(&cut.keys, Kind::Key, keys, &mut plans);
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
    };
    Ok((manifest, plans))
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

/// Write the cut into the set (INVARIANTS 2, 4, 5).
pub(crate) fn write_into_set(set: &Set, cut: &Cut, ctx: &Ctx) -> Result<Written, BackupError> {
    ensure_markers(set)?;
    for _ in 0..ATTEMPTS {
        ctx.check("backing up")?;
        let numbers = set.numbers()?;
        let prev = set.newest_whole()?.map(|(m, _)| m);
        let next = numbers.last().map_or(1, |n| n + 1);
        ctx.phase("plan");
        let (mut manifest, plans) = plan(set, prev.as_ref(), cut, ctx)?;
        if let Some(p) = &prev
            && manifest.same_content(p)
        {
            return Ok(Written {
                number: p.number,
                head: cut.head().clone(),
                new: false,
            });
        }
        ctx.phase("pieces");
        for p in &plans {
            ctx.check("backing up")?;
            execute(set, p, ctx)?;
        }
        manifest.number = next;
        manifest.written = now();
        let mut bytes = encode_manifest(&manifest);
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
        match put {
            Put::Created => {
                return Ok(Written {
                    number: next,
                    head: cut.head().clone(),
                    new: true,
                });
            }
            // Another program took the number: look again.
            Put::Existed => continue,
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
    if !dest.folder.is_dir() {
        return Err(BackupError::NoFolder);
    }
    let set = match locate(dest, cut.book(), ctx)? {
        Some(set) => set,
        None if !may_create => return Err(BackupError::NoBackupThere),
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
    };
    write_into_set(&set, cut, ctx)
}

/// Write the cut to the book's own local backup (`local-backup/` in its
/// folder, the same form, plain): what a format upgrade proves before it
/// begins. The caller holds the store's lock.
pub fn write_local(book_dir: &Path, cut: &Cut, ctx: &Ctx) -> Result<Written, BackupError> {
    let folder = crate::root::private_book_folder(book_dir, super::LOCAL_DIR)
        .map_err(|e| BackupError::Damaged(e.to_string()))?;
    write_destination(&Destination { folder, key: None }, cut, true, ctx)
}
