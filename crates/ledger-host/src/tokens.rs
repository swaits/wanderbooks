//! Where an agent's token is kept on this computer: the OS keyring (plan
//! 3.3, 3.5, D-23), read by the bridge, so no agent's configuration
//! holds it.
//!
//! ```text
//! GOAL: `wanderbooks token create` keeps the token it was shown where
//!       the bridge finds it, and nowhere an agent's configuration or the
//!       screen would show it.
//! SOURCE: plan 3.3 ("kept in the OS keyring, minted by wanderbooks
//!         connect, revocable"), 2.1 ("Local token: OS keyring, read by
//!         the bridge"), 3.5 ("plaintext in the user's keyring, which any
//!         process running as that user can read (advisory locally)").
//! INVARIANTS:
//!   1. A token is kept under the service "wanderbooks" and an account
//!      naming the root (a hash of its path, never the path), the book
//!      and the token's name, so two roots or two books never share one.
//!   2. Nothing here prints or logs a token's text, and no error repeats
//!      it.
//!   3. The keyring stands in for nothing else: with no keyring the
//!      command says so; only the test knob `file_keyring` keeps tokens
//!      in files instead (in the root's `run/`, 0600).
//! ```

use crate::knobs::Knobs;
use std::path::{Path, PathBuf};

/// The keyring service tokens are kept under (INVARIANT 1).
pub const SERVICE: &str = "wanderbooks";
/// A token's name when none is given.
pub const DEFAULT_NAME: &str = ledger_store::auth::DEFAULT_NAME;

/// A place tokens are kept.
pub trait Keeper: Send + Sync {
    /// The token kept for `account`, if any.
    fn get(&self, account: &Account) -> Result<Option<String>, String>;
    /// Keep `text` for `account`, replacing any.
    fn set(&self, account: &Account, text: &str) -> Result<(), String>;
    /// Forget `account`'s token (no matter if there is none).
    fn delete(&self, account: &Account) -> Result<(), String>;
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

    /// The keyring's account name: `<root tag>/<book>/<name>`.
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.root_tag, self.book, self.name)
    }

    /// What the keyring's own window calls it.
    fn label(&self) -> String {
        format!("wanderbooks token: the books {} ({})", self.book, self.name)
    }
}

/// The keeper in force: the OS keyring, or files for tests (INVARIANT 3).
pub fn keeper(run_dir: &Path, knobs: &Knobs) -> Box<dyn Keeper> {
    if knobs.file_keyring {
        Box::new(Files(run_dir.join("test-keyring")))
    } else {
        Box::new(Os)
    }
}

/// The OS keyring.
struct Os;

#[cfg(target_os = "linux")]
impl Os {
    fn entry(&self, account: &Account) -> Result<keyring_core::Entry, String> {
        use keyring_core::api::CredentialStoreApi;
        let store = zbus_secret_service_keyring_store::Store::new().map_err(|e| {
            format!("this computer's keyring can't be reached ({e}): is a Secret Service (GNOME Keyring, KWallet) running and unlocked?")
        })?;
        let label = account.label();
        let modifiers = std::collections::HashMap::from([("label", label.as_str())]);
        store
            .build(SERVICE, &account.key(), Some(&modifiers))
            .map_err(|e| format!("the keyring refused: {e}"))
    }
}

#[cfg(target_os = "linux")]
impl Keeper for Os {
    fn get(&self, account: &Account) -> Result<Option<String>, String> {
        match self.entry(account)?.get_password() {
            Ok(text) => Ok(Some(text)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("the keyring refused: {e}")),
        }
    }

    fn set(&self, account: &Account, text: &str) -> Result<(), String> {
        self.entry(account)?
            .set_password(text)
            .map_err(|e| format!("the keyring refused: {e}"))
    }

    fn delete(&self, account: &Account) -> Result<(), String> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("the keyring refused: {e}")),
        }
    }
}

#[cfg(not(target_os = "linux"))]
impl Keeper for Os {
    fn get(&self, _: &Account) -> Result<Option<String>, String> {
        Err(NOT_YET.into())
    }
    fn set(&self, _: &Account, _: &str) -> Result<(), String> {
        Err(NOT_YET.into())
    }
    fn delete(&self, _: &Account) -> Result<(), String> {
        Err(NOT_YET.into())
    }
}

#[cfg(not(target_os = "linux"))]
const NOT_YET: &str =
    "agents' tokens are kept in the keyring on Linux only, until the macOS and Windows apps (P11)";

/// Files, one a token, for tests (INVARIANT 3).
struct Files(PathBuf);

impl Files {
    fn path(&self, account: &Account) -> PathBuf {
        self.0
            .join(ledger_store::journal::sha256_hex(account.key().as_bytes()))
    }
}

impl Keeper for Files {
    fn get(&self, account: &Account) -> Result<Option<String>, String> {
        match std::fs::read_to_string(self.path(account)) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("the test keyring refused: {}", e.kind())),
        }
    }

    fn set(&self, account: &Account, text: &str) -> Result<(), String> {
        use std::io::Write;
        let fail = |e: std::io::Error| format!("the test keyring refused: {}", e.kind());
        std::fs::create_dir_all(&self.0).map_err(fail)?;
        let path = self.path(account);
        let _ = std::fs::remove_file(&path);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&path)
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .map_err(fail)
    }

    fn delete(&self, account: &Account) -> Result<(), String> {
        match std::fs::remove_file(self.path(account)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("the test keyring refused: {}", e.kind())),
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
        assert!(!a.label().contains('/'));
    }

    #[test]
    fn the_test_keyring_keeps_and_forgets() {
        let dir = tempfile::tempdir().unwrap();
        let knobs = Knobs {
            file_keyring: true,
            ..Knobs::default()
        };
        let k = keeper(dir.path(), &knobs);
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
        #[cfg(unix)]
        {
            k.set(&a, "wb_three").unwrap();
            use std::os::unix::fs::PermissionsExt;
            let files: Vec<_> = std::fs::read_dir(dir.path().join("test-keyring"))
                .unwrap()
                .collect();
            assert_eq!(files.len(), 1);
            let mode = files[0]
                .as_ref()
                .unwrap()
                .metadata()
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    /// The real keyring, on a desktop with one (`cargo test -p
    /// ledger-host -- --ignored os_keyring`): kept, read, replaced and
    /// forgotten, under a service no one else uses.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn os_keyring_keeps_and_forgets() {
        let dir = tempfile::tempdir().unwrap();
        let a = Account::new(dir.path(), "ignored-test", "agent");
        let k = Os;
        k.set(&a, "wb_test_one").unwrap();
        assert_eq!(k.get(&a).unwrap().as_deref(), Some("wb_test_one"));
        k.set(&a, "wb_test_two").unwrap();
        assert_eq!(k.get(&a).unwrap().as_deref(), Some("wb_test_two"));
        k.delete(&a).unwrap();
        assert_eq!(k.get(&a).unwrap(), None);
        k.delete(&a).unwrap();
    }
}
