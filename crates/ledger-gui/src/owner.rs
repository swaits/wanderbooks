//! The viewer's Books menu (plan P2 "Viewer menu", D-19, D-32): the
//! owner's own actions on the books, handed to the books server (back up
//! now, restore, new books, close the books, the format upgrade) or to the
//! owner's agent (connect). The viewer itself still never writes the
//! books.
//!
//! ```text
//! GOAL: the owner backs up, restores, starts new books, closes the books
//!       and connects an agent from the viewer, without a terminal.
//! SOURCE: plan P2 ("Viewer menu (D-19)": "Back up now", "Restore…",
//!         "New books…", "Close the books", "Connect Claude Code" and
//!         "Connect Codex"; "Nothing in the viewer writes a book"); D-32
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

/// A restore previewed: what it would do, and what it is of (INVARIANT 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestorePlan {
    /// The backup, as the server is told it.
    pub source: String,
    /// The books' id it comes back as.
    pub id: String,
    /// Whether it replaces the books of that id (kept in legacy/).
    pub replace: bool,
    /// The backup's head, proven.
    pub head: (u64, String),
    /// What it would do, a line each.
    pub lines: Vec<String>,
}

/// The owner's actions, done by the program the viewer runs in (ledger-cli
/// over the books server and the agents' own commands). Each blocks: the
/// menu calls it off the UI thread.
pub trait Owner: Send + Sync {
    /// Close the books: everything holding them lets go.
    fn close_books(&self) -> Result<String, String>;
    /// New, empty books `id` in `currency`.
    fn new_books(&self, id: &str, currency: &str) -> Result<String, String>;
    /// Back the books `book` up now.
    fn back_up_now(&self, book: &str) -> Result<Vec<String>, String>;
    /// Preview restoring the backup at `source` (as `id`, else its own
    /// books' id), replacing the books of that id or not.
    fn plan_restore(
        &self,
        source: &str,
        id: Option<String>,
        replace: bool,
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
    /// How to run `agent` in the books home of `book` (made if need be): the
    /// dock starts it on a terminal of its own (D-27).
    fn agent_start(&self, _agent: Agent, _book: &str) -> Result<AgentStart, String> {
        Err("not here".into())
    }
}

static OWNER: OnceLock<Arc<dyn Owner>> = OnceLock::new();

/// Give the viewer its owner (once, at start; INVARIANT 4).
pub fn set_owner(owner: Arc<dyn Owner>) {
    let _ = OWNER.set(owner);
}

/// The viewer's owner, if the program gave it one.
pub fn current() -> Option<Arc<dyn Owner>> {
    owner()
}

fn owner() -> Option<Arc<dyn Owner>> {
    OWNER.get().cloned()
}

/// The id of books of the data root (their folder's name), or None for
/// books outside one, the demo's (INVARIANT 4).
pub fn book_id(books: &Books) -> Option<String> {
    ledger_store::root::inside_a_root(&books.data_dir)
        .then(|| books.data_dir.file_name()?.to_str().map(str::to_string))
        .flatten()
}

type Outcome = Result<Vec<String>, String>;

/// A dialog open over the viewer.
#[derive(Default)]
enum Dialog {
    #[default]
    None,
    /// "New books…": their id and currency.
    NewBooks { id: String, currency: String },
    /// "Restore…": the backup, then its preview.
    Restore(Box<RestoreForm>),
    /// "Close the books": asked first.
    Close,
    /// The format upgrade: the go-ahead, with the owner passphrase.
    Upgrade(Box<UpgradeForm>),
}

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
    /// A check under way, and what it was asked (a result for anything
    /// since edited is dropped).
    planning: Option<Checking>,
    plan: Option<RestorePlan>,
    problem: Option<String>,
}

/// The menu's state: a dialog, the job under way, and the last outcome.
#[derive(Default)]
pub struct OwnerMenu {
    /// The books last shown (by identity) and their id: found once for
    /// each load, not every frame.
    book: Option<(usize, Option<String>)>,
    dialog: Dialog,
    job: Option<(String, Receiver<Outcome>)>,
    done: Option<(String, Outcome)>,
    /// The books whose format upgrade was asked for since the viewer
    /// opened them (INVARIANT 5: once each open).
    asked_upgrade: Option<String>,
}

impl OwnerMenu {
    /// The id of the books shown, if they are books of a data root: found
    /// once for each load.
    fn book_of(&mut self, books: &Arc<Books>) -> Option<String> {
        let key = Arc::as_ptr(books) as usize;
        if self.book.as_ref().is_none_or(|(k, _)| *k != key) {
            self.book = Some((key, book_id(books)));
        }
        self.book.as_ref().and_then(|(_, id)| id.clone())
    }

    /// The standing notice of a format upgrade the books wait for, above
    /// the page, and the go-ahead asked for once each open (INVARIANT 5).
    pub fn notice(&mut self, ui: &mut Ui, books: &Arc<Books>) {
        if owner().is_none() {
            return;
        }
        let Some(book) = self.book_of(books) else {
            return;
        };
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
                            "An older version of wanderbooks wrote them. They work on their old format until you agree; what needs the new one is refused.",
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
    pub fn ui(&mut self, ui: &mut Ui, books: &Arc<Books>, compact: bool) {
        let Some(owner) = owner() else {
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
        let mut start: Option<(String, Box<dyn FnOnce() -> Outcome + Send>)> = None;
        let response = ui
            .add_enabled_ui(!busy, |ui| {
                egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                    ui.set_min_width(240.0);
                    if ui
                        .button(format!("{} Back up now", icon::CLOUD_ARROW_UP))
                        .on_hover_text("Back these books up now, to every backup their settings name")
                        .clicked()
                    {
                        let (o, b) = (owner.clone(), book.clone());
                        start = Some(("Back up now".into(), Box::new(move || o.back_up_now(&b))));
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
                    for agent in [Agent::Claude, Agent::Codex] {
                        if ui
                            .button(format!("{} Connect {}", icon::PLUGS_CONNECTED, agent.label()))
                            .on_hover_text(format!(
                                "Let {} reach these books (it keeps their token in a file in the data root)",
                                agent.label()
                            ))
                            .clicked()
                        {
                            let (o, b) = (owner.clone(), book.clone());
                            start = Some((
                                format!("Connect {}", agent.label()),
                                Box::new(move || o.connect(agent, &b)),
                            ));
                        }
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
                        .button(format!("{} Close the books", icon::POWER))
                        .on_hover_text("Close the books now: agents' sessions and every other holder let go")
                        .clicked()
                    {
                        self.dialog = Dialog::Close;
                    }
                })
            })
            .inner;
        if let Some((what, work)) = start {
            self.start(ui.ctx(), what, work);
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

    fn start(
        &mut self,
        ctx: &egui::Context,
        what: String,
        work: Box<dyn FnOnce() -> Outcome + Send>,
    ) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
        self.job = Some((what, rx));
        self.done = None;
    }

    /// The open dialog, if any, for the books `book`.
    fn dialogs(&mut self, ctx: &egui::Context, owner: &Arc<dyn Owner>, book: &str) {
        let mut close = false;
        let mut run: Option<(String, Box<dyn FnOnce() -> Outcome + Send>)> = None;
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
                    upgrade_dialog(ctx, form, owner, book, &mut close);
                }
            }
            Dialog::Close => {
                egui::Modal::new(egui::Id::new("owner-close")).show(ctx, |ui| {
                    ui.heading("Close the books?");
                    ui.label("Agents' sessions and everything else holding them let go. They open again when something holds them.");
                    ui.horizontal(|ui| {
                        if ui.button("Close the books").clicked() {
                            let o = owner.clone();
                            run = Some(("Close the books".into(), Box::new(move || o.close_books().map(|l| vec![l]))));
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::NewBooks { id, currency } => {
                egui::Modal::new(egui::Id::new("owner-new-books")).show(ctx, |ui| {
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
                egui::Modal::new(egui::Id::new("owner-restore")).show(ctx, |ui| {
                    ui.set_max_width(520.0);
                    ui.heading("Restore books");
                    ui.label("The backup: its repository's address, or its folder (the books' .git folder in your backup folder).");
                    // Not changed while it is being checked.
                    let open = form.planning.is_none();
                    let edited = ui
                        .add_enabled(
                            open,
                            egui::TextEdit::singleline(&mut form.source)
                                .desired_width(f32::INFINITY)
                                .hint_text("/media/backup/acme.git, or git@host:you/books.git"),
                        )
                        .changed();
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
                            ui.label(line);
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
                                    let id = Some(form.id.trim().to_string()).filter(|i| !i.is_empty());
                                    let ctx = ui.ctx().clone();
                                    std::thread::spawn(move || {
                                        let _ = tx.send(o.plan_restore(&s, id, r));
                                        ctx.request_repaint();
                                    });
                                    let asked = (form.source.clone(), form.id.clone(), form.replace);
                                    form.planning = Some((rx, asked));
                                    form.problem = None;
                                }
                                if planning {
                                    ui.label("Checking the backup…");
                                }
                            }
                            Some(plan) => {
                                if ui.button("Restore it").clicked() {
                                    let (o, plan) = (owner.clone(), plan.clone());
                                    run = Some(("Restore".into(), Box::new(move || o.restore(&plan))));
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
        if let Some((what, work)) = run {
            self.start(ctx, what, work);
        }
    }
}

/// The format upgrade's dialog (INVARIANT 5): what it is, the owner
/// passphrase, and the go-ahead or not now.
fn upgrade_dialog(
    ctx: &egui::Context,
    form: &mut UpgradeForm,
    owner: &Arc<dyn Owner>,
    book: &str,
    close: &mut bool,
) {
    let p = crate::theme::pal(ctx);
    egui::Modal::new(egui::Id::new("owner-upgrade")).show(ctx, |ui| {
        ui.set_max_width(520.0);
        ui.heading("Upgrade these books' format?");
        ui.add_space(4.0);
        ui.label(format!(
            "An older version of wanderbooks wrote these books, in journal format {}. The upgrade adds {}.",
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
                std::thread::spawn(move || {
                    // The window is woken however this ends (a panic
                    // too), once the answer, or its absence, can be seen.
                    let _wake = Wake(ctx);
                    let (tx, passphrase) = (tx, passphrase);
                    let _ = tx.send(o.upgrade_format(&b, &passphrase));
                });
                form.working = Some(rx);
                form.problem = None;
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
