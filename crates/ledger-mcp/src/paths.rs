//! Paths as the server judges them: where a path really leads.

use std::path::{Component, Path, PathBuf};

/// `path` made absolute and resolved as the kernel would follow it, name
/// by name: every symlink followed (a dangling one too, to where it
/// points), `.` and `..` folded in order, and whatever doesn't exist kept
/// as written. For an existing path this is its canonical form; for a
/// missing one, where it would be.
pub fn resolved(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    // The names still to walk, last first.
    let mut todo: Vec<std::ffi::OsString> = Vec::new();
    let mut out = PathBuf::new();
    // An absolute path starts the walk over at its root.
    let push_all = |todo: &mut Vec<std::ffi::OsString>, p: &Path, out: &mut PathBuf| {
        *out = PathBuf::new();
        let mut parts = Vec::new();
        for c in p.components() {
            match c {
                Component::Prefix(_) | Component::RootDir => out.push(c.as_os_str()),
                other => parts.push(other.as_os_str().to_os_string()),
            }
        }
        todo.extend(parts.into_iter().rev());
    };
    push_all(&mut todo, &abs, &mut out);
    // Links followed at most this many times (as the kernel's ELOOP).
    let mut hops = 0;
    while let Some(name) = todo.pop() {
        if name == "." {
            continue;
        }
        if name == ".." {
            out.pop();
            continue;
        }
        let next = out.join(&name);
        let link = std::fs::symlink_metadata(&next)
            .is_ok_and(|m| m.file_type().is_symlink())
            .then(|| std::fs::read_link(&next).ok())
            .flatten();
        match link {
            Some(target) if hops < 40 => {
                hops += 1;
                if target.is_absolute() {
                    push_all(&mut todo, &target, &mut out);
                } else {
                    todo.extend(
                        target
                            .components()
                            .map(|c| c.as_os_str().to_os_string())
                            .rev(),
                    );
                }
            }
            _ => out = next,
        }
    }
    out
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// `resolved` follows as many links as the kernel does (40, as
    /// `canonicalize` does), and no more.
    #[test]
    fn resolved_follows_forty_links_and_no_more() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().canonicalize().unwrap();
        let file = b.join("receipt.pdf");
        std::fs::write(&file, "x").unwrap();
        let link = |n: usize| b.join(format!("link-{n}"));
        symlink(&file, link(1)).unwrap();
        for n in 2..=41 {
            symlink(link(n - 1), link(n)).unwrap();
        }
        assert_eq!(link(40).canonicalize().unwrap(), file);
        assert_eq!(resolved(&link(40)), file);
        assert!(link(41).canonicalize().is_err(), "the kernel stops at 40");
        assert_ne!(resolved(&link(41)), file);
    }

    /// `resolved` places a path where the kernel would follow it: the
    /// canonical form for one that exists, and through a dangling link to
    /// where it points.
    #[test]
    fn resolved_follows_every_link_a_dangling_one_too() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(b.join("Downloads/sub")).unwrap();
        std::fs::create_dir_all(b.join("data")).unwrap();
        std::fs::write(b.join("Downloads/receipt.pdf"), "x").unwrap();
        std::fs::write(b.join("data/journal.jsonl"), "x").unwrap();
        symlink(
            b.join("Downloads/receipt.pdf"),
            b.join("Downloads/good-link"),
        )
        .unwrap();
        symlink(b.join("data"), b.join("Downloads/data-link")).unwrap();
        for p in [
            b.join("Downloads/good-link"),
            b.join("Downloads/data-link/journal.jsonl"),
            b.join("Downloads/sub/../receipt.pdf"),
            b.join("Downloads/./good-link"),
        ] {
            assert_eq!(resolved(&p), p.canonicalize().unwrap(), "{}", p.display());
        }
        symlink("../elsewhere/gone.pdf", b.join("Downloads/rel-dangling")).unwrap();
        assert_eq!(
            resolved(&b.join("Downloads/rel-dangling")),
            b.join("elsewhere/gone.pdf")
        );
        symlink(b.join("Downloads/loop-b"), b.join("Downloads/loop-a")).unwrap();
        symlink(b.join("Downloads/loop-a"), b.join("Downloads/loop-b")).unwrap();
        // A loop ends (the hop limit) inside the folder it loops in.
        assert!(resolved(&b.join("Downloads/loop-a")).starts_with(b.join("Downloads")));
        assert_eq!(
            resolved(&b.join("Downloads/missing/../../elsewhere/x")),
            b.join("elsewhere/x")
        );
    }
}
