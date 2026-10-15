//! Telling a running viewer what to show.
//!
//! The agent can ask the viewer (`ledger-gui`) to open an entry, an
//! account, a report, a document, or a search. That's not bookkeeping, so
//! it never touches the books: the request is a small JSON file outside the
//! data directory, in the user's runtime directory, named for the books it
//! belongs to. The viewer watches it and navigates; the server writes it.
//! A request carries a sequence number so the same one never applies
//! twice, and a time so a viewer started later ignores stale ones.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where to go.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "show", rename_all = "snake_case")]
pub enum ViewerTarget {
    /// A top-level page: overview, cash, profit, transactions, reports,
    /// accounts, assets, parties, documents, company, journal.
    Page {
        /// Page name.
        page: String,
    },
    /// One entry, with its lines, receipt, and history.
    Entry {
        /// Entry id.
        entry_id: u64,
    },
    /// An account's register, optionally for a date range.
    Account {
        /// Account code.
        code: String,
        /// First date, YYYY-MM-DD.
        start: Option<String>,
        /// Last date, YYYY-MM-DD.
        end: Option<String>,
    },
    /// A report, optionally for a date range.
    Report {
        /// profit_and_loss, balance_sheet, cash_flow, members_equity,
        /// trial_balance, general_ledger, tax_summary, vendor_1099,
        /// ar_aging, ap_aging, fixed_asset_register.
        report: String,
        /// First date, YYYY-MM-DD.
        start: Option<String>,
        /// Last date, YYYY-MM-DD.
        end: Option<String>,
        /// Split into columns: total, month, quarter, or year.
        #[serde(default)]
        columns: Option<String>,
        /// Compare with: prior_period or prior_year.
        #[serde(default)]
        compare: Option<String>,
    },
    /// A stored receipt or document.
    Document {
        /// Attachment id.
        attachment_id: u64,
    },
    /// One fixed asset: its identity, schedule, entries, and documents.
    Asset {
        /// Asset id.
        asset_id: u64,
    },
    /// One party: who they are, their roles, and everything the company
    /// did with them.
    Party {
        /// Party id.
        party_id: u64,
    },
    /// One reconciliation: its account, with the account's statements and
    /// reconciliations.
    Reconciliation {
        /// Reconciliation id.
        id: u64,
    },
    /// The transactions list, searched.
    Search {
        /// Search text: description, account, vendor, amount.
        query: String,
    },
}

/// A request for the viewer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerCommand {
    /// Increases with every request.
    pub seq: u64,
    /// When it was made, RFC 3339.
    pub at: String,
    /// Where to go.
    pub target: ViewerTarget,
}

/// The command file for the books in `data_dir`: in `$XDG_RUNTIME_DIR`
/// (or the temp dir), under `wanderbooks/`, named by a hash of the data
/// directory's canonical path.
pub fn command_path(data_dir: &Path) -> PathBuf {
    let canonical = data_dir
        .canonicalize()
        .unwrap_or_else(|_| data_dir.to_path_buf());
    let key = crate::journal::sha256_hex(canonical.to_string_lossy().as_bytes());
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("wanderbooks")
        .join(format!("viewer-{}.json", &key[..16]))
}

/// Ask the viewer for `data_dir` to show `target`. Atomic (write then
/// rename), so the viewer never reads half a request.
pub fn request(data_dir: &Path, target: ViewerTarget) -> std::io::Result<ViewerCommand> {
    let path = command_path(data_dir);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let seq = read(data_dir).map_or(1, |c| c.seq + 1);
    let cmd = ViewerCommand {
        seq,
        at: chrono::Utc::now().to_rfc3339(),
        target,
    };
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec(&cmd).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, &path)?;
    Ok(cmd)
}

/// The latest request for `data_dir`, if any.
pub fn read(data_dir: &Path) -> Option<ViewerCommand> {
    let bytes = std::fs::read(command_path(data_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_and_count_up() {
        let dir = std::env::temp_dir().join(format!("wb-viewer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = request(&dir, ViewerTarget::Entry { entry_id: 7 }).unwrap();
        let b = request(
            &dir,
            ViewerTarget::Search {
                query: "adobe".into(),
            },
        )
        .unwrap();
        assert_eq!(b.seq, a.seq + 1);
        assert_eq!(
            read(&dir).unwrap().target,
            ViewerTarget::Search {
                query: "adobe".into()
            }
        );
        // Same books, same file; other books, another.
        assert_eq!(command_path(&dir), command_path(&dir.join(".")));
        assert_ne!(command_path(&dir), command_path(&std::env::temp_dir()));
        let _ = std::fs::remove_file(command_path(&dir));
    }
}
