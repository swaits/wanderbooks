//! Who is connected to each company, and how the window says so (plan P15
//! 3.3). The word in the window is "assistant".
//!
//! ```text
//! GOAL: a person can see, for each company, at a glance and without
//!       reading, whether an assistant is connected to it and whether it is
//!       working, and read in words who it is and what it last did; and the
//!       window never waits on the books server to say so.
//! SOURCE: plan/p15-app-and-relay.md 3.3 (every tab carries one of three
//!         states, as a shape and a word, not colour alone: no assistant
//!         connected, connected with its name, working; beside it the last
//!         thing done) and 3.1 (the sessions request); D-77, D-78;
//!         WB-assist-1, WB-assist-2.
//! INVARIANTS:
//!   1. A company is in exactly one state: `Link::None` (no assistant
//!      connected), `Link::Connected`, `Link::Working` (a call of an
//!      assistant is running), or `Link::Unknown` when the server has not
//!      answered lately (starting, gone, slow, or no server at all). The
//!      state is read from the server's own answer, never inferred from
//!      anything the window did.
//!   2. Nothing here waits for the server: one request is asked at a time,
//!      on a thread of its own, at most every [`POLL`]; the window reads
//!      the latest answer. An answer older than [`STALE`] is not believed:
//!      the state is `Unknown`, and nothing is drawn as though it were
//!      known. A request that never returns holds one idle thread and
//!      nothing else; the window never notices.
//!   3. What an assistant said of itself (its name, version, model) is
//!      shown as said and never trusted for anything: it is the server's
//!      masked copy, and no state, permission or revoke depends on it.
//!   4. Words say what happened and when ("posted an entry, 10 seconds
//!      ago"), in the person's terms: a tool's name is turned into what it
//!      did, and one this file does not know is said plainly ("ran trial
//!      balance"). A failed call is said to have failed.
//!   5. Nothing here holds a token's text: the server never sends one.
//! ```

use crate::owner::{Assistant, LastCall, Owner, Roster, TokenRow};
use chrono::{DateTime, Utc};
use egui_phosphor::regular as icon;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How often the server is asked who is connected, while it is up.
pub const POLL: Duration = Duration::from_secs(2);

/// How old an answer may be and still be believed (INVARIANT 2): a few
/// polls, and the time a slow request is allowed.
pub const STALE: Duration = Duration::from_secs(15);

/// Whether an assistant is connected to a company, and whether it is
/// working (INVARIANT 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// The server has not said lately, or there is no server.
    Unknown,
    /// No assistant is connected.
    None,
    /// At least one is, and none has a call running.
    Connected,
    /// At least one has a call running.
    Working,
}

impl Link {
    /// The shape that stands for the state: a different silhouette for
    /// each, so it reads without colour (plugs apart, plugs together, a
    /// spinning arc).
    pub fn glyph(self) -> Option<&'static str> {
        match self {
            Link::Unknown => None,
            Link::None => Some(icon::PLUGS),
            Link::Connected => Some(icon::PLUGS_CONNECTED),
            Link::Working => Some(icon::CIRCLE_NOTCH),
        }
    }
}

/// Everything the window shows of one company's assistants. Made by the
/// app each frame from the latest answer (`Connections::status_of`).
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    /// The window runs a books server: without one there is nothing to
    /// connect to, and none of this is shown.
    pub serves: bool,
    /// The server is not coming up: its start failed or it has exited
    /// (the notice under the tab bar says why).
    pub down: bool,
    /// The company's name, as the tab says it.
    pub company: String,
    /// The state.
    pub link: Link,
    /// The assistants connected to this company, the longest connected
    /// first.
    pub assistants: Vec<Assistant>,
    /// The tokens of this company: who may connect.
    pub tokens: Vec<TokenRow>,
    /// "Now", for "10 seconds ago" (fixed in tests and screenshots).
    pub now: DateTime<Utc>,
}

impl Status {
    /// A company in a window with no server: nothing to show.
    pub fn off(company: &str) -> Self {
        Self {
            serves: false,
            down: false,
            company: company.to_string(),
            link: Link::Unknown,
            assistants: Vec::new(),
            tokens: Vec::new(),
            now: Utc::now(),
        }
    }

    /// The state in words, for the person: "No assistant connected",
    /// "Claude Code connected", "Claude Code is working".
    pub fn word(&self) -> String {
        match self.link {
            Link::Unknown if self.down => "Assistants can't connect now".to_string(),
            Link::Unknown => "Checking for assistants".to_string(),
            Link::None => "No assistant connected".to_string(),
            Link::Connected => format!("{} connected", names(&self.assistants, false)),
            Link::Working => {
                let (names, many) = (names(&self.assistants, true), self.working_count() > 1);
                format!("{names} {} working", if many { "are" } else { "is" })
            }
        }
    }

    /// Sessions with a call running.
    fn working_count(&self) -> usize {
        self.assistants.iter().filter(|a| a.running > 0).count()
    }

    /// The last thing any connected assistant did, in words, and when:
    /// "posted an entry, 10 seconds ago". None when none has done anything.
    pub fn last_thing(&self) -> Option<String> {
        self.assistants
            .iter()
            .filter_map(|a| a.last.as_ref())
            .max_by_key(|l| parse_time(&l.at))
            .map(|l| did(l, self.now))
    }

    /// The second line under the state: the last thing done and when, else
    /// what to do about there being no assistant.
    pub fn detail(&self) -> String {
        match self.link {
            Link::Unknown => String::new(),
            Link::None => "Connect an assistant".to_string(),
            Link::Connected => self
                .last_thing()
                .map(|l| capitalize(&l))
                .unwrap_or_else(|| "Nothing done yet".to_string()),
            Link::Working => self
                .last_thing()
                .map(|l| format!("Last: {l}"))
                .unwrap_or_else(|| "On its first call".to_string()),
        }
    }

    /// The whole of it as one hover text, for the tab and the slot: the
    /// state, then what each connected assistant is.
    pub fn hover(&self) -> String {
        let mut out = self.word();
        for a in &self.assistants {
            out.push_str(&format!("\n{}", describe(a, self.now)));
        }
        out
    }

    /// The tokens' state for the Connect page: each token of this company
    /// with the sessions open on it.
    pub fn connections(&self) -> Vec<Connection<'_>> {
        let mut rows: Vec<Connection<'_>> = self
            .tokens
            .iter()
            .map(|t| Connection {
                token: Some(t),
                name: t.name.clone(),
                sessions: self
                    .assistants
                    .iter()
                    .filter(|a| a.token == t.name)
                    .collect(),
            })
            .collect();
        // A session whose token is not in the list (listed a moment apart
        // from it): shown, with nothing to revoke.
        for a in &self.assistants {
            if !self.tokens.iter().any(|t| t.name == a.token) {
                match rows
                    .iter_mut()
                    .find(|r| r.token.is_none() && r.name == a.token)
                {
                    Some(r) => r.sessions.push(a),
                    None => rows.push(Connection {
                        token: None,
                        name: a.token.clone(),
                        sessions: vec![a],
                    }),
                }
            }
        }
        rows
    }
}

/// One token of a company with the assistants connected on it (the Connect
/// page's "Connections" list).
#[derive(Debug)]
pub struct Connection<'a> {
    /// The token, if the list still has it.
    pub token: Option<&'a TokenRow>,
    /// Its name.
    pub name: String,
    /// The sessions open on it.
    pub sessions: Vec<&'a Assistant>,
}

impl Connection<'_> {
    /// What the token lets an assistant do, in words.
    pub fn may(&self) -> String {
        let Some(t) = self.token else {
            return String::new();
        };
        let has = |s: &str| t.scopes.iter().any(|x| x == s);
        let mut can: Vec<&str> = Vec::new();
        if has("books:read") {
            can.push("read the books");
        }
        if has("books:write") {
            can.push("draft entries");
        }
        if has("books:post") {
            can.push("post them");
        }
        if has("books:admin") {
            can.push("read the access record");
        }
        match can.as_slice() {
            [] => "Can do nothing".to_string(),
            [one] => format!("Can {one}"),
            [init @ .., last] => format!("Can {} and {last}", init.join(", ")),
        }
    }
}

/// An assistant's program, by the name it gave: a friendly name for the
/// ones this window knows, else what it said, else "An assistant"
/// (INVARIANT 3).
pub fn program_name(a: &Assistant) -> String {
    let said = a.client_name.as_deref().unwrap_or("").trim();
    let low = said.to_lowercase();
    let known = [
        ("claude-code", "Claude Code"),
        ("claude code", "Claude Code"),
        ("codex", "Codex"),
        ("gemini", "Gemini CLI"),
        ("grok", "Grok Build"),
        ("muse", "Muse Code"),
        ("cursor", "Cursor"),
        ("chatgpt", "ChatGPT"),
        ("openai", "ChatGPT"),
        ("claude", "Claude"),
    ];
    if let Some((_, name)) = known.iter().find(|(k, _)| low.contains(k)) {
        return (*name).to_string();
    }
    if said.is_empty() {
        "An assistant".to_string()
    } else {
        said.to_string()
    }
}

/// The names of the assistants (those working only, if `working`), each
/// once, with a count where one program has several sessions: "Claude
/// Code", "Claude Code and Codex", "Claude Code ×2", "3 assistants".
fn names(list: &[Assistant], working: bool) -> String {
    let mut seen: Vec<(String, usize)> = Vec::new();
    for a in list.iter().filter(|a| !working || a.running > 0) {
        let n = program_name(a);
        match seen.iter_mut().find(|(s, _)| *s == n) {
            Some((_, c)) => *c += 1,
            None => seen.push((n, 1)),
        }
    }
    match seen.as_slice() {
        [] => "An assistant".to_string(),
        [(n, 1)] => n.clone(),
        [(n, c)] => format!("{n} ×{c}"),
        [(a, 1), (b, 1)] => format!("{a} and {b}"),
        many => {
            let total = many.iter().fold(0usize, |t, (_, c)| t.saturating_add(*c));
            format!("{total} assistants")
        }
    }
}

/// One assistant in a line: "Claude Code 2.1 (token "agent"), connected 3
/// minutes ago, working".
pub fn describe(a: &Assistant, now: DateTime<Utc>) -> String {
    let mut out = program_name(a);
    if let Some(v) = a.client_version.as_deref().filter(|v| !v.is_empty()) {
        out.push(' ');
        out.push_str(v);
    }
    if let Some(m) = a.model.as_deref().filter(|m| !m.is_empty()) {
        out.push_str(&format!(" ({m})"));
    }
    out.push_str(&format!(
        ", connected {}",
        ago(parse_time(&a.connected_at), now)
    ));
    if a.running > 0 {
        out.push_str(", working");
    }
    if let Some(l) = &a.last {
        out.push_str(&format!(", last {}", did(l, now)));
    }
    out
}

/// A finished call as a person would say it, and when: "posted an entry,
/// 10 seconds ago" (INVARIANT 4).
pub fn did(l: &LastCall, now: DateTime<Utc>) -> String {
    let t = l.tool.strip_prefix("books.").unwrap_or(&l.tool);
    let what = match t {
        "post_entry" => "posted an entry".to_string(),
        "draft_entry" => "drafted an entry".to_string(),
        "add_line" => "added a line to a draft".to_string(),
        "reverse_entry" => "reversed an entry".to_string(),
        "start" => "began a session".to_string(),
        "handbook" => "read the handbook".to_string(),
        "digest" => "read the digest".to_string(),
        other if other.contains("://") || other.starts_with('/') => format!("read {other}"),
        other => format!("ran {}", other.replace(['_', '.'], " ")),
    };
    let how = match l.outcome.as_str() {
        "ok" => String::new(),
        "error" => ", and it failed".to_string(),
        refused => format!(
            ", and it was refused ({})",
            refused.strip_prefix("refused: ").unwrap_or(refused)
        ),
    };
    format!("{what}{how}, {}", ago(parse_time(&l.at), now))
}

fn parse_time(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// "just now", "10 seconds ago", "3 minutes ago", "2 hours ago", "5 days
/// ago"; "a while ago" for a time that could not be read.
pub fn ago(then: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(then) = then else {
        return "a while ago".to_string();
    };
    let s = (now - then).num_seconds().max(0);
    let unit = |n: i64, one: &str| {
        if n == 1 {
            format!("1 {one} ago")
        } else {
            format!("{n} {one}s ago")
        }
    };
    match s {
        0..=4 => "just now".to_string(),
        5..=59 => unit(s, "second"),
        60..=3599 => unit(s / 60, "minute"),
        3600..=86399 => unit(s / 3600, "hour"),
        _ => unit(s / 86400, "day"),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

// ---- asking the server ----------------------------------------------------

struct Shared {
    /// A request is out.
    in_flight: bool,
    /// When the last one was begun.
    last_begun: Option<Instant>,
    /// The latest answer and when it came.
    answer: Option<(Instant, Result<Roster, String>)>,
}

/// The latest answer of the server's sessions request (INVARIANT 2).
pub struct Connections {
    shared: Arc<Mutex<Shared>>,
}

impl Default for Connections {
    fn default() -> Self {
        Self::new()
    }
}

fn lock(m: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Clears "a request is out" however the request ends, a panic included.
struct Done(Arc<Mutex<Shared>>);

impl Drop for Done {
    fn drop(&mut self) {
        lock(&self.0).in_flight = false;
    }
}

impl Connections {
    /// Nothing asked yet.
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                in_flight: false,
                last_begun: None,
                answer: None,
            })),
        }
    }

    /// Ask the server who is connected, if it is time and no request is
    /// out (INVARIANT 2); `force` asks as soon as none is out. Returns at
    /// once; the answer is read with [`Connections::roster`].
    pub fn poll(&self, ctx: &egui::Context, owner: &Arc<dyn Owner>, force: bool) {
        {
            let mut s = lock(&self.shared);
            let due = s.last_begun.is_none_or(|t| t.elapsed() >= POLL);
            if s.in_flight || !(due || force) {
                ctx.request_repaint_after(POLL);
                return;
            }
            s.in_flight = true;
            s.last_begun = Some(Instant::now());
        }
        let (shared, owner, ctx2) = (self.shared.clone(), owner.clone(), ctx.clone());
        let spawned = std::thread::Builder::new()
            .name("connections".into())
            .spawn(move || {
                let _done = Done(shared.clone());
                let answer = owner.serve_roster();
                lock(&shared).answer = Some((Instant::now(), answer));
                ctx2.request_repaint();
            });
        if spawned.is_err() {
            lock(&self.shared).in_flight = false;
        }
        ctx.request_repaint_after(POLL);
    }

    /// The latest answer, if it is one the server gave and is not stale
    /// (INVARIANT 2).
    pub fn roster(&self) -> Option<Roster> {
        self.roster_within(STALE)
    }

    /// The latest answer the server gave, if it is no older than `age`: the
    /// close warning goes by a longer memory than the state does, so a
    /// server that is slow for a moment does not make an assistant that was
    /// connected seem gone.
    pub fn roster_within(&self, age: Duration) -> Option<Roster> {
        match &lock(&self.shared).answer {
            Some((at, Ok(roster))) if at.elapsed() <= age => Some(roster.clone()),
            _ => None,
        }
    }

    /// Why the last request failed, if it did and no later one has
    /// succeeded (for a notice; not for the state).
    pub fn trouble(&self) -> Option<String> {
        match &lock(&self.shared).answer {
            Some((_, Err(why))) => Some(why.clone()),
            _ => None,
        }
    }

    /// Whether a request is out now.
    pub fn is_asking(&self) -> bool {
        lock(&self.shared).in_flight
    }

    /// Forget the answer: the next frame asks again (after a token is
    /// ended, the state is not the one that was read before).
    pub fn forget(&self) {
        let mut s = lock(&self.shared);
        s.answer = None;
        s.last_begun = None;
    }

    /// Wait until no request is out, for at most `within`: true when none
    /// is (tests).
    pub fn wait_idle(&self, within: Duration) -> bool {
        let until = Instant::now() + within;
        while Instant::now() < until {
            if !self.is_asking() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        !self.is_asking()
    }

    /// What is shown of the company with books `book`, named `company`,
    /// from the latest answer; `serves` is whether the window runs a
    /// server at all.
    pub fn status_of(
        &self,
        serves: bool,
        down: bool,
        book: Option<&str>,
        company: &str,
        now: DateTime<Utc>,
    ) -> Status {
        self.status_within(STALE, serves, down, book, company, now)
    }

    /// [`Connections::status_of`], believing an answer no older than `age`.
    pub fn status_within(
        &self,
        age: Duration,
        serves: bool,
        down: bool,
        book: Option<&str>,
        company: &str,
        now: DateTime<Utc>,
    ) -> Status {
        let mut status = Status::off(company);
        status.now = now;
        status.serves = serves && book.is_some();
        status.down = down;
        let (true, Some(book)) = (status.serves, book) else {
            return status;
        };
        let Some(roster) = (if down { None } else { self.roster_within(age) }) else {
            return status;
        };
        status.assistants = roster
            .assistants
            .into_iter()
            .filter(|a| a.book == book)
            .collect();
        // The longest connected first (RFC 3339 times order as text).
        status
            .assistants
            .sort_by(|a, b| a.connected_at.cmp(&b.connected_at));
        status.tokens = roster
            .tokens
            .into_iter()
            .filter(|t| t.book == book)
            .collect();
        status.link = if status.assistants.is_empty() {
            Link::None
        } else if status.assistants.iter().any(|a| a.running > 0) {
            Link::Working
        } else {
            Link::Connected
        };
        status
    }
}
