//! The server's settings (`wanderbooks.toml`, and the receipt policy in
//! `books-data/config.json`), reloaded whenever their files change.
//!
//! ```text
//! GOAL: an edit to wanderbooks.toml takes effect on the next tool call,
//!       with no restart, and a broken edit never takes the server down.
//! SOURCE: DECISION (Stephen, 2026-10-15): the configuration is always
//!         auto-reloaded.
//! INVARIANTS:
//!   1. Every request (tool call, resource read) sees the files' contents
//!      as of its start, one version for the whole request: a request after
//!      an edit sees the edit (compared by content, not by timestamp, so two
//!      edits in one second are both seen).
//!   2. A file that fails to load leaves the last good settings in force,
//!      and the failure is reported (books.settings, stderr) until a later
//!      edit loads.
//!   3. A settings file that doesn't exist and wasn't asked for (the
//!      default wanderbooks.toml beside the books) means the defaults; one
//!      named with --books-toml that goes missing is a failure (2).
//!   4. Only the settings reload: the data dir and the books' base currency
//!      are fixed when the server starts.
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
}

impl Settings {
    /// The vault key file in force: `[vault] key_file`, else the default
    /// outside the books (`~/.config/wanderbooks/vault.key`).
    pub fn key_file(&self) -> PathBuf {
        self.vault_key_file
            .clone()
            .unwrap_or_else(ledger_vault::default_key_file)
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
        })
    }

    /// The settings in force.
    pub fn current(&self) -> Arc<Settings> {
        self.current.clone()
    }

    /// Reload if a file changed since the last look: the server calls this
    /// once at the start of every request, so the whole request sees one
    /// version.
    pub fn refresh(&mut self) {
        if let Some(source) = &self.source {
            let now = source.read();
            if now != self.seen {
                self.seen = now;
                match source.load() {
                    Ok(s) => {
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
        Status {
            books_toml: self.source.as_ref().map(|s| s.books_toml.clone()),
            books_toml_exists: self.source.as_ref().is_some_and(|s| s.books_toml.exists()),
            config_json_present: self.source.as_ref().is_some_and(|s| s.config_json.exists()),
            error: self.error.clone(),
            loads: self.loads,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("ledger-mcp-settings-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
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
}
