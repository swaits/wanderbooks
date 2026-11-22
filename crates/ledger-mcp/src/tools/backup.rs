//! `books.backup` and `books.back_up_now` (plan 3.7, D-91): the books'
//! backups as they stand, and one made now.
//!
//! ```text
//! GOAL: the agent can tell the human whether the books are backed up,
//!       and back them up now when asked.
//! SOURCE: plan 3.7 ("books.backup {status|run} reports it": two tools,
//!         one read-only), D-91.
//! INVARIANTS:
//!   1. A status reads the backup record only; a run makes and pushes a
//!      backup (backup.rs) and answers the status after it.
//! ```

use super::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct BackupParams {}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct BackUpNowParams {
    /// The backup folder's disk is new (or another): begin these books'
    /// backup there again. Only when the human says so: a folder that
    /// should hold one is otherwise a disk not connected.
    #[serde(default)]
    pub new_disk: bool,
    /// The backup folder holds a longer history of these books than they
    /// have now (a restore from an older backup): begin a new backup set
    /// beside the old one, which is never touched. Only when the human
    /// says so.
    #[serde(default)]
    pub new_set: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TargetOut {
    /// "folder".
    pub target: String,
    /// Where it is.
    pub at: String,
    /// When it last took a backup (UTC), if ever.
    pub last_ok_at: Option<String>,
    /// The journal seq it holds, if any.
    pub through_seq: Option<u64>,
    /// More than a day without the books' latest: tell the human.
    pub stale: bool,
    /// What kind of trouble the last backup or check had: `ahead` (the
    /// folder holds a longer history of these books: offer new_set),
    /// `elsewhere` (other or diverged books), `key_missing` (the backups
    /// are sealed and this computer lost the key: ask the human for the
    /// passphrase), `not_downloaded` (files a sync service has not put here
    /// yet), `newest_damaged` (a restore would give older books).
    pub error_kind: Option<String>,
    /// Files in the backup folder found damaged and written again (each is
    /// left where it is): tell the human.
    pub damaged_files: Vec<String>,
    /// Why its last backup failed, if it did.
    pub last_error: Option<String>,
    /// When its backup was last proven restorable (UTC), if ever: each
    /// is rebuilt and checked weekly.
    pub last_drill_at: Option<String>,
    /// Why that check failed, if it did: tell the human.
    pub drill_error: Option<String>,
    /// Protected by a passphrase.
    pub sealed: bool,
    /// Its disk isn't connected: tell the human to connect it.
    pub disconnected: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BackupOut {
    /// The books' journal seq now.
    pub head_seq: u64,
    /// Each backup the settings name.
    pub targets: Vec<TargetOut>,
    /// What to do, when none is set up.
    pub note: Option<String>,
}

impl From<crate::backup::Status> for BackupOut {
    fn from(s: crate::backup::Status) -> Self {
        BackupOut {
            head_seq: s.head_seq,
            note: s.note,
            targets: s
                .targets
                .into_iter()
                .map(|t| TargetOut {
                    target: t.target,
                    at: t.at,
                    last_ok_at: t.last_ok_at,
                    through_seq: t.through_seq,
                    stale: t.stale,
                    error_kind: t.error_kind,
                    damaged_files: t.damaged_files,
                    last_error: t.last_error,
                    last_drill_at: t.last_drill_at,
                    drill_error: t.drill_error,
                    sealed: t.sealed,
                    disconnected: t.disconnected,
                })
                .collect(),
        }
    }
}

#[tool_router(router = backup_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.backup",
        title = "Backups",
        description = "Whether the books are backed up: the backup folder the settings name (backup.folder: a folder on another disk, or one a sync service keeps), when it last took the books, through which journal seq, when it was last proven restorable, and why it failed if it did. stale: it lacks the latest, and the first try to back that up was more than a day ago: tell the human. disconnected: its disk isn't there. The books back themselves up a little after each change, when they open and before they close; books.back_up_now makes one now.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn backup(&self, _params: Parameters<BackupParams>) -> Result<Json<BackupOut>, String> {
        Ok(Json(self.backups.status().await.into()))
    }

    #[tool(
        name = "books.back_up_now",
        title = "Back up now",
        description = "Back the books up now to the backup folder the settings name, and answer how it went (as books.backup). new_disk: true only when the human says the backup folder's disk is new: the books' backup is then begun there again. new_set: true only when the human says so after a backup failed because the folder holds a longer history of these books (error_kind ahead): a new set begins beside the old one. Needs books:admin.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn back_up_now(
        &self,
        params: Parameters<BackUpNowParams>,
    ) -> Result<Json<BackupOut>, String> {
        // Stopped by a close (backup.rs INVARIANT 11): said so, and on
        // record as stopped, not as an answer it gave.
        match self
            .backups
            .run_with_set("asked", params.0.new_disk, params.0.new_set)
            .await
        {
            Ok(status) => Ok(Json(status.into())),
            Err(why) if why.starts_with(crate::server::STOPPED_BY_CLOSE_WORDS) => {
                crate::server::stopped_by_close();
                Err(why)
            }
            Err(why) => Err(why),
        }
    }
}
