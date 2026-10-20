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
    fn a_hidden_name_keeps_its_dot_when_numbered() {
        // `.env` is a stem, not an extension: `.env (1)`, never ` (1).env`.
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(
            save_new(d, ".env", "attachment-7", b"a").unwrap(),
            d.join(".env")
        );
        assert_eq!(
            save_new(d, ".env", "attachment-7", b"b").unwrap(),
            d.join(".env (1)")
        );
        assert_eq!(
            save_new(d, "r.tar.gz", "attachment-7", b"c").unwrap(),
            d.join("r.tar.gz")
        );
        assert_eq!(
            save_new(d, "r.tar.gz", "attachment-7", b"d").unwrap(),
            d.join("r.tar (1).gz")
        );
    }

    #[test]
    fn when_no_name_is_taken_the_folders_own_refusal_is_said() {
        // Both names too long for the folder: its refusal, not "no free
        // name", and nothing left behind.
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let e = save_new(d, &"x".repeat(300), &"y".repeat(300), b"ours").unwrap_err();
        assert!(!e.contains("no free name"), "{e}");
        assert!(
            e.ends_with(&std::io::ErrorKind::InvalidFilename.to_string()),
            "{e}"
        );
        assert_eq!(std::fs::read_dir(d).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn when_a_folder_refuses_the_copy_written_in_place_its_refusal_is_said() {
        // Both names in a folder that can't be written: the link is
        // refused (permission denied, so the copy is tried in place), and
        // so is the file in place. That refusal is said, not "no free
        // name", and the next name is never tried for it.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let ro = d.join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o500)).unwrap();
        // (Skipped where nothing is refused, as for root.)
        if std::fs::write(ro.join("probe"), b"").is_err() {
            let e = save_new(d, "ro/r.pdf", "ro/attachment-7", b"ours").unwrap_err();
            assert!(!e.contains("no free name"), "{e}");
            assert!(
                e.ends_with(&std::io::ErrorKind::PermissionDenied.to_string()),
                "{e}"
            );
            // A name of its own that the folder takes: that one.
            assert_eq!(
                save_new(d, "ro/r.pdf", "attachment-7", b"ours").unwrap(),
                d.join("attachment-7")
            );
        }
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_name_refused_for_another_reason_than_links_moves_on_never_written_in_place() {
        // A name reaching onto another file system: the link is refused
        // (it crosses devices), and the copy is never written there in
        // place as if the folder took no links; the fallback is used.
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let Ok(other) = tempfile::tempdir_in("/dev/shm") else {
            return;
        };
        let dev = |p: &std::path::Path| std::fs::metadata(p).unwrap().dev();
        if dev(d) == dev(other.path()) {
            return;
        }
        std::os::unix::fs::symlink(other.path(), d.join("elsewhere")).unwrap();
        assert_eq!(
            save_new(d, "elsewhere/r.pdf", "attachment-7", b"ours").unwrap(),
            d.join("attachment-7")
        );
        assert_eq!(std::fs::read_dir(other.path()).unwrap().count(), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_folder_without_links_gets_the_copy_written_in_place_never_over_a_file() {
        // Links refused as FAT refuses them (not supported, or not
        // permitted), and before the folder looks at the name: a name
        // already taken is then found only when the copy is written in
        // place, and the next name is tried, never the fallback, never
        // over the file.
        for errno in [libc::EOPNOTSUPP, libc::EPERM] {
            let dir = tempfile::tempdir().unwrap();
            let d = dir.path();
            std::fs::write(d.join("theirs.pdf"), b"theirs").unwrap();
            let Some(saved) = crate::refuse::refusing(crate::refuse::LINKS, errno, || {
                [
                    save_new(d, "r.pdf", "attachment-7", b"ours"),
                    save_new(d, "r.pdf", "attachment-7", b"again"),
                    save_new(d, "theirs.pdf", "attachment-7", b"mine"),
                ]
            }) else {
                eprintln!("skipped: no seccomp here");
                return;
            };
            assert_eq!(
                saved,
                [
                    Ok(d.join("r.pdf")),
                    Ok(d.join("r (1).pdf")),
                    Ok(d.join("theirs (1).pdf")),
                ],
                "errno {errno}"
            );
            assert_eq!(std::fs::read(d.join("r.pdf")).unwrap(), b"ours");
            assert_eq!(std::fs::read(d.join("r (1).pdf")).unwrap(), b"again");
            assert_eq!(std::fs::read(d.join("theirs.pdf")).unwrap(), b"theirs");
            assert_eq!(std::fs::read(d.join("theirs (1).pdf")).unwrap(), b"mine");
            // Nothing aside is left.
            let mut names: Vec<String> = std::fs::read_dir(d)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            assert_eq!(
                names,
                ["r (1).pdf", "r.pdf", "theirs (1).pdf", "theirs.pdf"],
                "errno {errno}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_link_refused_for_any_other_reason_is_never_written_in_place() {
        // Every link refused as crossing devices: no copy is written in
        // place as if the folder took no links; the folder's refusal is
        // said, and nothing is left behind.
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let Some(saved) = crate::refuse::refusing(crate::refuse::LINKS, libc::EXDEV, || {
            save_new(d, "r.pdf", "attachment-7", b"ours")
        }) else {
            eprintln!("skipped: no seccomp here");
            return;
        };
        let e = saved.unwrap_err();
        assert!(!e.contains("no free name"), "{e}");
        assert!(
            e.ends_with(&std::io::ErrorKind::CrossesDevices.to_string()),
            "{e}"
        );
        assert_eq!(std::fs::read_dir(d).unwrap().count(), 0);
    }

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
