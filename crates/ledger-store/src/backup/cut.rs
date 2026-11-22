//! A cut of the books: what a backup is made from, taken in one look
//! under the store's lock, and read from the book's folder after it.
//!
//! The journal is append-only and a stored file never changes once it is
//! there, so what the lock has to fix is small: how long the journal is
//! (and that its last line is the head), how much of the access log is
//! whole lines, and the vault and the keys, which can change. Everything
//! else is read afterwards, outside the lock, and cannot move: the bytes
//! of the journal before the cut's length are the same bytes a moment
//! later.

use super::form::{IndexLine, is_plain_name, is_sha};
use super::{BackupError, Ctx};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// A cut of one book. Take it with the store's lock held ([`Cut::take`]).
#[derive(Debug, Clone)]
pub struct Cut {
    book: String,
    dir: PathBuf,
    head: (u64, String),
    pub(crate) journal_len: u64,
    pub(crate) access_len: u64,
    pub(crate) vault: Vec<(String, Vec<u8>)>,
    pub(crate) keys: Vec<(String, Vec<u8>)>,
}

/// The bytes of the last line of `path`, which is `len` long and ends in a
/// newline, read from the end.
fn last_line(path: &Path, len: u64) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    let mut tail: Vec<u8> = Vec::new();
    let mut at = len;
    // Without the final newline; looking back for the one before it.
    loop {
        let step = at.min(64 * 1024);
        at -= step;
        f.seek(SeekFrom::Start(at))?;
        let mut chunk = vec![0u8; step as usize];
        f.read_exact(&mut chunk)?;
        chunk.extend_from_slice(&tail);
        tail = chunk;
        if tail.last() != Some(&b'\n') {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the journal does not end in a whole line",
            ));
        }
        let body = &tail[..tail.len() - 1];
        if let Some(i) = body.iter().rposition(|b| *b == b'\n') {
            return Ok(body[i + 1..].to_vec());
        }
        if at == 0 {
            return Ok(body.to_vec());
        }
    }
}

/// How much of `path`, `len` long, is whole lines.
fn whole_lines(path: &Path, len: u64) -> std::io::Result<u64> {
    let mut f = std::fs::File::open(path)?;
    let mut at = len;
    while at > 0 {
        let step = at.min(64 * 1024);
        at -= step;
        f.seek(SeekFrom::Start(at))?;
        let mut chunk = vec![0u8; step as usize];
        f.read_exact(&mut chunk)?;
        if let Some(i) = chunk.iter().rposition(|b| *b == b'\n') {
            return Ok(at + i as u64 + 1);
        }
    }
    Ok(0)
}

/// The plain files of a flat folder of a book, by name, with their bytes.
fn flat_files(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, BackupError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = std::fs::symlink_metadata(entry.path())?;
        if !meta.is_file() || !is_plain_name(&name) {
            return Err(BackupError::Damaged(format!(
                "a folder of the books holds something no book keeps ({name:?})"
            )));
        }
        out.push((name, std::fs::read(entry.path())?));
    }
    out.sort();
    Ok(out)
}

impl Cut {
    /// Take a cut of the book `book` kept in `dir`, whose journal's head
    /// is `head`. The caller holds the store's lock, so the journal is
    /// whole lines and the vault matches it.
    pub fn take(book: &str, dir: &Path, head: (u64, String)) -> Result<Cut, BackupError> {
        let journal = dir.join(crate::JOURNAL_FILE);
        let journal_len = match std::fs::metadata(&journal) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e.into()),
        };
        if head.0 == 0 {
            if journal_len != 0 {
                return Err(BackupError::Damaged(
                    "the journal has lines the books don't count".into(),
                ));
            }
        } else {
            let line = last_line(&journal, journal_len)?;
            if hex::encode(Sha256::digest(&line)) != head.1 {
                return Err(BackupError::Damaged(
                    "the journal's last line isn't the books' head".into(),
                ));
            }
        }
        let access = dir.join(crate::access::ACCESS_FILE);
        let access_len = match std::fs::metadata(&access) {
            Ok(m) => whole_lines(&access, m.len())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e.into()),
        };
        Ok(Cut {
            book: book.to_string(),
            dir: dir.to_path_buf(),
            head,
            journal_len,
            access_len,
            vault: flat_files(&dir.join(crate::root::VAULT_DIR))?,
            keys: flat_files(&dir.join(crate::root::KEYS_DIR))?,
        })
    }

    /// The books' id.
    pub fn book(&self) -> &str {
        &self.book
    }

    /// The journal's head at the cut: seq and hash.
    pub fn head(&self) -> &(u64, String) {
        &self.head
    }

    pub(crate) fn journal_path(&self) -> PathBuf {
        self.dir.join(crate::JOURNAL_FILE)
    }

    pub(crate) fn access_path(&self) -> PathBuf {
        self.dir.join(crate::access::ACCESS_FILE)
    }

    /// The stored files, by hash, with their lengths: those named as this
    /// program names them (`blobs/ab/cd/<sha256>`), nothing else.
    pub(crate) fn stored_files(&self, ctx: &Ctx) -> Result<Vec<IndexLine>, BackupError> {
        let root = self.dir.join(crate::store::BLOBS_DIR);
        let mut out = Vec::new();
        let level = |p: &Path| -> std::io::Result<Vec<std::fs::DirEntry>> {
            match std::fs::read_dir(p) {
                Ok(e) => e.collect(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
                Err(e) => Err(e),
            }
        };
        for a in level(&root)? {
            ctx.check("listing the stored files")?;
            let an = a.file_name().to_string_lossy().into_owned();
            if an.len() != 2 || !a.path().is_dir() {
                continue;
            }
            for b in level(&a.path())? {
                let bn = b.file_name().to_string_lossy().into_owned();
                if bn.len() != 2 || !b.path().is_dir() {
                    continue;
                }
                for f in level(&b.path())? {
                    let name = f.file_name().to_string_lossy().into_owned();
                    let meta = std::fs::symlink_metadata(f.path())?;
                    if meta.is_file()
                        && is_sha(&name)
                        && name.starts_with(&an)
                        && name[2..].starts_with(&bn)
                    {
                        out.push(IndexLine {
                            sha256: name,
                            bytes: meta.len(),
                        });
                    }
                }
            }
        }
        out.sort_by(|x, y| x.sha256.cmp(&y.sha256));
        Ok(out)
    }

    /// Where a stored file is in the book.
    pub(crate) fn blob_path(&self, sha: &str) -> PathBuf {
        self.dir
            .join(crate::store::BLOBS_DIR)
            .join(&sha[0..2])
            .join(&sha[2..4])
            .join(sha)
    }
}
