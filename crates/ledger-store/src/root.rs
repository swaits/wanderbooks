//! The data root: the one private folder a books server keeps every book
//! in (plan section 3.4).
//!
//! ```text
//! GOAL: the books live hidden away, as the human: a folder only their
//!       account can open, outside every project folder, found the same
//!       way by the server, the CLI and the viewer.
//! SOURCE: DECISION (owner, 2026-10-16: "you, hidden away"); plan/served.md
//!         3.4 (layout), 3.9 (the host never trusts the opener's
//!         environment), 5.5 (adopting existing books).
//! INVARIANTS:
//!   1. The root, every folder made in it, and every missing folder above
//!      it that setup makes, is created with mode 0700 set explicitly,
//!      never left to the umask; files are 0600.
//!   2. Opening a root refuses one that is a symlink, isn't a folder,
//!      belongs to another user, that other users can enter, that setup
//!      didn't make (no marker), or that sits under a folder another user
//!      could swap it in (one they own, or can write without the sticky
//!      bit).
//!   3. Finding the root never reads the environment: an explicit root,
//!      else the one `setup` recorded (in a file only this account can
//!      write), else the platform's default under the account's home (from
//!      the password database on Unix).
//!   4. Setup never moves books: run again, it keeps the root it recorded,
//!      and it refuses to make a root elsewhere while the current one holds
//!      books; it refuses a folder that holds other things.
//!   5. Adopting books never changes the source's journal, blobs or
//!      database (it only takes the source's writer lock, creating the lock
//!      file if the source never had one), and a new book appears in the
//!      root only whole: every stored file re-hashed against its name,
//!      every file the books name present, verified end to end, synced to
//!      disk, and its head equal to the source's.
//!   6. No error names the root or a path inside it: nothing an agent is
//!      shown names the books' folder.
//!   7. Restoring a backup (plan 3.7) is proven before use, as an adoption
//!      is (INVARIANT 5), and its head must be the one the preview read;
//!      it never overwrites: books it replaces are moved whole into
//!      `legacy/`, only when asked, and only while nothing holds them.
//!   8. Putting old copies aside (after an adoption) deletes nothing: each
//!      path is moved whole into a numbered folder of its own in `legacy/`,
//!      a list of where each came from written there first; only when the
//!      adopted books hold everything the old copy holds, byte for byte
//!      (journal, stored files, vault records, key); only while no
//!      wanderbooks process holds it open for writing; only on the root's
//!      own disk; and a move cut short puts back what it moved, never over
//!      anything.
//! ```

use crate::error::StoreError;
use crate::journal::JournalLine;
use crate::store::{JOURNAL_FILE, Store};
use ledger_core::Currency;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

/// The root's folder name in the platform's data folder.
pub const ROOT_NAME: &str = "wanderbooks";

/// The file `setup` writes beside the default root when the human chose
/// another one; it holds that root's absolute path.
pub const LOCATION_FILE: &str = "wanderbooks.location";

/// The file `setup` leaves in a root it made: a root without it isn't
/// opened, and a non-empty folder without it isn't made into one.
pub const MARKER_FILE: &str = ".wanderbooks-root";

/// Where books live inside the root.
pub const BOOKS_DIR: &str = "books";
/// The sockets and runtime state (plan 3.9).
pub const RUN_DIR: &str = "run";
/// Who may reach the books: principals and the owner passphrase.
pub const AUTH_DIR: &str = "auth";
/// Old copies moved aside at adoption, never deleted (plan 5.5).
pub const LEGACY_DIR: &str = "legacy";
/// A book's TIN vault folder, in its own folder.
pub const VAULT_DIR: &str = "vault";
/// A book's key folder, in its own folder.
pub const KEYS_DIR: &str = "keys";
/// The v1 vault key, in a book's `keys/` (P2 to P7).
pub const LEGACY_KEY_FILE: &str = "legacy.key";
/// Held while books are created or adopted, so only one is made at a time.
const BOOKS_LOCK: &str = "books.lock";
/// Staging folders under `books/` start so; none is a valid book id.
const STAGING_PREFIX: &str = ".adopt-";
/// A backup being restored, in `books/` (never a book: ids don't start
/// with a dot).
const RESTORE_PREFIX: &str = ".restore-";
/// Left in a restored book's `ops/`: the id of the books its backup held
/// (empty when it named none), for the server to note when it opens them.
pub const RESTORED_FROM: &str = "restored-from";

/// How the root is named in messages (INVARIANT 6).
pub const THE_ROOT: &str = "the books' private folder";

/// Why a root or book operation was refused. No message names the root or
/// a path inside it (INVARIANT 6).
#[derive(Debug, thiserror::Error)]
pub enum RootError {
    /// No home folder to put the root under.
    #[error("can't find this account's home folder, so there is no default place for the books")]
    NoHome,
    /// The root, or a folder in or above it, isn't safe to use.
    #[error("{0}")]
    Unsafe(String),
    /// No root yet: `wanderbooks setup` makes one.
    #[error("there is no books' private folder yet: run `wanderbooks setup` first")]
    Missing,
    /// A book id that isn't lowercase letters, digits and hyphens.
    #[error(
        "{0:?} can't name books: use 1 to 40 lowercase letters, digits and hyphens, starting with a letter or digit"
    )]
    BadId(String),
    /// A book with that id already exists.
    #[error("there are already books called {0:?}")]
    Exists(String),
    /// Adopting was refused, with why.
    #[error("{0}")]
    Adopt(String),
    /// The store refused (its message with the root's path taken out).
    #[error("{0}")]
    Store(String, #[source] StoreError),
    /// Filesystem error (its message with the root's path taken out).
    #[error("{0}")]
    Io(String, #[source] std::io::Error),
}

impl RootError {
    /// The store's error, if this is one.
    pub fn store_error(&self) -> Option<&StoreError> {
        match self {
            Self::Store(_, e) => Some(e),
            _ => None,
        }
    }
}

impl From<StoreError> for RootError {
    fn from(e: StoreError) -> Self {
        Self::Store(e.to_string(), e)
    }
}

impl From<std::io::Error> for RootError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(format!("io error: {e}"), e)
    }
}

/// A book's id within the root: its folder name under `books/`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BookId(String);

impl BookId {
    /// Check an id: 1 to 40 lowercase ASCII letters, digits and hyphens,
    /// starting with a letter or digit.
    pub fn parse(id: &str) -> Result<Self, RootError> {
        let ok = (1..=40).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !id.starts_with('-');
        if ok {
            Ok(Self(id.to_string()))
        } else {
            Err(RootError::BadId(id.to_string()))
        }
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BookId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// This account's home folder, from the password database on Unix (never
/// `$HOME`, which the process that started us chose), or the Windows
/// profile.
pub fn account_home() -> Result<PathBuf, RootError> {
    #[cfg(unix)]
    {
        unix_account()
            .map(|(home, _)| home)
            .ok_or(RootError::NoHome)
    }
    #[cfg(windows)]
    {
        // Read from the environment until the Windows build (P11) takes it
        // from the known-folder API.
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .ok_or(RootError::NoHome)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(RootError::NoHome)
    }
}

/// Call a `get*_r` lookup with a buffer that grows (16 KiB, doubling to
/// at most 1 MiB) while it answers `ERANGE` (too small). `call` gets the
/// buffer and returns the lookup's code with what it read (only on 0).
/// Any other code, or `ERANGE` at the largest size, is `None`.
#[cfg(unix)]
fn with_growing_buffer<T>(mut call: impl FnMut(&mut [u8]) -> (i32, Option<T>)) -> Option<T> {
    let mut size = 16 * 1024;
    loop {
        let mut buf = vec![0u8; size];
        match call(&mut buf) {
            (0, found) => return found,
            (libc::ERANGE, _) if size < 1024 * 1024 => size *= 2,
            _ => return None,
        }
    }
}

/// This account's home folder and login name, from the password database.
#[cfg(unix)]
fn unix_account() -> Option<(PathBuf, Vec<u8>)> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    with_growing_buffer(|buf| {
        // SAFETY: an all-zero passwd is a valid value of this plain C
        // struct (null pointers, zero ids); getpwuid_r overwrites it.
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: getpwuid_r writes into pwd and buf, both alive and sized
        // as passed; result is set to &pwd on success or left null.
        let rc = unsafe {
            libc::getpwuid_r(
                libc::getuid(),
                &mut pwd,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            )
        };
        if rc != 0 || result.is_null() || pwd.pw_dir.is_null() || pwd.pw_name.is_null() {
            return (rc, None);
        }
        // SAFETY: pw_dir and pw_name point into buf, NUL-terminated by
        // getpwuid_r, and buf outlives these borrows.
        let (dir, name) = unsafe { (CStr::from_ptr(pwd.pw_dir), CStr::from_ptr(pwd.pw_name)) };
        let home = PathBuf::from(std::ffi::OsStr::from_bytes(dir.to_bytes()));
        (
            0,
            home.is_absolute().then(|| (home, name.to_bytes().to_vec())),
        )
    })
}

/// The platform's data folder under `home`: where the default root sits.
pub fn platform_data_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else if cfg!(windows) {
        home.join("AppData").join("Local")
    } else {
        home.join(".local").join("share")
    }
}

/// The default root under `home`.
pub fn default_root(home: &Path) -> PathBuf {
    platform_data_dir(home).join(ROOT_NAME)
}

/// Whether `path` is a data root or inside one (any folder above it, or
/// it, holds the marker `setup` leaves). Books there belong to the books
/// server alone (plan 3.1); the viewer reads no settings file beside them.
pub fn inside_a_root(path: &Path) -> bool {
    // From here (a path that is already absolute stays as it is).
    let path = std::env::current_dir()
        .map(|here| here.join(path))
        .unwrap_or_else(|_| path.to_path_buf());
    // Where the deepest folder that exists really is: a folder not made
    // yet holds no marker, and a link counts as where it leads.
    let Some(existing) = path.ancestors().find(|dir| dir.exists()) else {
        return false;
    };
    let real = std::fs::canonicalize(existing).unwrap_or_else(|_| existing.to_path_buf());
    real.ancestors().any(|dir| dir.join(MARKER_FILE).is_file())
}

/// A folder directly in a book's own folder (`keys/`, `vault/`): made
/// 0700 if missing, made 0700 again if it is ours but others could enter
/// it; refused if it is a symlink or belongs to another user. The error
/// names no path (INVARIANT 6).
pub fn private_book_folder(book_dir: &Path, name: &str) -> Result<PathBuf, RootError> {
    let dir = book_dir.join(name);
    match std::fs::symlink_metadata(&dir) {
        Ok(meta) => {
            let open = check_private_dir(&meta, true, "a folder inside the books")?;
            tighten_if(&dir, open)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            make_private_dir(&dir)?;
            sync_dir(book_dir)?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(dir)
}

/// The longest socket path used as it is: a Unix socket's address holds
/// 104 bytes on macOS and 108 on Linux, the terminating zero included.
const SOCKET_PATH_MAX: usize = 100;

/// The path to a socket named `name` in `dir`, short enough for the
/// system: the path itself when it fits; else, on Linux, the same place
/// reached through `/proc/self/fd` and a handle on `dir` kept open while
/// this lives (it is good only in this process). Elsewhere a path too
/// long is refused, saying so plainly (no path named).
#[derive(Debug)]
pub struct SocketPath {
    path: PathBuf,
    _dir: Option<std::fs::File>,
}

impl SocketPath {
    /// The socket `name` in `dir`.
    pub fn new(dir: &Path, name: &str) -> std::io::Result<Self> {
        let full = dir.join(name);
        if full.as_os_str().len() <= SOCKET_PATH_MAX {
            return Ok(Self {
                path: full,
                _dir: None,
            });
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            let handle = std::fs::File::open(dir)?;
            let path = PathBuf::from(format!("/proc/self/fd/{}/{name}", handle.as_raw_fd()));
            Ok(Self {
                path,
                _dir: Some(handle),
            })
        }
        #[cfg(not(target_os = "linux"))]
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the books' private folder's path is too long for its sockets: choose a shorter place with `wanderbooks setup --root`",
        ))
    }

    /// The path to bind or connect to.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The data root.
#[derive(Clone, Debug)]
pub struct Root {
    path: PathBuf,
}

/// What [`Root::setup`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupReport {
    /// The root.
    pub root: PathBuf,
    /// It didn't exist and was made.
    pub created: bool,
    /// It, or a folder in it, existed with wider permissions, now 0700.
    pub tightened: bool,
    /// It isn't the default, so its location is recorded.
    pub recorded: bool,
}

impl Root {
    /// Find the root for this account: `explicit` (`--root`) if given,
    /// else the one `setup` recorded, else the default. The environment
    /// is never read (INVARIANT 3). The root must already exist, safely
    /// (INVARIANT 2).
    pub fn find(explicit: Option<&Path>) -> Result<Self, RootError> {
        Self::find_in(&account_home()?, explicit)
    }

    /// [`Self::find`] for the account whose home is `home`.
    pub fn find_in(home: &Path, explicit: Option<&Path>) -> Result<Self, RootError> {
        Self::open(&Self::locate(home, explicit)?)
    }

    /// Where the root is, by the rule of [`Self::find`], without opening it.
    pub fn locate(home: &Path, explicit: Option<&Path>) -> Result<PathBuf, RootError> {
        if let Some(path) = explicit {
            return resolve_new(path);
        }
        Ok(recorded_root(home)?.unwrap_or_else(|| default_root(home)))
    }

    /// Open the root at `path`: it must exist, have been made by setup,
    /// and be safe (INVARIANT 2).
    pub fn open(path: &Path) -> Result<Self, RootError> {
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(RootError::Missing),
            Err(e) => return Err(e.into()),
        };
        check_private_dir(&meta, false, THE_ROOT)?;
        check_ancestors(path)?;
        if !path.join(MARKER_FILE).is_file() {
            return Err(RootError::Unsafe(format!(
                "{THE_ROOT} wasn't made by `wanderbooks setup` (it has no marker); run setup to make one"
            )));
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    /// Make the root (or make an existing one private again) for the
    /// account whose home is `home`, and its `books/`, `run/` and
    /// `legacy/` folders. Which root: `chosen` (`--root`) if given; else
    /// the root already recorded, if any; else
    /// `$XDG_DATA_HOME/wanderbooks` when `xdg_data_home` is given and
    /// absolute; else the default. A root other than the default is
    /// recorded beside the default, so later finds need no environment.
    /// Setup never moves books (INVARIANT 4).
    pub fn setup(
        home: &Path,
        chosen: Option<&Path>,
        xdg_data_home: Option<&Path>,
    ) -> Result<(Self, SetupReport), RootError> {
        let default = default_root(home);
        let recorded = recorded_root(home)?;
        let current = recorded.clone().unwrap_or_else(|| default.clone());
        let path = match (chosen, &recorded, xdg_data_home) {
            (Some(path), _, _) => resolve_new(path)?,
            (None, Some(recorded), _) => recorded.clone(),
            (None, None, Some(xdg)) if xdg.is_absolute() => resolve_new(&xdg.join(ROOT_NAME))?,
            _ => default.clone(),
        };
        if path != current {
            // Only a root that isn't there holds no books: one that can't
            // be opened (other users can enter it, say) may well hold some.
            let holds_books = match Self::open(&current) {
                Ok(old) => !old.list_books()?.is_empty(),
                Err(RootError::Missing) => false,
                Err(_) => true,
            };
            if holds_books {
                let keep = if chosen.is_none() {
                    "unset XDG_DATA_HOME (or set it back) and run `wanderbooks setup` again to \
                     keep using it"
                } else {
                    "run `wanderbooks setup` with no --root to keep using it"
                };
                return Err(RootError::Unsafe(format!(
                    "{THE_ROOT} already exists somewhere else and may hold books: setup won't make \
                     a second one or move them ({keep})"
                )));
            }
        }
        // Nothing is changed until every folder above the root that exists
        // is known to be safe.
        let mut above = path.parent();
        while let Some(dir) = above {
            if dir.exists() {
                check_dir_and_above(dir)?;
                break;
            }
            above = dir.parent();
        }
        let (created, mut tightened) = match std::fs::symlink_metadata(&path) {
            Ok(meta) => {
                // Checked before anything is changed: a folder that holds
                // other things is left exactly as it is.
                let open = check_private_dir(&meta, true, "the folder chosen for the books")?;
                if !path.join(MARKER_FILE).is_file() && std::fs::read_dir(&path)?.next().is_some() {
                    return Err(RootError::Unsafe(
                        "the folder chosen for the books already holds other things: choose an \
                         empty folder, or one `wanderbooks setup` made"
                            .into(),
                    ));
                }
                (false, tighten_if(&path, open)?)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(parent) = path.parent() {
                    make_private_dirs(parent)?;
                }
                make_private_dir(&path)?;
                (true, false)
            }
            Err(e) => return Err(e.into()),
        };
        check_ancestors(&path)?;
        if !path.join(MARKER_FILE).is_file() {
            write_private_file(&path.join(MARKER_FILE), b"wanderbooks data root\n")?;
        }
        let root = Self { path: path.clone() };
        for sub in [BOOKS_DIR, RUN_DIR, LEGACY_DIR] {
            tightened |= root.private_subdir(sub, true)?.1;
        }
        sync_dir(&path)?;
        let pointer = platform_data_dir(home).join(LOCATION_FILE);
        let record = path != default;
        if record {
            make_private_dirs(&platform_data_dir(home))?;
            let tmp = pointer.with_extension("location.tmp");
            let _ = std::fs::remove_file(&tmp);
            write_private_file(&tmp, format!("{}\n", path.display()).as_bytes())?;
            std::fs::rename(&tmp, &pointer)?;
            sync_dir(&platform_data_dir(home))?;
        } else {
            match std::fs::remove_file(&pointer) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok((
            root,
            SetupReport {
                root: path,
                created,
                tightened,
                recorded: record,
            },
        ))
    }

    /// The root's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The root's `auth/` folder (its principals and the owner passphrase),
    /// made 0700 if missing and refused if it isn't private.
    pub fn auth_dir(&self) -> Result<PathBuf, RootError> {
        self.private_subdir(AUTH_DIR, false)
            .map(|(dir, _)| dir)
            .map_err(|e| self.hide(e))
    }

    /// The root's `run/` folder (its sockets and locks), made 0700 if
    /// missing and refused if it isn't private.
    pub fn run_dir(&self) -> Result<PathBuf, RootError> {
        self.private_subdir(RUN_DIR, false)
            .map(|(dir, _)| dir)
            .map_err(|e| self.hide(e))
    }

    /// The folder a book lives in.
    pub fn book_dir(&self, id: &BookId) -> PathBuf {
        self.path.join(BOOKS_DIR).join(id.as_str())
    }

    /// A folder directly in the root: made 0700 if missing; refused if it
    /// is a symlink, belongs to another user, or (unless `tighten`) others
    /// can enter it. Returns it and whether it was tightened.
    fn private_subdir(&self, name: &str, tighten: bool) -> Result<(PathBuf, bool), RootError> {
        let dir = self.path.join(name);
        let tightened = match std::fs::symlink_metadata(&dir) {
            Ok(meta) => tighten_if(
                &dir,
                check_private_dir(&meta, tighten, "a folder inside it")?,
            )?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                make_private_dir(&dir)?;
                false
            }
            Err(e) => return Err(e.into()),
        };
        Ok((dir, tightened))
    }

    /// Hide the root's path in an error (INVARIANT 6).
    fn hide(&self, e: RootError) -> RootError {
        let root = self.path.display().to_string();
        let clean = |text: String| text.replace(&root, THE_ROOT);
        match e {
            RootError::Store(text, e) => RootError::Store(clean(text), e),
            RootError::Io(text, e) => RootError::Io(clean(text), e),
            RootError::Adopt(text) => RootError::Adopt(clean(text)),
            RootError::Unsafe(text) => RootError::Unsafe(clean(text)),
            other => other,
        }
    }

    /// The books in the root, by id: each folder under `books/` with a
    /// valid id and a journal. Staging folders and anything else are left
    /// out.
    pub fn list_books(&self) -> Result<Vec<BookId>, RootError> {
        self.list_books_inner().map_err(|e| self.hide(e))
    }

    fn list_books_inner(&self) -> Result<Vec<BookId>, RootError> {
        let (books, _) = self.private_subdir(BOOKS_DIR, false)?;
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(books)? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let Ok(id) = BookId::parse(&name) else {
                continue;
            };
            if entry.file_type()?.is_dir() && entry.path().join(JOURNAL_FILE).is_file() {
                ids.push(id);
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Take the lock under which books are made: one at a time, across
    /// processes.
    fn books_lock(&self) -> Result<std::fs::File, RootError> {
        let (run, _) = self.private_subdir(RUN_DIR, false)?;
        let path = run.join(BOOKS_LOCK);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        set_mode(&path, 0o600)?;
        file.lock()?;
        Ok(file)
    }

    /// Start new, empty books called `id` in `base_currency`. Refused if
    /// books of that id exist. Returns the open store.
    pub fn create_book(&self, id: &BookId, base_currency: Currency) -> Result<Store, RootError> {
        self.create_book_inner(id, base_currency)
            .map_err(|e| self.hide(e))
    }

    fn create_book_inner(&self, id: &BookId, base_currency: Currency) -> Result<Store, RootError> {
        let _lock = self.books_lock()?;
        let (books, _) = self.private_subdir(BOOKS_DIR, false)?;
        let dir = books.join(id.as_str());
        if std::fs::symlink_metadata(&dir).is_ok() {
            return Err(RootError::Exists(id.to_string()));
        }
        make_private_dir(&dir)?;
        match Store::create(&dir, base_currency) {
            Ok(store) => {
                sync_dir(&dir)?;
                sync_dir(&books)?;
                Ok(store)
            }
            Err(e) => {
                // Only the folder this call just made, still without a
                // journal, is removed.
                if !dir.join(JOURNAL_FILE).exists() {
                    let _ = std::fs::remove_dir_all(&dir);
                }
                Err(e.into())
            }
        }
    }

    /// Adopt existing books from `source` (a data dir: `journal.jsonl`,
    /// `blobs/`) as books called `id` (plan 5.5, steps 1 to 5). The source
    /// is only read, under its writer lock (INVARIANT 5); the copy is
    /// built in a staging folder: every stored file re-hashed against its
    /// name, the copy opened (its database rebuilt from the journal) with
    /// the source's head, every file it names present, verified end to
    /// end, moved to this version's journal format if the source's is
    /// older (the copy is new books of this version; the source keeps its
    /// own), given its settings if the source keeps none (and verified
    /// again), synced, and only then renamed into place.
    pub fn adopt(
        &self,
        source: &Path,
        id: &BookId,
        options: &AdoptOptions,
    ) -> Result<AdoptReport, RootError> {
        self.adopt_inner(source, id, options)
            .map_err(|e| self.hide(e))
    }

    fn adopt_inner(
        &self,
        source: &Path,
        id: &BookId,
        options: &AdoptOptions,
    ) -> Result<AdoptReport, RootError> {
        let _lock = self.books_lock()?;
        let (books, _) = self.private_subdir(BOOKS_DIR, false)?;
        let dest = books.join(id.as_str());
        if std::fs::symlink_metadata(&dest).is_ok() {
            return Err(RootError::Exists(id.to_string()));
        }
        let source = self.adoptable(source)?;
        for (what, path) in [
            ("vault folder", &options.vault_dir),
            ("vault key", &options.vault_key),
        ] {
            if let Some(path) = path
                && !path.exists()
            {
                return Err(RootError::Adopt(format!(
                    "the {what} given isn't there: nothing was adopted"
                )));
            }
        }
        // Step 1: the source's writer lock, held to the end, so nothing
        // writes it while it is copied.
        let (_source_lock, source_lock_created) = lock_source(&source)?;
        // Step 2: verify the chain and record the head.
        let (raw, lines) = read_source_journal(&source)?;
        let head = head_of(&lines);
        if let Some(expected) = &options.expect_head
            && *expected != head
        {
            return Err(RootError::Adopt(format!(
                "those books changed since the preview (they are now at event {}): nothing was \
                 adopted; preview them again",
                head.0
            )));
        }
        let keeps_settings = lines.iter().any(|l| l.line_type == "settings.set");
        if !keeps_settings && options.settings.is_none() {
            return Err(RootError::Adopt(
                "these books keep no settings of their own yet: give their settings file \
                 (`--toml`), and its settings are moved in as they are adopted"
                    .into(),
            ));
        }
        // Staging: anything left from an adoption that never finished goes
        // first. None is in progress: every adoption holds the books lock.
        for entry in std::fs::read_dir(&books)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(STAGING_PREFIX))
            {
                std::fs::remove_dir_all(entry.path())?;
            }
        }
        let staging = books.join(format!("{STAGING_PREFIX}{id}-{}", std::process::id()));
        make_private_dir(&staging)?;
        let built = (|| -> Result<AdoptReport, RootError> {
            // Step 3: the journal as read, then every stored file.
            write_private_file(&staging.join(JOURNAL_FILE), &raw)?;
            let blob_dir = crate::store::BLOBS_DIR;
            let copied = copy_blobs(&source.join(blob_dir), &staging.join(blob_dir))?;
            // Step 5: the vault's records and the v1 key.
            let vault_files = match &options.vault_dir {
                Some(dir) => copy_flat_private(dir, &staging.join(VAULT_DIR))?,
                None => 0,
            };
            let key_copied = match &options.vault_key {
                Some(key) => {
                    let keys = staging.join(KEYS_DIR);
                    make_private_dir(&keys)?;
                    write_private_file(&keys.join(LEGACY_KEY_FILE), &std::fs::read(key)?)?;
                    sync_dir(&keys)?;
                    true
                }
                None => false,
            };
            let mut store = Store::open(&staging)?;
            if store.head() != head {
                return Err(RootError::Adopt(format!(
                    "the copy's head {:?} isn't the source's {:?}; nothing was adopted",
                    store.head(),
                    head
                )));
            }
            let named: BTreeSet<String> = store.stored_blob_shas()?.into_iter().collect();
            let missing: Vec<&String> = named.difference(&copied.shas).collect();
            if !missing.is_empty() {
                return Err(RootError::Adopt(format!(
                    "the books name stored files that aren't there ({}): nothing was adopted",
                    missing
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            store.verify_and_heal()?;
            let blobs_verified = store.verify_blobs()?;
            // The copy is new books of this version, in its journal format
            // (the original, left as it was, keeps its own): it waits for
            // no format upgrade, and its settings can move in.
            let format_upgraded = store.upgrade_format()?.is_some();
            let (settings_imported, settings_ignored) = match (&options.settings, keeps_settings) {
                (Some((changes, source)), false) => {
                    let n = store.adopt_settings(changes, source)?.len();
                    (n, false)
                }
                (Some(_), true) => (0, true),
                (None, _) => (0, false),
            };
            // Verified again for what was written on the way in.
            if store.head() != head {
                store.verify_and_heal()?;
            }
            let final_head = store.head();
            drop(store);
            sync_dir(&staging)?;
            Ok(AdoptReport {
                id: id.clone(),
                source_head: head.clone(),
                head: final_head,
                blob_files: copied.shas.len() as u64,
                blobs_verified,
                unnamed_blobs: copied.shas.difference(&named).count() as u64,
                skipped_files: copied.skipped,
                vault_files,
                key_copied,
                settings_imported,
                settings_ignored,
                source_lock_created,
                format_upgraded,
            })
        })();
        let renamed = built.and_then(|report| {
            std::fs::rename(&staging, &dest)?;
            sync_dir(&books)?;
            Ok(report)
        });
        if renamed.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        renamed
    }
}

/// What restoring a backup would bring in ([`Root::restore_preview`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestorePreview {
    /// The backup's head, `(seq, hash)`, proven.
    pub head: (u64, String),
    /// Stored files in it, each re-hashed.
    pub blobs: u64,
    /// Vault records in it.
    pub vault_files: u64,
    /// Whether books of the id asked for exist now.
    pub exists: bool,
    /// The id of the books it holds, as they were backed up.
    pub origin: Option<String>,
}

/// What [`Root::restore`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreReport {
    /// The restored book's id.
    pub id: BookId,
    /// Its head, `(seq, hash)`: the backup's.
    pub head: (u64, String),
    /// Stored files the books name, re-hashed through the store.
    pub blobs_verified: u64,
    /// Vault records restored.
    pub vault_files: u64,
    /// The `legacy/` folder the books it replaced were moved to.
    pub replaced: Option<String>,
    /// The id of the books it held, as they were backed up.
    pub origin: Option<String>,
}

impl Root {
    /// Clone the backup at `source` (a git repository's address, or its
    /// folder: `<book>.git` in a backup folder) into a scratch folder in `books/` and prove it
    /// whole (backup.rs INVARIANT 3); the caller holds the books lock and
    /// removes the folder.
    fn fetch_backup(
        &self,
        git: &crate::backup::Git,
        source: &str,
        books: &Path,
        id: &BookId,
    ) -> Result<(PathBuf, crate::backup::Verified), RootError> {
        // Anything left from one that never finished goes first: none is
        // in progress (each holds the books lock).
        for entry in std::fs::read_dir(books)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(RESTORE_PREFIX))
            {
                std::fs::remove_dir_all(entry.path())?;
            }
        }
        let scratch = books.join(format!("{RESTORE_PREFIX}{id}-{}", std::process::id()));
        match crate::backup::verify(git, source, &scratch) {
            Ok(v) => Ok((scratch, v)),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&scratch);
                Err(RootError::Adopt(format!(
                    "that backup can't be restored: {e}"
                )))
            }
        }
    }

    /// What restoring the backup at `source` as books `id` would bring in,
    /// proven as [`Self::restore`] proves it, adding nothing to the root.
    pub fn restore_preview(
        &self,
        git: &crate::backup::Git,
        source: &str,
        id: &BookId,
    ) -> Result<RestorePreview, RootError> {
        (|| {
            let _lock = self.books_lock()?;
            let (books, _) = self.private_subdir(BOOKS_DIR, false)?;
            let (scratch, v) = self.fetch_backup(git, source, &books, id)?;
            let vault = scratch.join(VAULT_DIR);
            let vault_files = if vault.is_dir() {
                std::fs::read_dir(&vault)
                    .map(|d| d.count() as u64)
                    .unwrap_or(0)
            } else {
                0
            };
            let _ = std::fs::remove_dir_all(&scratch);
            Ok(RestorePreview {
                head: v.head,
                blobs: v.blobs,
                vault_files,
                exists: std::fs::symlink_metadata(books.join(id.as_str())).is_ok(),
                origin: v.book,
            })
        })()
        .map_err(|e| self.hide(e))
    }

    /// Restore the backup at `source` as books `id` (INVARIANT 7): proven
    /// whole, its head `expect_head` (as the preview read it), built as an
    /// adoption builds (every stored file re-hashed, opened, verified,
    /// synced) with its backup history (so its next backups carry on from
    /// it), and only then renamed into place. Books `id` that exist are
    /// refused, unless `replace`: then they are moved into `legacy/`
    /// first, kept whole; refused while anything holds them open.
    pub fn restore(
        &self,
        git: &crate::backup::Git,
        source: &str,
        id: &BookId,
        expect_head: &(u64, String),
        replace: bool,
    ) -> Result<RestoreReport, RootError> {
        self.restore_inner(git, source, id, expect_head, replace)
            .map_err(|e| match e {
                // The copy's checks are the adoption's: said as a restore.
                RootError::Adopt(text) => {
                    RootError::Adopt(text.replace("nothing was adopted", "nothing was restored"))
                }
                other => other,
            })
            .map_err(|e| self.hide(e))
    }

    fn restore_inner(
        &self,
        git: &crate::backup::Git,
        source: &str,
        id: &BookId,
        expect_head: &(u64, String),
        replace: bool,
    ) -> Result<RestoreReport, RootError> {
        let _lock = self.books_lock()?;
        let (books, _) = self.private_subdir(BOOKS_DIR, false)?;
        let dest = books.join(id.as_str());
        let exists = std::fs::symlink_metadata(&dest).is_ok();
        if exists && !replace {
            return Err(RootError::Exists(id.to_string()));
        }
        if !exists && replace {
            return Err(RootError::Adopt(format!(
                "there are no books called {:?} to replace: nothing was restored",
                id.as_str()
            )));
        }
        // Books being replaced are moved only while nothing holds them.
        let _held = if exists {
            Some(lock_source(&dest).map_err(|_| {
                RootError::Adopt(format!(
                    "the books called {:?} are open: close them first; nothing was restored",
                    id.as_str()
                ))
            })?)
        } else {
            None
        };
        let (scratch, v) = self.fetch_backup(git, source, &books, id)?;
        let staging = books.join(format!("{RESTORE_PREFIX}{id}-{}-book", std::process::id()));
        let built = (|| -> Result<RestoreReport, RootError> {
            if v.head != *expect_head {
                return Err(RootError::Adopt(format!(
                    "that backup isn't the one previewed (it is now at event {}): nothing was restored; preview it again",
                    v.head.0
                )));
            }
            let (raw, lines) = read_source_journal(&scratch)?;
            if head_of(&lines) != v.head {
                return Err(RootError::Adopt(
                    "the backup's journal changed while it was read: nothing was restored".into(),
                ));
            }
            make_private_dir(&staging)?;
            write_private_file(&staging.join(JOURNAL_FILE), &raw)?;
            let blob_dir = crate::store::BLOBS_DIR;
            let copied = copy_blobs(&scratch.join(blob_dir), &staging.join(blob_dir))?;
            let vault_files = if scratch.join(VAULT_DIR).is_dir() {
                copy_flat_private(&scratch.join(VAULT_DIR), &staging.join(VAULT_DIR))?
            } else {
                0
            };
            if scratch.join(KEYS_DIR).is_dir() {
                copy_flat_private(&scratch.join(KEYS_DIR), &staging.join(KEYS_DIR))?;
            }
            let access = scratch.join(crate::access::ACCESS_FILE);
            if access.is_file() {
                write_private_file(
                    &staging.join(crate::access::ACCESS_FILE),
                    &std::fs::read(&access)?,
                )?;
            }
            // Its backup history: the next backup carries on from it.
            std::fs::rename(
                scratch.join(".git"),
                staging.join(crate::backup::ARCHIVE_DIR),
            )?;
            // Whose books they were, placed with them.
            crate::ops::prepare(&staging)?;
            write_private_file(
                &crate::ops::ops_dir(&staging).join(RESTORED_FROM),
                v.book.as_deref().unwrap_or("").as_bytes(),
            )?;
            let mut store = Store::open(&staging)?;
            if store.head() != v.head {
                return Err(RootError::Adopt(format!(
                    "the copy's head {:?} isn't the backup's {:?}: nothing was restored",
                    store.head(),
                    v.head
                )));
            }
            let named: BTreeSet<String> = store.stored_blob_shas()?.into_iter().collect();
            if !named.is_subset(&copied.shas) {
                return Err(RootError::Adopt(
                    "the backup lacks stored files its books name: nothing was restored".into(),
                ));
            }
            store.verify_and_heal()?;
            let blobs_verified = store.verify_blobs()?;
            let head = store.head();
            drop(store);
            sync_dir(&staging)?;
            Ok(RestoreReport {
                id: id.clone(),
                head,
                blobs_verified,
                vault_files,
                replaced: None,
                origin: v.book.clone(),
            })
        })();
        let _ = std::fs::remove_dir_all(&scratch);
        let placed = built.and_then(|mut report| {
            if exists {
                let (legacy, _) = self.private_subdir(LEGACY_DIR, false)?;
                let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
                let mut name = format!("{id}-{stamp}");
                let mut n = 1;
                while std::fs::symlink_metadata(legacy.join(&name)).is_ok() {
                    n += 1;
                    name = format!("{id}-{stamp}-{n}");
                }
                std::fs::rename(&dest, legacy.join(&name))?;
                if let Err(e) = std::fs::rename(&staging, &dest) {
                    // The books it would replace go back where they were.
                    if std::fs::rename(legacy.join(&name), &dest).is_err() {
                        return Err(RootError::Adopt(format!(
                            "the backup couldn't be put in place ({}), and the books it would \
                             replace are in the legacy folder as {name}: nothing was restored",
                            e.kind()
                        )));
                    }
                    return Err(e.into());
                }
                // In place: a folder that won't sync now is said no more
                // than any later write's would be.
                let _ = sync_dir(&legacy);
                report.replaced = Some(name);
            } else {
                std::fs::rename(&staging, &dest)?;
            }
            let _ = sync_dir(&books);
            Ok(report)
        });
        if placed.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        placed
    }

    /// The source of an adoption, made absolute: books outside the root
    /// (and not holding it) with a journal.
    fn adoptable(&self, source: &Path) -> Result<PathBuf, RootError> {
        let source = std::fs::canonicalize(source)
            .map_err(|e| RootError::Adopt(format!("can't read the books given: {}", e.kind())))?;
        let root = std::fs::canonicalize(&self.path)?;
        if source.starts_with(&root) || root.starts_with(&source) {
            return Err(RootError::Adopt(format!(
                "those books are inside {THE_ROOT} (or hold it): adopt books from outside it"
            )));
        }
        if !source.join(JOURNAL_FILE).is_file() {
            return Err(RootError::Adopt(format!(
                "there are no books there: it has no {JOURNAL_FILE}"
            )));
        }
        Ok(source)
    }

    /// What adopting `source` would start from: its head, its base
    /// currency, and whether it keeps its own settings, read under the
    /// checks [`Self::adopt`] makes, writing nothing (not even a lock
    /// file). The adoption itself reads it all again under the lock.
    pub fn adopt_preview(&self, source: &Path) -> Result<SourcePreview, RootError> {
        (|| {
            let source = self.adoptable(source)?;
            // Held for writing now: adoption would be refused, so is this.
            if let Ok(lock) = std::fs::File::open(source.join(crate::store::LOCK_FILE))
                && let Err(std::fs::TryLockError::WouldBlock) = lock.try_lock_shared()
            {
                return Err(RootError::from(StoreError::Locked(
                    "another wanderbooks process has those books open for writing (an agent \
                     session, or its MCP server); close it first"
                        .into(),
                )));
            }
            let (_, lines) = read_source_journal(&source)?;
            Ok(SourcePreview {
                head: head_of(&lines),
                currency: crate::store::journal_currency(&lines)?,
                keeps_settings: lines.iter().any(|l| l.line_type == "settings.set"),
            })
        })()
        .map_err(|e| self.hide(e))
    }

    /// Put the old copies of the books `id` aside (INVARIANT 8): the
    /// data folder they were adopted from, their TIN vault and its key,
    /// and other paths that hold them too (their settings file, the git
    /// history around them, old exports), each moved whole into a numbered
    /// folder of its own in one folder of `legacy/`, nothing deleted, with
    /// a list of where each came from written there first. Refused unless
    /// the books `id` hold everything the old copy does, byte for byte: its
    /// journal (its head is in theirs), every stored file, every vault
    /// record and the vault's key; while a wanderbooks process holds the
    /// old copy open for writing; when its head isn't the one a preview
    /// read (`expect_head`); for a path that isn't there, is a link, is
    /// inside the root or holds it, or is inside another one given; and
    /// for a path on another disk than the root (moving it there would be
    /// a copy). Without `confirm` it moves and writes nothing: what would
    /// move, and where. A move cut short puts back what it moved, never
    /// over anything.
    pub fn put_aside(
        &self,
        id: &BookId,
        old: &OldCopies<'_>,
        confirm: bool,
    ) -> Result<PutAside, RootError> {
        (|| {
            let root = std::fs::canonicalize(&self.path)?;
            let book = self.book_dir(id);
            if !book.join(JOURNAL_FILE).is_file() {
                return Err(RootError::Adopt(format!(
                    "there are no books called {:?}",
                    id.as_str()
                )));
            }
            let given: Vec<&Path> = std::iter::once(old.source)
                .chain(old.vault_dir)
                .chain(old.vault_key)
                .chain(old.also.iter().map(PathBuf::as_path))
                .collect();
            let mut items = Vec::new();
            for given in given {
                let given = std::path::absolute(given)?;
                let shown = given
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| given.display().to_string());
                let (Some(parent), Some(name)) = (given.parent(), given.file_name()) else {
                    return Err(RootError::Adopt(format!("{shown} can't be put aside")));
                };
                let path = std::fs::canonicalize(parent)
                    .map_err(|e| RootError::Adopt(format!("{shown}: {}", e.kind())))?
                    .join(name);
                let meta = std::fs::symlink_metadata(&path)
                    .map_err(|e| RootError::Adopt(format!("{shown}: {}", e.kind())))?;
                // A link would go aside as a link, its data left where it
                // is: name what it points to instead.
                if meta.file_type().is_symlink() {
                    return Err(RootError::Adopt(format!(
                        "{shown} is a link: name what it points to"
                    )));
                }
                // Named by its last part only: a path into the root never
                // shows where the root is (INVARIANT 6).
                if path.starts_with(&root) || root.starts_with(&path) {
                    return Err(RootError::Adopt(format!(
                        "{shown} is inside {THE_ROOT} (or holds it): only what is outside it is \
                         put aside"
                    )));
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if meta.dev() != std::fs::metadata(&root)?.dev() {
                        return Err(RootError::Adopt(format!(
                            "{} is on another disk than {THE_ROOT}: moving it there would be a \
                             copy, so nothing is put aside",
                            path.display()
                        )));
                    }
                }
                // A vault or its key among the other paths would go aside
                // unchecked: each comes by its own name, and is checked.
                let k = items.len();
                let checked = (k == 1 && old.vault_dir.is_some())
                    || (k == 1 + usize::from(old.vault_dir.is_some()) && old.vault_key.is_some());
                let beside_vault = items
                    .first()
                    .and_then(|s: &PathBuf| s.parent())
                    .is_some_and(|dir| path == dir.join(VAULT_DIR));
                if k > 0 && !checked && (beside_vault || name == "vault.key") {
                    return Err(RootError::Adopt(format!(
                        "{shown} looks like the TIN vault or its key: give it with --vault DIR \
                         --vault-key FILE, so it is checked against the books before it goes"
                    )));
                }
                items.push(path);
            }
            for (i, a) in items.iter().enumerate() {
                for (j, b) in items.iter().enumerate() {
                    if i != j && a.starts_with(b) {
                        return Err(RootError::Adopt(format!(
                            "{} is {} or inside it: name each only once",
                            a.display(),
                            b.display()
                        )));
                    }
                }
            }
            let source = &items[0];
            if !source.join(JOURNAL_FILE).is_file() {
                return Err(RootError::Adopt(format!(
                    "there are no books at {}: it has no {JOURNAL_FILE}",
                    source.display()
                )));
            }
            // Held for writing: nothing is moved from under it. A preview
            // only looks; the move holds the lock to the end.
            let _lock = if confirm {
                Some(lock_source(source)?.0)
            } else {
                if let Ok(lock) = std::fs::File::open(source.join(crate::store::LOCK_FILE))
                    && let Err(std::fs::TryLockError::WouldBlock) = lock.try_lock_shared()
                {
                    return Err(RootError::from(StoreError::Locked(
                        "another wanderbooks process has those books open for writing (an \
                         agent session, or its MCP server); close it first"
                            .into(),
                    )));
                }
                None
            };
            let (_, lines) = read_source_journal(source)?;
            let head = head_of(&lines);
            if let Some(expected) = old.expect_head
                && *expected != head
            {
                return Err(RootError::Adopt(format!(
                    "the old copy changed since it was looked at (now through event {}): look \
                     again",
                    head.0
                )));
            }
            let lacks = |what: String| {
                RootError::Adopt(format!(
                    "the books {:?} don't hold everything the old copy does ({what}): adopt \
                     it first, or name the books that hold it",
                    id.as_str()
                ))
            };
            if !journal_holds(&book, &head)? {
                return Err(lacks(format!("its journal through event {}", head.0)));
            }
            let (blob_files, left_behind) = blobs_held(
                &source.join(crate::store::BLOBS_DIR),
                &book.join(crate::store::BLOBS_DIR),
            )?
            .map_err(|rel| lacks(format!("the stored file {}", rel.display())))?;
            let (vault_files, vault_changed) = match old.vault_dir {
                Some(_) => vault_held(&items[1], &book.join(VAULT_DIR))?.map_err(|rel| {
                    lacks(format!(
                        "the vault record {} (never adopted, or purged since)",
                        rel.display()
                    ))
                })?,
                None => (0, Vec::new()),
            };
            if old.vault_key.is_some() {
                let key = &items[1 + usize::from(old.vault_dir.is_some())];
                let theirs = std::fs::read(book.join(KEYS_DIR).join(LEGACY_KEY_FILE)).ok();
                if theirs.as_deref() != Some(&std::fs::read(key)?[..]) {
                    return Err(lacks("the vault's key".into()));
                }
            }
            let legacy = root.join(LEGACY_DIR);
            let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
            let mut name = format!("{id}-old-{stamp}");
            let mut n = 1;
            while std::fs::symlink_metadata(legacy.join(&name)).is_ok() {
                n += 1;
                name = format!("{id}-old-{stamp}-{n}");
            }
            let report = PutAside {
                head,
                items: items.clone(),
                blob_files,
                left_behind,
                vault_files,
                vault_changed,
                key_checked: old.vault_key.is_some(),
                name: name.clone(),
                moved: false,
            };
            if !confirm {
                return Ok(report);
            }
            self.private_subdir(LEGACY_DIR, false)?;
            let into = legacy.join(&name);
            make_private_dir(&into)?;
            // Where each came from, written down before anything moves, so
            // a move cut short by a crash can be finished by hand.
            let mut list = format!(
                "wanderbooks put-aside: the old copies of the books \"{id}\", which hold \
                 everything they did (through event {}, {}).\nStarted {}.\n",
                report.head.0,
                report.head.1,
                chrono::Utc::now().to_rfc3339()
            );
            for (k, item) in items.iter().enumerate() {
                list.push_str(&format!("{}  {}\n", k + 1, item.display()));
            }
            let listed = write_private_file(&into.join(PUT_ASIDE_LIST), list.as_bytes())
                .and_then(|()| sync_dir(&into))
                .and_then(|()| sync_dir(&legacy));
            if let Err(e) = listed {
                // Nothing has moved: no list of moves that never happened.
                let _ = std::fs::remove_file(into.join(PUT_ASIDE_LIST));
                let _ = std::fs::remove_dir(&into);
                return Err(RootError::Adopt(format!(
                    "the list of where each came from couldn't be written ({e}): nothing was \
                     put aside"
                )));
            }
            // Each in a numbered folder of its own: no two can meet, and a
            // git history never sits beside a vault it was kept apart from.
            let mut moved: Vec<(&PathBuf, PathBuf)> = Vec::new();
            for (k, item) in items.iter().enumerate() {
                let own = into.join((k + 1).to_string());
                let dest = own.join(item.file_name().unwrap_or_default());
                let done = make_private_dir(&own)
                    .map_err(|e| std::io::Error::other(e.to_string()))
                    .and_then(|()| rename_new(item, &dest));
                if let Err(e) = done {
                    // Back where they were, the last moved first, never
                    // over anything that came since.
                    let mut stuck = Vec::new();
                    for (from, at) in moved.iter().rev() {
                        if rename_new(at, from).is_err() {
                            stuck.push(from.display().to_string());
                        }
                    }
                    if stuck.is_empty() {
                        // The list goes only once every numbered folder is
                        // gone, empty: else it stays to say what is where.
                        let emptied = (1..=items.len()).all(|k| {
                            match std::fs::remove_dir(into.join(k.to_string())) {
                                Ok(()) => true,
                                Err(e) => e.kind() == std::io::ErrorKind::NotFound,
                            }
                        });
                        if emptied {
                            let _ = std::fs::remove_file(into.join(PUT_ASIDE_LIST));
                            let _ = std::fs::remove_dir(&into);
                        }
                        return Err(RootError::Adopt(format!(
                            "{} couldn't be moved ({}): nothing was put aside",
                            item.display(),
                            e.kind()
                        )));
                    }
                    return Err(RootError::Adopt(format!(
                        "{} couldn't be moved ({}), and {} couldn't be put back: they are in \
                         the legacy folder as {name}, whole, with its {PUT_ASIDE_LIST} saying \
                         where each came from",
                        item.display(),
                        e.kind(),
                        stuck.join(", ")
                    )));
                }
                moved.push((item, dest));
            }
            // Every path has moved: the list saying so is a courtesy, and
            // its failing (a full disk) doesn't undo or deny the move.
            let _ = (|| -> std::io::Result<()> {
                use std::io::Write;
                let mut list = std::fs::OpenOptions::new()
                    .append(true)
                    .open(into.join(PUT_ASIDE_LIST))?;
                writeln!(
                    list,
                    "Finished {}: each path above is in the folder of its number.",
                    chrono::Utc::now().to_rfc3339()
                )?;
                list.sync_all()
            })();
            let _ = sync_dir(&into);
            let _ = sync_dir(&legacy);
            for item in &items {
                if let Some(parent) = item.parent() {
                    let _ = sync_dir(parent);
                }
            }
            Ok(PutAside {
                moved: true,
                ..report
            })
        })()
        .map_err(|e| self.hide(e))
    }
}

/// The list [`Root::put_aside`] leaves in the folder it fills: where each
/// numbered folder's path came from.
pub const PUT_ASIDE_LIST: &str = "put-aside.txt";

/// Rename `from` to `to`, never over something already at `to`.
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
        // A file system that can't refuse to replace: looked at, then moved.
        if e.raw_os_error() != Some(libc::EINVAL) {
            return Err(e);
        }
    }
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
    }
    std::fs::rename(from, to)
}

/// Whether the stored files under `old` (a books' `blobs/`) are all in
/// `new` at the same place with the same bytes: Ok((count, left behind))
/// if so, the left behind being what adoption leaves (hidden files, and
/// unfinished `<sha>.tmp` writes: never part of the books), else Ok(Err(the
/// first that isn't)). A folder that isn't there holds nothing.
fn blobs_held(old: &Path, new: &Path) -> Result<Result<(usize, Vec<PathBuf>), PathBuf>, RootError> {
    let is_hex = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit());
    let mut count = 0;
    let mut left = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let entries = match std::fs::read_dir(old.join(&rel)) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && rel.as_os_str().is_empty() => {
                return Ok(Ok((0, left)));
            }
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = rel.join(&name);
            let kind = entry.file_type()?;
            if kind.is_dir() {
                stack.push(rel);
            } else if kind.is_file()
                && (name.starts_with('.')
                    || name.strip_suffix(".tmp").is_some_and(|sha| is_hex(sha, 64)))
            {
                left.push(rel);
            } else if std::fs::read(new.join(&rel)).ok() == Some(std::fs::read(old.join(&rel))?) {
                count += 1;
            } else {
                return Ok(Err(rel));
            }
        }
    }
    left.sort();
    Ok(Ok((count, left)))
}

/// Whether the TIN vault `old` is held by the books' vault `new`: every
/// record there (a record the books sealed anew since is theirs, newer,
/// and listed), and the registry of files with full TINs only grown since.
/// Ok((count, changed)) if so, else Ok(Err(the first that isn't)).
fn vault_held(old: &Path, new: &Path) -> Result<Result<(usize, Vec<PathBuf>), PathBuf>, RootError> {
    let mut count = 0;
    let mut changed = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(old)?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<Result<_, _>>()?;
    names.sort();
    for name in names {
        let rel = PathBuf::from(&name);
        if !std::fs::symlink_metadata(old.join(&name))?.is_file() {
            return Ok(Err(rel));
        }
        let mine = std::fs::read(old.join(&name))?;
        let Ok(theirs) = std::fs::read(new.join(&name)) else {
            return Ok(Err(rel));
        };
        if name == TIN_REGISTRY_FILE {
            if !theirs.starts_with(&mine) {
                return Ok(Err(rel));
            }
        } else if theirs != mine {
            changed.push(rel);
        }
        count += 1;
    }
    Ok(Ok((count, changed)))
}

/// The vault's append-only list of files known to hold full TINs (kept by
/// the books server; ledger-mcp `tin_files`).
const TIN_REGISTRY_FILE: &str = "tin-files.sha256";

/// Whether the journal of the books in `book` holds the event `head`
/// names, `(seq, hash)`: its line of that seq hashes to that. Only whole
/// lines are read, so a line still being written is no matter.
fn journal_holds(book: &Path, head: &(u64, String)) -> Result<bool, RootError> {
    if head.0 == 0 {
        return Ok(true);
    }
    let raw = std::fs::read(book.join(JOURNAL_FILE))?;
    let Some(end) = raw.iter().rposition(|&b| b == b'\n') else {
        return Ok(false);
    };
    let (lines, _format) = crate::journal::verify_journal_and_format(&raw[..=end])?;
    Ok(lines
        .iter()
        .find(|l| l.seq == head.0)
        .is_some_and(|l| head_of(std::slice::from_ref(l)) == *head))
}

/// The old copies of books, for [`Root::put_aside`].
#[derive(Clone, Copy, Debug)]
pub struct OldCopies<'a> {
    /// The data folder they were adopted from.
    pub source: &'a Path,
    /// Their TIN vault: each record must be the books' own, byte for byte.
    pub vault_dir: Option<&'a Path>,
    /// The vault's key: it must be the books' legacy key, byte for byte.
    pub vault_key: Option<&'a Path>,
    /// Other paths that hold them (settings file, git history, exports),
    /// put aside as they are.
    pub also: &'a [PathBuf],
    /// The head a preview read: if it moved since, nothing is put aside.
    pub expect_head: Option<&'a (u64, String)>,
}

/// What [`Root::put_aside`] would move, or moved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PutAside {
    /// The old copy's head, `(seq, hash)`: the books hold it too.
    pub head: (u64, String),
    /// The paths that go aside, in order: the data folder, the vault, its
    /// key, then the others. Each goes into the folder of its number.
    pub items: Vec<PathBuf>,
    /// The old copy's stored files, each found in the books, byte for byte.
    pub blob_files: usize,
    /// Files in its stored files' folder that were never part of the books
    /// (hidden files, unfinished writes): left behind by adoption, going
    /// aside with the rest.
    pub left_behind: Vec<PathBuf>,
    /// The vault's records, each found in the books.
    pub vault_files: usize,
    /// Records the books have sealed anew since (theirs is newer).
    pub vault_changed: Vec<PathBuf>,
    /// Whether the vault's key was found to be the books' own.
    pub key_checked: bool,
    /// The folder of their own in `legacy/` they go to.
    pub name: String,
    /// Whether they were moved (false: a preview, nothing moved).
    pub moved: bool,
}

/// The source's journal, whole and verified: its bytes and lines.
fn read_source_journal(source: &Path) -> Result<(Vec<u8>, Vec<crate::JournalLine>), RootError> {
    let raw = std::fs::read(source.join(JOURNAL_FILE))?;
    if raw.last() != Some(&b'\n') {
        return Err(RootError::Adopt(
            "the source's last journal line is cut off: open the books once with the server \
             that wrote them (it repairs a cut-off line), then adopt them"
                .into(),
        ));
    }
    let (lines, _format) = crate::journal::verify_journal_and_format(&raw)?;
    Ok((raw, lines))
}

/// What [`Root::adopt_preview`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePreview {
    /// The source's head, `(seq, hash)`.
    pub head: (u64, String),
    /// Its base currency.
    pub currency: Currency,
    /// Whether it keeps its own settings.
    pub keeps_settings: bool,
}

/// Options for [`Root::adopt`].
#[derive(Clone, Debug, Default)]
pub struct AdoptOptions {
    /// Settings to move in, parsed from the books' settings file, for
    /// books that keep none of their own yet (journaled as their first
    /// `settings.set`). For books that keep their own they are ignored,
    /// and the report says so.
    pub settings: Option<(
        Vec<ledger_core::settings::Change>,
        ledger_core::settings::Source,
    )>,
    /// The source's TIN vault folder: its records are copied into the
    /// book's `vault/`. Given, it must exist.
    pub vault_dir: Option<PathBuf>,
    /// The v1 vault key file: copied to the book's `keys/legacy.key`.
    /// Given, it must exist.
    pub vault_key: Option<PathBuf>,
    /// The head the source must have, `(seq, hash)`, as a preview read
    /// it: if the books changed since, nothing is adopted.
    pub expect_head: Option<(u64, String)>,
}

/// What [`Root::adopt`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdoptReport {
    /// The new book's id.
    pub id: BookId,
    /// The source's head, `(seq, hash)`, verified before copying.
    pub source_head: (u64, String),
    /// The adopted books' head: the source's, plus this version's journal
    /// format if the source's is older (`format_upgraded`), plus (when
    /// settings were moved in) their `settings.set`.
    pub head: (u64, String),
    /// Stored files copied, each re-hashed against its name.
    pub blob_files: u64,
    /// Stored files the books name, re-hashed again through the store.
    pub blobs_verified: u64,
    /// Stored files copied that the books don't name (kept: a stored file
    /// is never thrown away).
    pub unnamed_blobs: u64,
    /// Files left behind among the stored ones: unfinished writes
    /// (`<sha>.tmp`, a put that never finished) and hidden files
    /// (`.DS_Store` and the like, which no wanderbooks writes).
    pub skipped_files: u64,
    /// Vault records copied.
    pub vault_files: u64,
    /// Whether the v1 key was copied.
    pub key_copied: bool,
    /// Settings journaled on the way in.
    pub settings_imported: usize,
    /// Settings were given but the books keep their own, so they weren't
    /// used.
    pub settings_ignored: bool,
    /// The source had no lock file, so taking its lock made one.
    pub source_lock_created: bool,
    /// The copy was moved to this version's journal format (the source's
    /// was older; the source keeps its own).
    pub format_upgraded: bool,
}

/// The source's writer lock: its lock file opened read-only (so a
/// read-only source can be adopted and nothing in it changes), or, for
/// books that never had one, created. Returns the lock and whether the
/// file was made.
fn lock_source(source: &Path) -> Result<(std::fs::File, bool), RootError> {
    let path = source.join(crate::store::LOCK_FILE);
    let (file, created) = match std::fs::File::open(&path) {
        Ok(file) => (file, false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (crate::store::lock_books_for_adoption(source)?, true)
        }
        Err(e) => return Err(e.into()),
    };
    if !created {
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(RootError::from(StoreError::Locked(
                    "another wanderbooks process has those books open for writing (an agent \
                     session, or its MCP server); close it first"
                        .into(),
                )));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
        }
    }
    Ok((file, created))
}

/// The head of a verified journal: the last line's seq and the hash the
/// next line would chain from.
fn head_of(lines: &[JournalLine]) -> (u64, String) {
    use sha2::{Digest, Sha256};
    match lines.last() {
        Some(last) => (
            last.seq,
            hex::encode(Sha256::digest(last.to_canonical().as_bytes())),
        ),
        None => (0, crate::journal::GENESIS_PREV.to_string()),
    }
}

/// The root setup recorded, if it did: the location file must be a plain
/// file of this account's that no one else can write (INVARIANT 3), naming
/// an absolute folder.
fn recorded_root(home: &Path) -> Result<Option<PathBuf>, RootError> {
    let pointer = platform_data_dir(home).join(LOCATION_FILE);
    let meta = match std::fs::symlink_metadata(&pointer) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let unsafe_pointer = || {
        RootError::Unsafe(
            "the note of where the books' private folder is (wanderbooks.location) isn't this \
             account's own private file: remove it and run `wanderbooks setup` again"
                .into(),
        )
    };
    if !meta.file_type().is_file() {
        return Err(unsafe_pointer());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: getuid has no preconditions and never fails.
        if meta.uid() != unsafe { libc::getuid() } || meta.permissions().mode() & 0o022 != 0 {
            return Err(unsafe_pointer());
        }
    }
    let text = std::fs::read_to_string(&pointer)?;
    let path = PathBuf::from(text.trim_end_matches(['\n', '\r']));
    if path.is_absolute() {
        Ok(Some(path))
    } else {
        Err(RootError::Unsafe(
            "the note of where the books' private folder is (wanderbooks.location) names a \
             relative folder: run `wanderbooks setup` again"
                .into(),
        ))
    }
}

/// A folder that may not exist yet, made absolute and named plainly: no
/// `..` anywhere (refused), the folders above it that exist resolved
/// fully (links and all), and its own last name kept as given, so a root
/// that is a link is still seen as one and refused.
fn resolve_new(path: &Path) -> Result<PathBuf, RootError> {
    let path = std::path::absolute(path)?;
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(RootError::Unsafe(
            "the folder chosen for the books has `..` in it: name it plainly".into(),
        ));
    }
    let (Some(name), Some(parent)) = (path.file_name(), path.parent()) else {
        return Err(RootError::Unsafe(
            "the folder chosen for the books can't be the top of the file system".into(),
        ));
    };
    let mut existing = parent;
    let mut rest = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(n), Some(p)) => {
                rest.push(n.to_os_string());
                existing = p;
            }
            _ => break,
        }
    }
    let mut out = std::fs::canonicalize(existing)?;
    for n in rest.into_iter().rev() {
        out.push(n);
    }
    out.push(name);
    Ok(out)
}

/// Make one folder, mode 0700 set explicitly (INVARIANT 1).
fn make_private_dir(path: &Path) -> Result<(), RootError> {
    std::fs::create_dir(path)?;
    set_mode(path, 0o700)?;
    Ok(())
}

/// Make a folder and any missing above it, each 0700 (INVARIANT 1).
fn make_private_dirs(path: &Path) -> Result<(), RootError> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        make_private_dirs(parent)?;
    }
    match make_private_dir(path) {
        Err(RootError::Io(_, e)) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        other => other,
    }
}

/// Write a new file, mode 0600 set explicitly, synced.
fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), RootError> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    set_mode(path, 0o600)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Make a folder's entries durable (Unix; NTFS journals its own).
fn sync_dir(path: &Path) -> Result<(), RootError> {
    #[cfg(unix)]
    std::fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    // Windows: the owner-only DACL comes with the Windows build (P11).
    Ok(())
}

/// Check an existing folder is private (INVARIANT 2). With `tighten`, a
/// folder of ours that others can enter is reported (true) for the caller
/// to make 0700 with [`tighten_if`]; without, it is refused. `what` names
/// it in messages.
fn check_private_dir(
    meta: &std::fs::Metadata,
    tighten: bool,
    what: &str,
) -> Result<bool, RootError> {
    if meta.file_type().is_symlink() {
        return Err(RootError::Unsafe(format!(
            "{what} is a symlink: it must be a real folder"
        )));
    }
    if !meta.is_dir() {
        return Err(RootError::Unsafe(format!("{what} isn't a folder")));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: getuid has no preconditions and never fails.
        let uid = unsafe { libc::getuid() };
        if meta.uid() != uid {
            return Err(RootError::Unsafe(format!(
                "{what} belongs to another user: it must be this account's own"
            )));
        }
        if meta.permissions().mode() & 0o077 != 0 {
            if !tighten {
                return Err(RootError::Unsafe(format!(
                    "other users can open {what}: run `wanderbooks setup` to make it private"
                )));
            }
            return Ok(true);
        }
    }
    #[cfg(not(unix))]
    let _ = tighten;
    Ok(false)
}

/// Every folder above `path` must be one another user can't swap it in
/// (INVARIANT 2): see [`check_dir_and_above`].
fn check_ancestors(path: &Path) -> Result<(), RootError> {
    match path.parent() {
        Some(parent) => check_dir_and_above(parent),
        None => Ok(()),
    }
}

/// `dir` and every folder above it: owned by this account or by root,
/// and writable by no other user. Others' write is allowed only with the
/// sticky bit (like `/tmp`); group write only when the group is this
/// account's own private group (its primary group, with no other members:
/// the usual umask-002 setup on many Linux systems).
fn check_dir_and_above(dir: &Path) -> Result<(), RootError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: getuid has no preconditions and never fails.
        let uid = unsafe { libc::getuid() };
        for d in dir.ancestors() {
            let meta = std::fs::metadata(d)?;
            let mode = meta.permissions().mode();
            let sticky = mode & 0o1000 != 0;
            let owner_ok = meta.uid() == uid || meta.uid() == 0;
            let others_ok = mode & 0o002 == 0 || sticky;
            let group_ok = mode & 0o020 == 0 || sticky || is_private_group(meta.gid());
            if !(owner_ok && others_ok && group_ok) {
                return Err(RootError::Unsafe(format!(
                    "a folder above {THE_ROOT} can be changed by another user: put the books \
                     somewhere only this account (or the system) controls"
                )));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// Whether `gid` is this account's own private group (the user-private
/// group convention): see [`private_group_rule`].
#[cfg(unix)]
fn is_private_group(gid: u32) -> bool {
    // SAFETY: getgid has no preconditions and never fails.
    let primary = unsafe { libc::getgid() };
    let Some((_, login)) = unix_account() else {
        return false;
    };
    let group = with_growing_buffer(|buf| {
        // SAFETY: an all-zero group is a valid value of this plain C
        // struct; getgrgid_r overwrites it.
        let mut grp: libc::group = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::group = std::ptr::null_mut();
        // SAFETY: getgrgid_r writes into grp and buf, both alive and sized
        // as passed; result is set to &grp on success or left null.
        let rc = unsafe {
            libc::getgrgid_r(
                gid,
                &mut grp,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut result,
            )
        };
        if rc != 0 || result.is_null() || grp.gr_name.is_null() {
            return (rc, None);
        }
        // SAFETY: gr_name points into buf, NUL-terminated by getgrgid_r;
        // gr_mem is a null-terminated array of pointers into buf, and only
        // its first element is read.
        let name = unsafe { std::ffi::CStr::from_ptr(grp.gr_name) }
            .to_bytes()
            .to_vec();
        let members = !grp.gr_mem.is_null() && !unsafe { *grp.gr_mem }.is_null();
        (0, Some((name, members)))
    });
    group.is_some_and(|(name, members)| private_group_rule(gid, primary, &name, &login, members))
}

/// The user-private group convention: a group is this account's own when
/// it is the account's primary group, is named as the account's login is,
/// and lists no members. A shared primary group (openSUSE's `users`) has
/// another name, so it isn't.
fn private_group_rule(gid: u32, primary: u32, name: &[u8], login: &[u8], members: bool) -> bool {
    gid == primary && name == login && !members
}

/// Tighten `path` to 0700 when setup found it open (see
/// [`check_private_dir`]); applied by the caller that asked to tighten.
fn tighten_if(path: &Path, tightened: bool) -> Result<bool, RootError> {
    if tightened {
        set_mode(path, 0o700)?;
    }
    Ok(tightened)
}

/// What [`copy_blobs`] copied.
struct CopiedBlobs {
    /// Each stored file's SHA-256, re-hashed from its bytes.
    shas: BTreeSet<String>,
    /// Unfinished writes (`.tmp`) and hidden files (`.DS_Store` and the
    /// like) left behind.
    skipped: u64,
}

/// Copy the blob store: each file must sit at `aa/bb/<sha>` and hash to
/// its name; an unfinished write (`<sha>.tmp`, a put that never finished)
/// is left behind and counted; anything else (a link, a stray file) is
/// refused. Folders 0700, files 0600, every folder synced.
fn copy_blobs(from: &Path, to: &Path) -> Result<CopiedBlobs, RootError> {
    use sha2::{Digest, Sha256};
    let mut copied = CopiedBlobs {
        shas: BTreeSet::new(),
        skipped: 0,
    };
    if !from.exists() {
        return Ok(copied);
    }
    make_private_dir(to)?;
    let stray = |p: &Path| {
        RootError::Adopt(format!(
            "{} isn't a stored file where the books keep them: nothing was adopted",
            p.display()
        ))
    };
    let is_hex = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit());
    // A hidden plain file (`.DS_Store`): no wanderbooks writes one; left
    // behind and counted.
    let hidden = |name: &str, kind: std::fs::FileType| name.starts_with('.') && kind.is_file();
    for a in std::fs::read_dir(from)? {
        let a = a?;
        let a_name = a.file_name().to_string_lossy().into_owned();
        if hidden(&a_name, a.file_type()?) {
            copied.skipped += 1;
            continue;
        }
        if !a.file_type()?.is_dir() || !is_hex(&a_name, 2) {
            return Err(stray(&a.path()));
        }
        let a_to = to.join(&a_name);
        make_private_dir(&a_to)?;
        for b in std::fs::read_dir(a.path())? {
            let b = b?;
            let b_name = b.file_name().to_string_lossy().into_owned();
            if hidden(&b_name, b.file_type()?) {
                copied.skipped += 1;
                continue;
            }
            if !b.file_type()?.is_dir() || !is_hex(&b_name, 2) {
                return Err(stray(&b.path()));
            }
            let b_to = a_to.join(&b_name);
            make_private_dir(&b_to)?;
            for f in std::fs::read_dir(b.path())? {
                let f = f?;
                let name = f.file_name().to_string_lossy().into_owned();
                let kind = f.file_type()?;
                if hidden(&name, kind)
                    || (kind.is_file()
                        && name.strip_suffix(".tmp").is_some_and(|sha| is_hex(sha, 64)))
                {
                    copied.skipped += 1;
                    continue;
                }
                let placed = kind.is_file()
                    && is_hex(&name, 64)
                    && name.starts_with(&a_name)
                    && name[2..].starts_with(&b_name);
                if !placed {
                    if kind.is_symlink() {
                        return Err(RootError::Adopt(format!(
                            "{} is a link: the books' files are never links",
                            f.path().display()
                        )));
                    }
                    return Err(stray(&f.path()));
                }
                let bytes = std::fs::read(f.path())?;
                if hex::encode(Sha256::digest(&bytes)) != name.to_lowercase() {
                    return Err(RootError::Adopt(format!(
                        "the stored file {name} doesn't match its fingerprint (its bytes changed): \
                         nothing was adopted"
                    )));
                }
                write_private_file(&b_to.join(&name), &bytes)?;
                copied.shas.insert(name.to_lowercase());
            }
            sync_dir(&b_to)?;
        }
        sync_dir(&a_to)?;
    }
    sync_dir(to)?;
    Ok(copied)
}

/// Copy the regular files directly in `from` (the vault's records) into a
/// new private folder `to`; folders and links in it are refused.
fn copy_flat_private(from: &Path, to: &Path) -> Result<u64, RootError> {
    make_private_dir(to)?;
    let mut n = 0;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Err(RootError::Adopt(format!(
                "{} isn't a plain file: the vault holds only its records",
                entry.path().display()
            )));
        }
        write_private_file(&to.join(entry.file_name()), &std::fs::read(entry.path())?)?;
        n += 1;
    }
    sync_dir(to)?;
    Ok(n)
}

#[cfg(all(test, unix))]
mod unit {
    use super::*;

    #[test]
    fn a_buffer_grows_while_the_lookup_says_too_small() {
        // Too small twice, then found: 16, 32, 64 KiB tried, in order.
        let mut sizes = Vec::new();
        let found = with_growing_buffer(|buf| {
            sizes.push(buf.len());
            if sizes.len() < 3 {
                (libc::ERANGE, None)
            } else {
                (0, Some(7))
            }
        });
        assert_eq!(found, Some(7));
        assert_eq!(sizes, vec![16 * 1024, 32 * 1024, 64 * 1024]);
        // Too small at every size: it stops at 1 MiB with nothing.
        let mut sizes = Vec::new();
        let found: Option<u8> = with_growing_buffer(|buf| {
            sizes.push(buf.len());
            (libc::ERANGE, Some(1))
        });
        assert_eq!(found, None);
        assert_eq!(sizes.last(), Some(&(1024 * 1024)));
        assert_eq!(sizes.len(), 7);
        // Any other failure: nothing, at once.
        let mut calls = 0;
        let found: Option<u8> = with_growing_buffer(|_| {
            calls += 1;
            (libc::ENOENT, Some(1))
        });
        assert_eq!((found, calls), (None, 1));
        // Found with nothing in it (no such entry): nothing.
        assert_eq!(with_growing_buffer(|_| (0, None::<u8>)), None);
    }

    #[test]
    fn a_private_group_is_the_account_s_own_name_with_no_members() {
        assert!(private_group_rule(1000, 1000, b"pat", b"pat", false));
        assert!(
            !private_group_rule(1001, 1000, b"pat", b"pat", false),
            "not primary"
        );
        assert!(
            !private_group_rule(100, 100, b"users", b"pat", false),
            "shared"
        );
        assert!(
            !private_group_rule(1000, 1000, b"pat", b"pat", true),
            "has members"
        );
    }

    #[test]
    fn root_s_group_is_not_this_account_s_own() {
        // SAFETY: getgid has no preconditions.
        if unsafe { libc::getgid() } != 0 {
            assert!(!is_private_group(0));
        }
    }

    #[test]
    fn the_account_s_home_is_an_absolute_folder() {
        let home = account_home().unwrap();
        assert!(home.is_absolute() && home.is_dir(), "{}", home.display());
        let (_, login) = unix_account().unwrap();
        assert!(!login.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_folder_the_disk_wont_look_up_is_never_made_over() {
        // The disk can't say what is under the name (an I/O error, where
        // making the folder would go through): refused with the disk's
        // own error, and no folder made over whatever is there.
        let dir = tempfile::tempdir().unwrap();
        let book = dir.path();
        let Some(got) = crate::refuse::refusing(crate::refuse::LOOKUPS, libc::EIO, || {
            private_book_folder(book, "keys")
        }) else {
            eprintln!("skipped: no seccomp here");
            return;
        };
        let Err(RootError::Io(why, e)) = got else {
            panic!("refused as an I/O error: {got:?}")
        };
        assert_eq!(e.raw_os_error(), Some(libc::EIO));
        assert!(!why.contains(&book.display().to_string()), "{why}");
        assert!(!book.join("keys").exists());
        // Looked up as it should be: made, private.
        let keys = private_book_folder(book, "keys").unwrap();
        assert_eq!(keys, book.join("keys"));
    }
}
