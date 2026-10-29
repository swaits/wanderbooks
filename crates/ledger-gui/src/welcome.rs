//! The first run (plan P11b, found by P11a: "onboards and posts an entry
//! with no terminal" was false while the viewer opened only an existing
//! book and `setup` and `books create` needed a terminal): the page the
//! viewer shows when there are no books yet. It sets up the books' private
//! folder, asks for the owner passphrase, makes the first books and connects
//! an agent, each through the [`Owner`], then opens the books.
//!
//! ```text
//! GOAL: a person who installed the app and opened it, with no terminal
//!       and no admin password, has books, an agent that can keep them and
//!       the view of them, in minutes.
//! SOURCE: plan/served.md P11a As built (Done-when 1: the AppImage "onboards
//!         and posts an entry with no terminal"), P11b; D-19 (the viewer's
//!         "New books…" and "Connect" menu items; one app); D-27 (the books
//!         home, the dock, never required); D-29 (no passphrase handling
//!         beyond the owner's own: the owner passphrase is typed here and
//!         handed to the owner, as the upgrade dialog does).
//! INVARIANTS:
//!   1. The page only asks the [`Owner`]; nothing here writes a file of the
//!      books or of the data root.
//!   2. Each step is shown from what exists now ([`FirstRun`]): a step done
//!      (by the command line, say) is not asked again, and the page opens at
//!      the first step missing.
//!   3. The owner passphrase may be left for later (the books work without
//!      it); an agent may be left for later, or never: any agent reaches the
//!      books through the MCP.
//!   4. A passphrase typed is wiped from memory once handed over, or when the
//!      page goes.
//!   5. Each step runs off the UI thread and says what came of it, in
//!      words; a refusal stays on the page, with the step.
//!   6. The books are opened only once they exist.
//! ```

use crate::owner::{Agent, AgentStart, FirstRun, Owner};
use egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

/// The least an owner passphrase may be (the store's rule, said here).
pub const MIN_PASSPHRASE: usize = ledger_store::passphrase::MIN_CHARS;

/// What a step's worker sends back: what it said (or why it was refused),
/// and where the first run stands now.
type Done = (Result<Vec<String>, String>, FirstRun);

/// The first-run page's state.
pub struct Welcome {
    state: FirstRun,
    /// The id and currency typed for the first books.
    id: String,
    currency: String,
    /// The passphrase typed, twice.
    pass: String,
    again: String,
    /// The passphrase step was left for later.
    pass_later: bool,
    /// The agents asked for, and what came of each.
    agents: Vec<(Agent, Result<Vec<String>, String>)>,
    /// The agents may be left for later.
    agent_later: bool,
    /// A step under way: what it is, the agent it connects (if it does),
    /// and its answer.
    job: Option<(String, Option<Agent>, Receiver<Done>)>,
    /// What the last step said or why it was refused.
    said: Option<Result<Vec<String>, String>>,
    /// The books to open, once the person says.
    open: Option<String>,
}

impl Drop for Welcome {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.pass);
        zeroize::Zeroize::zeroize(&mut self.again);
    }
}

impl Welcome {
    /// The page for a first run that stands at `state`.
    pub fn new(state: FirstRun) -> Self {
        Self {
            state,
            id: String::new(),
            currency: "USD".into(),
            pass: String::new(),
            again: String::new(),
            pass_later: false,
            agents: Vec::new(),
            agent_later: false,
            job: None,
            said: None,
            open: None,
        }
    }

    /// Where the first run stands.
    pub fn state(&self) -> &FirstRun {
        &self.state
    }

    /// The books the person chose to open, once.
    pub fn take_open(&mut self) -> Option<String> {
        self.open.take()
    }

    /// Show why the books could not be opened.
    pub fn refused(&mut self, why: String) {
        self.said = Some(Err(why));
    }

    fn run(
        &mut self,
        ctx: &egui::Context,
        what: &str,
        agent: Option<Agent>,
        owner: &Arc<dyn Owner>,
        work: impl FnOnce(&dyn Owner) -> Result<Vec<String>, String> + Send + 'static,
    ) {
        let (tx, rx) = channel();
        let (o, ctx) = (owner.clone(), ctx.clone());
        std::thread::spawn(move || {
            let done = work(o.as_ref());
            let _ = tx.send((done, o.first_run()));
            ctx.request_repaint();
        });
        self.job = Some((what.to_string(), agent, rx));
        self.said = None;
    }

    /// The page, in the window's central area.
    pub fn show(&mut self, ui: &mut Ui, owner: &Arc<dyn Owner>) {
        let p = crate::theme::pal(ui.ctx());
        if let Some((_, agent, rx)) = &self.job {
            let agent = *agent;
            match rx.try_recv() {
                Ok((done, state)) => {
                    self.job = None;
                    self.state = state;
                    if let Some(agent) = agent {
                        self.agents.retain(|(a, _)| *a != agent);
                        self.agents.push((agent, done.clone()));
                    }
                    self.said = Some(done);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.job = None;
                    self.said = Some(Err(
                        "it ended without an answer: look again at what exists, and try again"
                            .into(),
                    ));
                    self.state = owner.first_run();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let busy = self.job.is_some();
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.set_max_width(640.0);
                ui.add_space(36.0);
                ui.label(RichText::new("Welcome to wanderbooks").font(crate::theme::bold(28.0)));
                ui.label(
                    RichText::new(
                        "Your company's books, kept by an agent and checked by you. A few short steps; nothing here needs a terminal.",
                    )
                    .color(p.muted),
                );
                ui.add_space(18.0);
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    self.steps(ui, owner, busy);
                });
            });
        });
    }

    /// A step's frame: its number, title and whether it is done.
    fn card(
        ui: &mut Ui,
        n: usize,
        title: &str,
        done: bool,
        active: bool,
        body: impl FnOnce(&mut Ui),
    ) {
        let p = crate::theme::pal(ui.ctx());
        egui::Frame::new()
            .fill(p.surface)
            .stroke(egui::Stroke::new(
                1.0,
                if active { p.accent } else { p.border },
            ))
            .corner_radius(10)
            .inner_margin(egui::Margin::symmetric(18, 14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (mark, color) = if done {
                        (icon::CHECK_CIRCLE.to_string(), p.positive)
                    } else {
                        (format!("{n}."), if active { p.accent } else { p.faint })
                    };
                    ui.label(
                        RichText::new(mark)
                            .font(crate::theme::semibold(16.0))
                            .color(color),
                    );
                    ui.label(RichText::new(title).font(crate::theme::semibold(16.0)));
                });
                if active || done {
                    body(ui);
                }
            });
        ui.add_space(10.0);
    }

    fn steps(&mut self, ui: &mut Ui, owner: &Arc<dyn Owner>, busy: bool) {
        let p = crate::theme::pal(ui.ctx());
        let st = self.state.clone();
        let have_books = !st.books.is_empty();
        // The first step missing is the one asked for (INVARIANT 2).
        let now = if !st.root {
            1
        } else if !st.passphrase_set && !self.pass_later {
            2
        } else if !have_books {
            3
        } else if self.agents.iter().all(|(_, r)| r.is_err()) && !self.agent_later {
            4
        } else {
            5
        };

        // 1. The private folder.
        Self::card(
            ui,
            1,
            "A private place for your books",
            st.root,
            now == 1,
            |ui| {
                if st.root {
                    ui.label(RichText::new("Made. Only your account can open it.").color(p.muted));
                } else {
                    ui.label("Wanderbooks keeps your books in a folder that only your account can open. Your agent reaches them through the app, never by looking in the folder.");
                    if ui
                        .add_enabled(!busy, egui::Button::new("Set it up"))
                        .clicked()
                    {
                        let ctx = ui.ctx().clone();
                        self.run(&ctx, "Setting up", None, owner, |o| o.set_up());
                    }
                }
            },
        );

        // 2. The owner passphrase.
        Self::card(
            ui,
            2,
            "Your owner passphrase",
            st.passphrase_set,
            now == 2,
            |ui| {
                if st.passphrase_set {
                    ui.label(RichText::new("Set.").color(p.muted));
                    return;
                }
                ui.label(format!(
                    "The books ask for it before anything only you may do, like moving them to a newer format. Pick one you will remember: {MIN_PASSPHRASE} characters or more. It is not kept anywhere you can read."
                ));
                ui.add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.pass)
                        .password(true)
                        .hint_text("passphrase")
                        .desired_width(f32::INFINITY),
                );
                ui.add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.again)
                        .password(true)
                        .hint_text("the same again")
                        .desired_width(f32::INFINITY),
                );
                let long = self.pass.chars().count() >= MIN_PASSPHRASE;
                let same = self.pass == self.again;
                if !self.pass.is_empty() && !long {
                    ui.colored_label(
                        p.warning,
                        format!("{MIN_PASSPHRASE} characters or more, please."),
                    );
                } else if !self.again.is_empty() && !same {
                    ui.colored_label(p.warning, "The two don't match.");
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!busy && long && same, egui::Button::new("Set passphrase"))
                        .clicked()
                    {
                        // Handed over, and gone from the page (INVARIANT 4).
                        let typed = zeroize::Zeroizing::new(std::mem::take(&mut self.pass));
                        zeroize::Zeroize::zeroize(&mut self.again);
                        let ctx = ui.ctx().clone();
                        self.run(&ctx, "Setting the passphrase", None, owner, move |o| {
                            o.set_passphrase(&typed)
                                .map(|()| vec!["The owner passphrase is set.".to_string()])
                        });
                    }
                    if ui
                        .add_enabled(!busy, egui::Button::new("Not now"))
                        .on_hover_text(
                            "The books work without it; `wanderbooks passphrase` sets it later",
                        )
                        .clicked()
                    {
                        self.pass_later = true;
                    }
                });
            },
        );

        // 3. The first books.
        let ready = st.root;
        Self::card(
            ui,
            3,
            "Your first books",
            have_books,
            ready && now == 3,
            |ui| {
                if have_books {
                    ui.label(
                        RichText::new(format!("Made: {}.", st.books.join(", "))).color(p.muted),
                    );
                    return;
                }
                ui.label("A short name for the books: lowercase letters, digits and hyphens (your company's name, say). You can name the company itself afterwards, with your agent.");
                ui.add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.id)
                        .hint_text("my-company")
                        .desired_width(f32::INFINITY),
                );
                ui.label("Their base currency (fixed for good):");
                ui.add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.currency).desired_width(120.0),
                );
                let ok = !self.id.trim().is_empty() && !self.currency.trim().is_empty();
                if ui
                    .add_enabled(!busy && ok, egui::Button::new("Make the books"))
                    .clicked()
                {
                    let (id, cur) = (self.id.trim().to_string(), self.currency.trim().to_string());
                    let ctx = ui.ctx().clone();
                    self.run(&ctx, "Making the books", None, owner, move |o| {
                        let made = o.new_books(&id, &cur)?;
                        // Held open while the viewer runs.
                        o.hold(&id);
                        Ok(vec![made])
                    });
                }
            },
        );

        // 4. An agent.
        let book = st.books.first().cloned();
        let step4_active = have_books && now == 4;
        let agents = self.agents.clone();
        Self::card(
            ui,
            4,
            "An agent to keep them",
            agents.iter().any(|(_, r)| r.is_ok()),
            step4_active,
            |ui| {
                ui.label("An agent (Claude Code or Codex) records your transactions, checks receipts and answers questions, through the books' own tools. This connects it; the app also has a panel along the bottom of its window that runs it for you. You never need either: any agent you run yourself can reach the same books.");
                for (agent, result) in &agents {
                    match result {
                        Ok(lines) => {
                            ui.label(RichText::new(lines.join(" ")).color(p.positive));
                        }
                        Err(why) => {
                            ui.colored_label(p.warning, format!("{}: {why}", agent.label()));
                        }
                    }
                }
                let Some(book) = book.clone() else {
                    return;
                };
                ui.horizontal(|ui| {
                    for agent in [Agent::Claude, Agent::Codex] {
                        if ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(format!("Connect {}", agent.label())),
                            )
                            .clicked()
                        {
                            let b = book.clone();
                            let ctx = ui.ctx().clone();
                            self.run(&ctx, "Connecting", Some(agent), owner, move |o| {
                                match o.connect(agent, &b) {
                                    Ok(lines) => Ok(lines),
                                    // Not installed: how to get it.
                                    Err(why) => match o.agent_start(agent, &b) {
                                        Ok(AgentStart::Missing(i)) => Err(format!(
                                            "{why}. Install it with `{}` in a terminal ({}), then connect it here.",
                                            i.command, i.page
                                        )),
                                        _ => Err(why),
                                    },
                                }
                            });
                        }
                    }
                    if ui
                        .add_enabled(!busy, egui::Button::new("I'll use my own agent"))
                        .clicked()
                    {
                        self.agent_later = true;
                    }
                });
            },
        );

        // 5. Open.
        if have_books {
            let id = st.books[0].clone();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(
                            RichText::new(format!("{} Open the books", icon::BOOK_OPEN))
                                .font(crate::theme::semibold(14.0)),
                        )
                        .min_size(egui::vec2(180.0, 36.0)),
                    )
                    .clicked()
                {
                    self.open = Some(id);
                }
            });
        }

        // What the last step said.
        if let Some((what, _, _)) = &self.job {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("{what}…")).color(p.muted));
        } else if let Some(said) = &self.said {
            ui.add_space(8.0);
            match said {
                Ok(lines) => ui.label(RichText::new(lines.join(" ")).color(p.muted)),
                Err(why) => ui.colored_label(p.warning, format!("{} {why}", icon::WARNING)),
            };
        }
    }
}
