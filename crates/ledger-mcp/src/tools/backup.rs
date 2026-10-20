//! `books.backup` and `books.back_up_now` (plan 3.7, D-30): the books'
//! backups as they stand, and one made now.
//!
//! ```text
//! GOAL: the agent can tell the human whether the books are backed up,
//!       and back them up now when asked.
//! SOURCE: plan 3.7 ("books.backup {status|run} reports it": two tools,
//!         one read-only), D-30.
//! INVARIANTS:
//!   1. Only books served from the data root are backed up: over stdio
//!      both tools are hidden, and refuse.
//!   2. A status reads the backup record only; a run makes and pushes a
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
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TargetOut {
    /// "repository" (the private repository) or "folder".
    pub target: String,
    /// Where it is.
    pub at: String,
    /// When it last took a backup (UTC), if ever.
    pub last_ok_at: Option<String>,
    /// The journal seq it holds, if any.
    pub through_seq: Option<u64>,
    /// More than a day without the books' latest: tell the human.
    pub stale: bool,
    /// Why its last backup failed, if it did.
    pub last_error: Option<String>,
    /// When its backup was last proven restorable (UTC), if ever: each
    /// is cloned and checked weekly.
    pub last_drill_at: Option<String>,
    /// Why that check failed, if it did: tell the human.
    pub drill_error: Option<String>,
    /// Not backed up there: these books were restored as a copy of others.
    pub paused: bool,
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
                    last_error: t.last_error,
                    last_drill_at: t.last_drill_at,
                    drill_error: t.drill_error,
                    paused: t.paused,
                })
                .collect(),
        }
    }
}

impl BooksServer {
    fn backups(&self) -> Result<&std::sync::Arc<crate::backup::Backups>, String> {
        self.backups.as_ref().ok_or_else(|| {
            "these books aren't backed up by this server: only books kept in the books' own private folder are".to_string()
        })
    }
}

#[tool_router(router = backup_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.backup",
        title = "Backups",
        description = "Whether the books are backed up: each backup target the settings name (backup.git_remote, a private repository; backup.folder, a folder on another disk), when it last took the books, through which journal seq, and why it failed if it did. stale: it lacks the latest, and the first try to back that up was more than a day ago: tell the human. The books back themselves up a little after each change, when they open and before they close; books.back_up_now makes one now.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn backup(&self, _params: Parameters<BackupParams>) -> Result<Json<BackupOut>, String> {
        Ok(Json(self.backups()?.status().await.into()))
    }

    #[tool(
        name = "books.back_up_now",
        title = "Back up now",
        description = "Back the books up now to every target the settings name, and answer how each went (as books.backup). new_disk: true only when the human says the backup folder's disk is new: the books' backup is then begun there again. Needs books:admin.",
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
        let b = self.backups()?;
        Ok(Json(b.run_with("asked", params.0.new_disk).await?.into()))
    }
}
