//! The lock files a git leaves in a repository, found and (only where it
//! is ours to) removed through a directory handle (ledger-store `backup`
//! INVARIANT 5).
//!
//! ```text
//! GOAL: a lock file a stopped git left can be seen, and removed from the
//!       one repository this program owns, without ever touching anything
//!       a swapped folder or link points to.
//! SOURCE: the books server's close (ledger-mcp local.rs INVARIANT 10, 13:
//!         a close is answered with no git of ours still working, and
//!         leaves nothing a person must clean up); git's own lock files
//!         (git-config, gitrepository-layout: `HEAD.lock`, `index.lock`,
//!         `<ref>.lock`, `packed-refs.lock`, `shallow.lock`,
//!         `config.lock`).
//! INVARIANTS:
//!   1. A repository is opened once (`RepoDir::open`, `open_in`): a
//!      directory that is not a link, the folder it was found in kept as
//!      the book's own. Every look and every removal after that goes
//!      through that handle: the folders below it (`refs/`, and those
//!      under it) are each opened from the one above with links refused,
//!      so a folder swapped for a link, or the repository's own path swapped
//!      for one, after the handle was opened, sends nothing anywhere: the
//!      handle still names the folder that was opened.
//!   2. Only a regular file is ever removed (never a link, never a folder,
//!      never a file whose type changed), and only one named as git names
//!      its locks, in the places git puts them (`is_lock`). A name that
//!      isn't one is refused.
//!   3. Where there is no directory handle to be had (not Unix) each step
//!      looks at the path itself and refuses a link: the same rules,
//!      without the guarantee against a swap in between. Nothing else
//!      about the platform's behaviour is designed here.
//! ```

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Locks git makes in a repository's own folder.
const TOP: [&str; 5] = [
    "HEAD.lock",
    "index.lock",
    "config.lock",
    "packed-refs.lock",
    "shallow.lock",
];
/// Locks git makes below it, outside `refs/`.
const BELOW: [&str; 2] = [
    "objects/info/commit-graph.lock",
    "objects/pack/multi-pack-index.lock",
];
/// How deep under `refs/` a lock is looked for.
const REFS_DEPTH: usize = 12;

/// Whether `rel` (a path in a repository, with `/` between its parts) is
/// named as git names a lock, in a place git puts one (INVARIANT 2).
pub fn is_lock(rel: &str) -> bool {
    if TOP.contains(&rel) || BELOW.contains(&rel) {
        return true;
    }
    let Some(under) = rel.strip_prefix("refs/") else {
        return false;
    };
    let parts: Vec<&str> = under.split('/').collect();
    parts.len() <= REFS_DEPTH
        && parts
            .iter()
            .all(|p| !p.is_empty() && *p != "." && *p != "..")
        && parts
            .last()
            .is_some_and(|name| name.len() > ".lock".len() && name.ends_with(".lock"))
}

/// A lock found: its path in the repository, and when it was last written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// `index.lock`, `refs/heads/main.lock`.
    pub lock: String,
    /// Its modification time, when the system said.
    pub modified: Option<SystemTime>,
}

/// A repository's folder, opened once (INVARIANT 1).
#[derive(Debug)]
pub struct RepoDir {
    path: PathBuf,
    inner: imp::Dir,
}

impl RepoDir {
    /// The folder at `path`, which must not itself be a link.
    pub fn open(path: &Path) -> io::Result<RepoDir> {
        Ok(RepoDir {
            path: path.to_path_buf(),
            inner: imp::open(path)?,
        })
    }

    /// The folder `name` inside the folder `parent` (the book's own folder
    /// holding its archive): `name` must not be a link.
    pub fn open_in(parent: &Path, name: &str) -> io::Result<RepoDir> {
        Ok(RepoDir {
            path: parent.join(name),
            inner: imp::open_in(parent, name)?,
        })
    }

    /// The path it was opened by (for words, never for an operation).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The locks in it, sorted: regular files named as git names its locks.
    pub fn locks(&self) -> Vec<Found> {
        let mut out = Vec::new();
        for rel in TOP.iter().chain(BELOW.iter()) {
            if let Some(modified) = self.inner.regular(rel) {
                out.push(Found {
                    lock: (*rel).to_string(),
                    modified,
                });
            }
        }
        self.inner.refs(&mut out, REFS_DEPTH);
        out.sort_by(|a, b| a.lock.cmp(&b.lock));
        out
    }

    /// Remove the lock `rel` (INVARIANTS 1, 2): Ok(true) if a regular file
    /// of that name was removed, Ok(false) if there was none (or it was
    /// not a regular file: nothing was done), Err if the name isn't a
    /// lock's or the system refused.
    pub fn remove(&self, rel: &str) -> io::Result<bool> {
        if !is_lock(rel) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a name git gives a lock",
            ));
        }
        self.inner.remove(rel)
    }
}

#[cfg(unix)]
mod imp {
    use super::{Found, SystemTime};
    use std::ffi::{CStr, CString, OsStr};
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    #[derive(Debug)]
    pub struct Dir(OwnedFd);

    const FLAGS: libc::c_int = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC;

    fn c(text: &OsStr) -> io::Result<CString> {
        CString::new(text.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a name with a NUL"))
    }

    fn owned(fd: libc::c_int) -> io::Result<OwnedFd> {
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: a file descriptor just returned by the system, owned
            // by no one else.
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }
    }

    pub fn open(path: &Path) -> io::Result<Dir> {
        let path = c(path.as_os_str())?;
        // SAFETY: a NUL-ended path; the flags refuse a link at its end.
        owned(unsafe { libc::open(path.as_ptr(), FLAGS | libc::O_NOFOLLOW) }).map(Dir)
    }

    pub fn open_in(parent: &Path, name: &str) -> io::Result<Dir> {
        let parent = c(parent.as_os_str())?;
        // SAFETY: as in `open`; the parent may be reached through links (it
        // is the book's own folder), only `name` may not be one.
        let parent = owned(unsafe { libc::open(parent.as_ptr(), FLAGS) })?;
        sub(parent.as_raw_fd(), name).map(Dir)
    }

    /// The folder `name` of `dir`, a link refused.
    fn sub(dir: RawFd, name: &str) -> io::Result<OwnedFd> {
        let name = c(OsStr::new(name))?;
        // SAFETY: `dir` is open for the call; the name is NUL-ended.
        owned(unsafe { libc::openat(dir, name.as_ptr(), FLAGS | libc::O_NOFOLLOW) })
    }

    /// `name` in `dir` as it is (a link is not followed): its stat.
    fn stat(dir: RawFd, name: &str) -> io::Result<libc::stat> {
        let name = c(OsStr::new(name))?;
        // SAFETY: zeroed stat is a valid out-parameter; `dir` is open.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let found =
            unsafe { libc::fstatat(dir, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) };
        if found == 0 {
            Ok(st)
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn is_regular(st: &libc::stat) -> bool {
        (st.st_mode & libc::S_IFMT) == libc::S_IFREG
    }

    fn is_dir(st: &libc::stat) -> bool {
        (st.st_mode & libc::S_IFMT) == libc::S_IFDIR
    }

    fn modified(st: &libc::stat) -> Option<SystemTime> {
        let secs = u64::try_from(st.st_mtime).ok()?;
        let nanos = u32::try_from(st.st_mtime_nsec).ok()?;
        Some(SystemTime::UNIX_EPOCH + std::time::Duration::new(secs, nanos))
    }

    /// The names in the folder `dir` (a fresh look each time).
    fn names(dir: RawFd) -> Vec<String> {
        // SAFETY: a duplicate of an open descriptor, handed to fdopendir,
        // which owns it from then on (closedir closes it).
        let copy = unsafe { libc::dup(dir) };
        if copy < 0 {
            return Vec::new();
        }
        let stream = unsafe { libc::fdopendir(copy) };
        if stream.is_null() {
            // SAFETY: fdopendir didn't take it.
            unsafe { libc::close(copy) };
            return Vec::new();
        }
        let mut out = Vec::new();
        // SAFETY: `stream` is a live directory stream until closedir; each
        // entry's name is a NUL-ended string valid until the next readdir.
        unsafe {
            libc::rewinddir(stream);
            loop {
                let entry = libc::readdir(stream);
                if entry.is_null() {
                    break;
                }
                let name = CStr::from_ptr((*entry).d_name.as_ptr());
                if let Ok(name) = name.to_str()
                    && name != "."
                    && name != ".."
                {
                    out.push(name.to_string());
                }
            }
            libc::closedir(stream);
        }
        out
    }

    impl Dir {
        /// The folder holding `rel`, opened step by step from this one
        /// with a link refused at each, and `rel`'s last part.
        fn parent_of<'a>(&self, rel: &'a str) -> io::Result<(OwnedFd, &'a str)> {
            let mut parts: Vec<&str> = rel.split('/').collect();
            let last = parts.pop().ok_or(io::ErrorKind::InvalidInput)?;
            // SAFETY: a duplicate of the open descriptor, owned here.
            let mut here = owned(unsafe { libc::dup(self.0.as_raw_fd()) })?;
            for part in parts {
                here = sub(here.as_raw_fd(), part)?;
            }
            Ok((here, last))
        }

        /// A regular file at `rel`: its modification time (when known).
        pub fn regular(&self, rel: &str) -> Option<Option<SystemTime>> {
            let (parent, name) = self.parent_of(rel).ok()?;
            let st = stat(parent.as_raw_fd(), name).ok()?;
            is_regular(&st).then(|| modified(&st))
        }

        /// The locks under `refs/`, in `out`.
        pub fn refs(&self, out: &mut Vec<Found>, depth: usize) {
            let Ok(refs) = sub(self.0.as_raw_fd(), "refs") else {
                return;
            };
            fn walk(dir: RawFd, prefix: &str, depth: usize, out: &mut Vec<Found>) {
                for name in names(dir) {
                    let Ok(st) = stat(dir, &name) else { continue };
                    let rel = format!("{prefix}{name}");
                    if is_regular(&st) {
                        if super::is_lock(&rel) {
                            out.push(Found {
                                lock: rel,
                                modified: modified(&st),
                            });
                        }
                    } else if is_dir(&st)
                        && depth > 0
                        && let Ok(inner) = sub(dir, &name)
                    {
                        walk(inner.as_raw_fd(), &format!("{rel}/"), depth - 1, out);
                    }
                }
            }
            walk(refs.as_raw_fd(), "refs/", depth, out);
        }

        pub fn remove(&self, rel: &str) -> io::Result<bool> {
            let (parent, name) = match self.parent_of(rel) {
                Ok(found) => found,
                // A folder that isn't there (or is a link): nothing to remove.
                Err(_) => return Ok(false),
            };
            match stat(parent.as_raw_fd(), name) {
                Ok(st) if is_regular(&st) => {}
                _ => return Ok(false),
            }
            let name = c(OsStr::new(name))?;
            // SAFETY: the parent is open; the name is NUL-ended and relative
            // to it, and names a regular file just looked at.
            let removed = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) };
            if removed == 0 {
                Ok(true)
            } else {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::NotFound {
                    Ok(false)
                } else {
                    Err(e)
                }
            }
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use super::{Found, SystemTime};
    use std::io;
    use std::path::{Path, PathBuf};

    #[derive(Debug)]
    pub struct Dir(PathBuf);

    fn plain(path: &Path) -> io::Result<()> {
        let meta = std::fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a plain folder",
            ));
        }
        Ok(())
    }

    pub fn open(path: &Path) -> io::Result<Dir> {
        plain(path)?;
        Ok(Dir(path.to_path_buf()))
    }

    pub fn open_in(parent: &Path, name: &str) -> io::Result<Dir> {
        open(&parent.join(name))
    }

    impl Dir {
        fn at(&self, rel: &str) -> io::Result<PathBuf> {
            let mut path = self.0.clone();
            let mut parts: Vec<&str> = rel.split('/').collect();
            let last = parts.pop().ok_or(io::ErrorKind::InvalidInput)?;
            for part in parts {
                path.push(part);
                plain(&path)?;
            }
            path.push(last);
            Ok(path)
        }

        pub fn regular(&self, rel: &str) -> Option<Option<SystemTime>> {
            let path = self.at(rel).ok()?;
            let meta = std::fs::symlink_metadata(path).ok()?;
            meta.file_type()
                .is_file()
                .then(|| meta.modified().ok())
        }

        pub fn refs(&self, out: &mut Vec<Found>, depth: usize) {
            fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<Found>) {
                let Ok(entries) = std::fs::read_dir(dir) else {
                    return;
                };
                for e in entries.flatten() {
                    let Ok(meta) = std::fs::symlink_metadata(e.path()) else {
                        continue;
                    };
                    let rel = format!("{prefix}{}", e.file_name().to_string_lossy());
                    if meta.file_type().is_file() {
                        if super::is_lock(&rel) {
                            out.push(Found {
                                lock: rel,
                                modified: meta.modified().ok(),
                            });
                        }
                    } else if meta.is_dir() && !meta.file_type().is_symlink() && depth > 0 {
                        walk(&e.path(), &format!("{rel}/"), depth - 1, out);
                    }
                }
            }
            let refs = self.0.join("refs");
            if plain(&refs).is_ok() {
                walk(&refs, "refs/", depth, out);
            }
        }

        pub fn remove(&self, rel: &str) -> io::Result<bool> {
            let Ok(path) = self.at(rel) else {
                return Ok(false);
            };
            match std::fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_file() => {
                    std::fs::remove_file(path).map(|()| true)
                }
                _ => Ok(false),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(dir: &Path) -> PathBuf {
        let repo = dir.join("repo.git");
        std::fs::create_dir_all(repo.join("refs/heads")).unwrap();
        std::fs::create_dir_all(repo.join("objects/pack")).unwrap();
        std::fs::create_dir_all(repo.join("objects/info")).unwrap();
        repo
    }

    #[test]
    fn only_the_names_git_gives_its_locks_are_locks() {
        for yes in [
            "HEAD.lock",
            "index.lock",
            "config.lock",
            "packed-refs.lock",
            "shallow.lock",
            "refs/heads/main.lock",
            "refs/heads/feature/x.lock",
            "refs/tags/v1.lock",
            "objects/info/commit-graph.lock",
            "objects/pack/multi-pack-index.lock",
        ] {
            assert!(is_lock(yes), "{yes}");
        }
        for no in [
            "",
            ".lock",
            "refs/.lock",
            "refs/heads/",
            "refs/heads/main",
            "refs/heads/../../x.lock",
            "refs//x.lock",
            "refs/heads/./x.lock",
            "other.lock",
            "objects/pack/pack-1.pack",
            "objects/other.lock",
            "../index.lock",
            "/etc/x.lock",
            "hooks/pre-commit.lock",
            "refs",
        ] {
            assert!(!is_lock(no), "{no:?}");
        }
    }

    #[test]
    fn locks_are_found_and_only_regular_files_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let repo = repo(dir.path());
        std::fs::write(repo.join("index.lock"), b"").unwrap();
        std::fs::write(repo.join("refs/heads/main.lock"), b"").unwrap();
        // Named like locks, but not files: neither is listed nor removed.
        std::fs::create_dir(repo.join("HEAD.lock")).unwrap();
        std::fs::write(dir.path().join("elsewhere"), b"").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), repo.join("config.lock")).unwrap();
        // Not a lock's name.
        std::fs::write(repo.join("refs/heads/main"), b"").unwrap();
        let handle = RepoDir::open(&repo).unwrap();
        let found: Vec<String> = handle.locks().into_iter().map(|f| f.lock).collect();
        assert_eq!(found, ["index.lock", "refs/heads/main.lock"]);
        assert!(handle.locks().iter().all(|f| f.modified.is_some()));
        assert!(handle.remove("index.lock").unwrap());
        assert!(!handle.remove("index.lock").unwrap(), "already gone");
        assert!(!handle.remove("HEAD.lock").unwrap(), "a folder stays");
        assert!(repo.join("HEAD.lock").is_dir());
        #[cfg(unix)]
        {
            assert!(!handle.remove("config.lock").unwrap(), "a link stays");
            assert!(repo.join("config.lock").is_symlink());
            assert!(dir.path().join("elsewhere").exists());
        }
        assert!(handle.remove("refs/heads/main").is_err(), "not a lock");
        assert!(repo.join("refs/heads/main").exists());
        assert!(handle.remove("../elsewhere.lock").is_err());
    }

    /// INVARIANT 1: a folder (or the repository's own path) swapped for a
    /// link after the handle was opened sends nothing anywhere.
    #[cfg(unix)]
    #[test]
    fn a_folder_swapped_for_a_link_after_the_handle_was_opened_touches_nothing_outside() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(outside.join("refs/heads")).unwrap();
        for lock in ["index.lock", "HEAD.lock", "refs/heads/main.lock"] {
            std::fs::write(outside.join(lock), b"theirs").unwrap();
        }
        // `refs` swapped for a link to a folder holding a lock of that name.
        let repo_path = repo(dir.path());
        std::fs::write(repo_path.join("refs/heads/main.lock"), b"ours").unwrap();
        let handle = RepoDir::open(&repo_path).unwrap();
        std::fs::rename(repo_path.join("refs"), dir.path().join("refs-moved")).unwrap();
        std::os::unix::fs::symlink(outside.join("refs"), repo_path.join("refs")).unwrap();
        assert!(!handle.remove("refs/heads/main.lock").unwrap());
        assert!(handle.locks().is_empty(), "a link is not looked through");
        assert_eq!(
            std::fs::read(outside.join("refs/heads/main.lock")).unwrap(),
            b"theirs"
        );
        assert_eq!(
            std::fs::read(dir.path().join("refs-moved/heads/main.lock")).unwrap(),
            b"ours",
            "nor was the folder it moved to touched through the link"
        );
        // The repository's own path swapped for a link: the handle still
        // names the folder it opened (moved aside), and the link's target
        // is never touched.
        let second = dir.path().join("second.git");
        std::fs::create_dir_all(second.join("refs")).unwrap();
        std::fs::write(second.join("index.lock"), b"ours").unwrap();
        let handle = RepoDir::open(&second).unwrap();
        std::fs::rename(&second, dir.path().join("second-moved.git")).unwrap();
        std::os::unix::fs::symlink(&outside, &second).unwrap();
        assert!(handle.remove("index.lock").unwrap());
        assert!(!dir.path().join("second-moved.git/index.lock").exists());
        assert_eq!(
            std::fs::read(outside.join("index.lock")).unwrap(),
            b"theirs",
            "the link's target was touched"
        );
        assert_eq!(
            std::fs::read(outside.join("HEAD.lock")).unwrap(),
            b"theirs"
        );
        // And a repository that is a link when the handle is opened is
        // refused, with the archive's own `open_in` too.
        assert!(RepoDir::open(&second).is_err());
        assert!(RepoDir::open_in(dir.path(), "second.git").is_err());
        assert!(RepoDir::open_in(dir.path(), "second-moved.git").is_ok());
    }
}
