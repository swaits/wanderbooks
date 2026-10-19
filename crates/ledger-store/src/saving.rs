//! Saving a copy of a file for the human (`wanderbooks download`, the
//! viewer's "Save a copy" and exports): always a new file, whole.
//!
//! ```text
//! GOAL: a copy saved for the human never replaces a file of theirs and
//!       never leaves part of itself under the name.
//! INVARIANTS:
//!   1. The copy goes under `named`, or `named (1)`, `named (2)`... the
//!      first free, else `fallback` (a folder that takes no such name),
//!      never over a file.
//!   2. It is written to a hidden file in the same folder and linked in
//!      whole; on a file system without links it is written in place,
//!      new, and removed if it can't be finished. The hidden file is gone
//!      whatever happens (a killed process excepted).
//!   3. A failure says why: the folder's own error, not "no free name".
//! ```

use std::path::PathBuf;

/// Save `body` in `folder` as `named`, or `named (1)`, `named (2)`... the
/// first free; `fallback` if the folder takes no such name. Written aside
/// (a hidden file) and linked in whole, so the name never holds part of
/// it; never over another file. The path it went to.
pub fn save_new(
    folder: &std::path::Path,
    named: &str,
    fallback: &str,
    body: &[u8],
) -> Result<PathBuf, String> {
    use std::io::Write;
    let fail = |what: &std::path::Path, e: std::io::Error| {
        format!("can't write {}: {}", what.display(), e.kind())
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let aside = folder.join(format!(".wanderbooks-{}-{nanos}.part", std::process::id()));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&aside)
        .map_err(|e| fail(folder, e))?;
    if let Err(e) = f.write_all(body).and_then(|()| f.sync_all()) {
        drop(f);
        let _ = std::fs::remove_file(&aside);
        return Err(fail(folder, e));
    }
    drop(f);
    let candidates = |name: &str| {
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
            _ => (name.to_string(), String::new()),
        };
        let name = name.to_string();
        (0..1000).map(move |n| {
            if n == 0 {
                name.clone()
            } else {
                format!("{stem} ({n}){ext}")
            }
        })
    };
    let mut outcome = Err(format!("{} has no free name for it", folder.display()));
    // Why the last name tried was refused, if not for being taken: what
    // the human is told when no name is left (a full disk, not a name).
    let mut last: Option<std::io::Error> = None;
    'names: for name in [named, fallback] {
        for candidate in candidates(name) {
            let path = folder.join(&candidate);
            match std::fs::hard_link(&aside, &path) {
                Ok(()) => {
                    outcome = Ok(path);
                    break 'names;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                // A file system with no links (FAT): written in place, never
                // over another file, removed if it can't be finished.
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::Unsupported | std::io::ErrorKind::PermissionDenied
                    ) =>
                {
                    match std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                    {
                        Ok(mut f) => {
                            if let Err(e) = f.write_all(body).and_then(|()| f.sync_all()) {
                                drop(f);
                                let _ = std::fs::remove_file(&path);
                                outcome = Err(fail(&path, e));
                                last = None;
                            } else {
                                outcome = Ok(path);
                            }
                            break 'names;
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(e) => {
                            last = Some(e);
                            continue 'names;
                        }
                    }
                }
                // A name this folder won't take (too long, a character it
                // refuses): the next name.
                Err(e) => {
                    last = Some(e);
                    continue 'names;
                }
            }
        }
    }
    let _ = std::fs::remove_file(&aside);
    match (outcome, last) {
        (Err(_), Some(e)) => Err(fail(folder, e)),
        (outcome, _) => outcome,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_copy_is_new_and_whole_beside_what_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(d.join("r.pdf"), b"theirs").unwrap();
        let first = save_new(d, "r.pdf", "attachment-7", b"ours").unwrap();
        assert_eq!(first, d.join("r (1).pdf"));
        let second = save_new(d, "r.pdf", "attachment-7", b"ours").unwrap();
        assert_eq!(second, d.join("r (2).pdf"));
        assert_eq!(std::fs::read(d.join("r.pdf")).unwrap(), b"theirs");
        assert_eq!(std::fs::read(&first).unwrap(), b"ours");
        // A name the folder won't take: the fallback.
        let long = "x".repeat(300);
        assert_eq!(
            save_new(d, &long, "attachment-7", b"ours").unwrap(),
            d.join("attachment-7")
        );
        // Nothing aside is left.
        let mut names: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["attachment-7", "r (1).pdf", "r (2).pdf", "r.pdf"]);
        // A folder that can't be written: refused, nothing made.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let ro = d.join("ro");
            std::fs::create_dir(&ro).unwrap();
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o500)).unwrap();
            if std::fs::write(ro.join("probe"), b"").is_err() {
                assert!(save_new(&ro, "r.pdf", "attachment-7", b"ours").is_err());
            }
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
}
