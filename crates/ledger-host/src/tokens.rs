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
//!   4. One account, one item: keeping a token first removes every item
//!      kept for its account in any collection, then makes one and reads
//!      it back (else it is refused); reading finds the items in every
//!      collection and, finding several, says so instead of giving one
//!      of them. A keyring that is locked or can't be reached is an
//!      error (KeyringError::Unavailable), never "no token".
//! ```

use crate::knobs::Knobs;
use std::path::{Path, PathBuf};

/// The keyring service tokens are kept under (INVARIANT 1).
pub const SERVICE: &str = "wanderbooks";
/// A token's name when none is given.
pub const DEFAULT_NAME: &str = ledger_store::auth::DEFAULT_NAME;

/// Why the keyring gave no answer (INVARIANT 4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyringError {
    /// Locked, unreachable, or it refused, with why: a token kept there
    /// is untouched.
    Unavailable(String),
    /// Several items for one token, where one belongs.
    Duplicates {
        /// How many.
        count: usize,
        /// The books.
        book: String,
        /// The token's name.
        name: String,
    },
}

impl std::fmt::Display for KeyringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(why) => f.write_str(why),
            Self::Duplicates { count, book, name } => {
                let named = if name == DEFAULT_NAME {
                    String::new()
                } else {
                    format!(" --name {name}")
                };
                write!(
                    f,
                    "this computer's keyring holds {count} items for the token {name:?} of the books {book}, where one belongs, so none of them is used: `wanderbooks token create --book {book}{named} --replace` keeps one again"
                )
            }
        }
    }
}

impl From<KeyringError> for String {
    fn from(e: KeyringError) -> Self {
        e.to_string()
    }
}

/// A place tokens are kept.
pub trait Keeper: Send + Sync {
    /// The token kept for `account`, if any (INVARIANT 4).
    fn get(&self, account: &Account) -> Result<Option<String>, KeyringError>;
    /// Keep `text` for `account`, replacing any (INVARIANT 4).
    fn set(&self, account: &Account, text: &str) -> Result<(), KeyringError>;
    /// Forget `account`'s token (no matter if there is none).
    fn delete(&self, account: &Account) -> Result<(), KeyringError>;
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

    fn duplicates(&self, count: usize) -> KeyringError {
        KeyringError::Duplicates {
            count,
            book: self.book.clone(),
            name: self.name.clone(),
        }
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

/// Items kept under an account, perhaps in several collections: the
/// keyring's or the test files' (INVARIANT 4 is kept once, over these).
trait Items {
    /// One item.
    type Item;
    /// Every item kept for `account`, in any collection.
    fn find(&self, account: &Account) -> Result<Vec<Self::Item>, String>;
    /// An item's text.
    fn read(&self, item: &Self::Item) -> Result<String, String>;
    /// Remove an item.
    fn remove(&self, item: Self::Item) -> Result<(), String>;
    /// Make an item for `account` in the default collection.
    fn make(&self, account: &Account, text: &str) -> Result<(), String>;
}

/// The one token kept for `account` (INVARIANT 4).
fn get_one<I: Items>(items: &I, account: &Account) -> Result<Option<String>, KeyringError> {
    let mut found = items.find(account).map_err(KeyringError::Unavailable)?;
    match found.len() {
        0 => Ok(None),
        1 => items
            .read(&found.remove(0))
            .map(Some)
            .map_err(KeyringError::Unavailable),
        n => Err(account.duplicates(n)),
    }
}

/// Keep `text` as the one item for `account`, read back (INVARIANT 4).
fn set_one<I: Items>(items: &I, account: &Account, text: &str) -> Result<(), KeyringError> {
    delete_all(items, account)?;
    items
        .make(account, text)
        .map_err(KeyringError::Unavailable)?;
    match get_one(items, account)? {
        Some(back) if back == text => Ok(()),
        _ => Err(KeyringError::Unavailable(
            "it didn't give back the token it was given to keep".into(),
        )),
    }
}

/// Remove every item kept for `account`.
fn delete_all<I: Items>(items: &I, account: &Account) -> Result<(), KeyringError> {
    for item in items.find(account).map_err(KeyringError::Unavailable)? {
        items.remove(item).map_err(KeyringError::Unavailable)?;
    }
    Ok(())
}

/// The OS keyring.
struct Os;

#[cfg(target_os = "linux")]
mod os {
    use super::{Account, Items, SERVICE};
    use keyring_core::api::CredentialStoreApi;
    use std::collections::HashMap;
    use std::sync::Arc;
    use zbus_secret_service_keyring_store::Store;

    /// The Secret Service, reached for one keeper call.
    pub(super) struct Service(Arc<Store>);

    impl Service {
        pub(super) fn reach() -> Result<Self, String> {
            Store::new().map(Self).map_err(|e| {
                format!("this computer's keyring can't be reached ({e}): is a Secret Service (GNOME Keyring, KWallet) running and unlocked?")
            })
        }
    }

    /// What a keyring error says (INVARIANT 4: locked is said as such).
    fn refused(e: keyring_core::Error) -> String {
        match e {
            keyring_core::Error::NoStorageAccess(_) => {
                format!("this computer's keyring is locked, or wouldn't let it be read ({e})")
            }
            e => format!("the keyring refused: {e}"),
        }
    }

    impl Items for Service {
        type Item = keyring_core::Entry;

        fn find(&self, account: &Account) -> Result<Vec<Self::Item>, String> {
            let key = account.key();
            let spec = HashMap::from([("service", SERVICE), ("username", key.as_str())]);
            self.0.search(&spec).map_err(refused)
        }

        fn read(&self, item: &Self::Item) -> Result<String, String> {
            item.get_password().map_err(refused)
        }

        fn remove(&self, item: Self::Item) -> Result<(), String> {
            match item.delete_credential() {
                Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
                Err(e) => Err(refused(e)),
            }
        }

        fn make(&self, account: &Account, text: &str) -> Result<(), String> {
            let label = account.label();
            let modifiers = HashMap::from([("label", label.as_str())]);
            self.0
                .build(SERVICE, &account.key(), Some(&modifiers))
                .and_then(|entry| entry.set_password(text))
                .map_err(refused)
        }
    }
}

#[cfg(target_os = "linux")]
impl Keeper for Os {
    fn get(&self, account: &Account) -> Result<Option<String>, KeyringError> {
        let service = os::Service::reach().map_err(KeyringError::Unavailable)?;
        get_one(&service, account)
    }

    fn set(&self, account: &Account, text: &str) -> Result<(), KeyringError> {
        let service = os::Service::reach().map_err(KeyringError::Unavailable)?;
        set_one(&service, account, text)
    }

    fn delete(&self, account: &Account) -> Result<(), KeyringError> {
        let service = os::Service::reach().map_err(KeyringError::Unavailable)?;
        delete_all(&service, account)
    }
}

#[cfg(not(target_os = "linux"))]
impl Keeper for Os {
    fn get(&self, _: &Account) -> Result<Option<String>, KeyringError> {
        Err(KeyringError::Unavailable(NOT_YET.into()))
    }
    fn set(&self, _: &Account, _: &str) -> Result<(), KeyringError> {
        Err(KeyringError::Unavailable(NOT_YET.into()))
    }
    fn delete(&self, _: &Account) -> Result<(), KeyringError> {
        Err(KeyringError::Unavailable(NOT_YET.into()))
    }
}

#[cfg(not(target_os = "linux"))]
const NOT_YET: &str =
    "agents' tokens are kept in the keyring on Linux only, until the macOS and Windows apps (P11)";

/// Files, for tests (INVARIANT 3): a folder per collection (`default/`
/// where tokens are kept), a file a token. A collection holding a file
/// `locked` can't be read; the keyring holding a file `unreachable`
/// can't be reached. So the tests can lock it, or plant a second item.
struct Files(PathBuf);

/// The test keyring's default collection.
pub const TEST_DEFAULT: &str = "default";
/// In a test collection: its items can't be read.
pub const TEST_LOCKED: &str = "locked";
/// In the test keyring: it can't be reached.
pub const TEST_UNREACHABLE: &str = "unreachable";

impl Files {
    fn file_name(account: &Account) -> String {
        ledger_store::journal::sha256_hex(account.key().as_bytes())
    }
}

impl Items for Files {
    type Item = PathBuf;

    fn find(&self, account: &Account) -> Result<Vec<PathBuf>, String> {
        let fail = |e: std::io::Error| format!("the test keyring refused: {}", e.kind());
        if self.0.join(TEST_UNREACHABLE).exists() {
            return Err("the test keyring can't be reached".into());
        }
        let collections = match std::fs::read_dir(&self.0) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(fail(e)),
        };
        let name = Self::file_name(account);
        let mut found = Vec::new();
        for c in collections {
            let file = c.map_err(fail)?.path().join(&name);
            if file.is_file() {
                found.push(file);
            }
        }
        found.sort();
        Ok(found)
    }

    fn read(&self, item: &PathBuf) -> Result<String, String> {
        if item.with_file_name(TEST_LOCKED).exists() {
            return Err("the test keyring is locked".into());
        }
        std::fs::read_to_string(item).map_err(|e| format!("the test keyring refused: {}", e.kind()))
    }

    fn remove(&self, item: PathBuf) -> Result<(), String> {
        match std::fs::remove_file(item) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("the test keyring refused: {}", e.kind())),
        }
    }

    fn make(&self, account: &Account, text: &str) -> Result<(), String> {
        use std::io::Write;
        let fail = |e: std::io::Error| format!("the test keyring refused: {}", e.kind());
        let dir = self.0.join(TEST_DEFAULT);
        std::fs::create_dir_all(&dir).map_err(fail)?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(dir.join(Self::file_name(account)))
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .map_err(fail)
    }
}

impl Keeper for Files {
    fn get(&self, account: &Account) -> Result<Option<String>, KeyringError> {
        get_one(self, account)
    }

    fn set(&self, account: &Account, text: &str) -> Result<(), KeyringError> {
        set_one(self, account, text)
    }

    fn delete(&self, account: &Account) -> Result<(), KeyringError> {
        delete_all(self, account)
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
            let files: Vec<_> =
                std::fs::read_dir(dir.path().join("test-keyring").join(TEST_DEFAULT))
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

    /// The test keyring with a second collection (INVARIANT 4).
    fn two_collections() -> (tempfile::TempDir, Files, Account, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let files = Files(dir.path().join("test-keyring"));
        let a = Account::new(dir.path(), "acme", "agent");
        let other = files.0.join("login");
        std::fs::create_dir_all(&other).unwrap();
        (dir, files, a, other)
    }

    #[test]
    fn two_items_for_one_token_are_said_never_one_of_them_given() {
        let (_dir, k, a, other) = two_collections();
        k.set(&a, "wb_new").unwrap();
        std::fs::write(other.join(Files::file_name(&a)), "wb_old").unwrap();
        let err = k.get(&a).unwrap_err();
        assert_eq!(err, a.duplicates(2));
        let said = err.to_string();
        assert!(
            said.contains("2 items") && said.contains("token create --book acme --replace"),
            "{said}"
        );
        assert!(!said.contains("wb_"), "{said}");
        // Kept again: every item goes first, and one is left.
        k.set(&a, "wb_newer").unwrap();
        assert_eq!(k.get(&a).unwrap().as_deref(), Some("wb_newer"));
        assert!(!other.join(Files::file_name(&a)).exists());
        // Forgotten in every collection.
        std::fs::write(other.join(Files::file_name(&a)), "wb_old").unwrap();
        k.delete(&a).unwrap();
        assert_eq!(k.get(&a).unwrap(), None);
        // Another token's name is said in the repair.
        let b = Account::new(_dir.path(), "acme", "codex");
        assert!(
            b.duplicates(3)
                .to_string()
                .contains("--name codex --replace")
        );
    }

    #[test]
    fn a_locked_or_unreachable_keyring_is_an_error_never_no_token() {
        let (_dir, k, a, other) = two_collections();
        // The only item, in a locked collection.
        std::fs::write(other.join(Files::file_name(&a)), "wb_old").unwrap();
        std::fs::write(other.join(TEST_LOCKED), "").unwrap();
        let err = k.get(&a).unwrap_err();
        assert!(matches!(err, KeyringError::Unavailable(_)), "{err:?}");
        std::fs::remove_file(other.join(TEST_LOCKED)).unwrap();
        assert_eq!(k.get(&a).unwrap().as_deref(), Some("wb_old"));
        std::fs::write(k.0.join(TEST_UNREACHABLE), "").unwrap();
        for err in [
            k.get(&a).unwrap_err(),
            k.set(&a, "wb_new").unwrap_err(),
            k.delete(&a).unwrap_err(),
        ] {
            assert!(matches!(err, KeyringError::Unavailable(_)), "{err:?}");
        }
        std::fs::remove_file(k.0.join(TEST_UNREACHABLE)).unwrap();
        assert_eq!(k.get(&a).unwrap().as_deref(), Some("wb_old"), "untouched");
    }

    /// A keyring that keeps something other than it was given.
    struct Forgetful;

    impl Items for Forgetful {
        type Item = ();
        fn find(&self, _: &Account) -> Result<Vec<()>, String> {
            Ok(vec![()])
        }
        fn read(&self, _: &()) -> Result<String, String> {
            Ok("wb_someone_elses".into())
        }
        fn remove(&self, _: ()) -> Result<(), String> {
            Ok(())
        }
        fn make(&self, _: &Account, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn a_token_kept_is_read_back_or_refused() {
        let dir = tempfile::tempdir().unwrap();
        let a = Account::new(dir.path(), "acme", "agent");
        let err = set_one(&Forgetful, &a, "wb_mine").unwrap_err();
        assert!(
            matches!(&err, KeyringError::Unavailable(w) if w.contains("didn't give back")),
            "{err:?}"
        );
        assert!(!err.to_string().contains("wb_"));
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
