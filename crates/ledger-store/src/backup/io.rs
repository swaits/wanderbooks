//! Where a backup's files live, behind one seam: a folder on a disk
//! ([`DirSet`]), or the one-file form ([`crate::backup::single`]).
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
//! * Only an error that means "this disk makes no links" ([`link_unsupported`],
//!   named per platform) sends a put down the no-link path. Any other
//!   error is an error: it is never taken to mean that a rename, which
//!   replaces what is at the new name on POSIX and on Windows, will do.
//! * Where links are refused, the finished scratch file is moved into
//!   place by a move that *refuses to replace*: `renameat2` with
//!   `RENAME_NOREPLACE` on Linux, `renamex_np` with `RENAME_EXCL` on macOS,
//!   `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` on Windows
//!   ([`rename_new`]). Where even that is not offered (a file system that
//!   cannot say no), the last resort is exclusive create (`create_new`)
//!   under the real name and a copy of the bytes in: nothing is replaced,
//!   but a crash in the middle leaves a short file under the real name,
//!   which the next proof finds (it re-hashes) and repairs by writing the
//!   piece again under a new generation (write.rs); a short manifest is
//!   accepted by no reader (it carries its own check), and its number is
//!   spent.
//! * A file that is there already is never touched: a piece is taken as
//!   the piece (the proof after the backup re-hashes it, and a piece found
//!   damaged is written again beside it); a manifest number taken is
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

    /// Whether the sync service's stand-in for the file at `rel` is there
    /// (`.name.icloud` beside where the file should be).
    fn stand_in_for(&self, rel: &str) -> bool {
        let path = self.path(rel);
        match (path.parent(), path.file_name().and_then(|n| n.to_str())) {
            (Some(dir), Some(name)) => dir.join(format!(".{name}.icloud")).exists(),
            _ => false,
        }
    }

    /// The finished file at `tmp` given its real name `path` on a disk
    /// that makes no links: by a move that refuses to replace, else by
    /// exclusive create and copy. The scratch file is gone after.
    fn place_without_links(&self, tmp: &Path, path: &Path) -> std::io::Result<Put> {
        let moved = if self.ctx.refuses_noreplace() {
            Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
        } else {
            rename_new(tmp, path)
        };
        match moved {
            Ok(()) => Ok(Put::Created),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(tmp);
                Ok(Put::Existed)
            }
            Err(e) if e.kind() == std::io::ErrorKind::Unsupported => {
                // The last resort: made exclusively under the real name,
                // and the finished scratch file copied into it.
                let mut file = match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                {
                    Ok(f) => f,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        let _ = std::fs::remove_file(tmp);
                        return Ok(Put::Existed);
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(tmp);
                        return Err(e);
                    }
                };
                let copied = (|| -> std::io::Result<()> {
                    let mut from = std::fs::File::open(tmp)?;
                    {
                        let mut w = std::io::BufWriter::with_capacity(1 << 16, &mut file);
                        std::io::copy(&mut from, &mut w)?;
                        w.flush()?;
                    }
                    file.sync_all()
                })();
                let _ = std::fs::remove_file(tmp);
                copied?;
                Ok(Put::Created)
            }
            Err(e) => {
                let _ = std::fs::remove_file(tmp);
                Err(e)
            }
        }
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

/// Whether Windows file attributes say the file is a placeholder of an
/// online-only service (OneDrive, Dropbox Smart Sync, iCloud for Windows):
/// `FILE_ATTRIBUTE_OFFLINE` (0x1000), `FILE_ATTRIBUTE_RECALL_ON_OPEN`
/// (0x40000) or `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` (0x400000). Reading
/// one makes Windows fetch it, which may take long or fail with no
/// network, so it is reported as not downloaded, not read.
#[allow(dead_code)] // used where Windows attributes are read; tested everywhere
pub(crate) fn placeholder_attributes(attrs: u32) -> bool {
    attrs & (0x1000 | 0x40000 | 0x400000) != 0
}

/// Whether macOS file flags say the file is dataless (an iCloud or file
/// provider stand-in whose contents are not on this disk): `SF_DATALESS`.
#[allow(dead_code)] // used where macOS flags are read; tested everywhere
pub(crate) fn dataless_flags(flags: u32) -> bool {
    flags & 0x4000_0000 != 0
}

/// Whether the name is the stand-in a sync service leaves for a file it
/// has not downloaded (iCloud before macOS 14: `.name.icloud`).
pub(crate) fn is_stand_in_name(name: &str) -> bool {
    name.len() > ".x.icloud".len() - 1 && name.starts_with('.') && name.ends_with(".icloud")
}

/// Whether the file's metadata says it is online only. Where nothing can
/// tell (Linux, whatever the sync client), false: such a file reads as
/// what it is, and a short or missing one is found by the proof.
fn online_only(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        placeholder_attributes(meta.file_attributes())
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        dataless_flags(meta.st_flags())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = meta;
        false
    }
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

/// Whether a failed hard link means the disk does not make links (FAT,
/// exFAT, some network shares and sync folders), as against the name
/// being taken or the disk refusing for another reason. The errors that
/// mean it, by platform:
///
/// * Unix: `EPERM` (what Linux's vfat and exFAT answer), `ENOSYS`,
///   `EOPNOTSUPP` and `ENOTSUP` (network and FUSE file systems), and the
///   portable kind `Unsupported`.
/// * Windows: `ERROR_INVALID_FUNCTION` (1, FAT and exFAT), `ERROR_NOT_SUPPORTED`
///   (50), `ERROR_INVALID_PARAMETER` (87) and `ERROR_CALL_NOT_IMPLEMENTED`
///   (120).
pub(crate) fn link_unsupported(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::Unsupported {
        return true;
    }
    let Some(n) = e.raw_os_error() else {
        return false;
    };
    #[cfg(unix)]
    {
        n == libc::EPERM || n == libc::ENOSYS || n == libc::EOPNOTSUPP || n == libc::ENOTSUP
    }
    #[cfg(windows)]
    {
        matches!(n, 1 | 50 | 87 | 120)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = n;
        false
    }
}

/// Move `from` to `to`, which must not be there: never over something
/// already at `to` (that is `AlreadyExists`). `Unsupported` when the file
/// system cannot refuse to replace, which the caller meets by exclusive
/// create and copy.
fn rename_new(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;
        let f = std::ffi::CString::new(from.as_os_str().as_bytes())?;
        let t = std::ffi::CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: two NUL-terminated paths that live past the call.
        let rc = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                f.as_ptr(),
                libc::AT_FDCWD,
                t.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if rc == 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(n) if n == libc::EINVAL || n == libc::ENOSYS || n == libc::ENOTSUP => {
                Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
            }
            _ => Err(e),
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let f = std::ffi::CString::new(from.as_os_str().as_bytes())?;
        let t = std::ffi::CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: two NUL-terminated paths that live past the call.
        let rc = unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) };
        if rc == 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(n) if n == libc::ENOTSUP || n == libc::EINVAL || n == libc::ENOSYS => {
                Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
            }
            _ => Err(e),
        }
    }
    #[cfg(windows)]
    {
        // MoveFileExW without MOVEFILE_REPLACE_EXISTING refuses an
        // existing target itself.
        crate::winsec::rename_new(from, to)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (from, to);
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}

impl SetIo for DirSet {
    fn list(&self, dir: &str) -> std::io::Result<Vec<String>> {
        match std::fs::read_dir(self.path(dir)) {
            Ok(entries) => {
                let mut out = Vec::new();
                for e in entries {
                    if let Some(n) = e?.file_name().to_str() {
                        // A stand-in of a file not downloaded: it is
                        // there, though not as the file itself.
                        if is_stand_in_name(n) {
                            return Err(carry(BackupError::NotDownloaded));
                        }
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
        self.ctx.traced("len", rel);
        match std::fs::symlink_metadata(self.path(rel)) {
            // A link is not a file of a backup: it could lead anywhere.
            Ok(m) if m.is_file() && online_only(&m) => Err(carry(BackupError::NotDownloaded)),
            Ok(m) if m.is_file() => Ok(Some(m.len())),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if self.stand_in_for(rel) {
                    Err(carry(BackupError::NotDownloaded))
                } else {
                    Ok(None)
                }
            }
            Err(e) => Err(e),
        }
    }

    fn read_with(
        &self,
        rel: &str,
        f: &mut dyn FnMut(&mut dyn Read) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        self.ctx.traced("read", rel);
        let path = self.path(rel);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && self.stand_in_for(rel) => {
                return Err(carry(BackupError::NotDownloaded));
            }
            Err(e) => return Err(e),
        };
        if online_only(&meta) {
            return Err(carry(BackupError::NotDownloaded));
        }
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
            if self.ctx.syncs() {
                file.sync_all()
            } else {
                Ok(())
            }
        })();
        if let Err(e) = written {
            // Something of ours that failed on its own: its scratch file
            // goes (a death, which `point` reports, leaves it).
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        self.ctx.point("written", rel).map_err(carry)?;
        let linked = if self.ctx.refuses_links() {
            // A test's disk that makes no links.
            Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
        } else {
            std::fs::hard_link(&tmp, &path)
        };
        let put = match linked {
            Ok(()) => {
                let _ = std::fs::remove_file(&tmp);
                Put::Created
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(&tmp);
                Put::Existed
            }
            Err(e) if link_unsupported(&e) => self.place_without_links(&tmp, &path)?,
            // Any other error is an error: never a reason to replace.
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        };
        if self.ctx.syncs() {
            sync_dir(&parent);
        }
        self.ctx.point("published", rel).map_err(carry)?;
        Ok(put)
    }
}
