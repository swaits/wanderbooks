//! The viewer's Books menu (plan P2 "Viewer menu", D-19, D-32): the
//! owner's own actions on the books, handed to the books server (back up
//! now, restore, new books, close the books) or to the owner's agent
//! (connect). The viewer itself still never writes the books.
//!
//! ```text
//! GOAL: the owner backs up, restores, starts new books, closes the books
//!       and connects an agent from the viewer, without a terminal.
//! SOURCE: plan P2 ("Viewer menu (D-19)": "Back up now", "Restore…",
//!         "New books…", "Close the books", "Connect Claude Code" and
//!         "Connect Codex"; "Nothing in the viewer writes a book"); D-32
//!         (no confinement: the viewer asks the server itself, as the
//!         command line does).
//! INVARIANTS:
//!   1. Every action goes through the [`Owner`] the program gives the
//!      viewer (the books server, or the agent's own command): nothing
//!      here writes a file of the books.
//!   2. Each runs off the UI thread; what came of it is shown, in words.
//!   3. A restore is shown before it is done (the backup proven, what it
//!      would do), and done only for that backup, at that head.
//!   4. Without an owner (the viewer on its own, older books dirs, the
//!      demo) the menu isn't there.
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
    fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
        }
    }
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
}

static OWNER: OnceLock<Arc<dyn Owner>> = OnceLock::new();

/// Give the viewer its owner (once, at start; INVARIANT 4).
pub fn set_owner(owner: Arc<dyn Owner>) {
    let _ = OWNER.set(owner);
}

fn owner() -> Option<Arc<dyn Owner>> {
    OWNER.get().cloned()
}

/// The id of books of the data root (their folder's name), or None for
/// an older books dir (INVARIANT 4).
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
}

impl OwnerMenu {
    /// The menu button and a note of the last outcome; its dialogs.
    pub fn ui(&mut self, ui: &mut Ui, books: &Arc<Books>, compact: bool) {
        let Some(owner) = owner() else {
            return;
        };
        let key = Arc::as_ptr(books) as usize;
        if self.book.as_ref().is_none_or(|(k, _)| *k != key) {
            self.book = Some((key, book_id(books)));
        }
        let Some(book) = self.book.as_ref().and_then(|(_, id)| id.clone()) else {
            return;
        };
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
            button = button.min_size(vec2(32.0, 32.0));
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
                                "Let {} reach these books (it keeps their token in this computer's keyring)",
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
        self.dialogs(ui.ctx(), &owner);
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

    /// The open dialog, if any.
    fn dialogs(&mut self, ctx: &egui::Context, owner: &Arc<dyn Owner>) {
        let mut close = false;
        let mut run: Option<(String, Box<dyn FnOnce() -> Outcome + Send>)> = None;
        match &mut self.dialog {
            Dialog::None => {}
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
