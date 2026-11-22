//! The viewer's Books menu (plan P2 "Viewer menu", D-19, D-32): the
//! owner's own actions on the books, handed to the books server (back up
//! now, restore, new books, the format upgrade) or to the
//! owner's agent (connect). The viewer itself still never writes the
//! books.
//!
//! ```text
//! GOAL: the owner backs up, restores, starts new books and connects an
//!       agent from the viewer, without a terminal. (There is no "Close the
//!       books": the app is the host, and a company's tab, or the window,
//!       closes it: WB-app-2, WB-app-4.)
//! SOURCE: plan P2 ("Viewer menu (D-19)": "Back up now", "Restore…",
//!         "New books…" and "Connect an assistant…"; the
//!         page it opens carries the two connect buttons; "Nothing in the viewer writes a book"); D-32
//!         (no confinement: the viewer asks the server itself, as the
//!         command line does); plan 3.9 and D-39 (the format upgrade: the
//!         go-ahead asked at every open until given, with the owner
//!         passphrase, and a standing notice).
//! INVARIANTS:
//!   1. Every action goes through the [`Owner`] the program gives the
//!      viewer (the books server, or the agent's own command): nothing
//!      here writes a file of the books.
//!   2. Each runs off the UI thread; what came of it is shown, in words.
//!   3. A restore is shown before it is done (the backup proven, what it
//!      would do), and done only for that backup, at that head.
//!   4. Without an owner (the viewer on its own, books outside a data
//!      root: the demo) the menu isn't there.
//!   5. Books waiting for a format upgrade show a standing notice above
//!      every page, and the go-ahead is asked once each time the viewer
//!      opens them: the owner passphrase, typed here and handed to the
//!      owner's `upgrade_format` (the server checks it; nothing here
//!      does). "Not now" changes nothing; a refusal (a wrong passphrase)
//!      is said in the dialog, which stays.
//!   6. An action that changes or replaces the books, their format or
//!      what the owner holds (restore, the format upgrade, new books,
//!      connect, close the books, check for updates, update) is never
//!      abandoned: while one runs, its tab's close control is off, with
//!      the words of what is running ("Restoring these books: wait for it
//!      to finish"), and the window's close is held (app.rs); its result
//!      is shown in that tab when it ends. A read-only one (back up now,
//!      the restore's check of a backup) is stopped with the tab: it does
//!      not start once the tab has closed, and what it sends after is
//!      dropped (a request the books server already has it finishes: the
//!      viewer only reads its answer).
//! ```

use crate::books::Books;
use egui::{RichText, Ui, vec2};
use egui_phosphor::regular as icon;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, OnceLock};

/// The owner's agents the viewer can connect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    /// Claude Code.
    Claude,
    /// Codex.
    Codex,
}

impl Agent {
    /// The key it is kept under (the viewer's preference) and the word
    /// the owner's commands know it by.
    pub fn key(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }

    /// The agent kept under `key`.
    pub fn from_key(key: &str) -> Option<Agent> {
        [Agent::Claude, Agent::Codex]
            .into_iter()
            .find(|a| a.key() == key)
    }

    /// Its name.
    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
        }
    }
}

/// Where a first run stands: what exists already (the viewer's welcome
/// page starts from the first missing step).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FirstRun {
    /// The books' private folder is there.
    pub root: bool,
    /// The ids of the books in it.
    pub books: Vec<String>,
    /// The owner passphrase is set.
    pub passphrase_set: bool,
}

/// How a person gets an agent's program: the vendor's own installer, never
/// bundled (D-27).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installer {
    /// The agent's name.
    pub label: String,
    /// The vendor's install command, for a terminal.
    pub command: String,
    /// The vendor's page about it.
    pub page: String,
}

/// What it takes to run an agent in a books home (the dock runs it on a
/// terminal of its own; nothing here starts a process).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentStart {
    /// Run `program args` in `dir` with `env` added.
    Ready {
        /// The agent's name.
        label: String,
        /// The program.
        program: std::path::PathBuf,
        /// Its arguments.
        args: Vec<String>,
        /// The books home it starts in.
        dir: std::path::PathBuf,
        /// Environment to add.
        env: Vec<(String, String)>,
    },
    /// The agent isn't installed: the vendor's installer.
    Missing(Installer),
}

/// A passphrase typed into the window: never shown by `Debug`, wiped from
/// memory when dropped.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

/// How the books' backups stand, in words, for the Backups window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupState {
    /// What to tell the person, a line each (when it last backed up, whether
    /// it was proven restorable, a disk that isn't connected).
    pub lines: Vec<String>,
    /// The backup folder chosen, if one is.
    pub folder: Option<String>,
    /// Protected by a passphrase.
    pub sealed: bool,
    /// The folder holds a longer history of these books than they have
    /// now (the books were restored from an older backup): a new backup set
    /// can be started beside the old one.
    pub ahead: bool,
}

/// A restore previewed: what it would do, and what it is of (INVARIANT 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestorePlan {
    /// The backup, as the server is told it: a folder or a file.
    pub source: String,
    /// Its passphrase when it has one: never shown, wiped when dropped.
    pub passphrase: Option<Secret>,
    /// The books' id it comes back as.
    pub id: String,
    /// Whether it replaces the books of that id (kept in legacy/).
    pub replace: bool,
    /// The backup's head, proven.
    pub head: (u64, String),
    /// What it would do, a line each.
    pub lines: Vec<String>,
}

/// An update the host has downloaded and checked, waiting for the owner to
/// accept it (P11a step 4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateWaiting {
    /// Its version.
    pub version: String,
    /// When it was staged (UTC, RFC 3339).
    pub staged_at: String,
}

/// A call an assistant made last, as the server says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastCall {
    /// The tool (or the resource's address).
    pub tool: String,
    /// "ok", "error", or "refused: CODE".
    pub outcome: String,
    /// When it finished (UTC, RFC 3339).
    pub at: String,
}

/// One assistant connected to a company, from the server's sessions
/// request (plan P15 3.1): what it said of itself (asserted, never
/// checked) and what it is doing. Never a token's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assistant {
    /// The company it is connected to (the id of its books).
    pub book: String,
    /// The NAME of the token it opened its session with.
    pub token: String,
    /// The program's name, as it said at the start.
    pub client_name: Option<String>,
    /// Its version.
    pub client_version: Option<String>,
    /// The model it said it is, if it did.
    pub model: Option<String>,
    /// When it connected (UTC, RFC 3339).
    pub connected_at: String,
    /// Calls of it running now.
    pub running: u32,
    /// The last call it made, if it has made one.
    pub last: Option<LastCall>,
}

/// A token, as listed: who may connect to a company (never its text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenRow {
    /// Its id: what a revoke names, so a row of a key since replaced
    /// cannot end the new one.
    pub id: String,
    /// The company it opens.
    pub book: String,
    /// Its name.
    pub name: String,
    /// What it may do (books:read, books:write, books:post, books:admin).
    pub scopes: Vec<String>,
    /// When it was made (UTC).
    pub created: String,
}

/// A web assistant waiting for the person to allow it (plan P15 D-83): who
/// is asking, as the server saw it. What it calls itself is its own word,
/// cleaned and never trusted; where it is sent back to and the address it
/// asked from are what the server saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebRequest {
    /// What an answer names.
    pub id: String,
    /// The company it asks to reach.
    pub book: String,
    /// What it calls itself: its own word, cleaned and short, possibly
    /// empty. Never a product's name made from where it is sent back to.
    pub client_name: String,
    /// Whether the address it is sent back to is one of the vendors' known
    /// hosts. Only the warning depends on it; nothing is shown as known or
    /// trusted for it.
    pub known_host: bool,
    /// The short code to match with the one its browser page shows.
    pub code: String,
    /// The address it is sent back to, in full.
    pub redirect: String,
    /// That address's host.
    pub redirect_host: String,
    /// The caller's address, as the relay said it.
    pub from: String,
    /// It asked to post entries too.
    pub wants_post: bool,
    /// Seconds until it is turned away unanswered.
    pub expires_in_secs: u64,
}

/// A web connection the person allowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebApproval {
    /// Its id.
    pub id: String,
    /// The company.
    pub book: String,
    /// What it is called.
    pub label: String,
    /// The key that carries it: what "Who can reach these books" revokes.
    pub token_id: String,
    /// It may post entries (else read and draft).
    pub may_post: bool,
    /// When it last did anything (unix seconds).
    pub last_used: Option<i64>,
}

/// How far web assistants have got for a company (plan P15 B3): the steps
/// the page tells in words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebStep {
    /// Off.
    Off,
    /// Getting an address.
    GettingAddress,
    /// Making the line to the relay.
    Connecting,
    /// The line is up; the certificate is being got.
    GettingCertificate,
    /// A web assistant can reach the company now.
    Connected,
}

/// What the Connect page shows of one company's web assistants. Read from
/// memory by the owner, every frame, never a wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebConnect {
    /// The switch.
    pub on: bool,
    /// How far it has got.
    pub step: WebStep,
    /// The address for the whole installation, while on.
    pub address: Option<String>,
    /// The address an assistant is given for this company.
    pub connector: Option<String>,
    /// A line of progress, in words.
    pub words: String,
    /// What is wrong, in words.
    pub problem: Option<String>,
    /// The problem needs the person (it will not mend itself).
    pub needs_person: bool,
    /// An address is kept for when web assistants are turned on again.
    pub kept: bool,
}

/// Who is connected and who may connect, across the open companies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roster {
    /// Every live session.
    pub assistants: Vec<Assistant>,
    /// Every live token.
    pub tokens: Vec<TokenRow>,
    /// Web assistants waiting for the person.
    pub web_pending: Vec<WebRequest>,
    /// Web connections the person allowed.
    pub web_approvals: Vec<WebApproval>,
}

/// The owner's actions, done by the program the viewer runs in (ledger-cli
/// over the books server and the agents' own commands). Each blocks: the
/// menu calls it off the UI thread.
pub trait Owner: Send + Sync {
    /// Whether this program hosts the books' server for the window (plan
    /// P15 3.1: the app starts it, as a child process, and closes it): when
    /// it does not, the window asks nothing of a server (`serve_*`).
    fn serves(&self) -> bool {
        false
    }
    /// Start the books server with the companies `open` (their ids) open,
    /// and return when it is ready, or why it is not. Blocks.
    fn serve_start(&self, _open: &[String]) -> Result<(), String> {
        Ok(())
    }
    /// Whether another program's host holds the books now (an earlier
    /// build's, or `wanderbooks serve`'s): asked when the start was
    /// refused, to offer the person a button that closes it
    /// (`serve_close_host`) beside the line that says what to type.
    fn serve_host_holds(&self) -> bool {
        false
    }
    /// Ask that host to close the books and wait until it has (what
    /// `wanderbooks close` does, for a person with no command line on
    /// their path): `Ok` says what came of it, `Err` why not. Blocks.
    fn serve_close_host(&self) -> Result<String, String> {
        Err("not here".into())
    }
    /// Open the company `book` in the server (a tab opened): `Ok(None)`
    /// when it is open now. Blocks.
    fn serve_open(&self, _book: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    /// Close the company `book` in the server (a tab closed): `Ok(None)`
    /// when it is closed, `Ok(Some(words))` when it is closed but its last
    /// backup wasn't whole, `Err` when it could not be asked or refused.
    /// Blocks (a backup is made first).
    fn serve_close(&self, _book: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    /// Of the companies `wanted` (asked to be open at the start), those
    /// the server does not have open now, each with why in words: asked
    /// once the server is ready. Blocks.
    fn serve_not_open(&self, _wanted: &[String]) -> Vec<(String, String)> {
        Vec::new()
    }
    /// Close the server (the window closed): each open company is closed
    /// first, then the server's input, and the exit is waited for, for a
    /// bounded time. `Ok(Some(words))` says what was not whole, a line
    /// each: a company whose last backup was incomplete, a server that
    /// exited badly, one still finishing by itself after the wait. Blocks.
    fn serve_stop(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
    /// Why the server is gone, if it has exited on its own. Cheap: asked
    /// on every frame.
    fn serve_exited(&self) -> Option<String> {
        None
    }
    /// Who is connected to the open companies, and which tokens are live:
    /// two quick requests to the server, an `Err` (in words) when it is
    /// starting, gone or does not answer in a few seconds. Blocks, for that
    /// long at most: the window asks off its UI thread.
    fn serve_roster(&self) -> Result<Roster, String> {
        Ok(Roster::default())
    }
    /// End the token `name`, whose id is `id`, of the company `book` (what
    /// `wanderbooks token revoke` does, for that one key): `Ok` says what
    /// was done, in words; a key already replaced or revoked is an `Err`
    /// that says so. Blocks.
    fn serve_revoke(&self, _book: &str, _name: &str, _id: &str) -> Result<String, String> {
        Err("not here".into())
    }
    /// The person's answer to a web assistant waiting to be allowed:
    /// `approve` it or refuse it; `post` lets it post entries too, not
    /// only read and draft. `Ok` says what was done, in words. Blocks.
    fn web_decide(&self, _id: &str, _approve: bool, _post: bool) -> Result<String, String> {
        Err("not here".into())
    }
    /// Let a web connection post entries, or take that away. Blocks.
    fn web_set_post(&self, _approval: &str, _post: bool) -> Result<String, String> {
        Err("not here".into())
    }
    /// The company's web assistants as they stand: `None` when this window
    /// has no way to offer them (the switch is not drawn). Reads memory;
    /// never waits.
    fn web_state(&self, _book: &str) -> Option<WebConnect> {
        None
    }
    /// Turn web assistants on for the company: `Ok` says what happens next
    /// in words. Blocks (a request or two of the server).
    fn web_turn_on(&self, _book: &str) -> Result<String, String> {
        Err("not here".into())
    }
    /// Turn them off for the company. Blocks.
    fn web_turn_off(&self, _book: &str) -> Result<String, String> {
        Err("not here".into())
    }
    /// A new address for web assistants: the old one stops working. Blocks.
    fn web_new_address(&self, _book: &str) -> Result<String, String> {
        Err("not here".into())
    }
    /// This program run with `words`, as a line to type in a terminal (the
    /// program's own path, the words, the data root if one was named): what
    /// the Connect page shows for an assistant it has no button for.
    fn command_line(&self, _words: &[&str]) -> Result<String, String> {
        Err("not here".into())
    }
    /// The Claude desktop extension for the company `book` (shown as
    /// `company`): the one-click package written where a person can open it,
    /// the key made first if there was none. Blocks.
    fn claude_desktop_extension(
        &self,
        _book: &str,
        _company: &str,
    ) -> Result<ExtensionMade, String> {
        Err("not here".into())
    }
    /// The key an assistant needs to reach the company `book` exists (made
    /// now if it did not), able to read, draft and post but not to see the
    /// access record: `Ok(true)` when it was made now. It never replaces a
    /// key: one that exists with no copy here is an `Err` that says so.
    /// Blocks.
    fn make_key(&self, _book: &str) -> Result<bool, String> {
        Err("not here".into())
    }
    /// New, empty books `id` in `currency`.
    fn new_books(&self, id: &str, currency: &str) -> Result<String, String>;
    /// Back the books `book` up now.
    fn back_up_now(&self, book: &str) -> Result<Vec<String>, String>;
    /// Start a new backup set beside the old one in the backup folder and
    /// back the books up to it (their history is the beginning of the old
    /// set's): the old set is never touched.
    fn start_new_backup_set(&self, _book: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    /// How the books `book`'s backups stand. Blocks.
    fn backup_state(&self, _book: &str) -> Result<BackupState, String> {
        Err("not here".into())
    }
    /// Set the books' backups up: the backup folder, and the passphrase
    /// (set, or taken off). What was done, a line each. Blocks.
    fn set_up_backups(
        &self,
        _book: &str,
        _folder: Option<String>,
        _passphrase: Option<Secret>,
        _remove_passphrase: bool,
    ) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    /// Save a backup of the books as one file at `file`. Blocks.
    fn save_backup(&self, _book: &str, _file: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    /// Preview restoring the backup at `source` (as `id`, else its own
    /// books' id), replacing the books of that id or not.
    fn plan_restore(
        &self,
        source: &str,
        id: Option<String>,
        replace: bool,
        passphrase: Option<Secret>,
    ) -> Result<RestorePlan, String>;
    /// Restore as `plan` previewed it.
    fn restore(&self, plan: &RestorePlan) -> Result<Vec<String>, String>;
    /// Let `agent` reach the books `book`.
    fn connect(&self, agent: Agent, book: &str) -> Result<Vec<String>, String>;
    /// The owner's go-ahead for the books `book`'s format upgrade, with the
    /// owner passphrase they typed (the server checks it).
    fn upgrade_format(&self, book: &str, passphrase: &str) -> Result<Vec<String>, String>;
    /// Where a first run stands (P11b: onboarding with no terminal).
    fn first_run(&self) -> FirstRun {
        FirstRun::default()
    }
    /// Make the books' private folder (what `wanderbooks setup` does).
    fn set_up(&self) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    /// Set the owner passphrase for the first time, as typed (twice) in the
    /// viewer.
    fn set_passphrase(&self, _new: &str) -> Result<(), String> {
        Err("not here".into())
    }
    /// Hold the books `book` open while the viewer runs (the viewer's lease).
    fn hold(&self, _book: &str) {}
    /// The update waiting for the owner's accept, if one is (read from the
    /// data root; cheap, called every few seconds).
    fn update_waiting(&self) -> Option<UpdateWaiting> {
        None
    }
    /// "Check for updates": look at the release feed now and stage a newer
    /// version if there is one. What came of it, in plain words.
    fn check_for_updates(&self) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    /// A newer version exists, and where to get it (WB-pkg-3): what the
    /// program's daily look found. Cheap: called every few seconds.
    fn newer_version(&self) -> Option<NewerVersion> {
        None
    }
    /// Accept the waiting update: the books' host installs it and restarts
    /// onto it when no call is running, the books staying open.
    fn apply_update(&self) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    /// How to run `agent` in the books home of `book` (made if need be): the
    /// dock starts it on a terminal of its own (D-27).
    fn agent_start(&self, _agent: Agent, _book: &str) -> Result<AgentStart, String> {
        Err("not here".into())
    }
}

/// A newer version of the program that the person may get and install over
/// this one (WB-pkg-3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewerVersion {
    /// Its version.
    pub version: String,
    /// The page to get it from: the program's own, never a feed's.
    pub page: String,
}

/// The Claude desktop extension the Connect page made (WB-pkg-5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionMade {
    /// The package file, in full.
    pub file: String,
    /// The folder it is in.
    pub folder: String,
}

/// The owner the program gave the viewer, for every company's view that
/// isn't given one of its own. It is the program's (one per process, as
/// the program is one), not a company's: each view takes a handle to it
/// when it is made, and holds that (`CompanyView::with_owner` gives a view
/// another).
static OWNER: OnceLock<Arc<dyn Owner>> = OnceLock::new();

/// Give the viewer its owner (once, at start; INVARIANT 4).
pub fn set_owner(owner: Arc<dyn Owner>) {
    let _ = OWNER.set(owner);
}

/// The viewer's owner, if the program gave it one.
pub fn current() -> Option<Arc<dyn Owner>> {
    OWNER.get().cloned()
}

/// The id of books of the data root (their folder's name), or None for
/// books outside one, the demo's (INVARIANT 4).
pub fn book_id(books: &Books) -> Option<String> {
    book_id_of_dir(&books.data_dir)
}

/// [`book_id`] of the books in `dir`.
pub fn book_id_of_dir(dir: &std::path::Path) -> Option<String> {
    ledger_store::root::inside_a_root(dir)
        .then(|| dir.file_name()?.to_str().map(str::to_string))
        .flatten()
}

type Outcome = Result<Vec<String>, String>;

/// What a job does to the books (INVARIANT 6).
#[derive(Clone, Copy)]
enum Effect {
    /// It only reads: stopped with its tab.
    Reads,
    /// It changes something and is never abandoned; the words say what is
    /// running ("Restoring these books").
    Changes(&'static str),
}

impl Effect {
    fn hold(self) -> crate::jobs::Hold {
        match self {
            Effect::Reads => crate::jobs::Hold::Free,
            Effect::Changes(what) => {
                crate::jobs::Hold::Keep(format!("{what}: wait for it to finish"))
            }
        }
    }
}

/// A job to run off the UI thread: what it is called, what it does to the
/// books, and the work.
type Start = (String, Effect, Box<dyn FnOnce() -> Outcome + Send>);

/// A dialog open over the viewer.
#[derive(Default)]
enum Dialog {
    #[default]
    None,
    /// "New books…": their id and currency.
    NewBooks { id: String, currency: String },
    /// "Backups…": the folder, the passphrase, and how they stand.
    Backups(Box<BackupForm>),
    /// "Restore…": the backup, then its preview.
    Restore(Box<RestoreForm>),
    /// The format upgrade: the go-ahead, with the owner passphrase.
    Upgrade(Box<UpgradeForm>),
}

/// The Backups window: how they stand, the folder to choose, the passphrase
/// to offer.
struct BackupForm {
    /// What the server said of them, once it has.
    state: Option<Result<BackupState, String>>,
    /// The reading under way.
    loading: Option<Receiver<Result<BackupState, String>>>,
    /// The folder, as typed or chosen.
    folder: String,
    /// Whether a passphrase is being set (or changed).
    protect: bool,
    /// The passphrase, twice (wiped when the form is dropped).
    pass: String,
    again: String,
    /// Why the last try was refused, or what it did.
    problem: Option<String>,
    said: Vec<String>,
}

impl BackupForm {
    fn new() -> Self {
        Self {
            state: None,
            loading: None,
            folder: String::new(),
            protect: false,
            pass: String::new(),
            again: String::new(),
            problem: None,
            said: Vec::new(),
        }
    }
}

impl Drop for BackupForm {
    /// What was typed and not handed over is wiped from memory.
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.pass);
        zeroize::Zeroize::zeroize(&mut self.again);
    }
}

/// The words a person is told when setting a passphrase: what it does, and
/// that it cannot be recovered (D-92).
/// What the first line of a restore that is not of the newest backup begins
/// with (ledger-host `older_backup_words`): the window shows it prominently.
pub const OLDER_BACKUP: &str = "NOT THE NEWEST BACKUP";

pub const PASSPHRASE_WARNING: &str = "Every file in the backup folder is sealed: only this passphrase opens it, and the files say nothing about your books but how many and how big. Your computer keeps the key so backups stay automatic. A forgotten passphrase cannot be recovered, by anyone, and backups made without it before this stay as they were. Write it down somewhere safe.";

/// The format upgrade's dialog (INVARIANT 5).
struct UpgradeForm {
    /// What the books wait for.
    upgrade: ledger_store::FormatUpgrade,
    /// The owner passphrase, as typed (cleared once handed over).
    passphrase: String,
    /// The upgrade under way.
    working: Option<Receiver<Outcome>>,
    /// Why the last try was refused.
    problem: Option<String>,
}

impl UpgradeForm {
    fn new(upgrade: ledger_store::FormatUpgrade) -> Self {
        Self {
            upgrade,
            passphrase: String::new(),
            working: None,
            problem: None,
        }
    }
}

impl Drop for UpgradeForm {
    /// What was typed and not handed over is wiped from memory, as the
    /// command line wipes what is typed at a terminal.
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.passphrase);
    }
}

/// Wakes the window when dropped: a worker's last act, however it ends.
struct Wake(egui::Context);

impl Drop for Wake {
    fn drop(&mut self) {
        self.0.request_repaint();
    }
}

/// A restore's check under way, and what it was asked: the backup, the
/// id, whether it replaces.
type Checking = (
    Receiver<Result<RestorePlan, String>>,
    (String, String, bool),
);

#[derive(Default)]
struct RestoreForm {
    source: String,
    id: String,
    replace: bool,
    /// The backup's passphrase, if it has one (wiped when the form is
    /// dropped).
    passphrase: String,
    /// A check under way, and what it was asked (a result for anything
    /// since edited is dropped).
    planning: Option<Checking>,
    plan: Option<RestorePlan>,
    problem: Option<String>,
}

impl Drop for RestoreForm {
    /// What was typed and not handed over is wiped from memory.
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.passphrase);
    }
}

/// The menu's state: a dialog, the job under way, and the last outcome.
#[derive(Default)]
pub struct OwnerMenu {
    /// The view that owns it: its dialogs' ids are scoped by it, so two
    /// views' dialogs never share egui state.
    scope: Option<egui::Id>,
    /// The books last shown (by identity) and their id: found once for
    /// each load, not every frame.
    book: Option<(usize, Option<String>)>,
    dialog: Dialog,
    job: Option<(String, Receiver<Outcome>)>,
    done: Option<(String, Outcome)>,
    /// The books whose format upgrade was asked for since the viewer
    /// opened them (INVARIANT 5: once each open).
    asked_upgrade: Option<String>,
    /// The update waiting, as last read, and when (re-read every few
    /// seconds and after any job).
    update: Option<(std::time::Instant, Option<UpdateWaiting>)>,
    /// A newer version the person may get, as last read, and when.
    newer: Option<(std::time::Instant, Option<NewerVersion>)>,
    /// The tab's threads: every job is one of them (INVARIANT 6).
    jobs: crate::jobs::Spawner,
}

/// How long a reading of the waiting update is kept.
const UPDATE_LOOK: std::time::Duration = std::time::Duration::from_secs(5);

impl OwnerMenu {
    /// A menu whose ids belong to the view `scope`, and whose jobs are
    /// threads of that view (`jobs`).
    pub fn scoped(scope: egui::Id, jobs: crate::jobs::Spawner) -> Self {
        Self {
            scope: Some(scope),
            jobs,
            ..Default::default()
        }
    }

    /// The id of the books shown, if they are books of a data root: found
    /// once for each load.
    fn book_of(&mut self, books: &Arc<Books>) -> Option<String> {
        let key = Arc::as_ptr(books) as usize;
        if self.book.as_ref().is_none_or(|(k, _)| *k != key) {
            self.book = Some((key, book_id(books)));
        }
        self.book.as_ref().and_then(|(_, id)| id.clone())
    }

    /// The update waiting, if any (read now and then, not every frame).
    fn update_waiting(
        &mut self,
        ctx: &egui::Context,
        owner: &Arc<dyn Owner>,
    ) -> Option<UpdateWaiting> {
        if self
            .update
            .as_ref()
            .is_none_or(|(at, _)| at.elapsed() > UPDATE_LOOK)
        {
            self.update = Some((std::time::Instant::now(), owner.update_waiting()));
        }
        ctx.request_repaint_after(UPDATE_LOOK);
        self.update.as_ref().and_then(|(_, u)| u.clone())
    }

    /// A newer version to get, if any (read now and then, not every frame).
    fn newer_version(
        &mut self,
        ctx: &egui::Context,
        owner: &Arc<dyn Owner>,
    ) -> Option<NewerVersion> {
        if self
            .newer
            .as_ref()
            .is_none_or(|(at, _)| at.elapsed() > UPDATE_LOOK)
        {
            self.newer = Some((std::time::Instant::now(), owner.newer_version()));
        }
        ctx.request_repaint_after(UPDATE_LOOK);
        self.newer.as_ref().and_then(|(_, n)| n.clone())
    }

    /// The standing notices above the page: an update waiting for the
    /// owner's accept, and a format upgrade the books wait for, whose
    /// go-ahead is asked for once each open (INVARIANT 5).
    pub fn notice(&mut self, ui: &mut Ui, owner: Option<Arc<dyn Owner>>, books: &Arc<Books>) {
        let Some(owner) = owner else {
            return;
        };
        let Some(book) = self.book_of(books) else {
            return;
        };
        if let Some(update) = self.update_waiting(ui.ctx(), &owner) {
            let p = crate::theme::pal(ui.ctx());
            let mut start: Option<Box<dyn FnOnce() -> Outcome + Send>> = None;
            egui::Frame::new()
                .fill(p.warning_soft)
                .inner_margin(egui::Margin::symmetric(20, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} Version {} of Wanderbooks is ready.",
                                icon::ARROW_FAT_UP,
                                update.version
                            ))
                            .font(crate::theme::semibold(13.0))
                            .color(p.warning),
                        );
                        ui.label(
                            RichText::new(
                                "It is downloaded and checked. The books stay open while it starts, and no work in progress is cut off.",
                            )
                            .font(crate::theme::regular(13.0))
                            .color(p.text),
                        );
                        let free = self.job.is_none();
                        if ui
                            .add_enabled(free, egui::Button::new("Update now"))
                            .on_hover_text("Start using the new version")
                            .clicked()
                        {
                            let o = owner.clone();
                            start = Some(Box::new(move || o.apply_update()));
                        }
                    });
                });
            if let Some(work) = start {
                self.start(
                    ui.ctx(),
                    (
                        "Update".into(),
                        Effect::Changes("Updating Wanderbooks"),
                        work,
                    ),
                );
            }
        }
        // No update of the program's own is waiting: is a newer version
        // out for the person to get and install over this one?
        else if let Some(newer) = self.newer_version(ui.ctx(), &owner) {
            let p = crate::theme::pal(ui.ctx());
            egui::Frame::new()
                .fill(p.warning_soft)
                .inner_margin(egui::Margin::symmetric(20, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} Version {} of Wanderbooks is available.",
                                icon::ARROW_FAT_UP,
                                newer.version
                            ))
                            .font(crate::theme::semibold(13.0))
                            .color(p.warning),
                        );
                        ui.label(
                            RichText::new(
                                "Download it and install it over this one. Your books, keys and connections stay as they are.",
                            )
                            .font(crate::theme::regular(13.0))
                            .color(p.text),
                        );
                        if ui
                            .button("Open the download page")
                            .on_hover_text(newer.page.clone())
                            .clicked()
                        {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(newer.page.clone()));
                        }
                    });
                });
        }
        let Some(upgrade) = books.format_upgrade.clone() else {
            return;
        };
        if self.asked_upgrade.as_deref() != Some(book.as_str())
            && matches!(self.dialog, Dialog::None)
            && self.job.is_none()
        {
            self.asked_upgrade = Some(book);
            self.dialog = Dialog::Upgrade(Box::new(UpgradeForm::new(upgrade.clone())));
        }
        let p = crate::theme::pal(ui.ctx());
        egui::Frame::new()
            .fill(p.warning_soft)
            .inner_margin(egui::Margin::symmetric(20, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} These books wait for a format upgrade.",
                            icon::WARNING
                        ))
                        .font(crate::theme::semibold(13.0))
                        .color(p.warning),
                    );
                    ui.label(
                        RichText::new(
                            "An older version of Wanderbooks wrote them. They work on their old format until you agree; what needs the new one is refused.",
                        )
                        .font(crate::theme::regular(13.0))
                        .color(p.text),
                    );
                    let open = matches!(self.dialog, Dialog::None) && self.job.is_none();
                    if ui
                        .add_enabled(open, egui::Button::new("Upgrade…"))
                        .on_hover_text("Give the go-ahead, with your owner passphrase: a verified backup is made first")
                        .clicked()
                    {
                        self.dialog = Dialog::Upgrade(Box::new(UpgradeForm::new(upgrade)));
                    }
                });
            });
    }

    /// The menu button and a note of the last outcome; its dialogs.
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        owner: Option<Arc<dyn Owner>>,
        books: &Arc<Books>,
        compact: bool,
    ) {
        let Some(owner) = owner else {
            return;
        };
        let Some(book) = self.book_of(books) else {
            return;
        };
        // Upgraded elsewhere meanwhile (at a terminal): nothing to ask.
        if books.format_upgrade.is_none()
            && let Dialog::Upgrade(form) = &self.dialog
            && form.working.is_none()
        {
            self.dialog = Dialog::None;
        }
        let p = crate::theme::pal(ui.ctx());
        if let Some((what, rx)) = &self.job
            && let Ok(done) = rx.try_recv()
        {
            self.done = Some((what.clone(), done));
            self.job = None;
            self.update = None;
        }
        let text = if compact {
            RichText::new(icon::BOOKS).size(16.0).color(p.muted)
        } else {
            RichText::new(format!("{} Books", icon::BOOKS))
                .font(crate::theme::regular(12.5))
                .color(p.muted)
        };
        let mut button = egui::Button::new(text).frame(false);
        if compact {
            button = button.min_size(vec2(32.0, ui.spacing().interact_size.y + 4.0));
        }
        let busy = self.job.is_some();
        let waiting = self.update_waiting(ui.ctx(), &owner);
        let mut start: Option<Start> = None;
        let response = ui
            .add_enabled_ui(!busy, |ui| {
                egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                    ui.set_min_width(240.0);
                    if ui
                        .button(format!("{} Back up now", icon::CLOUD_ARROW_UP))
                        .on_hover_text("Back these books up now, to the backup folder")
                        .clicked()
                    {
                        let (o, b) = (owner.clone(), book.clone());
                        start = Some((
                            "Back up now".into(),
                            Effect::Reads,
                            Box::new(move || o.back_up_now(&b)),
                        ));
                    }
                    if ui
                        .button(format!("{} Backups…", icon::CLOUD_ARROW_UP))
                        .on_hover_text("Choose where these books back up to, protect the backups with a passphrase, and see how they stand")
                        .clicked()
                    {
                        self.dialog = Dialog::Backups(Box::new(BackupForm::new()));
                    }
                    if ui
                        .button(format!("{} Save a backup…", icon::CLOUD_ARROW_UP))
                        .on_hover_text("Save these books as one backup file to keep or carry")
                        .clicked()
                    {
                        let name = format!(
                            "{}-{}.wbbackup",
                            book,
                            chrono::Local::now().format("%Y-%m-%d")
                        );
                        if let Some(file) = rfd::FileDialog::new()
                            .set_title("Save a backup")
                            .set_file_name(name)
                            .add_filter("Wanderbooks backup", &[backup_extension()])
                            .save_file()
                        {
                            let (o, b) = (owner.clone(), book.clone());
                            let file = file.display().to_string();
                            start = Some((
                                "Save a backup".into(),
                                Effect::Reads,
                                Box::new(move || o.save_backup(&b, &file)),
                            ));
                        }
                        ui.close();
                    }
                    if ui
                        .button(format!("{} Restore…", icon::CLOCK_COUNTER_CLOCKWISE))
                        .on_hover_text("Bring back books from a backup, proven whole first")
                        .clicked()
                    {
                        self.dialog = Dialog::Restore(Box::default());
                    }
                    if ui
                        .button(format!("{} New books…", icon::BOOK_OPEN))
                        .on_hover_text("Start new, empty books beside these")
                        .clicked()
                    {
                        self.dialog = Dialog::NewBooks {
                            id: String::new(),
                            currency: "USD".into(),
                        };
                    }
                    ui.separator();
                    if ui
                        .button(format!("{} Connect an assistant…", icon::PLUGS_CONNECTED))
                        .on_hover_text("Open the page that connects Claude Code, Codex or another assistant to these books")
                        .clicked()
                    {
                        crate::connect::open_page(ui.ctx());
                        ui.close();
                    }
                    if let Some(upgrade) = &books.format_upgrade {
                        ui.separator();
                        if ui
                            .button(format!("{} Upgrade the books' format…", icon::ARROW_FAT_UP))
                            .on_hover_text("These books wait for a format upgrade: give the go-ahead, with your owner passphrase")
                            .clicked()
                        {
                            self.dialog = Dialog::Upgrade(Box::new(UpgradeForm::new(upgrade.clone())));
                        }
                    }
                    ui.separator();
                    if ui
                        .button(format!("{} Check for updates", icon::ARROWS_CLOCKWISE))
                        .on_hover_text("Look for a newer version of Wanderbooks now")
                        .clicked()
                    {
                        let o = owner.clone();
                        start = Some((
                            "Check for updates".into(),
                            Effect::Changes("Checking for updates"),
                            Box::new(move || o.check_for_updates()),
                        ));
                    }
                    if let Some(u) = &waiting
                        && ui
                            .button(format!("{} Update to {}", icon::ARROW_FAT_UP, u.version))
                            .on_hover_text("Start using the version that is downloaded and checked: the books stay open")
                            .clicked()
                    {
                        let o = owner.clone();
                        start = Some((
                            "Update".into(),
                            Effect::Changes("Updating Wanderbooks"),
                            Box::new(move || o.apply_update()),
                        ));
                    }
                })
            })
            .inner;
        if let Some(start) = start {
            self.start(ui.ctx(), start);
        }
        self.dialogs(ui.ctx(), &owner, &book);
        let said = if self.job.is_some() {
            Some(("Working…".to_string(), p.muted))
        } else {
            self.done.as_ref().map(|(what, done)| match done {
                Ok(lines) => (format!("{what}: {}", lines.join(" ")), p.muted),
                Err(why) => (format!("{what}: {why}"), p.warning),
            })
        };
        match said {
            Some((text, _)) if compact => {
                response.0.on_hover_text(text);
            }
            Some((text, color)) => {
                ui.add(egui::Label::new(RichText::new(text).size(12.0).color(color)).wrap());
            }
            None => {}
        }
    }

    fn start(&mut self, ctx: &egui::Context, (what, effect, work): Start) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let started = self.jobs.spawn(&what.clone(), effect.hold(), move |stop| {
            // A read-only job of a tab that has closed is not begun.
            if matches!(effect, Effect::Reads) && stop.requested() {
                return;
            }
            let _ = tx.send(work());
            ctx.request_repaint();
        });
        if !started {
            self.done = Some((what, Err("the system gave no thread for it".into())));
            return;
        }
        self.job = Some((what, rx));
        self.done = None;
    }

    /// The open dialog, if any, for the books `book`.
    fn dialogs(&mut self, ctx: &egui::Context, owner: &Arc<dyn Owner>, book: &str) {
        let mut close = false;
        let mut run: Option<Start> = None;
        let jobs = self.jobs.clone();
        match &mut self.dialog {
            Dialog::None => {}
            Dialog::Upgrade(form) => {
                let done = form.working.as_ref().and_then(|rx| match rx.try_recv() {
                    Ok(done) => Some(done),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    // Its worker ended without a word (it died): the
                    // dialog, which holds the whole window, isn't left at
                    // work for good.
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(
                        "it ended without an answer, so it may or may not have been done: if these books still wait, try again"
                            .into(),
                    )),
                });
                if let Some(done) = done {
                    form.working = None;
                    match done {
                        Ok(lines) => {
                            self.done = Some(("Format upgrade".into(), Ok(lines)));
                            close = true;
                        }
                        Err(why) => form.problem = Some(why),
                    }
                }
                if !close {
                    upgrade_dialog(
                        ctx,
                        self.scope.unwrap_or(egui::Id::NULL),
                        form,
                        owner,
                        book,
                        &jobs,
                        &mut close,
                    );
                }
            }
            Dialog::NewBooks { id, currency } => {
                egui::Modal::new(self.scope.unwrap_or(egui::Id::NULL).with("owner-new-books"))
                    .show(ctx, |ui| {
                        ui.heading("New books");
                        ui.label("Their id: lowercase letters, digits and hyphens.");
                        ui.text_edit_singleline(id);
                        ui.label("Their base currency (fixed for good):");
                        ui.text_edit_singleline(currency);
                        ui.horizontal(|ui| {
                            let ready = !id.trim().is_empty() && !currency.trim().is_empty();
                            if ui
                                .add_enabled(ready, egui::Button::new("Make them"))
                                .clicked()
                            {
                                let (o, i, c) = (owner.clone(), id.clone(), currency.clone());
                                run = Some((
                                    "New books".into(),
                                    Effect::Changes("Making new books"),
                                    Box::new(move || o.new_books(&i, &c).map(|l| vec![l])),
                                ));
                                close = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            Dialog::Backups(form) => {
                backups_dialog(
                    ctx,
                    self.scope.unwrap_or(egui::Id::NULL),
                    form,
                    owner,
                    book,
                    &jobs,
                    &mut close,
                    &mut run,
                );
            }
            Dialog::Restore(form) => {
                if let Some((rx, asked)) = &form.planning
                    && let Ok(planned) = rx.try_recv()
                {
                    let current = (form.source.clone(), form.id.clone(), form.replace);
                    let still = *asked == current;
                    form.planning = None;
                    match planned {
                        // What was checked is still what is asked.
                        Ok(plan) if still => {
                            form.id = plan.id.clone();
                            form.plan = Some(plan);
                            form.problem = None;
                        }
                        Err(why) if still => form.problem = Some(why),
                        _ => {}
                    }
                }
                egui::Modal::new(self.scope.unwrap_or(egui::Id::NULL).with("owner-restore")).show(ctx, |ui| {
                    ui.set_max_width(520.0);
                    ui.heading("Restore books");
                    ui.label("The backup: your backup folder, the folder of one set of books in it, or a backup file (.wbbackup).");
                    // Not changed while it is being checked.
                    let open = form.planning.is_none();
                    let edited = ui
                        .add_enabled(
                            open,
                            egui::TextEdit::singleline(&mut form.source)
                                .desired_width(f32::INFINITY)
                                .hint_text("/media/backup, or /home/you/acme.wbbackup"),
                        )
                        .changed();
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(open, egui::Button::new("Choose a folder…"))
                            .clicked()
                            && let Some(p) = rfd::FileDialog::new()
                                .set_title("Choose the backup folder")
                                .pick_folder()
                        {
                            form.source = p.display().to_string();
                            form.plan = None;
                            form.problem = None;
                        }
                        if ui
                            .add_enabled(open, egui::Button::new("Choose a file…"))
                            .clicked()
                            && let Some(p) = rfd::FileDialog::new()
                                .set_title("Choose a backup file")
                                .add_filter("Wanderbooks backup", &[backup_extension()])
                                .pick_file()
                        {
                            form.source = p.display().to_string();
                            form.plan = None;
                            form.problem = None;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Passphrase, if the backup has one");
                        if ui
                            .add_enabled(
                                open,
                                egui::TextEdit::singleline(&mut form.passphrase).password(true),
                            )
                            .changed()
                        {
                            form.plan = None;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("As the books");
                        let id_edited = ui
                            .add_enabled(
                                open,
                                egui::TextEdit::singleline(&mut form.id).hint_text("its own books' id"),
                            )
                            .changed();
                        let replace_edited = ui
                            .add_enabled(
                                open,
                                egui::Checkbox::new(&mut form.replace, "replacing them (kept in the legacy folder)"),
                            )
                            .changed();
                        if id_edited || replace_edited {
                            form.plan = None;
                        }
                    });
                    if edited {
                        form.plan = None;
                        form.problem = None;
                    }
                    if let Some(why) = &form.problem {
                        ui.colored_label(crate::theme::pal(ui.ctx()).warning, why);
                    }
                    if let Some(plan) = &form.plan {
                        ui.separator();
                        for line in &plan.lines {
                            // A restore that is not of the newest backup
                            // says so first and in the warning colour.
                            if line.starts_with(OLDER_BACKUP) {
                                ui.colored_label(
                                    crate::theme::pal(ui.ctx()).warning,
                                    egui::RichText::new(line).strong(),
                                );
                            } else {
                                ui.label(line);
                            }
                        }
                    }
                    ui.horizontal(|ui| {
                        let planning = form.planning.is_some();
                        match &form.plan {
                            None => {
                                let ready = !planning && !form.source.trim().is_empty();
                                if ui.add_enabled(ready, egui::Button::new("Check it")).clicked() {
                                    let (tx, rx) = channel();
                                    let (o, s, r) = (owner.clone(), form.source.clone(), form.replace);
                                    let pass = Some(Secret(form.passphrase.clone()))
                                        .filter(|p| !p.0.is_empty());
                                    let id = Some(form.id.trim().to_string()).filter(|i| !i.is_empty());
                                    let ctx = ui.ctx().clone();
                                    // Only reads the backup: stopped with its tab.
                                    let started = jobs.spawn(
                                        "restore check",
                                        crate::jobs::Hold::Free,
                                        move |stop| {
                                            if stop.requested() {
                                                return;
                                            }
                                            let _ = tx.send(o.plan_restore(&s, id, r, pass));
                                            ctx.request_repaint();
                                        },
                                    );
                                    if started {
                                        let asked =
                                            (form.source.clone(), form.id.clone(), form.replace);
                                        form.planning = Some((rx, asked));
                                        form.problem = None;
                                    } else {
                                        form.problem =
                                            Some("the system gave no thread for it".into());
                                    }
                                }
                                if planning {
                                    ui.label("Checking the backup…");
                                }
                            }
                            Some(plan) => {
                                if ui.button("Restore it").clicked() {
                                    let (o, plan) = (owner.clone(), plan.clone());
                                    run = Some((
                                        "Restore".into(),
                                        Effect::Changes("Restoring these books"),
                                        Box::new(move || o.restore(&plan)),
                                    ));
                                    close = true;
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
        }
        if close {
            self.dialog = Dialog::None;
        }
        if let Some(run) = run {
            self.start(ctx, run);
        }
    }
}

/// The extension of a backup file, without the dot.
fn backup_extension() -> &'static str {
    ledger_store::backup::FILE_EXTENSION
}

/// The Backups window: how the books' backups stand, the folder (chosen with
/// the system's folder dialog, or typed), the passphrase offer (off unless
/// asked for, with its warning), and a go.
#[allow(clippy::too_many_arguments)]
fn backups_dialog(
    ctx: &egui::Context,
    scope: egui::Id,
    form: &mut BackupForm,
    owner: &Arc<dyn Owner>,
    book: &str,
    jobs: &crate::jobs::Spawner,
    close: &mut bool,
    run: &mut Option<Start>,
) {
    let p = crate::theme::pal(ctx);
    // Read how they stand, once, and again after a change.
    if form.state.is_none() && form.loading.is_none() {
        let (tx, rx) = channel();
        let (o, b, c) = (owner.clone(), book.to_string(), ctx.clone());
        let started = jobs.spawn("backup state", crate::jobs::Hold::Free, move |stop| {
            if stop.requested() {
                return;
            }
            let _ = tx.send(o.backup_state(&b));
            c.request_repaint();
        });
        if started {
            form.loading = Some(rx);
        } else {
            form.state = Some(Err("the system gave no thread for it".into()));
        }
    }
    if let Some(rx) = &form.loading
        && let Ok(state) = rx.try_recv()
    {
        form.loading = None;
        if let Ok(s) = &state
            && form.folder.is_empty()
        {
            form.folder = s.folder.clone().unwrap_or_default();
        }
        form.state = Some(state);
    }
    egui::Modal::new(scope.with("owner-backups")).show(ctx, |ui| {
        ui.set_max_width(540.0);
        ui.heading("Backups");
        ui.add_space(4.0);
        match &form.state {
            None => {
                ui.label("Reading how the backups stand…");
            }
            Some(Err(why)) => {
                ui.colored_label(p.warning, why);
            }
            Some(Ok(state)) => {
                for line in &state.lines {
                    ui.label(line);
                }
                if state.ahead
                    && ui
                        .button("Start a new backup set here")
                        .on_hover_text("The old set stays in the folder, untouched; these books back up to a new one beside it")
                        .clicked()
                {
                    let (o, b) = (owner.clone(), book.to_string());
                    *run = Some((
                        "Backups".into(),
                        Effect::Changes("Starting a new backup set"),
                        Box::new(move || o.start_new_backup_set(&b)),
                    ));
                    *close = true;
                }
            }
        }
        let sealed = matches!(&form.state, Some(Ok(s)) if s.sealed);
        ui.separator();
        ui.label("Back up to this folder: another disk, or one your cloud drive (Dropbox, iCloud, OneDrive, Google Drive) keeps in sync.");
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut form.folder)
                    .desired_width(330.0)
                    .hint_text("choose a folder"),
            );
            if ui.button("Choose a folder…").clicked()
                && let Some(folder) = rfd::FileDialog::new()
                    .set_title("Choose the backup folder")
                    .pick_folder()
            {
                form.folder = folder.display().to_string();
            }
        });
        ui.add_space(4.0);
        if sealed {
            ui.label("These backups are protected by a passphrase.");
        }
        ui.checkbox(
            &mut form.protect,
            if sealed {
                "Change the passphrase"
            } else {
                "Protect the backups with a passphrase (optional)"
            },
        );
        if form.protect {
            ui.label(PASSPHRASE_WARNING);
            ui.add(
                egui::TextEdit::singleline(&mut form.pass)
                    .password(true)
                    .hint_text("passphrase (at least 12 characters)")
                    .desired_width(f32::INFINITY),
            );
            ui.add(
                egui::TextEdit::singleline(&mut form.again)
                    .password(true)
                    .hint_text("the same passphrase again")
                    .desired_width(f32::INFINITY),
            );
        }
        if let Some(why) = &form.problem {
            ui.colored_label(p.warning, why);
        }
        for line in &form.said {
            ui.label(line);
        }
        ui.horizontal(|ui| {
            let folder = form.folder.trim().to_string();
            let known = matches!(&form.state, Some(Ok(s)) if s.folder.as_deref() == Some(folder.as_str()));
            let pass_ready = !form.protect
                || (form.pass.chars().count() >= 12 && form.pass == form.again);
            let wants = (!folder.is_empty() && !known) || form.protect;
            if ui
                .add_enabled(wants && pass_ready, egui::Button::new("Save these choices"))
                .on_disabled_hover_text(
                    "Choose a folder, or set a passphrase of at least 12 characters, typed twice the same",
                )
                .clicked()
            {
                let (o, b) = (owner.clone(), book.to_string());
                let pass = form
                    .protect
                    .then(|| Secret(std::mem::take(&mut form.pass)))
                    .filter(|s| !s.0.is_empty());
                zeroize::Zeroize::zeroize(&mut form.again);
                form.protect = false;
                let folder = Some(folder).filter(|f| !f.is_empty() && !known);
                *run = Some((
                    "Backups".into(),
                    Effect::Changes("Setting up the backups"),
                    Box::new(move || o.set_up_backups(&b, folder, pass, false)),
                ));
                *close = true;
            }
            if sealed
                && ui
                    .button("Take the passphrase off")
                    .on_hover_text("Later backups are written without protection; earlier sealed ones stay sealed")
                    .clicked()
            {
                let (o, b) = (owner.clone(), book.to_string());
                *run = Some((
                    "Backups".into(),
                    Effect::Changes("Setting up the backups"),
                    Box::new(move || o.set_up_backups(&b, None, None, true)),
                ));
                *close = true;
            }
            if ui.button("Close").clicked() {
                *close = true;
            }
        });
    });
}

/// The format upgrade's dialog (INVARIANT 5): what it is, the owner
/// passphrase, and the go-ahead or not now.
fn upgrade_dialog(
    ctx: &egui::Context,
    scope: egui::Id,
    form: &mut UpgradeForm,
    owner: &Arc<dyn Owner>,
    book: &str,
    jobs: &crate::jobs::Spawner,
    close: &mut bool,
) {
    let p = crate::theme::pal(ctx);
    egui::Modal::new(scope.with("owner-upgrade")).show(ctx, |ui| {
        ui.set_max_width(520.0);
        ui.heading("Upgrade these books' format?");
        ui.add_space(4.0);
        ui.label(format!(
            "An older version of Wanderbooks wrote these books, in journal format {}. The upgrade adds {}.",
            form.upgrade.from.format,
            form.upgrade.adds().join(", ")
        ));
        ui.label("Until you agree they work as they are: everything that needs nothing new works, and what needs the new format is refused.");
        ui.label("Upgrading first makes a backup of the books, their vault and keys included, and proves it whole; then it moves them to the new format, in one step. After that an older version of wanderbooks can't open them.");
        ui.add_space(6.0);
        let working = form.working.is_some();
        ui.label("Your owner passphrase:");
        let field = ui.add_enabled(
            !working,
            egui::TextEdit::singleline(&mut form.passphrase)
                .password(true)
                .desired_width(f32::INFINITY),
        );
        if let Some(why) = &form.problem {
            ui.colored_label(p.warning, why);
        }
        let go = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            let ready = !working && !form.passphrase.is_empty();
            if (ui
                .add_enabled(ready, egui::Button::new("Upgrade"))
                .clicked()
                || (go && ready))
                && form.working.is_none()
            {
                // Handed over, and gone from the form (wiped once used).
                let passphrase = zeroize::Zeroizing::new(std::mem::take(&mut form.passphrase));
                let (o, b) = (owner.clone(), book.to_string());
                let (tx, rx) = channel();
                let ctx = ui.ctx().clone();
                // Changes the books' format: never abandoned (INVARIANT 6).
                let started = jobs.spawn(
                    "format upgrade",
                    crate::jobs::Hold::Keep(
                        "Upgrading these books' format: wait for it to finish".into(),
                    ),
                    move |_| {
                        // The window is woken however this ends (a panic
                        // too), once the answer, or its absence, can be seen.
                        let _wake = Wake(ctx);
                        let (tx, passphrase) = (tx, passphrase);
                        let _ = tx.send(o.upgrade_format(&b, &passphrase));
                    },
                );
                if started {
                    form.working = Some(rx);
                    form.problem = None;
                } else {
                    form.problem = Some("the system gave no thread for it".into());
                }
            }
            if ui
                .add_enabled(!working, egui::Button::new("Not now"))
                .on_hover_text("They keep working as they are; you'll be asked again the next time they open")
                .clicked()
            {
                *close = true;
            }
            if working {
                ui.label("Backing up, then upgrading…");
            }
        });
    });
}
