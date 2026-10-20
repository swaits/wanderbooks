//! The server's settings: the books' own (journaled, ledger_core::
//! settings), or, for books that hold none yet, the settings files
//! (`wanderbooks.toml`, and the receipt policy in `books-data/config.json`)
//! reloaded whenever they change.
//!
//! ```text
//! GOAL: every request sees the settings in force, one version for the
//!       whole request; books that keep their own settings never read a
//!       file; a broken file edit never takes the server down.
//! SOURCE: plan/served.md I-3 (the journal holds all books state; the
//!         server never reads a settings file for books that hold their
//!         settings), 2.1 ("dev-stdio keeps legacy_files mode until a book
//!         is adopted"); DECISION (owner, 2026-10-15): file settings are
//!         always auto-reloaded.
//! INVARIANTS:
//!   1. Books whose journal holds settings (any settings.set) use those,
//!      and from then on no file is read: sync takes them whenever the
//!      journal's last settings.set changes.
//!   2. Otherwise every request (tool call, resource read) sees the files'
//!      contents as of its start: a request after an edit sees the edit
//!      (compared by content, not by timestamp, so two edits in one second
//!      are both seen).
//!   3. A file that fails to load leaves the last good settings in force,
//!      and the failure is reported (books.settings, stderr) until a later
//!      edit loads.
//!   4. A settings file that doesn't exist and wasn't asked for (the
//!      default wanderbooks.toml beside the books) means the defaults; one
//!      named with --books-toml that goes missing is a failure (3).
//!   5. Only the settings change while serving: the data dir, the books'
//!      base currency, and a vault key file given on the command line are
//!      fixed when the server starts (the key file given wins over any
//!      file's [vault] key_file).
//!   6. A book in the data root (plan 3.4) has no settings file at all:
//!      its settings are its own, the defaults until the first is set,
//!      and no file is ever read for it.
//! ```

use crate::policy::ReceiptPolicy;
use ledger_config::Company;
use std::path::PathBuf;
use std::sync::Arc;

/// `[books]` account roles for the cash flow statement, used when a call
/// doesn't pass its own: cash, financing, and investing account codes.
#[derive(Clone, Debug, Default)]
pub struct CashRoles {
    /// `[books] cash_accounts`.
    pub cash: Vec<String>,
    /// `[books] financing_accounts`.
    pub financing: Vec<String>,
    /// `[books] investing_accounts`.
    pub investing: Vec<String>,
}

/// What the settings files say, as last loaded.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    /// Which entries need a receipt.
    pub policy: ReceiptPolicy,
    /// `[company]`.
    pub company: Company,
    /// `[books]` cash flow roles.
    pub roles: CashRoles,
    /// The prefix of generated asset tags (`[books] asset_tag_prefix`, or
    /// the company's initials).
    pub asset_tag_prefix: String,
    /// `[vault] key_file`: the TIN vault's key file (unset: the vault's
    /// default, `ledger_vault::default_key_file`).
    pub vault_key_file: Option<PathBuf>,
    /// `[books] parties`: suggest, require, or never mention each entry's
    /// counterparty.
    pub parties: ledger_config::PartyPolicy,
    /// The company's time zone (an IANA name), if set: what "today" means.
    pub timezone: Option<String>,
    /// Whether these are the books' own settings (journaled) rather than a
    /// file's.
    pub journaled: bool,
}

impl Settings {
    /// The vault key file in force: `[vault] key_file`, else the default
    /// outside the books (`~/.config/wanderbooks/vault.key`).
    pub fn key_file(&self) -> PathBuf {
        self.vault_key_file
            .clone()
            .unwrap_or_else(ledger_vault::default_key_file)
    }

    /// The settings the books keep (`ledger_core::settings`), as the
    /// server uses them.
    pub fn from_books(books: &ledger_core::settings::Settings, currency: &str) -> Self {
        let b = ledger_config::codec::books_settings(books, currency);
        Settings {
            policy: ledger_config::codec::receipt_policy(books),
            asset_tag_prefix: b.asset_tag_prefix(),
            vault_key_file: None,
            parties: b.parties,
            timezone: b.timezone.clone(),
            roles: CashRoles {
                cash: b.cash_accounts,
                financing: b.financing_accounts,
                investing: b.investing_accounts,
            },
            company: b.company,
            journaled: true,
        }
    }

    /// The prefix for generated asset tags ("FA" when none is set).
    pub fn tag_prefix(&self) -> &str {
        if self.asset_tag_prefix.is_empty() {
            "FA"
        } else {
            &self.asset_tag_prefix
        }
    }
}

/// Where the settings come from.
#[derive(Clone, Debug)]
pub struct Source {
    /// The books' wanderbooks.toml.
    pub books_toml: PathBuf,
    /// Whether it must exist (named with --books-toml) or may be absent
    /// (the default beside the books: then the defaults apply).
    pub required: bool,
    /// `config.json`, for its receipt policy (optional).
    pub config_json: PathBuf,
}

/// Both files' bytes (`None`: absent), to tell when either changed.
type Seen = (Option<Vec<u8>>, Option<Vec<u8>>);

impl Source {
    fn read(&self) -> Seen {
        (
            std::fs::read(&self.books_toml).ok(),
            std::fs::read(&self.config_json).ok(),
        )
    }

    /// Load the settings now.
    pub fn load(&self) -> Result<Settings, String> {
        let books = if self.required || self.books_toml.exists() {
            ledger_config::load(&self.books_toml)?
        } else {
            ledger_config::BooksSettings::default()
        };
        // config.json sits in the data dir, which no message names.
        let policy = ledger_config::effective_receipt_policy(&self.config_json, &books)
            .map_err(|e| e.replace(&self.config_json.display().to_string(), "config.json"))?;
        Ok(Settings {
            asset_tag_prefix: books.asset_tag_prefix(),
            vault_key_file: books.vault_key_file.clone(),
            parties: books.parties,
            timezone: books.timezone.clone(),
            journaled: false,
            policy,
            roles: CashRoles {
                cash: books.cash_accounts,
                financing: books.financing_accounts,
                investing: books.investing_accounts,
            },
            company: books.company,
        })
    }
}

/// The settings in force, and how they got there.
#[derive(Debug)]
pub struct Live {
    current: Arc<Settings>,
    source: Option<Source>,
    seen: Seen,
    error: Option<String>,
    loads: u64,
    /// The seq of the books' last settings.set taken (INVARIANT 1).
    journal_seq: Option<u64>,
    /// A vault key file given on the command line (INVARIANT 5).
    vault_key_file: Option<PathBuf>,
    /// Whether the settings files may be read: false once the books keep
    /// their own (INVARIANT 1).
    read_files: bool,
    /// A book in the data root: its settings are only ever its own
    /// (INVARIANT 6).
    own: bool,
}

/// What books.settings reports about the files.
#[derive(Clone, Debug)]
pub struct Status {
    /// The wanderbooks.toml read, if the settings come from files.
    pub books_toml: Option<PathBuf>,
    /// Whether that file exists now.
    pub books_toml_exists: bool,
    /// Whether a config.json is read (it sits in the data dir, which is
    /// never named).
    pub config_json_present: bool,
    /// Why the last change failed to load (the last good settings stay in
    /// force), or `None`.
    pub error: Option<String>,
    /// How many times the settings have loaded, the first included.
    pub loads: u64,
    /// Whether the settings are the books' own (journaled): then no file
    /// is read.
    pub journaled: bool,
}

impl Live {
    /// Settings that never reload (tests, and servers built in code).
    pub fn fixed(settings: Settings) -> Self {
        Self {
            current: Arc::new(settings),
            source: None,
            seen: (None, None),
            error: None,
            loads: 1,
            journal_seq: None,
            vault_key_file: None,
            read_files: false,
            own: false,
        }
    }

    /// For a book in the data root (INVARIANT 6): no settings file, now
    /// or later; `sync` brings the books' settings in once they hold any.
    pub fn own() -> Self {
        Self {
            current: Arc::new(Settings {
                journaled: true,
                ..Settings::default()
            }),
            source: None,
            seen: (None, None),
            error: None,
            loads: 0,
            journal_seq: None,
            vault_key_file: None,
            read_files: false,
            own: true,
        }
    }

    /// Load from `source`, and reload from it whenever it changes. Fails
    /// only on this first load: a server doesn't start on broken settings.
    pub fn watching(source: Source) -> Result<Self, String> {
        let seen = source.read();
        let settings = source.load()?;
        Ok(Self {
            current: Arc::new(settings),
            source: Some(source),
            seen,
            error: None,
            loads: 1,
            journal_seq: None,
            vault_key_file: None,
            read_files: true,
            own: false,
        })
    }

    /// For books that keep their own settings: nothing is read from
    /// `source` (INVARIANT 1), not even at start, so a broken settings
    /// file beside them can't stop the server; `sync` brings the books'
    /// settings in.
    pub fn journaled(source: Source) -> Self {
        Self {
            current: Arc::new(Settings {
                journaled: true,
                ..Settings::default()
            }),
            source: Some(source),
            seen: (None, None),
            error: None,
            loads: 0,
            journal_seq: None,
            vault_key_file: None,
            read_files: false,
            own: false,
        }
    }

    /// Use this vault key file whatever the settings say (INVARIANT 5).
    pub fn with_vault_key_file(mut self, path: Option<PathBuf>) -> Self {
        self.vault_key_file = path;
        if let Some(p) = &self.vault_key_file {
            Arc::make_mut(&mut self.current).vault_key_file = Some(p.clone());
        }
        self
    }

    /// Take the books' own settings when they hold any and they changed
    /// since the last look (INVARIANT 1). The server calls this whenever
    /// it holds the books.
    pub fn sync(&mut self, books: &ledger_core::settings::Settings, currency: &str) {
        if !books.is_journaled() || books.last_seq() == self.journal_seq {
            return;
        }
        let mut s = Settings::from_books(books, currency);
        s.vault_key_file = self.vault_key_file.clone();
        self.current = Arc::new(s);
        self.journal_seq = books.last_seq();
        self.read_files = false;
        self.error = None;
        self.loads += 1;
    }

    /// The settings files' texts as the server reads them now
    /// (`wanderbooks.toml`, `config.json`; `None` for one absent), when the
    /// settings come from files; `None` when the books keep their own or
    /// the settings were built in code.
    pub fn file_texts(&self) -> Option<(Option<String>, Option<String>)> {
        let source = self.source.as_ref().filter(|_| self.read_files)?;
        Some((
            std::fs::read_to_string(&source.books_toml).ok(),
            std::fs::read_to_string(&source.config_json).ok(),
        ))
    }

    /// The vault key file given on the command line, if any.
    pub fn vault_key_override(&self) -> Option<PathBuf> {
        self.vault_key_file.clone()
    }

    /// Whether the settings in force are the books' own.
    pub fn is_journaled(&self) -> bool {
        self.own || self.journal_seq.is_some() || (self.source.is_some() && !self.read_files)
    }

    /// The settings in force.
    pub fn current(&self) -> Arc<Settings> {
        self.current.clone()
    }

    /// Reload if a file changed since the last look: the server calls this
    /// once at the start of every request, so the whole request sees one
    /// version.
    pub fn refresh(&mut self) {
        if !self.read_files {
            // The books keep their own settings (or the settings were
            // built in code): no file is read (INVARIANT 1).
            return;
        }
        if let Some(source) = &self.source {
            let now = source.read();
            if now != self.seen {
                self.seen = now;
                match source.load() {
                    Ok(mut s) => {
                        if let Some(p) = &self.vault_key_file {
                            s.vault_key_file = Some(p.clone());
                        }
                        self.current = Arc::new(s);
                        self.error = None;
                        self.loads += 1;
                        eprintln!("ledger-mcp: reloaded {}", source.books_toml.display());
                    }
                    Err(e) => {
                        eprintln!(
                            "ledger-mcp: settings not reloaded (keeping the last good ones): {e}"
                        );
                        self.error = Some(e);
                    }
                }
            }
        }
    }

    /// Change the settings in force (builders; never reloads them away
    /// unless a watched file changes).
    #[cfg(test)]
    pub fn edit(&mut self, f: impl FnOnce(&mut Settings)) {
        f(Arc::make_mut(&mut self.current));
    }

    /// Where the settings come from and whether the last change loaded.
    pub fn status(&mut self) -> Status {
        self.refresh();
        let journaled = self.is_journaled();
        Status {
            books_toml: self
                .source
                .as_ref()
                .filter(|_| !journaled)
                .map(|s| s.books_toml.clone()),
            books_toml_exists: self.source.as_ref().is_some_and(|s| s.books_toml.exists()),
            config_json_present: self.source.as_ref().is_some_and(|s| s.config_json.exists()),
            error: self.error.clone(),
            loads: self.loads,
            journaled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test's dir, removed when the test ends, pass or panic.
    struct TestDir(std::path::PathBuf);

    impl std::ops::Deref for TestDir {
        type Target = std::path::Path;
        fn deref(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn dir(tag: &str) -> TestDir {
        let d =
            std::env::temp_dir().join(format!("ledger-mcp-settings-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        TestDir(d)
    }

    fn source(d: &std::path::Path, required: bool) -> Source {
        Source {
            books_toml: d.join("wanderbooks.toml"),
            required,
            config_json: d.join("config.json"),
        }
    }

    /// INVARIANT 3: an absent default means the defaults, and the file
    /// loads once it appears; an unchanged file is not reloaded.
    #[test]
    fn an_absent_default_is_the_defaults_until_it_appears() {
        let d = dir("absent");
        let mut live = Live::watching(source(&d, false)).unwrap();
        assert_eq!(live.current().company.name, Company::default().name);
        live.refresh();
        assert_eq!(live.status().loads, 1, "nothing changed");
        std::fs::write(
            d.join("wanderbooks.toml"),
            "[company]\nname = \"Late Co\"\n",
        )
        .unwrap();
        live.refresh();
        assert_eq!(live.current().company.name, "Late Co");
        assert_eq!(live.status().loads, 2);
        assert!(live.status().books_toml_exists);
    }

    /// INVARIANT 3: a named file must exist to start; INVARIANT 2: the
    /// first load fails on broken settings, a later one keeps the last.
    #[test]
    fn a_named_file_must_load_to_start() {
        let d = dir("named");
        assert!(Live::watching(source(&d, true)).is_err());
        std::fs::write(d.join("wanderbooks.toml"), "[books\n").unwrap();
        assert!(Live::watching(source(&d, true)).is_err());
        std::fs::write(d.join("wanderbooks.toml"), "[company]\nname = \"One\"\n").unwrap();
        let mut live = Live::watching(source(&d, true)).unwrap();
        std::fs::write(d.join("wanderbooks.toml"), "[company]\nname = 5\n").unwrap();
        live.refresh();
        assert_eq!(live.current().company.name, "One");
        assert!(live.status().error.is_some());
    }

    /// config.json sits in the data dir, which no tool result names: a
    /// broken one is reported by name, never by its path.
    #[test]
    fn a_broken_config_json_never_names_the_data_dir() {
        let d = dir("json-broken");
        std::fs::write(d.join("config.json"), "{ not json").unwrap();
        let err = Live::watching(source(&d, false)).unwrap_err();
        assert!(err.contains("config.json"), "{err}");
        assert!(!err.contains(d.to_str().unwrap()), "{err}");
        let status = Live::fixed(Settings::default()).status();
        assert!(!status.config_json_present);
    }

    /// The receipt policy in config.json is watched too.
    #[test]
    fn the_config_json_policy_reloads() {
        let d = dir("json");
        std::fs::write(
            d.join("wanderbooks.toml"),
            "[books]\nreceipt_threshold = \"25.00\"\n",
        )
        .unwrap();
        let mut live = Live::watching(source(&d, true)).unwrap();
        let before = live.current().policy.rule("USD");
        std::fs::write(
            d.join("config.json"),
            r#"{"receipt_policy": {"require_receipts_above": "0.00"}}"#,
        )
        .unwrap();
        live.refresh();
        assert_ne!(live.current().policy.rule("USD"), before);
        assert!(live.status().error.is_none(), "{:?}", live.status().error);
    }

    /// Settings built in code never reload.
    #[test]
    fn fixed_settings_stay() {
        let mut live = Live::fixed(Settings::default());
        live.refresh();
        let s = live.status();
        assert_eq!((s.loads, s.books_toml), (1, None));
    }

    /// Books that keep their own settings: no file is read, even a broken
    /// one; their settings come in once per change (INVARIANT 1).
    #[test]
    fn journaled_books_take_their_own_settings_once_per_change() {
        let d = dir("journaled");
        std::fs::write(d.join("wanderbooks.toml"), "[books\n").unwrap();
        let mut live = Live::journaled(source(&d, true));
        assert!(live.current().journaled);
        assert!(live.is_journaled());
        assert!(live.file_texts().is_none());
        let mut books = ledger_core::settings::Settings::default();
        live.sync(&books, "USD");
        assert_eq!(live.status().loads, 0, "books without settings");
        let src = ledger_core::settings::Source {
            decided_by: "Owner".into(),
            decided_on: chrono::NaiveDate::from_ymd_opt(2026, 10, 17).unwrap(),
            citation: None,
            note: None,
        };
        let change = |name: &str| ledger_core::settings::Change {
            key: "company.name".into(),
            value: Some(serde_json::json!(name)),
        };
        books.apply(&[change("Kept Co")], &src, 7).unwrap();
        live.sync(&books, "USD");
        assert_eq!(live.current().company.name, "Kept Co");
        assert_eq!(live.status().loads, 1);
        live.sync(&books, "USD");
        assert_eq!(live.status().loads, 1, "unchanged: not taken again");
        books.apply(&[change("Kept Co 2")], &src, 9).unwrap();
        live.sync(&books, "USD");
        assert_eq!(live.current().company.name, "Kept Co 2");
        assert_eq!(live.status().loads, 2);
        assert!(live.current().journaled && live.is_journaled());
    }

    /// Settings built in code are the books' own once synced.
    #[test]
    fn fixed_settings_synced_are_journaled() {
        let mut live = Live::fixed(Settings::default());
        assert!(!live.is_journaled());
        let mut books = ledger_core::settings::Settings::default();
        let src = ledger_core::settings::Source {
            decided_by: "Owner".into(),
            decided_on: chrono::NaiveDate::from_ymd_opt(2026, 10, 17).unwrap(),
            citation: None,
            note: None,
        };
        books
            .apply(
                &[ledger_core::settings::Change {
                    key: "company.name".into(),
                    value: Some(serde_json::json!("Kept Co")),
                }],
                &src,
                3,
            )
            .unwrap();
        live.sync(&books, "USD");
        assert!(live.is_journaled());
    }

    /// Books in the data root keep their own settings from the start
    /// (INVARIANT 6): before they hold any, the settings in force say so
    /// too, so no message sends anyone to a settings file; none is read.
    #[test]
    fn books_in_the_data_root_keep_their_own_settings_before_they_hold_any() {
        let mut live = Live::own();
        assert!(live.current().journaled);
        assert!(live.is_journaled());
        assert!(live.file_texts().is_none());
        live.sync(&ledger_core::settings::Settings::default(), "USD");
        assert!(live.current().journaled);
        let s = live.status();
        assert!(s.journaled && s.books_toml.is_none(), "{s:?}");
    }
}
