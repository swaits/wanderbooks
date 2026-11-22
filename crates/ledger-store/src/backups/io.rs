//! Where a backup's files live, behind one seam: a folder on a disk
//! ([`DirSet`]), or the one-file form ([`crate::backups::single`]).
//!
//! The only operations a backup needs of a place are four: list a folder,
//! ask how long a file is, read a file, and put a file there that was not
//! there. Nothing deletes, nothing replaces (INVARIANT 1 of the module).
//!
//! How a file is put (the whole of the argument for two writers):
//!
//! * It is written under a name no reader looks at (`.wb-tmp-…`, in the
//!   folder it is for), synced, and then given its real name by a
//!   *hard link*, which every disk and every network share this program
//!   meets either does atomically and exclusively (the name is made, or
//!   it exists already: Unix, NTFS, SMB, NFS) or refuses outright (FAT and
//!   exFAT disks, some sync folders). A rename is not used for a manifest
//!   because on Windows a rename replaces what is at the new name, and on
//!   some network shares it does so without telling.
//! * Where hard links are refused, a *piece* (a file named by what is in
//!   it) is moved into place by a rename when its name is free: two writers
//!   racing for one piece write the same bytes, so whichever lands, the
//!   piece is right. A *manifest* is created exclusively under its own name
//!   (`create_new`) and written in place: a crash in the middle leaves a
//!   short manifest, which no reader accepts (it carries its own check),
//!   and its number is simply spent.
//! * A file that is there already is never touched: a piece is taken as
//!   the piece (the next proof re-hashes it); a manifest number taken is
//!   reported to the writer, which looks again and tries the next.
//!
//! The scratch names are the one thing this code removes from a
//! destination, and only the file it made itself a moment before; one left
//! by a process that died stays (a person may delete `.wb-tmp-*` by hand).

use super::{BackupError, Ctx};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// What kind of file is being put: how an existing file at the name is met.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PutKind {
    /// Named by what is in it: if it is there, it is the one.
    Piece,
    /// One for the whole set, written once (the read-me, the header): if
    /// it is there, it stays.
    Once,
    /// A manifest: the name is a number, taken exclusively. The decision
    /// point of a backup.
    Manifest,
}

/// How a put came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Put {
    /// This call made the file.
    Created,
    /// It was there already: untouched.
    Existed,
}

/// Fill a file: the callback writes its bytes.
pub(crate) type Fill<'a> = &'a mut dyn FnMut(&mut dyn Write) -> std::io::Result<()>;

/// Where a set's files live.
pub(crate) trait SetIo: Send + Sync {
    /// The names directly in `dir` (a path in the set, `/` between parts),
    /// none if it is not there.
    fn list(&self, dir: &str) -> std::io::Result<Vec<String>>;
    /// How long the file at `rel` is, None if it is not there.
    fn len(&self, rel: &str) -> std::io::Result<Option<u64>>;
    /// Read the file at `rel` through `f`.
    fn read_with(
        &self,
        rel: &str,
        f: &mut dyn FnMut(&mut dyn Read) -> std::io::Result<()>,
    ) -> std::io::Result<()>;
    /// Put a file at `rel` that is not there ([`PutKind`]).
    fn put(&self, rel: &str, kind: PutKind, fill: Fill<'_>) -> std::io::Result<Put>;
}

/// A backup error carried through an `io::Error` (a callback that has to
/// stop the file it is writing).
#[derive(Debug)]
struct Carried(BackupError);

impl std::fmt::Display for Carried {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for Carried {}

/// `e` as an `io::Error` a callback can return.
pub(crate) fn carry(e: BackupError) -> std::io::Error {
    std::io::Error::other(Carried(e))
}

/// A backup error back out of an `io::Error` (one that was carried, or the
/// disk's own).
pub(crate) fn uncarry(e: std::io::Error) -> BackupError {
    match e.get_ref().and_then(|i| i.downcast_ref::<Carried>()) {
        Some(c) => c.0.clone(),
        None => BackupError::Io(e.kind()),
    }
}

/// A set that is a folder on a disk.
pub(crate) struct DirSet {
    root: PathBuf,
    ctx: Ctx,
}

impl DirSet {
    /// The set in the folder `root` (which need not exist yet).
    pub(crate) fn new(root: PathBuf, ctx: Ctx) -> Self {
        Self { root, ctx }
    }

    fn path(&self, rel: &str) -> PathBuf {
        let mut p = self.root.clone();
        for part in rel.split('/').filter(|s| !s.is_empty()) {
            p.push(part);
        }
        p
    }
}

/// A name for a scratch file: unique to this process and moment.
fn scratch_name() -> String {
    let mut r = [0u8; 6];
    let _ = getrandom::getrandom(&mut r);
    format!(
        ".wb-tmp-{}-{}",
        std::process::id(),
        r.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

/// Make a folder's entries durable (Unix; NTFS journals its own).
pub(crate) fn sync_dir(path: &Path) {
    #[cfg(unix)]
    if let Ok(dir) = std::fs::File::open(path) {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Whether a link could not be made because the disk does not make them
/// (rather than because the name is taken).
fn link_unsupported(e: &std::io::Error) -> bool {
    e.kind() != std::io::ErrorKind::AlreadyExists
}

impl SetIo for DirSet {
    fn list(&self, dir: &str) -> std::io::Result<Vec<String>> {
        match std::fs::read_dir(self.path(dir)) {
            Ok(entries) => {
                let mut out = Vec::new();
                for e in entries {
                    if let Some(n) = e?.file_name().to_str() {
                        out.push(n.to_string());
                    }
                }
                out.sort();
                Ok(out)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    fn len(&self, rel: &str) -> std::io::Result<Option<u64>> {
        match std::fs::symlink_metadata(self.path(rel)) {
            // A link is not a file of a backup: it could lead anywhere.
            Ok(m) if m.is_file() => Ok(Some(m.len())),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn read_with(
        &self,
        rel: &str,
        f: &mut dyn FnMut(&mut dyn Read) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        let path = self.path(rel);
        let meta = std::fs::symlink_metadata(&path)?;
        if !meta.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not a plain file",
            ));
        }
        let mut file = std::fs::File::open(path)?;
        f(&mut file)
    }

    fn put(&self, rel: &str, kind: PutKind, fill: Fill<'_>) -> std::io::Result<Put> {
        let path = self.path(rel);
        let parent = path.parent().unwrap_or(&self.root).to_path_buf();
        std::fs::create_dir_all(&parent)?;
        if kind != PutKind::Manifest && std::fs::symlink_metadata(&path).is_ok() {
            return Ok(Put::Existed);
        }
        let tmp = parent.join(scratch_name());
        self.ctx.point("create", rel).map_err(carry)?;
        let written = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            {
                let mut w = std::io::BufWriter::with_capacity(1 << 16, &mut file);
                fill(&mut w)?;
                w.flush()?;
            }
            file.sync_all()
        })();
        if let Err(e) = written {
            // Something of ours that failed on its own: its scratch file
            // goes (a death, which `point` reports, leaves it).
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        self.ctx.point("written", rel).map_err(carry)?;
        let put = match std::fs::hard_link(&tmp, &path) {
            Ok(()) => {
                let _ = std::fs::remove_file(&tmp);
                Put::Created
            }
            Err(e) if !link_unsupported(&e) => {
                let _ = std::fs::remove_file(&tmp);
                Put::Existed
            }
            Err(_) => {
                // A disk that makes no links.
                match kind {
                    PutKind::Manifest => {
                        let _ = std::fs::remove_file(&tmp);
                        match std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&path)
                        {
                            Ok(mut file) => {
                                {
                                    let mut w = std::io::BufWriter::new(&mut file);
                                    fill(&mut w)?;
                                    w.flush()?;
                                }
                                file.sync_all()?;
                                Put::Created
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Put::Existed,
                            Err(e) => return Err(e),
                        }
                    }
                    PutKind::Piece | PutKind::Once => {
                        if std::fs::symlink_metadata(&path).is_ok() {
                            let _ = std::fs::remove_file(&tmp);
                            Put::Existed
                        } else {
                            std::fs::rename(&tmp, &path)?;
                            Put::Created
                        }
                    }
                }
            }
        };
        sync_dir(&parent);
        self.ctx.point("published", rel).map_err(carry)?;
        Ok(put)
    }
}
