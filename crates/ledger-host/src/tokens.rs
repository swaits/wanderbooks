//! Where an agent's token is kept on this computer: a file in the data
//! root's `run/` folder, read by the bridge, so no agent's configuration
//! holds it.
//!
//! Removed (D-63): the OS keyring stores (Secret Service, macOS Keychain,
//! Windows Credential Manager) and the `file_keyring` test knob, because
//! the 0700 data root is the trust boundary already (I-1: hidden, not
//! locked), so a keyring added no wall and cost headless servers a D-Bus
//! session and every OS its own store.
//!
//! ```text
//! GOAL: `wanderbooks token create` keeps the token it was shown where
//!       the bridge finds it, and nowhere an agent's configuration or the
//!       screen would show it.
//! SOURCE: plan 3.3 and D-63 ("a 0600 file under the 0700 data root, on
//!         every OS"), 2.1 ("Local token ... read by the bridge"), 3.5
//!         (any process running as that user can read it: advisory
//!         locally, I-1).
//! INVARIANTS:
//!   1. A token is kept in a file named by a hash of an account naming
//!      the root (a hash of its path, never the path), the book and the
//!      token's name, so two roots or two books never share one.
//!   2. Nothing here prints or logs a token's text, and no error repeats
//!      it.
//!   3. The file is in the root's `run/tokens/`, mode 0600 on Unix, and
//!      written atomically: a temporary file in the same folder, made
//!      0600 before any byte is written, then renamed over the old one,
//!      so a reader sees the old token or the new one, never half.
//!   4. A token file that is there but can't be read is an error, never
//!      "no token"; only a missing file is no token.
//! ```

use std::path::{Path, PathBuf};

/// A token's name when none is given.
pub const DEFAULT_NAME: &str = ledger_store::auth::DEFAULT_NAME;

/// Why the token file couldn't be read, kept or removed (INVARIANT 4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenFileError(String);

impl TokenFileError {
    /// An error with `why`, which never holds a token's text.
    pub(crate) fn new(why: &str) -> Self {
        Self(why.to_string())
    }
}

impl std::fmt::Display for TokenFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<TokenFileError> for String {
    fn from(e: TokenFileError) -> Self {
        e.to_string()
    }
}

fn refused(e: std::io::Error) -> TokenFileError {
    TokenFileError(format!("the token file refused: {}", e.kind()))
}

/// Whose token: a root's book's token of a name (INVARIANT 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    root_tag: String,
    book: String,
    name: String,
}

impl Account {
    /// The token `name` of `book` in the root at `root`.
    pub fn new(root: &Path, book: &str, name: &str) -> Self {
        let real = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let hash = ledger_store::journal::sha256_hex(real.as_os_str().as_encoded_bytes());
        Self {
            root_tag: hash[..16].to_string(),
            book: book.to_string(),
            name: name.to_string(),
        }
    }

    /// The account's name: `<root tag>/<book>/<name>`.
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.root_tag, self.book, self.name)
    }

    fn file_name(&self) -> String {
        ledger_store::journal::sha256_hex(self.key().as_bytes())
    }
}

/// The place tokens are kept: files in a root's `run/` folder.
pub struct Keeper(PathBuf);

/// The keeper for the root whose `run/` folder is `run_dir`.
pub fn keeper(run_dir: &Path) -> Keeper {
    Keeper(run_dir.join("tokens"))
}

impl Keeper {
    /// Where `account`'s token file is.
    pub(crate) fn file(&self, account: &Account) -> PathBuf {
        self.0.join(account.file_name())
    }

    /// The token kept for `account`, if any (INVARIANT 4).
    pub fn get(&self, account: &Account) -> Result<Option<String>, TokenFileError> {
        match std::fs::read_to_string(self.file(account)) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(refused(e)),
        }
    }

    /// Keep `text` for `account`, replacing any (INVARIANT 3).
    pub fn set(&self, account: &Account, text: &str) -> Result<(), TokenFileError> {
        use std::io::Write;
        std::fs::create_dir_all(&self.0).map_err(refused)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700))
                .map_err(refused)?;
        }
        // Windows: this account's alone, and what is made in it too.
        #[cfg(windows)]
        ledger_store::winsec::set_owner_only(&self.0).map_err(refused)?;
        let name = account.file_name();
        let tmp = self.0.join(format!("{name}.tmp{}", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let written = options
            .open(&tmp)
            .and_then(|mut f| f.write_all(text.as_bytes()).and_then(|()| f.sync_all()))
            .and_then(|()| std::fs::rename(&tmp, self.0.join(&name)));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(refused(e));
        }
        Ok(())
    }

    /// Forget `account`'s token (no matter if there is none).
    pub fn delete(&self, account: &Account) -> Result<(), TokenFileError> {
        match std::fs::remove_file(self.file(account)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(refused(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_names_the_root_by_hash_and_the_book_and_token() {
        let dir = tempfile::tempdir().unwrap();
        let a = Account::new(dir.path(), "acme", "agent");
        let key = a.key();
        assert!(key.ends_with("/acme/agent"), "{key}");
        assert!(!key.contains(&dir.path().display().to_string()));
        assert_eq!(key.split('/').next().unwrap().len(), 16);
        // The same root by another spelling is the same account.
        let again = Account::new(&dir.path().join("."), "acme", "agent");
        assert_eq!(again, a);
        let other = tempfile::tempdir().unwrap();
        assert_ne!(Account::new(other.path(), "acme", "agent").key(), key);
        assert_ne!(Account::new(dir.path(), "beta", "agent").key(), key);
        assert_ne!(Account::new(dir.path(), "acme", "codex").key(), key);
    }

    #[test]
    fn the_token_file_keeps_and_forgets() {
        let dir = tempfile::tempdir().unwrap();
        let k = keeper(dir.path());
        let a = Account::new(dir.path(), "acme", "agent");
        assert_eq!(k.get(&a).unwrap(), None);
        k.set(&a, "wb_one").unwrap();
        assert_eq!(k.get(&a).unwrap().as_deref(), Some("wb_one"));
        k.set(&a, "wb_two").unwrap();
        assert_eq!(k.get(&a).unwrap().as_deref(), Some("wb_two"));
        let b = Account::new(dir.path(), "acme", "codex");
        assert_eq!(k.get(&b).unwrap(), None);
        k.delete(&a).unwrap();
        k.delete(&a).unwrap();
        assert_eq!(k.get(&a).unwrap(), None);
    }

    #[test]
    fn a_token_file_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let k = keeper(dir.path());
        let a = Account::new(dir.path(), "acme", "agent");
        k.set(&a, "wb_first").unwrap();
        // A second create for the same name replaces the first; one file
        // remains, and no temporary file is left behind.
        k.set(&a, "wb_second").unwrap();
        assert_eq!(k.get(&a).unwrap().as_deref(), Some("wb_second"));
        let files: Vec<_> = std::fs::read_dir(dir.path().join("tokens"))
            .unwrap()
            .map(|e| e.unwrap())
            .collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name().to_string_lossy(), a.file_name());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = files[0].metadata().unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let folder = dir.path().join("tokens").metadata().unwrap();
            assert_eq!(folder.permissions().mode() & 0o777, 0o700);
        }
    }

    #[test]
    fn an_unreadable_token_file_is_an_error_never_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let k = keeper(dir.path());
        let a = Account::new(dir.path(), "acme", "agent");
        // A directory where the file belongs.
        std::fs::create_dir_all(dir.path().join("tokens").join(a.file_name())).unwrap();
        let err = k.get(&a).unwrap_err();
        assert!(err.to_string().contains("token file"), "{err}");
        assert!(!err.to_string().contains("wb_"));
        assert!(k.set(&a, "wb_new").is_err());
        assert!(k.delete(&a).is_err());
    }
}
