//! The backup as one file ("Save a backup..."): a zip of the folder form
//! of one set, holding one manifest and only the pieces it names. A zip is
//! a file any computer can carry and a person can open to see what is
//! inside. Sealed when the backups are: the pieces in it are the sealed
//! pieces, so the file says no more than a sealed folder does.
//!
//! The extension is `.wbbackup`.

use super::cut::Cut;
use super::io::{Fill, Put, PutKind, SetIo};
use super::read::{Found, Set, verify_found};
use super::write::{Written, proof_scratch, write_into_set};
use super::{BackupError, Ctx};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The file's extension, without the dot.
pub const FILE_EXTENSION: &str = "wbbackup";

/// A set read from a zip.
struct ZipRead {
    archive: Mutex<zip::ZipArchive<std::fs::File>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl SetIo for ZipRead {
    fn list(&self, dir: &str) -> std::io::Result<Vec<String>> {
        let prefix = format!("{}/", dir.trim_end_matches('/'));
        let archive = lock(&self.archive);
        let mut out: Vec<String> = archive
            .file_names()
            .filter_map(|n| n.ok())
            .filter_map(|n| n.strip_prefix(prefix.as_str()).map(str::to_string))
            .filter(|rest| !rest.is_empty() && !rest.contains('/'))
            .collect();
        out.sort();
        Ok(out)
    }

    fn len(&self, rel: &str) -> std::io::Result<Option<u64>> {
        let mut archive = lock(&self.archive);
        match archive.by_name(rel) {
            Ok(f) if f.is_file() => Ok(Some(f.size())),
            Ok(_) | Err(zip::result::ZipError::FileNotFound) => Ok(None),
            Err(e) => Err(std::io::Error::other(e)),
        }
    }

    fn read_with(
        &self,
        rel: &str,
        f: &mut dyn FnMut(&mut dyn Read) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        let mut archive = lock(&self.archive);
        let mut file = archive
            .by_name(rel)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::NotFound))?;
        f(&mut file)
    }

    fn put(&self, _rel: &str, _kind: PutKind, _fill: Fill<'_>) -> std::io::Result<Put> {
        Err(std::io::Error::other("a backup file is read only"))
    }
}

/// A zip being written. The writer is shared with [`save_file`], which
/// finishes it.
struct ZipWrite {
    writer: Arc<Mutex<Option<zip::ZipWriter<std::fs::File>>>>,
    names: Mutex<BTreeSet<String>>,
}

impl SetIo for ZipWrite {
    fn list(&self, _dir: &str) -> std::io::Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn len(&self, _rel: &str) -> std::io::Result<Option<u64>> {
        Ok(None)
    }

    fn read_with(
        &self,
        _rel: &str,
        _f: &mut dyn FnMut(&mut dyn Read) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
    }

    fn put(&self, rel: &str, _kind: PutKind, fill: Fill<'_>) -> std::io::Result<Put> {
        if !lock(&self.names).insert(rel.to_string()) {
            return Ok(Put::Existed);
        }
        // Text compresses; stored files and sealed pieces do not.
        let text = ["journal/", "access/", "files-index/", "manifests/"]
            .iter()
            .any(|p| rel.starts_with(p))
            || rel == super::form::READ_ME;
        let method = if text {
            zip::CompressionMethod::Deflated
        } else {
            zip::CompressionMethod::Stored
        };
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(method)
            .large_file(true);
        let mut guard = lock(&self.writer);
        let writer = guard
            .as_mut()
            .ok_or_else(|| std::io::Error::other("the backup file is finished"))?;
        writer
            .start_file(rel, options)
            .map_err(std::io::Error::other)?;
        fill(writer)?;
        Ok(Put::Created)
    }
}

/// Open a backup file for reading.
pub(crate) fn open(path: &Path, _ctx: &Ctx) -> Result<Box<dyn SetIo>, BackupError> {
    let file = std::fs::File::open(path)?;
    let archive = zip::ZipArchive::new(file).map_err(|_| BackupError::NotABackup)?;
    Ok(Box::new(ZipRead {
        archive: Mutex::new(archive),
    }))
}

/// Write the cut as one file at `path` (which must not be there): the same
/// contents as a backup folder, sealed when `key` is given. Written aside
/// and moved into place only when whole, so a file by that name is a
/// whole backup.
pub fn save_file(
    path: &Path,
    cut: &Cut,
    key: Option<&super::BackupKey>,
    ctx: &Ctx,
) -> Result<Written, BackupError> {
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(BackupError::Damaged(
            "a file by that name is there already: choose another name".into(),
        ));
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(BackupError::NotABackup)?;
    let aside = path.with_file_name(format!(".{name}.partial"));
    let _ = std::fs::remove_file(&aside);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&aside)?;
    let shared = Arc::new(Mutex::new(Some(zip::ZipWriter::new(file))));
    let set = Set {
        io: Box::new(ZipWrite {
            writer: shared.clone(),
            names: Mutex::new(BTreeSet::new()),
        }),
        keys: key.map(|k| k.keys()),
        ctx: ctx.clone(),
    };
    let result = write_into_set(&set, cut, ctx, false).and_then(|written| {
        // The zip's directory, then the file's.
        let writer = lock(&shared)
            .take()
            .ok_or_else(|| BackupError::Damaged("the backup file was finished twice".into()))?;
        let file = writer
            .finish()
            .map_err(|e| BackupError::Io(std::io::Error::other(e).kind()))?;
        file.sync_all()?;
        // Read back as a restore would read it, and proven whole (every
        // piece re-hashed, the chain replayed, the books opened), before it
        // is given its name: a file by that name is a backup that proved.
        let reader = Set {
            io: open(&aside, ctx)?,
            keys: key.map(|k| k.keys()),
            ctx: ctx.clone(),
        };
        let found = Found::of(reader, key.cloned(), cut.book());
        let scratch = proof_scratch(cut)?;
        let proven = verify_found(&found, &scratch, ctx);
        let _ = std::fs::remove_dir_all(&scratch);
        proven?;
        Ok(written)
    });
    match result {
        Ok(written) => {
            // Moved into place only if the name is still free.
            let placed = std::fs::hard_link(&aside, path).or_else(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists
                    || std::fs::symlink_metadata(path).is_ok()
                {
                    Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
                } else {
                    std::fs::rename(&aside, path)
                }
            });
            let _ = std::fs::remove_file(&aside);
            placed?;
            Ok(written)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&aside);
            Err(e)
        }
    }
}
