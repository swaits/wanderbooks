//! The registry: the one small durable table (D-85).
//!
//! It holds, per name, the app's public key, when the name was claimed
//! and whether the operator has blocked it. Nothing else about an app is
//! kept anywhere: not an address, not a label, not a log of connections.
//! A claim, and the limit on new names, are one transaction.
//!
//! SQLite is driven from one thread of its own, through a bounded queue,
//! so a slow disk makes the relay say "busy" and never makes it hold more.
//!
//! ```text
//! GOAL: a name belongs to the first key that claims it and to no other,
//!       for as long as the registry file lives, however many connections
//!       race for it and however often the relay restarts.
//! SOURCE: WB-relay-4 (the registry), D-85 in plan/p15-app-and-relay.md.
//! INVARIANTS:
//!   1. A name has at most one key: a second claim, with the same key or
//!      another, never changes the row.
//!   2. A claim is durable before it is acknowledged (synchronous = FULL).
//!   3. At most `max_new` names are claimed in any `window`, counted from
//!      the rows' own creation times, so a restart does not reset it.
//!   4. The queue to the database thread is bounded: past it a request
//!      is refused as busy, not held.
//! SCENARIOS:
//!   1. Two claims of one name with different keys: one wins, the other
//!      sees `Exists`.
//!   2. A claim, a restart (reopen), a lookup: the key is still there.
//!   3. The window limit counts rows made before a reopen.
//! ```

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::{mpsc, oneshot};

use crate::names::Name;

/// What the registry holds for a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The app's Ed25519 public key.
    pub public_key: [u8; 32],
    /// True when the operator has blocked the name.
    pub blocked: bool,
}

/// How a claim ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claimed {
    /// This claim made the row.
    New,
    /// The name was already held, by this entry.
    Exists(Entry),
    /// Too many names were claimed in the window.
    LimitReached,
}

/// Why the registry could not answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// The queue to the database is full.
    #[error("the registry is busy")]
    Busy,
    /// The database thread is gone.
    #[error("the registry is closed")]
    Closed,
    /// The database refused.
    #[error("registry database: {0}")]
    Db(String),
}

type Reply<T> = oneshot::Sender<Result<T, RegistryError>>;

enum Cmd {
    Lookup(String, Reply<Option<Entry>>),
    Claim {
        name: String,
        key: [u8; 32],
        now: i64,
        window_secs: i64,
        max_new: u32,
        reply: Reply<Claimed>,
    },
    SetBlocked(String, bool, Reply<bool>),
    Count(Reply<u64>),
}

/// A handle to the registry. Clones share one database thread.
#[derive(Clone)]
pub struct Registry {
    tx: mpsc::Sender<Cmd>,
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn db(e: rusqlite::Error) -> RegistryError {
    RegistryError::Db(e.to_string())
}

impl Registry {
    /// Open (or create) the registry file, and start its thread. `queue`
    /// is how many requests may wait.
    pub fn open(path: &Path, queue: usize) -> Result<Self, RegistryError> {
        let conn = Connection::open(path).map_err(db)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS apps (
                 name       TEXT PRIMARY KEY,
                 public_key BLOB NOT NULL CHECK (length(public_key) = 32),
                 created    INTEGER NOT NULL,
                 blocked    INTEGER NOT NULL DEFAULT 0
             ) WITHOUT ROWID;
             CREATE INDEX IF NOT EXISTS apps_created ON apps (created);",
        )
        .map_err(db)?;
        let (tx, mut rx) = mpsc::channel::<Cmd>(queue.max(1));
        std::thread::Builder::new()
            .name("relay-registry".into())
            .spawn(move || {
                while let Some(cmd) = rx.blocking_recv() {
                    serve(&conn, cmd);
                }
            })
            .map_err(|e| RegistryError::Db(e.to_string()))?;
        Ok(Registry { tx })
    }

    async fn ask<T>(&self, make: impl FnOnce(Reply<T>) -> Cmd) -> Result<T, RegistryError> {
        let (reply, answer) = oneshot::channel();
        self.tx.try_send(make(reply)).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => RegistryError::Busy,
            mpsc::error::TrySendError::Closed(_) => RegistryError::Closed,
        })?;
        answer.await.map_err(|_| RegistryError::Closed)?
    }

    /// The entry for a name, if claimed.
    pub async fn lookup(&self, name: &Name) -> Result<Option<Entry>, RegistryError> {
        self.ask(|r| Cmd::Lookup(name.to_string(), r)).await
    }

    /// Claim a name for a key, unless more than `max_new` names were
    /// claimed within `window` already.
    pub async fn claim(
        &self,
        name: &Name,
        key: [u8; 32],
        window: std::time::Duration,
        max_new: u32,
    ) -> Result<Claimed, RegistryError> {
        self.ask(|reply| Cmd::Claim {
            name: name.to_string(),
            key,
            now: unix_now(),
            window_secs: i64::try_from(window.as_secs()).unwrap_or(i64::MAX),
            max_new,
            reply,
        })
        .await
    }

    /// Block or unblock a name; false when there is no such name.
    pub async fn set_blocked(&self, name: &Name, blocked: bool) -> Result<bool, RegistryError> {
        self.ask(|r| Cmd::SetBlocked(name.to_string(), blocked, r))
            .await
    }

    /// How many names are claimed.
    pub async fn count(&self) -> Result<u64, RegistryError> {
        self.ask(Cmd::Count).await
    }
}

fn entry_of(key: Vec<u8>, blocked: i64) -> Option<Entry> {
    Some(Entry {
        public_key: key.try_into().ok()?,
        blocked: blocked != 0,
    })
}

fn lookup(conn: &Connection, name: &str) -> Result<Option<Entry>, RegistryError> {
    let row: Option<(Vec<u8>, i64)> = conn
        .query_row(
            "SELECT public_key, blocked FROM apps WHERE name = ?1",
            params![name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(db)?;
    match row {
        None => Ok(None),
        Some((key, blocked)) => entry_of(key, blocked)
            .map(Some)
            .ok_or_else(|| RegistryError::Db("a stored key is not 32 bytes".into())),
    }
}

fn serve(conn: &Connection, cmd: Cmd) {
    match cmd {
        Cmd::Lookup(name, reply) => {
            let _ = reply.send(lookup(conn, &name));
        }
        Cmd::Count(reply) => {
            let n = conn
                .query_row("SELECT count(*) FROM apps", [], |r| r.get::<_, i64>(0))
                .map(|n| u64::try_from(n).unwrap_or(0))
                .map_err(db);
            let _ = reply.send(n);
        }
        Cmd::SetBlocked(name, blocked, reply) => {
            let r = conn
                .execute(
                    "UPDATE apps SET blocked = ?2 WHERE name = ?1",
                    params![name, i64::from(blocked)],
                )
                .map(|n| n > 0)
                .map_err(db);
            let _ = reply.send(r);
        }
        Cmd::Claim {
            name,
            key,
            now,
            window_secs,
            max_new,
            reply,
        } => {
            let _ = reply.send(claim(conn, &name, key, now, window_secs, max_new));
        }
    }
}

fn claim(
    conn: &Connection,
    name: &str,
    key: [u8; 32],
    now: i64,
    window_secs: i64,
    max_new: u32,
) -> Result<Claimed, RegistryError> {
    // One immediate transaction: the count and the insert are one step.
    conn.execute_batch("BEGIN IMMEDIATE").map_err(db)?;
    let result = (|| {
        if let Some(e) = lookup(conn, name)? {
            return Ok(Claimed::Exists(e));
        }
        let recent: i64 = conn
            .query_row(
                "SELECT count(*) FROM apps WHERE created > ?1",
                params![now.saturating_sub(window_secs)],
                |r| r.get(0),
            )
            .map_err(db)?;
        if recent >= i64::from(max_new) {
            return Ok(Claimed::LimitReached);
        }
        conn.execute(
            "INSERT INTO apps (name, public_key, created, blocked) VALUES (?1, ?2, ?3, 0)",
            params![name, key.as_slice(), now],
        )
        .map_err(db)?;
        Ok(Claimed::New)
    })();
    match result {
        Ok(c) => {
            conn.execute_batch("COMMIT").map_err(db)?;
            Ok(c)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const DAY: Duration = Duration::from_secs(86_400);

    fn name(s: &str) -> Name {
        Name::parse(s).unwrap()
    }

    #[tokio::test]
    async fn the_first_claim_wins_and_the_row_never_changes() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("r.db"), 16).unwrap();
        let n = name("ledger-accrual-tally-mint");
        assert_eq!(reg.lookup(&n).await.unwrap(), None);
        assert_eq!(reg.claim(&n, [1; 32], DAY, 10).await.unwrap(), Claimed::New);
        let held = Entry {
            public_key: [1; 32],
            blocked: false,
        };
        assert_eq!(
            reg.claim(&n, [2; 32], DAY, 10).await.unwrap(),
            Claimed::Exists(held)
        );
        assert_eq!(
            reg.claim(&n, [1; 32], DAY, 10).await.unwrap(),
            Claimed::Exists(held)
        );
        assert_eq!(reg.lookup(&n).await.unwrap(), Some(held));
        assert_eq!(reg.count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn racing_claims_have_one_winner() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("r.db"), 64).unwrap();
        let n = name("ledger-accrual-tally-mint");
        let mut tasks = Vec::new();
        for i in 0..20u8 {
            let reg = reg.clone();
            tasks.push(tokio::spawn(async move {
                reg.claim(&n, [i; 32], DAY, 100).await.unwrap()
            }));
        }
        let mut wins = 0;
        for t in tasks {
            if t.await.unwrap() == Claimed::New {
                wins += 1;
            }
        }
        assert_eq!(wins, 1);
    }

    #[tokio::test]
    async fn a_claim_outlives_a_restart_and_the_window_counts_old_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.db");
        {
            let reg = Registry::open(&path, 16).unwrap();
            for w in ["ledger-accrual-tally-mint", "ledger-accrual-tally-vault"] {
                assert_eq!(
                    reg.claim(&name(w), [7; 32], DAY, 2).await.unwrap(),
                    Claimed::New
                );
            }
        }
        let reg = Registry::open(&path, 16).unwrap();
        let held = reg
            .lookup(&name("ledger-accrual-tally-mint"))
            .await
            .unwrap();
        assert_eq!(held.map(|e| e.public_key), Some([7; 32]));
        assert_eq!(
            reg.claim(&name("ledger-accrual-tally-audit"), [8; 32], DAY, 2)
                .await
                .unwrap(),
            Claimed::LimitReached
        );
        // The limit is for new names only: an existing one still answers.
        assert!(matches!(
            reg.claim(&name("ledger-accrual-tally-mint"), [8; 32], DAY, 2)
                .await
                .unwrap(),
            Claimed::Exists(_)
        ));
        // A window that has passed counts nothing.
        assert_eq!(
            reg.claim(
                &name("ledger-accrual-tally-audit"),
                [8; 32],
                Duration::from_secs(0),
                2
            )
            .await
            .unwrap(),
            Claimed::New
        );
    }

    #[tokio::test]
    async fn blocking_a_name_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::open(&dir.path().join("r.db"), 16).unwrap();
        let n = name("ledger-accrual-tally-mint");
        assert!(!reg.set_blocked(&n, true).await.unwrap(), "no such name");
        reg.claim(&n, [1; 32], DAY, 10).await.unwrap();
        assert!(reg.set_blocked(&n, true).await.unwrap());
        assert!(reg.lookup(&n).await.unwrap().unwrap().blocked);
    }
}
