//! The viewer's side channel (plan 3.6): what the agent asks the viewer
//! to show (`books.show`), and whether a viewer has the books open. Not
//! bookkeeping, so never journaled, but kept with the books, in their own
//! `ops/` folder: the server writes; the viewer, which never writes, reads.
//!
//! ```text
//! GOAL: the agent can move a viewer that has the books open, and know
//!       whether one has, without the viewer writing anything.
//! SOURCE: plan 3.6 ("the server appends {seq, ts, target} to
//!         ops/show.jsonl (bounded, rotated, not journaled)"; "the GUI holds
//!         a shared flock on ops/viewer.lock, opened read-only. The server
//!         probes the lock to report viewer.open").
//! INVARIANTS:
//!   1. A show request is a line of `ops/show.jsonl`, `{seq, at, target}`,
//!      its seq one more than the last; the file holds at most KEPT lines
//!      and is replaced whole (written aside, renamed), so a reader sees
//!      the old file or the new one, each line whole.
//!   2. `ops/` (0700) and `ops/viewer.lock` (0600) are made by the server
//!      (`prepare`), never by the viewer; a request only navigates, so a
//!      stray writer can do no more than move the view.
//!   3. A viewer is open while some process holds a shared lock on
//!      `ops/viewer.lock`; the viewer takes it through a read-only
//!      descriptor, and `viewer_open` probes it without taking it.
//! ```

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The books' side-channel folder, in their own folder.
pub const OPS_DIR: &str = "ops";
/// The show requests (INVARIANT 1).
pub const SHOW_FILE: &str = "show.jsonl";
/// The viewer's presence lock (INVARIANT 3).
pub const VIEWER_LOCK: &str = "viewer.lock";
/// The most show requests kept.
pub const KEPT: usize = 32;

/// The most columns the viewer splits a report into: past it, the
/// viewer shows the report in one total column and says why, so
/// `books.show` refuses such a split (it would be an option the viewer
/// ignores).
pub const MAX_REPORT_COLUMNS: i64 = 240;

/// How many columns splitting `start..=end` into periods of
/// `months_per_column` months makes (each started period a column),
/// counted from the months, before building any: a range across chrono's
/// calendar is millions of months. The one count `books.show` and the
/// viewer share.
pub fn report_columns(
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
    months_per_column: u32,
) -> i64 {
    use chrono::Datelike;
    let months = (i64::from(end.year()) - i64::from(start.year())) * 12 + i64::from(end.month())
        - i64::from(start.month());
    months / i64::from(months_per_column.max(1)) + 1
}

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
    /// An account's register, for a date range.
    Account {
        /// Account code.
        code: String,
        /// First date, YYYY-MM-DD. `books.show` always sends both dates,
        /// resolved (rules.md B-30); `None` only from an earlier writer,
        /// which the viewer resolves by the same rule.
        start: Option<String>,
        /// Last date, YYYY-MM-DD (see `start`).
        end: Option<String>,
    },
    /// A report, for a date range.
    Report {
        /// profit_and_loss, balance_sheet, cash_flow, members_equity,
        /// trial_balance, general_ledger, tax_summary, vendor_1099,
        /// ar_aging, ap_aging, fixed_asset_register.
        report: String,
        /// First date, YYYY-MM-DD. `books.show` always sends both dates,
        /// resolved (rules.md B-30); `None` only from an earlier writer,
        /// which the viewer resolves by the same rule.
        start: Option<String>,
        /// Last date, YYYY-MM-DD (see `start`): the as-of day of a report
        /// as of a day.
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

/// The `ops/` folder of the books in `data_dir`.
pub fn ops_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(OPS_DIR)
}

/// Make the books' `ops/` folder and the viewer's lock (INVARIANT 2): the
/// server does, when it opens the books and before it writes a request.
pub fn prepare(data_dir: &Path) -> std::io::Result<()> {
    let dir = crate::root::private_book_folder(data_dir, OPS_DIR)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let lock = dir.join(VIEWER_LOCK);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(&lock)?;
    // Not the books: kept out of a books dir's version history (an older
    // books dir lives in git), by a rule in the folder itself.
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, "# The viewer's side channel: not the books.\n*\n")?;
    }
    Ok(())
}

/// Every request kept, oldest first; lines that aren't whole requests are
/// skipped.
pub fn requests(data_dir: &Path) -> Vec<ViewerCommand> {
    std::fs::read_to_string(ops_dir(data_dir).join(SHOW_FILE))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// The latest request for the books in `data_dir`, if any.
pub fn read(data_dir: &Path) -> Option<ViewerCommand> {
    requests(data_dir).pop()
}

/// Ask the viewer of the books in `data_dir` to show `target`
/// (INVARIANT 1).
pub fn request(data_dir: &Path, target: ViewerTarget) -> std::io::Result<ViewerCommand> {
    prepare(data_dir)?;
    let mut kept = requests(data_dir);
    let cmd = ViewerCommand {
        seq: kept.last().map_or(1, |c| c.seq + 1),
        at: chrono::Utc::now().to_rfc3339(),
        target,
    };
    kept.push(cmd.clone());
    let from = kept.len().saturating_sub(KEPT);
    let mut text = String::new();
    for c in &kept[from..] {
        text.push_str(&serde_json::to_string(c).map_err(std::io::Error::other)?);
        text.push('\n');
    }
    let dir = ops_dir(data_dir);
    let aside = dir.join(format!(".{SHOW_FILE}.new"));
    let _ = std::fs::remove_file(&aside);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(&aside)?;
    f.write_all(text.as_bytes())?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&aside, dir.join(SHOW_FILE))?;
    Ok(cmd)
}

/// A viewer's presence: a shared lock on `ops/viewer.lock`, through a
/// read-only descriptor, held while this lives (INVARIANT 3).
#[derive(Debug)]
pub struct Presence(#[allow(dead_code)] std::fs::File);

impl Presence {
    /// Take it for the books in `data_dir`, if the server made the lock
    /// (None until it has: the viewer makes nothing).
    pub fn hold(data_dir: &Path) -> Option<Presence> {
        let f = std::fs::File::open(ops_dir(data_dir).join(VIEWER_LOCK)).ok()?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: flock on a descriptor this function owns.
            if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
                return None;
            }
        }
        Some(Presence(f))
    }
}

/// Whether a viewer has the books in `data_dir` open (INVARIANT 3): some
/// process holds a shared lock on their `ops/viewer.lock`.
pub fn viewer_open(data_dir: &Path) -> bool {
    let Ok(f) = std::fs::File::open(ops_dir(data_dir).join(VIEWER_LOCK)) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: flock on a descriptor this function owns; a lock taken
        // is let go at once (and with the descriptor anyway).
        let free = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
        if free {
            // SAFETY: as above.
            unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_UN) };
        }
        !free
    }
    #[cfg(not(unix))]
    {
        drop(f);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_count_up_and_only_so_many_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()), None);
        let a = request(dir.path(), ViewerTarget::Entry { entry_id: 7 }).unwrap();
        let b = request(
            dir.path(),
            ViewerTarget::Search {
                query: "adobe".into(),
            },
        )
        .unwrap();
        assert_eq!((a.seq, b.seq), (1, 2));
        assert_eq!(read(dir.path()), Some(b));
        for n in 0..(KEPT as u64 + 5) {
            request(dir.path(), ViewerTarget::Entry { entry_id: n }).unwrap();
        }
        let kept = requests(dir.path());
        assert_eq!(kept.len(), KEPT);
        assert_eq!(kept.last().unwrap().seq, 2 + KEPT as u64 + 5);
        assert!(kept.windows(2).all(|w| w[1].seq == w[0].seq + 1));
        // A torn line (an old writer's) is skipped, never the whole file.
        let file = ops_dir(dir.path()).join(SHOW_FILE);
        let mut text = std::fs::read_to_string(&file).unwrap();
        text.push_str("{\"seq\": 99, \"at\"");
        std::fs::write(&file, text).unwrap();
        assert_eq!(requests(dir.path()).len(), KEPT);
    }

    #[cfg(unix)]
    #[test]
    fn the_server_makes_the_folder_and_the_viewer_only_reads_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        // Nothing made: no presence, and taking it makes nothing.
        assert!(Presence::hold(dir.path()).is_none());
        assert!(!ops_dir(dir.path()).exists());
        assert!(!viewer_open(dir.path()));
        prepare(dir.path()).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&ops_dir(dir.path())), 0o700);
        assert_eq!(mode(&ops_dir(dir.path()).join(VIEWER_LOCK)), 0o600);
        assert!(!viewer_open(dir.path()));
        let held = Presence::hold(dir.path()).unwrap();
        assert!(viewer_open(dir.path()));
        // Two viewers at once share it.
        let second = Presence::hold(dir.path()).unwrap();
        drop(held);
        assert!(viewer_open(dir.path()));
        drop(second);
        assert!(!viewer_open(dir.path()));
        // Made again, nothing changes.
        prepare(dir.path()).unwrap();
        assert!(!viewer_open(dir.path()));
        // Kept out of a books dir's version history by a rule of its own.
        let rule = std::fs::read_to_string(ops_dir(dir.path()).join(".gitignore")).unwrap();
        assert!(rule.lines().any(|l| l == "*"), "{rule}");
    }
}
