//! `ledger-gui`: a read-only, live view of wanderbooks books for the people
//! who own them and the accountant who checks them. See `plan/gui.md`.

#![allow(missing_docs)]

pub mod app;
pub mod attention;
pub mod books;
pub mod charts;
pub mod data;
pub mod demo;
pub mod export;
pub mod fmt;
pub mod nav;
pub mod pages;
pub mod palette;
mod receipts;
#[cfg(test)]
mod test_support;
mod theme;
mod widgets;

pub use app::App;
pub use data::Source;

/// The vault key file named on the command line (`--vault-key-file`),
/// which wins over a settings file's `[vault] key_file`, as it does for
/// the server: books that keep their own settings never name it.
static VAULT_KEY_FILE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Name the vault key file (once, at start).
pub fn set_vault_key_file(path: std::path::PathBuf) {
    let _ = VAULT_KEY_FILE.set(path);
}

/// The vault key file named on the command line, if one was.
pub fn vault_key_file() -> Option<std::path::PathBuf> {
    VAULT_KEY_FILE.get().cloned()
}
