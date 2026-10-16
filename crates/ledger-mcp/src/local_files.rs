//! Files the server reads from its own disk for an agent
//! (`books.import_local_file`, `books.set_w9`'s `w9_file_path`): only
//! regular files inside the import roots, and never a file of the books
//! themselves.
//!
//! ```text
//! GOAL: an agent can file a receipt it saved (a download, a rendered PDF,
//!       a pasted screenshot) without being able to read the books' own
//!       files, the TIN vault's key, or anything else of the human's,
//!       through the server.
//! SOURCE: DECISION (Stephen, 2026-10-16; plan/served.md section 6,
//!         "Immediate hole (P0)"): an explicit --import-root allowlist
//!         (default: the downloads folder only), canonicalized, refusing
//!         the data dir, the books dir, the vault dir, the key's dir,
//!         symlink escapes, and non-regular files; the P0 reviews
//!         (2026-10-16) for INVARIANTS 0, 2, 3 and 6.
//! INVARIANTS:
//!   0. No import root is `/`, the home folder or a folder holding it, a
//!      folder holding the books dir, the data dir, the vault or a key
//!      file, or a folder inside the books dir, the data dir, the vault or
//!      ~/.config/wanderbooks (ImportRoots::guard): such a root is refused at startup when
//!      named (--import-root) and dropped when it is the default. The
//!      default is the downloads folder, falling back to ~/Downloads, and
//!      only when that is a real folder that isn't the home folder; else
//!      there are no roots, and every read says so (NO_ROOTS).
//!   1. A path is read only if it is absolute, has no `.` or `..` name,
//!      and where it leads (every symlink resolved) is a regular file
//!      strictly inside a canonical import root.
//!   2. A path that is missing, a dangling link, outside every root, or
//!      with a `.` or `..` name gets one refusal, NO_SUCH_FILE, worded for
//!      all: a refusal never tells what lies outside the roots, or whether
//!      a link's target exists. A `.` or `..` name is refused before
//!      anything is looked up: resolving `a/b/..` succeeds only when `b`
//!      is a folder, which would tell whether folders outside the roots
//!      exist.
//!   3. Inside a root, a path in a sealed place (the data dir, the vault,
//!      the key files and the folders Protected::of seals) is refused as
//!      the books' or the TIN vault's, existing or not; one elsewhere in
//!      the books dir is refused with "save a copy first".
//!   4. A file with more than one hard link is refused: a hard link would
//!      carry a protected file into a root without a symlink to follow.
//!   5. A refusal names the rule it applies, never a path.
//!   6. The file read is the file checked: it is opened by a walk from
//!      `/`, one name of its canonical path at a time, never following a
//!      symlink (a folder swapped for a link after the check fails the
//!      walk) and never blocking (a pipe swapped in can't hang a thread);
//!      no folder on the walk, and not the file, is a protected place (by
//!      device and inode: a protected folder renamed into a root); and the
//!      file opened is the checked file (device and inode), with one link.
//! ```

use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// The folders `books.import_local_file` may read from.
#[derive(Clone, Debug)]
pub struct ImportRoots {
    dirs: Vec<PathBuf>,
    /// Named with --import-root (a bad one is refused), not the default
    /// (a bad one is dropped).
    explicit: bool,
}

/// What is never read through a path, even inside an import root
/// (INVARIANT 3).
#[derive(Clone, Debug, Default)]
pub struct Protected {
    /// The books' own records and the TIN vault's: the data dir, the
    /// vault, the key files and the folders [`Protected::of`] seals.
    pub sealed: Vec<PathBuf>,
    /// The books dir: its other files (the settings, the company's papers)
    /// are filed from a copy, never in place.
    pub books: Vec<PathBuf>,
}

/// Where the books keep what an import must never read.
#[derive(Clone, Debug, Default)]
pub struct Places {
    /// The data dir (the journal, the database, the receipts).
    pub data_dir: PathBuf,
    /// The books dir (the data dir's parent).
    pub books_dir: PathBuf,
    /// The TIN vault's folder.
    pub vault_dir: PathBuf,
    /// The vault key files: the one in force and the default one.
    pub key_files: Vec<PathBuf>,
    /// The home folder, if known.
    pub home: Option<PathBuf>,
}

impl Places {
    /// What no import root may hold (INVARIANT 0): the data dir, the books
    /// dir, the vault, each key file and where it leads, and the
    /// wanderbooks folder under the home folder's .config.
    pub fn keep_out(&self) -> Vec<PathBuf> {
        let mut out = vec![
            self.data_dir.clone(),
            self.books_dir.clone(),
            self.vault_dir.clone(),
        ];
        for k in &self.key_files {
            out.push(k.clone());
            out.push(resolved(k));
        }
        out.extend(self.home_config());
        out
    }

    /// `~/.config/wanderbooks`, whatever XDG_CONFIG_HOME says: a key left
    /// there from before still unlocks the vault.
    fn home_config(&self) -> Option<PathBuf> {
        self.home
            .as_ref()
            .map(|h| h.join(".config").join("wanderbooks"))
    }
}

impl Protected {
    /// What these books protect, given the import roots in use.
    ///
    /// ```text
    /// GOAL: the books and the vault key are never read through an import,
    ///       and sealing them never blocks every import.
    /// SOURCE: DECISION (P0 review round 2, 2026-10-16).
    /// INVARIANTS:
    ///   1. Sealed always: the data dir, the vault, each key file (the one
    ///      in force and the default) and where it leads, and
    ///      ~/.config/wanderbooks with where it leads.
    ///   2. A key file's folder is sealed too, but only when it is not the
    ///      home folder, not a root, and not a folder holding a root
    ///      (sealing it would seal everything an import can read).
    ///   3. The books dir is `books` (filed from a copy, INVARIANT 3).
    /// ```
    pub fn of(places: &Places, roots: &[PathBuf]) -> Self {
        let home = places.home.as_deref().map(resolved);
        let mut sealed = vec![
            places.data_dir.clone(),
            resolved(&places.data_dir),
            places.vault_dir.clone(),
            resolved(&places.vault_dir),
        ];
        if let Some(c) = places.home_config() {
            sealed.push(resolved(&c));
            sealed.push(c);
        }
        for k in &places.key_files {
            for file in [k.clone(), resolved(k)] {
                if let Some(folder) = file.parent().map(resolved) {
                    let holds_a_root = roots.iter().any(|r| r.starts_with(&folder));
                    if home.as_ref() != Some(&folder) && !holds_a_root {
                        sealed.push(folder);
                    }
                }
                sealed.push(file);
            }
        }
        Protected {
            sealed,
            books: vec![places.books_dir.clone()],
        }
    }

    /// Device and inode of every protected place that exists (links
    /// followed): what no step of an open may land on (INVARIANT 6).
    #[cfg(unix)]
    pub fn inodes(&self) -> Vec<(u64, u64)> {
        use std::os::unix::fs::MetadataExt;
        self.sealed
            .iter()
            .chain(&self.books)
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| (m.dev(), m.ino()))
            .collect()
    }

    /// No inodes off Unix: the walk itself is Unix only.
    #[cfg(not(unix))]
    pub fn inodes(&self) -> Vec<(u64, u64)> {
        Vec::new()
    }
}

/// Why `root` can't be an import root, if it can't (INVARIANT 0): it is
/// `/`, the home folder or a folder holding it, a folder holding one of
/// `keep_out`, or a folder inside one. A root that doesn't exist reads
/// nothing, and is let be.
pub fn refused_root(
    root: &Path,
    home: Option<&Path>,
    keep_out: &[PathBuf],
) -> Option<&'static str> {
    let root = root.canonicalize().ok()?;
    if root.parent().is_none() {
        return Some("it is the whole filesystem");
    }
    if home.is_some_and(|h| resolved(h).starts_with(&root)) {
        return Some("it is the home folder, or holds it");
    }
    if keep_out.iter().any(|k| resolved(k).starts_with(&root)) {
        return Some("it holds the books, the TIN vault, or a vault key");
    }
    // Inside them, every file would be refused (INVARIANT 3), with a
    // remedy (save a copy in an import folder) that leads back in.
    if keep_out.iter().any(|k| root.starts_with(resolved(k))) {
        return Some("it is inside the books or the TIN vault, whose files are never imported");
    }
    None
}

impl ImportRoots {
    /// Exactly these folders (`--import-root`, repeatable).
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        Self {
            dirs,
            explicit: true,
        }
    }

    /// The default when no `--import-root` is given: the user's downloads
    /// folder, only (plan/served.md section 6; INVARIANT 0).
    pub fn defaults() -> Self {
        Self::defaults_from(
            std::env::var_os("XDG_DOWNLOAD_DIR"),
            std::env::var_os("HOME").map(PathBuf::from),
        )
    }

    /// [`Self::defaults`] for these values of `XDG_DOWNLOAD_DIR` and
    /// `HOME`.
    pub fn defaults_from(xdg: Option<std::ffi::OsString>, home: Option<PathBuf>) -> Self {
        Self {
            dirs: default_root(xdg, home.as_deref()).into_iter().collect(),
            explicit: false,
        }
    }

    /// The roots, with every one [`refused_root`] refuses taken out, and
    /// those refused with why. A server refuses to start with a named
    /// root refused; a refused default is just dropped.
    pub fn guard(
        self,
        home: Option<&Path>,
        keep_out: &[PathBuf],
    ) -> (Self, Vec<(PathBuf, &'static str)>) {
        let mut refused = Vec::new();
        let dirs = self
            .dirs
            .into_iter()
            .filter(|d| match refused_root(d, home, keep_out) {
                Some(why) => {
                    refused.push((d.clone(), why));
                    false
                }
                None => true,
            })
            .collect();
        (
            Self {
                dirs,
                explicit: self.explicit,
            },
            refused,
        )
    }

    /// Whether the roots were named (`--import-root`), not the default.
    pub fn explicit(&self) -> bool {
        self.explicit
    }

    /// The roots in use: each that exists, canonical, and never `/` or a
    /// folder holding the home folder, whatever it has become since the
    /// server started (INVARIANT 0).
    pub fn usable(&self) -> Vec<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        self.dirs
            .iter()
            .filter(|d| refused_root(d, home.as_deref(), &[]).is_none())
            .filter_map(|r| r.canonicalize().ok())
            .filter(|r| r.is_dir())
            .collect()
    }

    /// Whether `path` (existing or not) is strictly inside a root: where a
    /// file it names could be read back in.
    pub fn contains(&self, path: &Path) -> bool {
        inside(&self.usable(), &resolved(path))
    }

    /// The bytes of the file at `path`, if the invariants allow it, with
    /// its canonical path (for the file name and MIME type). `limit` is
    /// the most bytes accepted.
    pub fn read(
        &self,
        path: &str,
        protected: &Protected,
        limit: usize,
    ) -> Result<(PathBuf, Vec<u8>), String> {
        let roots = self.usable();
        // Where the protected places are now, before anything is checked:
        // one moved into a root later is still known by its inode.
        let avoid = protected.inodes();
        let (canonical, checked) = inspect(&roots, path, protected)?;
        let bytes = read_checked(&canonical, &checked, &avoid, limit)?;
        Ok((canonical, bytes))
    }

    /// The canonical path of `path` if it may be read (INVARIANTS 1-5).
    #[cfg(test)]
    pub fn check(&self, path: &str, protected: &Protected) -> Result<PathBuf, String> {
        inspect(&self.usable(), path, protected).map(|(p, _)| p)
    }
}

/// The default import root (INVARIANT 0): `$XDG_DOWNLOAD_DIR` (as
/// user-dirs.dirs writes it, "$HOME/Downloads"), else `~/Downloads`; each
/// only if it is an absolute, real folder that isn't `/`, the home folder
/// or a folder holding it.
fn default_root(xdg: Option<std::ffi::OsString>, home: Option<&Path>) -> Option<PathBuf> {
    let ok = |dir: PathBuf| {
        (dir.is_absolute() && dir.is_dir() && refused_root(&dir, home, &[]).is_none())
            .then(|| dir.canonicalize().ok())
            .flatten()
    };
    let from_xdg = xdg.and_then(|x| {
        let x = x.to_string_lossy().into_owned();
        let rest = x
            .strip_prefix("${HOME}")
            .or_else(|| x.strip_prefix("$HOME"));
        match (rest, home) {
            (Some(rest), Some(h)) => Some(h.join(rest.trim_start_matches('/'))),
            (Some(_), None) => None,
            (None, _) => Some(PathBuf::from(x)),
        }
    });
    from_xdg
        .and_then(ok)
        .or_else(|| home.and_then(|h| ok(h.join("Downloads"))))
}

/// Whether `path` has a `.` or `..` name (INVARIANT 2), however it is
/// separated (`/`, or a backslash where that separates names too).
fn has_dot_name(path: &str) -> bool {
    path.split(std::path::is_separator)
        .any(|name| name == "." || name == "..")
}

/// Whether `place` is strictly inside one of `roots` (canonical).
fn inside(roots: &[PathBuf], place: &Path) -> bool {
    roots.iter().any(|r| place != r && place.starts_with(r))
}

/// The canonical path of `path` if it may be read from `roots`
/// (INVARIANTS 1-5), with the checked file's metadata.
fn inspect(
    roots: &[PathBuf],
    path: &str,
    protected: &Protected,
) -> Result<(PathBuf, std::fs::Metadata), String> {
    if roots.is_empty() {
        return Err(NO_ROOTS.to_string());
    }
    let given = Path::new(path);
    if !given.is_absolute() {
        return Err("path must be absolute".to_string());
    }
    // INVARIANT 2: no `.` or `..` name, before anything is looked up.
    // Split by hand: `Path::components` drops a `.` that isn't first.
    if has_dot_name(path) {
        return Err(NO_SUCH_FILE.to_string());
    }
    // Where the path leads, existing or not, every link followed, a
    // dangling one included (INVARIANT 2): the kernel's own answer when
    // there is a file there (nothing there, a link loop, or a name under
    // a file, `shot.png/sub/..`, is missing), else where the links lead.
    // Outside every root, nothing more is said.
    let canonical = given.canonicalize();
    let place = match &canonical {
        Ok(c) => c.clone(),
        Err(_) => resolved(given),
    };
    if !inside(roots, &place) {
        return Err(NO_SUCH_FILE.to_string());
    }
    let under = |dirs: &[PathBuf]| {
        dirs.iter()
            .map(|p| resolved(p))
            .any(|p| place.starts_with(&p))
    };
    if under(&protected.sealed) {
        return Err(SEALED.to_string());
    }
    if under(&protected.books) {
        return Err(IN_THE_BOOKS.to_string());
    }
    if canonical.is_err() {
        return Err(NO_SUCH_FILE.to_string());
    }
    let meta = std::fs::metadata(&place).map_err(|e| cant_read(&e))?;
    if !meta.is_file() {
        return Err(NOT_REGULAR.into());
    }
    if hard_linked(&meta) {
        return Err(HARD_LINKED.to_string());
    }
    Ok((place, meta))
}

/// Open and read the file `inspect` checked (INVARIANT 6): by a walk that
/// never follows a symlink, never blocks, and never lands on a protected
/// place, and only if the handle is the checked file.
fn read_checked(
    canonical: &Path,
    checked: &std::fs::Metadata,
    avoid: &[(u64, u64)],
    limit: usize,
) -> Result<Vec<u8>, String> {
    let mut file = open_walk(canonical, avoid)?;
    // The handle must still be the regular file just checked.
    let meta = file.metadata().map_err(|e| cant_read(&e))?;
    if !meta.is_file() || hard_linked(&meta) {
        return Err(NOT_REGULAR.into());
    }
    same_file(checked, &meta)?;
    // At most one byte past the limit is read, whatever the file's size
    // says (it may grow while it is read).
    let mut bytes = Vec::new();
    file.by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| cant_read(&e))?;
    if bytes.len() > limit {
        return Err("file exceeds 25 MB limit".to_string());
    }
    Ok(bytes)
}

/// Open `canonical` by walking it from `/` (INVARIANT 6): each folder
/// with `openat(O_DIRECTORY | O_NOFOLLOW)` on the one before, the file
/// with `openat(O_NOFOLLOW | O_NONBLOCK)`. A name on the path swapped for
/// a symlink since the check fails the walk (ELOOP, ENOTDIR), and so does
/// a step onto a protected place (`avoid`: device and inode).
#[cfg(unix)]
fn open_walk(canonical: &Path, avoid: &[(u64, u64)]) -> Result<std::fs::File, String> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    let step = |dir: Option<&std::fs::File>, name: &[u8], flags: libc::c_int| {
        let name = std::ffi::CString::new(name).map_err(|_| NO_SUCH_FILE.to_string())?;
        let file = openat(dir, &name, flags).map_err(|e| match e.raw_os_error() {
            Some(libc::ELOOP | libc::ENOTDIR | libc::ENOENT) => CHANGED.to_string(),
            _ => cant_read(&e),
        })?;
        let meta = file.metadata().map_err(|e| cant_read(&e))?;
        if avoid.contains(&(meta.dev(), meta.ino())) {
            return Err(CHANGED.to_string());
        }
        Ok(file)
    };
    let mut names: Vec<&[u8]> = Vec::new();
    for c in canonical.components() {
        match c {
            Component::RootDir => {}
            Component::Normal(n) => names.push(n.as_bytes()),
            // A canonical path has no `.`, `..` or prefix.
            _ => return Err(CHANGED.to_string()),
        }
    }
    let (file, folders) = names.split_last().ok_or_else(|| NOT_REGULAR.to_string())?;
    let mut dir = step(None, b"/", libc::O_RDONLY | libc::O_DIRECTORY)?;
    for name in folders {
        dir = step(
            Some(&dir),
            name,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )?;
    }
    step(
        Some(&dir),
        file,
        libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK,
    )
}

/// `openat(2)` relative to `dir` (or the working folder, for an absolute
/// `name`), close-on-exec.
#[cfg(unix)]
fn openat(
    dir: Option<&std::fs::File>,
    name: &std::ffi::CStr,
    flags: libc::c_int,
) -> std::io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let at = dir.map_or(libc::AT_FDCWD, |d| d.as_raw_fd());
    // SAFETY: `name` is a valid NUL-terminated string that outlives the
    // call, and `at` is AT_FDCWD or a descriptor `dir` keeps open for it.
    let fd = unsafe { libc::openat(at, name.as_ptr(), flags | libc::O_CLOEXEC) };
    // openat(2) returns -1 on error, else the new descriptor.
    if fd == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: openat just returned this descriptor, open and owned by no
    // one else.
    Ok(std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) }))
}

#[cfg(not(unix))]
fn open_walk(canonical: &Path, _avoid: &[(u64, u64)]) -> Result<std::fs::File, String> {
    std::fs::File::open(canonical).map_err(|e| cant_read(&e))
}

/// INVARIANT 6: the opened handle is the file that was checked.
#[cfg(unix)]
fn same_file(checked: &std::fs::Metadata, opened: &std::fs::Metadata) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    if (checked.dev(), checked.ino()) == (opened.dev(), opened.ino()) {
        Ok(())
    } else {
        Err(CHANGED.to_string())
    }
}

#[cfg(not(unix))]
fn same_file(_checked: &std::fs::Metadata, _opened: &std::fs::Metadata) -> Result<(), String> {
    Ok(())
}

/// A file that changed between its check and its open.
pub const CHANGED: &str = "that file changed while it was being opened (a folder on its path moved or became a link): try again";

/// Every path that is missing, a dangling link, or outside the import
/// folders (INVARIANT 2).
pub const NO_SUCH_FILE: &str = "no such file in the server's import folders (by default your downloads folder): save it there first";

/// Every read when the server has no import folder (INVARIANT 0).
pub const NO_ROOTS: &str = "the books server has no import folder to read from (by default your downloads folder, which it can't use here): file it as base64 instead, or ask the human to set up an import folder for the books server";

/// A path into the books' or the vault's own files (INVARIANT 3).
pub const SEALED: &str = "that file belongs to the books or the TIN vault, which are never imported: these tools are the only way to them";

/// Any other file beside the books (INVARIANT 3).
pub const IN_THE_BOOKS: &str = "that file is one of the books' own, which are never read in place: save a copy of it in the server's import folders first";

/// A file with more than one hard link (INVARIANT 4).
pub const HARD_LINKED: &str = "that file has more than one hard link: save a plain copy of it in the server's import folders first";

/// A folder, pipe, or device.
pub const NOT_REGULAR: &str = "not a regular file (a folder, pipe, or device can't be imported)";

/// A read failure, without the path (it may be where a symlink led).
fn cant_read(e: &std::io::Error) -> String {
    format!("can't read that file: {}", e.kind())
}

#[cfg(unix)]
fn hard_linked(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.nlink() > 1
}

#[cfg(not(unix))]
fn hard_linked(_meta: &std::fs::Metadata) -> bool {
    false
}

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
    use proptest::prelude::*;
    use std::collections::HashMap;
    use std::os::unix::fs::symlink;

    /// A tree like a real machine's: a downloads root, a temp root holding
    /// the books (data dir, vault) and the vault key's dir, a folder
    /// outside every root, and a sealed folder outside every root (the
    /// default key folder). `allowed` maps each canonical file path to
    /// whether it may be imported: the oracle, written down by hand.
    struct Tree {
        base: PathBuf,
        roots: ImportRoots,
        protected: Protected,
        allowed: HashMap<PathBuf, bool>,
    }

    fn tree(tag: &str) -> Tree {
        let base = std::env::temp_dir().join(format!(
            "ledger-mcp-local-files-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let dl = base.join("Downloads");
        let tmp = base.join("tmp");
        let books = tmp.join("books");
        let data = books.join("books-data");
        let vault = books.join("vault");
        let keys = tmp.join("keys");
        let out = base.join("elsewhere");
        let secret = base.join("secret");
        for d in [
            secret.clone(),
            dl.join("sub"),
            dl.join("dir"),
            data.join("blobs"),
            vault.clone(),
            keys.clone(),
            out.clone(),
        ] {
            std::fs::create_dir_all(&d).unwrap();
        }
        let write = |p: &Path| std::fs::write(p, b"bytes").unwrap();
        let mut allowed = HashMap::new();
        for (p, ok) in [
            (dl.join("receipt.pdf"), true),
            (dl.join("sub/scan.png"), true),
            (tmp.join("shot.png"), true),
            (books.join("wanderbooks.toml"), false),
            (data.join("journal.jsonl"), false),
            (data.join("blobs/ab12"), false),
            (vault.join("records.json"), false),
            (keys.join("vault.key"), false),
            (out.join("other.pdf"), false),
            (secret.join("vault.key"), false),
        ] {
            write(&p);
            allowed.insert(p.canonicalize().unwrap(), ok);
        }
        symlink(keys.join("vault.key"), dl.join("key-link")).unwrap();
        symlink(dl.join("receipt.pdf"), dl.join("good-link")).unwrap();
        symlink(out.join("other.pdf"), dl.join("out-link")).unwrap();
        symlink(&data, dl.join("data-link")).unwrap();
        std::fs::hard_link(keys.join("vault.key"), tmp.join("hard.key")).unwrap();
        allowed.insert(tmp.join("hard.key").canonicalize().unwrap(), false);
        let fifo = dl.join("fifo");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if made {
            allowed.insert(fifo.canonicalize().unwrap(), false);
        }
        Tree {
            roots: ImportRoots::new(vec![dl, tmp]),
            protected: Protected {
                sealed: vec![data, vault, keys, secret],
                books: vec![books],
            },
            base,
            allowed,
        }
    }

    impl Tree {
        fn check(&self, p: &Path) -> Result<PathBuf, String> {
            self.roots.check(p.to_str().unwrap(), &self.protected)
        }
    }

    #[test]
    fn import_local_file_refuses_key_and_journal() {
        let t = tree("refuses");
        let b = &t.base;
        let refused = [
            (b.join("tmp/keys/vault.key"), "belongs to the books"),
            (
                b.join("tmp/books/books-data/journal.jsonl"),
                "belongs to the books",
            ),
            (
                b.join("tmp/books/books-data/blobs/ab12"),
                "belongs to the books",
            ),
            (b.join("tmp/books/wanderbooks.toml"), "save a copy"),
            (b.join("Downloads/key-link"), "belongs to the books"),
            (
                b.join("Downloads/data-link/journal.jsonl"),
                "belongs to the books",
            ),
            (
                b.join("Downloads/../tmp/books/books-data/journal.jsonl"),
                "no such file",
            ),
            (b.join("tmp/hard.key"), "more than one hard link"),
            (b.join("Downloads/dir"), "not a regular file"),
            (b.join("Downloads"), "no such file"),
            (b.join("Downloads/out-link"), "no such file"),
            (b.join("Downloads/../elsewhere/other.pdf"), "no such file"),
            (b.join("Downloads/missing.pdf"), "no such file"),
        ];
        for (p, why) in &refused {
            let err = t
                .check(p)
                .expect_err(&format!("{} must be refused", p.display()));
            assert!(err.contains(why), "{}: {err}", p.display());
            // INVARIANT 5: the refusal never names a protected path.
            assert!(!err.contains(b.to_str().unwrap()), "{err}");
        }
        if t.allowed.keys().any(|p| p.ends_with("fifo")) {
            let err = t.check(&b.join("Downloads/fifo")).unwrap_err();
            assert!(err.contains("not a regular file"), "{err}");
        }
        assert!(
            t.check(Path::new("relative.pdf"))
                .unwrap_err()
                .contains("absolute")
        );
        for ok in [
            b.join("Downloads/receipt.pdf"),
            b.join("Downloads/good-link"),
            b.join("Downloads/sub/scan.png"),
            b.join("tmp/shot.png"),
        ] {
            assert!(t.check(&ok).is_ok(), "{} must be allowed", ok.display());
        }
        let (at, bytes) = t
            .roots
            .read(
                b.join("Downloads/good-link").to_str().unwrap(),
                &t.protected,
                5,
            )
            .unwrap();
        assert_eq!(bytes, b"bytes");
        assert!(at.ends_with("Downloads/receipt.pdf"));
        assert!(
            t.roots
                .read(
                    b.join("Downloads/receipt.pdf").to_str().unwrap(),
                    &t.protected,
                    4
                )
                .unwrap_err()
                .contains("limit")
        );
        let _ = std::fs::remove_dir_all(b);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        /// Any absolute path built from the tree's names, `.` and `..`, and
        /// its symlinks is allowed exactly when it has no `.` or `..` name
        /// and resolves to a file the oracle allows: a regular file inside
        /// a root and outside every protected dir.
        #[test]
        fn import_allowed_iff_regular_file_inside_a_root_outside_the_books(
            start in 0usize..7,
            parts in proptest::collection::vec(0usize..14, 0..5),
        ) {
            use std::sync::OnceLock;
            static TREE: OnceLock<Tree> = OnceLock::new();
            let t = TREE.get_or_init(|| tree("prop"));
            let starts = ["Downloads", "tmp", "tmp/books", "tmp/books/books-data", "tmp/keys", "elsewhere", "secret"];
            let names = [
                "..", ".", "sub", "dir", "receipt.pdf", "scan.png", "shot.png", "journal.jsonl",
                "vault.key", "key-link", "good-link", "data-link", "hard.key", "fifo",
            ];
            let mut p = t.base.join(starts[start]);
            for i in parts {
                p.push(names[i]);
            }
            // A `.` or `..` name is refused before anything is looked up.
            let dotted = p.to_str().unwrap().split('/').any(|n| n == "." || n == "..");
            let want = match p.canonicalize() {
                Ok(c) if !dotted => t.allowed.get(&c).copied().unwrap_or(false),
                _ => false,
            };
            prop_assert_eq!(t.check(&p).is_ok(), want, "{}", p.display());
        }
    }

    /// INVARIANT 2: a path that is missing, a dangling link, or outside
    /// the roots gets the same one refusal, whether a file is there or
    /// not, sealed or not, and whether a link's target exists or not, so
    /// a refusal can't be used to look around. Inside a root, sealed
    /// places are refused as the books' own (existing or not, directly or
    /// through a link, dangling or not), and the rest of the books dir is
    /// filed from a copy.
    #[test]
    fn refusals_never_tell_what_is_there() {
        let t = tree("probe");
        let b = &t.base;
        let dl = b.join("Downloads");
        symlink(b.join("elsewhere/missing.pdf"), dl.join("dangling-out")).unwrap();
        symlink(dl.join("missing.pdf"), dl.join("dangling-in")).unwrap();
        symlink(b.join("tmp/keys/missing.key"), dl.join("dangling-key")).unwrap();
        symlink(b.join("tmp/keys/vault.key"), dl.join("key-link-2")).unwrap();
        for p in [
            dl.join("missing.pdf"),
            dl.join("dangling-in"),
            dl.join("dangling-out"),
            b.join("elsewhere/other.pdf"),
            b.join("elsewhere/missing.pdf"),
            b.join("elsewhere"),
            b.join("secret/vault.key"),
            b.join("secret/missing.key"),
            b.join("no-such-dir/x.pdf"),
            b.join("Downloads/out-link"),
            b.join("Downloads/../secret/vault.key"),
            b.join("Downloads/missing/../../secret/vault.key"),
        ] {
            assert_eq!(
                t.check(&p),
                Err(NO_SUCH_FILE.to_string()),
                "{}",
                p.display()
            );
        }
        for p in [
            b.join("tmp/books/books-data/journal.jsonl"),
            b.join("tmp/books/books-data/no-such-line"),
            b.join("tmp/books/vault/records.json"),
            b.join("tmp/keys/vault.key"),
            b.join("tmp/keys/missing.key"),
            dl.join("key-link-2"),
            dl.join("dangling-key"),
        ] {
            assert_eq!(t.check(&p), Err(SEALED.to_string()), "{}", p.display());
        }
        for p in [
            b.join("tmp/books/wanderbooks.toml"),
            b.join("tmp/books/company/missing.pdf"),
        ] {
            assert_eq!(
                t.check(&p),
                Err(IN_THE_BOOKS.to_string()),
                "{}",
                p.display()
            );
        }
        let _ = std::fs::remove_dir_all(b);
    }

    /// INVARIANT 2: a path with a `.` or `..` name is refused as missing
    /// before anything is looked up, so it can't tell whether a folder
    /// outside the roots exists (`a/b/..` resolves only when `b` is a
    /// folder), even when it would lead back to a file the roots allow.
    #[test]
    fn a_dot_or_dotdot_name_never_tells_whether_folders_outside_exist() {
        let t = tree("dots");
        let b = &t.base;
        let base = b.to_str().unwrap();
        for p in [
            format!("{base}/Downloads/../elsewhere/../Downloads/receipt.pdf"),
            format!("{base}/Downloads/../no-such-dir/../Downloads/receipt.pdf"),
            format!("{base}/elsewhere/../Downloads/receipt.pdf"),
            format!("{base}/no-such-dir/../Downloads/receipt.pdf"),
            format!("{base}/Downloads/sub/../receipt.pdf"),
            format!("{base}/Downloads/./receipt.pdf"),
            format!("{base}/Downloads/receipt.pdf/."),
            format!("{base}/Downloads/sub/.."),
            format!("{base}/tmp/books/books-data/../books-data/journal.jsonl"),
            format!("{base}/tmp/keys/./vault.key"),
        ] {
            assert_eq!(
                t.roots.check(&p, &t.protected),
                Err(NO_SUCH_FILE.to_string()),
                "{p}"
            );
            assert_eq!(
                t.roots.read(&p, &t.protected, 1 << 20).unwrap_err(),
                NO_SUCH_FILE,
                "{p}"
            );
        }
        // The same file, named plainly, is read.
        assert!(
            t.roots
                .check(&format!("{base}/Downloads/receipt.pdf"), &t.protected)
                .is_ok()
        );
        // Names that merely contain dots are names.
        std::fs::write(b.join("Downloads/..receipt.pdf"), b"bytes").unwrap();
        std::fs::write(b.join("Downloads/receipt..pdf"), b"bytes").unwrap();
        for name in ["..receipt.pdf", "receipt..pdf"] {
            assert!(
                t.roots
                    .check(&format!("{base}/Downloads/{name}"), &t.protected)
                    .is_ok(),
                "{name}"
            );
        }
        let _ = std::fs::remove_dir_all(b);
    }

    /// INVARIANT 6: a file swapped after the check is never read. Another
    /// file renamed over it differs by inode; a symlink is never followed;
    /// a pipe neither blocks the open nor is read.
    #[test]
    fn a_file_swapped_after_the_check_is_never_read() {
        let t = tree("swap");
        let dl = t.base.join("Downloads");
        let target = dl.join("swapped.pdf");
        let check = |what: &str| {
            std::fs::write(&target, b"checked bytes").unwrap();
            let (canonical, meta) =
                inspect(&t.roots.usable(), target.to_str().unwrap(), &t.protected)
                    .unwrap_or_else(|e| panic!("{what}: {e}"));
            std::fs::remove_file(&target).unwrap();
            (canonical, meta)
        };

        let (canonical, meta) = check("rename");
        std::fs::write(dl.join("other.pdf"), b"other bytes").unwrap();
        std::fs::rename(dl.join("other.pdf"), &target).unwrap();
        assert_eq!(
            read_checked(&canonical, &meta, &t.protected.inodes(), 100),
            Err(CHANGED.to_string())
        );

        let (canonical, meta) = check("symlink");
        symlink(t.base.join("tmp/keys/vault.key"), &target).unwrap();
        assert_eq!(
            read_checked(&canonical, &meta, &t.protected.inodes(), 100),
            Err(CHANGED.to_string())
        );
        std::fs::remove_file(&target).unwrap();

        let (canonical, meta) = check("fifo");
        let made = std::process::Command::new("mkfifo")
            .arg(&target)
            .status()
            .is_ok_and(|s| s.success());
        if made {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                tx.send(read_checked(&canonical, &meta, &t.protected.inodes(), 100))
            });
            let got = rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("opening a pipe blocked");
            assert_eq!(got, Err(NOT_REGULAR.to_string()));
        }
        let _ = std::fs::remove_dir_all(&t.base);
    }

    /// Where a file could be read back in: strictly inside a root,
    /// existing or not; a root itself, or a path outside, is not.
    #[test]
    fn contains_is_strictly_inside_a_root() {
        let t = tree("contains");
        let b = &t.base;
        assert!(t.roots.contains(&b.join("Downloads/receipt.pdf")));
        assert!(t.roots.contains(&b.join("Downloads/new/file.zip")));
        assert!(t.roots.contains(&b.join("Downloads/good-link")));
        assert!(!t.roots.contains(&b.join("Downloads")));
        assert!(!t.roots.contains(&b.join("elsewhere/x.zip")));
        assert!(!t.roots.contains(&b.join("Downloads/../elsewhere/x.zip")));
        assert!(!t.roots.contains(&b.join("Downloads/out-link")));
        let _ = std::fs::remove_dir_all(b);
    }

    /// INVARIANT 6, the reviewer's race (P0 review round 2): a folder on
    /// the checked path is swapped for a link to the data dir after the
    /// check, and the file's metadata is taken after the swap (so its
    /// device and inode are the journal's). The walk refuses the link,
    /// with no help from the kernel's /proc paths; and a protected folder
    /// renamed into place (no link to refuse) is refused by its inode.
    #[test]
    fn a_folder_swapped_after_the_check_is_never_walked_through() {
        let t = tree("swap-dir");
        let dl = t.base.join("Downloads");
        let data = t.base.join("tmp/books/books-data");
        std::fs::write(dl.join("sub/journal.jsonl"), b"decoy").unwrap();
        let given = dl.join("sub/journal.jsonl");
        let canonical = t.check(&given).unwrap();
        // Taken as a read takes them: before the check.
        let avoid = t.protected.inodes();

        // A link: the walk fails at it (ELOOP or ENOTDIR).
        std::fs::rename(dl.join("sub"), dl.join("sub.old")).unwrap();
        symlink(&data, dl.join("sub")).unwrap();
        let meta = std::fs::metadata(&canonical).unwrap();
        assert_eq!(
            read_checked(&canonical, &meta, &avoid, 1000),
            Err(CHANGED.to_string())
        );
        std::fs::remove_file(dl.join("sub")).unwrap();

        // The data dir itself renamed into place: every name on the walk
        // is a real folder, but one of them is protected.
        std::fs::rename(&data, dl.join("sub")).unwrap();
        let meta = std::fs::metadata(&canonical).unwrap();
        assert_eq!(
            read_checked(&canonical, &meta, &avoid, 1000),
            Err(CHANGED.to_string())
        );
        std::fs::rename(dl.join("sub"), &data).unwrap();
        std::fs::rename(dl.join("sub.old"), dl.join("sub")).unwrap();

        // Untouched, the same walk reads the checked file.
        let meta = std::fs::metadata(&canonical).unwrap();
        assert_eq!(
            read_checked(&canonical, &meta, &avoid, 1000).unwrap(),
            b"decoy"
        );
        let _ = std::fs::remove_dir_all(&t.base);
    }

    /// INVARIANT 6, links and pipes on the walk: a folder on the checked
    /// path swapped for a link to a folder outside every root (nothing
    /// protected there, and the file's metadata taken after the swap, so
    /// it is the other file's) is never walked through; a folder swapped
    /// for a pipe neither blocks the walk nor is opened.
    #[test]
    fn a_folder_swapped_for_a_link_or_a_pipe_is_never_walked_through() {
        let t = tree("swap-link");
        let dl = t.base.join("Downloads");
        let out = t.base.join("elsewhere");
        std::fs::write(dl.join("sub/other.pdf"), b"checked").unwrap();
        let canonical = t.check(&dl.join("sub/other.pdf")).unwrap();
        let checked = std::fs::metadata(&canonical).unwrap();
        let avoid = t.protected.inodes();
        std::fs::rename(dl.join("sub"), dl.join("sub.old")).unwrap();

        symlink(&out, dl.join("sub")).unwrap();
        let meta = std::fs::metadata(&canonical).unwrap();
        assert_eq!(
            std::fs::read(&canonical).unwrap(),
            b"bytes",
            "the link leads to the other file"
        );
        assert_eq!(
            read_checked(&canonical, &meta, &avoid, 1000),
            Err(CHANGED.to_string())
        );
        std::fs::remove_file(dl.join("sub")).unwrap();

        let made = std::process::Command::new("mkfifo")
            .arg(dl.join("sub"))
            .status()
            .is_ok_and(|s| s.success());
        if made {
            let (tx, rx) = std::sync::mpsc::channel();
            let (c, a) = (canonical.clone(), avoid.clone());
            std::thread::spawn(move || tx.send(read_checked(&c, &checked, &a, 1000)));
            let got = rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("walking onto a pipe blocked");
            assert_eq!(got, Err(CHANGED.to_string()));
        }
        let _ = std::fs::remove_dir_all(&t.base);
    }

    /// A read failure names what went wrong, never the path.
    #[test]
    fn cant_read_names_the_failure_not_the_path() {
        let e = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "/home/x/secret");
        assert_eq!(cant_read(&e), "can't read that file: permission denied");
    }

    /// `resolved` follows as many links as the kernel does (40, as
    /// `canonicalize` does), and no more.
    #[test]
    fn resolved_follows_forty_links_and_no_more() {
        let t = tree("hops");
        let b = t.base.canonicalize().unwrap();
        let file = b.join("Downloads/receipt.pdf");
        // link-n leads to link-(n-1), ..., link-1 to the file: n links.
        let link = |n: usize| b.join(format!("Downloads/link-{n}"));
        symlink(&file, link(1)).unwrap();
        for n in 2..=41 {
            symlink(link(n - 1), link(n)).unwrap();
        }
        assert_eq!(link(40).canonicalize().unwrap(), file);
        assert_eq!(resolved(&link(40)), file);
        assert!(link(41).canonicalize().is_err(), "the kernel stops at 40");
        assert_ne!(resolved(&link(41)), file);
        let _ = std::fs::remove_dir_all(&t.base);
    }

    /// `resolved` places a path where the kernel would follow it: the
    /// canonical form for one that exists, and through a dangling link to
    /// where it points.
    #[test]
    fn resolved_follows_every_link_a_dangling_one_too() {
        let t = tree("resolved");
        let b = t.base.canonicalize().unwrap();
        for p in [
            b.join("Downloads/good-link"),
            b.join("Downloads/data-link/journal.jsonl"),
            b.join("Downloads/sub/../receipt.pdf"),
            b.join("Downloads/./key-link"),
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
        let _ = std::fs::remove_dir_all(&t.base);
    }

    /// INVARIANT 0, the default root: the downloads folder, never `/` or
    /// the home folder (an XDG_DOWNLOAD_DIR of "$HOME/", "$HOME", "/"
    /// falls back to ~/Downloads), and ~/Downloads only when it is a real
    /// folder that isn't the home folder; else no roots, and every read
    /// says so.
    #[test]
    fn the_default_root_is_never_the_home_folder_or_the_filesystem() {
        let base = std::env::temp_dir().join(format!(
            "ledger-mcp-local-files-{}-defaults",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let home = base.join("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh/id_ed25519"), b"PRIVATE KEY").unwrap();
        let home = home.canonicalize().unwrap();
        let os = |s: &str| Some(std::ffi::OsString::from(s));
        let roots = |xdg: Option<std::ffi::OsString>| {
            ImportRoots::defaults_from(xdg, Some(home.clone())).usable()
        };
        // No ~/Downloads: nothing to fall back to.
        for xdg in [
            os("$HOME/"),
            os("$HOME"),
            os("${HOME}"),
            os("/"),
            os("Downloads"),
            None,
        ] {
            assert_eq!(roots(xdg.clone()), Vec::<PathBuf>::new(), "{xdg:?}");
        }
        let err = ImportRoots::defaults_from(os("$HOME/"), Some(home.clone()))
            .read(
                home.join(".ssh/id_ed25519").to_str().unwrap(),
                &Protected::default(),
                100,
            )
            .unwrap_err();
        assert_eq!(err, NO_ROOTS);
        // ~/Downloads a link to the home folder: still the home folder.
        symlink(&home, home.join("Downloads")).unwrap();
        assert!(roots(None).is_empty());
        std::fs::remove_file(home.join("Downloads")).unwrap();
        // A real ~/Downloads: the fallback, and the default.
        std::fs::create_dir_all(home.join("Downloads")).unwrap();
        let downloads = vec![home.join("Downloads")];
        for xdg in [
            os("$HOME/"),
            os("$HOME"),
            os("/"),
            os("$HOME/Downloads"),
            None,
        ] {
            assert_eq!(roots(xdg.clone()), downloads, "{xdg:?}");
        }
        // Another real folder named by XDG_DOWNLOAD_DIR wins.
        std::fs::create_dir_all(home.join("Inbox")).unwrap();
        assert_eq!(roots(os("$HOME/Inbox")), vec![home.join("Inbox")]);
        // Without HOME, "$HOME/..." means nothing.
        assert!(
            ImportRoots::defaults_from(os("$HOME/Inbox"), None)
                .usable()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// INVARIANT 0, a named root: `/`, the home folder, a folder holding
    /// it, and a folder holding the books, the vault or a key are refused
    /// (with why, never silently); anything else is kept.
    #[test]
    fn a_root_holding_the_books_or_the_home_folder_is_refused() {
        let t = tree("guard");
        let b = t.base.canonicalize().unwrap();
        let home = b.join("elsewhere");
        let keep_out = vec![
            b.join("tmp/books/books-data"),
            b.join("tmp/books"),
            b.join("tmp/keys/vault.key"),
        ];
        let roots = ImportRoots::new(vec![
            PathBuf::from("/"),
            home.clone(),
            b.clone(),
            b.join("tmp"),
            b.join("tmp/keys"),
            b.join("Downloads"),
            b.join("no-such-root"),
            b.join("tmp/books/books-data/blobs"),
            b.join("tmp/books/vault"),
        ]);
        let (kept, refused) = roots.guard(Some(&home), &keep_out);
        assert!(kept.explicit());
        assert_eq!(kept.usable(), vec![b.join("Downloads")]);
        let refused: Vec<(PathBuf, &str)> = refused;
        assert_eq!(
            refused.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(),
            vec![
                PathBuf::from("/"),
                home.clone(),
                b.clone(),
                b.join("tmp"),
                b.join("tmp/keys"),
                b.join("tmp/books/books-data/blobs"),
                b.join("tmp/books/vault"),
            ]
        );
        assert!(refused[0].1.contains("whole filesystem"));
        assert!(refused[1].1.contains("home folder"));
        assert!(refused[3].1.contains("holds the books"));
        assert!(
            refused[5].1.contains("inside the books"),
            "{}",
            refused[5].1
        );
        assert!(
            refused[6].1.contains("inside the books"),
            "{}",
            refused[6].1
        );
        let _ = std::fs::remove_dir_all(&t.base);
    }

    /// Protected::of INVARIANTS 1-2: the key files are sealed wherever
    /// they are, with where they lead; their folders only when sealing
    /// one wouldn't seal a whole root (the home folder, a root, a folder
    /// holding one), so a key kept beside receipts doesn't block every
    /// import. ~/.config/wanderbooks is sealed whatever XDG_CONFIG_HOME
    /// says.
    #[test]
    fn sealing_a_key_never_seals_a_whole_root() {
        let t = tree("seal-key");
        let b = t.base.canonicalize().unwrap();
        let dl = b.join("Downloads");
        let home = b.join("home");
        std::fs::create_dir_all(home.join(".config/wanderbooks")).unwrap();
        std::fs::write(home.join(".config/wanderbooks/vault.key"), b"old key").unwrap();
        std::fs::write(home.join("vault.key"), b"key in home").unwrap();
        std::fs::write(dl.join("vault.key"), b"key in a root").unwrap();
        std::fs::write(dl.join("sub/vault.key"), b"key in a folder").unwrap();
        let roots = vec![dl.clone(), home.join("Downloads")];
        let protected = |key: PathBuf| {
            Protected::of(
                &Places {
                    data_dir: b.join("tmp/books/books-data"),
                    books_dir: b.join("tmp/books"),
                    vault_dir: b.join("tmp/books/vault"),
                    key_files: vec![key, b.join("xdg/wanderbooks/vault.key")],
                    home: Some(home.clone()),
                },
                &roots,
            )
        };
        let r = ImportRoots::new(vec![dl.clone()]);
        let check = |key: PathBuf, path: PathBuf| r.check(path.to_str().unwrap(), &protected(key));
        // In a root: the key is sealed, the root isn't.
        assert_eq!(
            check(dl.join("vault.key"), dl.join("vault.key")),
            Err(SEALED.into())
        );
        assert!(check(dl.join("vault.key"), dl.join("receipt.pdf")).is_ok());
        // In a folder inside a root: that folder is sealed, the root isn't.
        assert_eq!(
            check(dl.join("sub/vault.key"), dl.join("sub/scan.png")),
            Err(SEALED.into())
        );
        assert!(check(dl.join("sub/vault.key"), dl.join("receipt.pdf")).is_ok());
        // In the home folder: the home folder (holding a root) isn't.
        let p = protected(home.join("vault.key"));
        assert!(!p.sealed.contains(&home));
        assert!(p.sealed.contains(&home.join("vault.key")));
        // A link to a key: the link and its target are both sealed (the
        // target is outside the root here, so a path to it is refused
        // first as outside: INVARIANT 2).
        assert_eq!(
            check(dl.join("key-link"), dl.join("key-link")),
            Err(NO_SUCH_FILE.into())
        );
        let p = protected(dl.join("key-link"));
        assert!(p.sealed.contains(&dl.join("key-link")));
        assert!(p.sealed.contains(&b.join("tmp/keys/vault.key")));
        assert!(p.sealed.contains(&b.join("tmp/keys")));
        // Always: ~/.config/wanderbooks, and the default key's folder.
        assert!(p.sealed.contains(&home.join(".config/wanderbooks")));
        assert!(p.sealed.contains(&b.join("xdg/wanderbooks")));
        let _ = std::fs::remove_dir_all(&t.base);
    }
}
