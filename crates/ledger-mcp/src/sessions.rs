//! Who is connected to a book right now: the in-memory board of one
//! book's live MCP sessions (plan/p15-app-and-relay.md 3.1, D-77).
//!
//! ```text
//! GOAL: the app can say, for each open book, which assistants are
//!       connected and what each is doing, without reading the access log
//!       back and without ever holding a token.
//! SOURCE: plan/p15-app-and-relay.md 3.1 (the sessions request: client name
//!         and version, the token's name, in-flight calls, the last tool
//!         and when); D-77 (the app is the host); D-78 (one tab per
//!         company: a connection names its company).
//! INVARIANTS:
//!   1. The board holds what the client and the agent SAID of themselves
//!      (asserted, never checked), each masked of any full TIN and cut to
//!      the access log's own limit (ledger_store::access::asserted); never
//!      a token, its text or its hash.
//!   2. A call is counted as running from the moment it is made until its
//!      blocking work and its record are done, even if its client hung
//!      up (the count rides in the call, as the store's own does).
//!   3. A session that ended is not listed: the board is read only for
//!      the sessions the server still binds (local.rs `session_tokens`),
//!      and a note for another is dropped.
//!   4. Nothing here is durable: a restarted server knows no session, as
//!      its clients know theirs ended.
//! ```

use ledger_store::access::{Agent, Client, Outcome, asserted};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// What one session has said and done.
#[derive(Clone, Debug, Default)]
pub(crate) struct Note {
    /// The client program, as it named itself at `initialize`.
    pub client: Option<Client>,
    /// What the agent said about itself, the latest time it did.
    pub agent: Option<Agent>,
    /// Calls running now (INVARIANT 2).
    pub running: usize,
    /// The last call that finished: its tool (or resource), what came of
    /// it, and when (seconds since the epoch, UTC).
    pub last: Option<Last>,
}

/// A call that finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Last {
    /// The tool's name, or the resource's address.
    pub subject: String,
    /// What came of it.
    pub outcome: Outcome,
    /// When (seconds since the epoch, UTC).
    pub at: i64,
}

/// One book's live sessions' notes, by MCP session id.
#[derive(Default)]
pub(crate) struct Board {
    notes: Mutex<HashMap<String, Note>>,
}

/// A call running (INVARIANT 2): counted while it lives.
pub(crate) struct Live {
    board: Arc<Board>,
    sid: String,
}

impl Drop for Live {
    fn drop(&mut self) {
        let mut notes = self.board.notes();
        if let Some(note) = notes.get_mut(&self.sid) {
            note.running = note.running.saturating_sub(1);
        }
    }
}

fn client_of(client: Client) -> Client {
    Client {
        name: asserted(&client.name),
        version: asserted(&client.version),
    }
}

fn agent_of(agent: Agent) -> Agent {
    Agent {
        model: agent.model.as_deref().map(asserted),
        reasoning_effort: agent.reasoning_effort.as_deref().map(asserted),
        harness: agent.harness.as_deref().map(asserted),
    }
}

impl Board {
    fn notes(&self) -> std::sync::MutexGuard<'_, HashMap<String, Note>> {
        self.notes.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The session `sid` said who its client is (`initialize`).
    pub fn initialized(&self, sid: &str, client: Client) {
        self.notes().entry(sid.to_string()).or_default().client = Some(client_of(client));
    }

    /// A call of `sid` began (INVARIANT 2).
    pub fn begin(self: &Arc<Self>, sid: &str) -> Live {
        self.notes().entry(sid.to_string()).or_default().running += 1;
        Live {
            board: self.clone(),
            sid: sid.to_string(),
        }
    }

    /// A call of `sid` is done and recorded: what it was, what came of
    /// it, and what its client and agent said, if they did.
    pub fn finished(
        &self,
        sid: &str,
        subject: &str,
        outcome: &Outcome,
        client: Option<&Client>,
        agent: Option<&Agent>,
        at: i64,
    ) {
        let mut notes = self.notes();
        let note = notes.entry(sid.to_string()).or_default();
        if let Some(client) = client {
            note.client = Some(client_of(client.clone()));
        }
        if let Some(agent) = agent {
            note.agent = Some(agent_of(agent.clone()));
        }
        note.last = Some(Last {
            subject: asserted(subject),
            outcome: outcome.clone(),
            at,
        });
    }

    /// The session `sid` ended.
    pub fn forget(&self, sid: &str) {
        self.notes().remove(sid);
    }

    /// The notes of the sessions `live` still names, and no other
    /// (INVARIANT 3): the others are dropped.
    pub fn snapshot(&self, live: &[String]) -> HashMap<String, Note> {
        let mut notes = self.notes();
        notes.retain(|sid, note| note.running > 0 || live.contains(sid));
        live.iter()
            .map(|sid| (sid.clone(), notes.get(sid).cloned().unwrap_or_default()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(name: &str) -> Client {
        Client {
            name: name.into(),
            version: "1.2".into(),
        }
    }

    #[test]
    fn a_call_counts_as_running_until_its_guard_is_dropped() {
        let board = Arc::new(Board::default());
        let (a, b) = (board.begin("s1"), board.begin("s1"));
        assert_eq!(board.snapshot(&["s1".into()])["s1"].running, 2);
        drop(a);
        assert_eq!(board.snapshot(&["s1".into()])["s1"].running, 1);
        drop(b);
        assert_eq!(board.snapshot(&["s1".into()])["s1"].running, 0);
    }

    #[test]
    fn the_last_call_and_what_the_client_said_are_kept_and_masked() {
        let board = Board::default();
        board.initialized("s1", client("Claude Code"));
        board.finished(
            "s1",
            "books.chart",
            &Outcome::Ok,
            Some(&client("Claude Code 123-45-6789")),
            Some(&Agent {
                model: Some("m".into()),
                reasoning_effort: None,
                harness: Some("h".into()),
            }),
            100,
        );
        board.finished("s1", "books.trial_balance", &Outcome::Error, None, None, 200);
        let note = &board.snapshot(&["s1".into()])["s1"];
        let said = note.client.as_ref().unwrap();
        assert!(!said.name.contains("6789"), "a TIN is masked: {said:?}");
        assert_eq!(note.agent.as_ref().unwrap().model.as_deref(), Some("m"));
        assert_eq!(
            note.last,
            Some(Last {
                subject: "books.trial_balance".into(),
                outcome: Outcome::Error,
                at: 200
            })
        );
    }

    #[test]
    fn a_session_that_ended_is_not_listed_and_its_note_is_dropped() {
        let board = Board::default();
        board.initialized("gone", client("x"));
        board.initialized("kept", client("y"));
        let listed = board.snapshot(&["kept".into()]);
        assert_eq!(listed.keys().collect::<Vec<_>>(), vec!["kept"]);
        assert!(board.notes().get("gone").is_none());
        board.forget("kept");
        assert!(board.notes().is_empty());
    }
}
