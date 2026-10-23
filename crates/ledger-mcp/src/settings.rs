//! The server's settings: the books' own (journaled, ledger_core::
//! settings), the defaults until the first is set.
//!
//! ```text
//! GOAL: every request sees the settings in force, one version for the
//!       whole request, and they are only ever the books' own.
//! SOURCE: plan/served.md I-3 (the journal holds all books state; the
//!         server never reads a settings file), 3.4 (a book in the data
//!         root keeps its own settings), D-44 (one way to serve books: no
//!         settings file is read by any of them).
//! INVARIANTS:
//!   1. The settings in force are the books' own: the defaults until
//!      their journal holds any settings.set, then those; sync takes them
//!      whenever the journal's last settings.set changes, and only then.
//!   2. No settings file is ever read.
//! ```

use crate::policy::ReceiptPolicy;
use ledger_config::Company;
use std::sync::Arc;

/// `[books]` account roles for the cash flow statement, used when a call
/// doesn't pass its own: cash, financing, and investing account codes.
#[derive(Clone, Debug, Default)]
pub struct CashRoles {
    /// `books.cash_accounts`.
    pub cash: Vec<String>,
    /// `books.financing_accounts`.
    pub financing: Vec<String>,
    /// `books.investing_accounts`.
    pub investing: Vec<String>,
}

/// The settings in force, as the server uses them.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    /// Which entries need a receipt.
    pub policy: ReceiptPolicy,
    /// The company (`company.*`).
    pub company: Company,
    /// The cash flow roles.
    pub roles: CashRoles,
    /// The prefix of generated asset tags (`books.asset_tag_prefix`, or
    /// the company's initials).
    pub asset_tag_prefix: String,
    /// `policy.parties`: suggest, require, or never mention each entry's
    /// counterparty.
    pub parties: ledger_config::PartyPolicy,
    /// The company's time zone (an IANA name), if set: what "today" means.
    pub timezone: Option<String>,
}

impl Settings {
    /// The settings the books keep (`ledger_core::settings`), as the
    /// server uses them.
    pub fn from_books(books: &ledger_core::settings::Settings, currency: &str) -> Self {
        let b = ledger_config::codec::books_settings(books, currency);
        Settings {
            policy: ledger_config::codec::receipt_policy(books),
            asset_tag_prefix: b.asset_tag_prefix(),
            parties: b.parties,
            timezone: b.timezone.clone(),
            roles: CashRoles {
                cash: b.cash_accounts,
                financing: b.financing_accounts,
                investing: b.investing_accounts,
            },
            company: b.company,
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

/// The settings in force, and the books' settings.set they came from.
#[derive(Debug)]
pub struct Live {
    current: Arc<Settings>,
    /// The seq of the books' last settings.set taken (INVARIANT 1).
    journal_seq: Option<u64>,
}

impl Live {
    /// For a book (INVARIANT 1): the defaults; `sync` brings the books'
    /// settings in once they hold any.
    pub fn own() -> Self {
        Self {
            current: Arc::new(Settings::default()),
            journal_seq: None,
        }
    }

    /// Take the books' own settings when they hold any and they changed
    /// since the last look (INVARIANT 1). The server calls this whenever
    /// it holds the books.
    pub fn sync(&mut self, books: &ledger_core::settings::Settings, currency: &str) {
        if !books.is_journaled() || books.last_seq() == self.journal_seq {
            return;
        }
        self.current = Arc::new(Settings::from_books(books, currency));
        self.journal_seq = books.last_seq();
    }

    /// The settings in force.
    pub fn current(&self) -> Arc<Settings> {
        self.current.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// INVARIANT 1: books that hold no settings keep the defaults; their
    /// settings come in once per change, and an unchanged journal leaves
    /// the settings in force as they are.
    #[test]
    fn books_take_their_own_settings_once_per_change() {
        let mut live = Live::own();
        assert_eq!(live.current().company.name, Company::default().name);
        let mut books = ledger_core::settings::Settings::default();
        let before = live.current();
        live.sync(&books, "USD");
        assert!(
            Arc::ptr_eq(&before, &live.current()),
            "books without settings"
        );
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
        let taken = live.current();
        live.sync(&books, "USD");
        assert!(
            Arc::ptr_eq(&taken, &live.current()),
            "unchanged: not taken again"
        );
        books.apply(&[change("Kept Co 2")], &src, 9).unwrap();
        live.sync(&books, "USD");
        assert_eq!(live.current().company.name, "Kept Co 2");
    }
}
