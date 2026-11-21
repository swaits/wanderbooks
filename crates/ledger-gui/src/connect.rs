//! What the window says about assistants: the state mark, the top bar's
//! assistant state, "Ask your assistant", and the Connect page (plan P15
//! 3.3). The window only reads the books; to change anything a person asks
//! an assistant, and these are where it says so. The word is "assistant".
//!
//! ```text
//! GOAL: a person who has never heard of any of this can see that they may
//!       read everything here, that to change anything they ask an
//!       assistant, whether one is connected, and how to connect one; and
//!       they can end an assistant's access.
//! SOURCE: plan/p15-app-and-relay.md 3.3 (the word is "assistant"; every
//!         tab carries one of three states as a shape and a word; "Ask your
//!         assistant" where a button that edits would be, copying a ready
//!         sentence; Connect is a page per company: on this computer, a row
//!         per tool, and the connections with Revoke); D-77 (bookkeeping is
//!         through an assistant, the window reads); WB-assist-1,
//!         WB-assist-3, WB-assist-4.
//! INVARIANTS:
//!   1. One place per fact: a tab carries the state as a shape (the same
//!      three shapes everywhere: plugs apart, plugs together, a turning
//!      arc), with the words in its hover; the front company's state is
//!      said in words once, at the right of its top bar, with what it last
//!      did; the Connect page lists the keys and who is on each. None of
//!      them repeats the company's name or the status card's "Live". The
//!      state takes no height: the sidebar's links must all fit at
//!      1479x519 (`tests/sidenav.rs`), so nothing is added above them.
//!   2. The state is never colour alone: each has its own shape and its
//!      own word; colour only adds.
//!   3. "Ask your assistant" copies one ready sentence naming the company
//!      and what the page is for ("In my Wanderbooks books for Acme,
//!      please create a new invoice."); with no assistant connected it
//!      says so and where to connect one. It writes nothing: it only puts
//!      the sentence on the clipboard.
//!   4. The Connect page shows exact instructions, and only ones this
//!      build can stand behind: Claude Code and Codex have a button that
//!      does what `wanderbooks connect` does and the command it runs; for
//!      any other tool the page gives the program and arguments an
//!      assistant must run, and says the tool is untested. No config format
//!      is shown that the repository does not document.
//!   5. Revoke asks once, then calls what `wanderbooks token revoke` calls
//!      (the owner's `serve_revoke`); the window writes no books. Its job
//!      holds the window open until it ends (owner.rs INVARIANT 6).
//!   6. Nothing here blocks the frame: every request of the server runs
//!      off the UI thread; what is shown while it is not known is "not
//!      known", never a guess.
//!   7. The web half of the page (the switch, the address, the steps for
//!      Claude and ChatGPT) is unit B3's: `State::web_section` is its
//!      place, and it draws nothing until then.
//! ```

use crate::connections::{Connection, Link, Status, describe, program_name};
use crate::owner::Owner;
use crate::theme::{self, pal};
use egui::{Align, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

// ---- the shape of a state ---------------------------------------------------

/// Paint the mark of `link` centred on `at`, `size` points across: plugs
/// apart (no assistant), plugs together (connected), a turning arc
/// (working). Nothing for `Link::Unknown`.
pub fn paint_mark(ui: &Ui, at: egui::Pos2, size: f32, link: Link) {
    let p = pal(ui.ctx());
    match link {
        Link::Unknown => {}
        Link::None => {
            ui.painter().text(
                at,
                egui::Align2::CENTER_CENTER,
                icon::PLUGS,
                egui::FontId::proportional(size),
                p.faint,
            );
        }
        Link::Connected => {
            ui.painter().text(
                at,
                egui::Align2::CENTER_CENTER,
                icon::PLUGS_CONNECTED,
                egui::FontId::proportional(size),
                p.positive,
            );
        }
        Link::Working => {
            // A three-quarter arc, turning.
            let r = size * 0.36;
            let turn = (ui.input(|i| i.time) * 1.6).fract() as f32 * std::f32::consts::TAU;
            let points: Vec<egui::Pos2> = (0..=18)
                .map(|k| {
                    let a = turn + k as f32 / 18.0 * std::f32::consts::TAU * 0.75;
                    at + vec2(a.cos(), a.sin()) * r
                })
                .collect();
            ui.painter()
                .add(egui::Shape::line(points, Stroke::new(2.0, p.accent)));
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}

// ---- the state in the top bar -------------------------------------------------

/// How much of the state the top bar has room for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Room {
    /// The shape, the words and what was last done (two lines).
    Full,
    /// The shape and the words.
    Word,
    /// The shape alone (the words are its hover).
    Icon,
}

impl Room {
    /// How wide the state takes, in points.
    pub fn width(self) -> f32 {
        match self {
            Room::Full => 272.0,
            Room::Word => 196.0,
            Room::Icon => 30.0,
        }
    }
}

/// The state of the front company in the top bar, at the right, where a
/// status is looked for: its shape, the state in words and what was last
/// done, or how to connect one. True when it was clicked (the Connect page
/// is wanted). Nothing for a window that runs no server. It costs no height:
/// the sidebar's links must fit at 1479x519 (`tests/sidenav.rs`), so nothing
/// is added above them.
pub fn chip(ui: &mut Ui, status: &Status, room: Room) -> bool {
    if !status.serves {
        return false;
    }
    let p = pal(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(room.width(), 32.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), p.hover);
    }
    let known = status.link != Link::Unknown;
    paint_mark(
        ui,
        egui::pos2(rect.left() + 16.0, rect.center().y),
        18.0,
        status.link,
    );
    let text_x = rect.left() + 34.0;
    let room_w = (rect.right() - 6.0 - text_x).max(10.0);
    let line = |ui: &Ui, text: String, font: egui::FontId, color: egui::Color32, y: f32| {
        let mut job =
            egui::text::LayoutJob::single_section(text, egui::TextFormat::simple(font, color));
        job.wrap.max_width = room_w;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let g = ui.painter().layout_job(job);
        ui.painter()
            .galley(egui::pos2(text_x, y - g.size().y / 2.0), g, color);
    };
    if room != Room::Icon {
        let two = room == Room::Full && known;
        let y = rect.center().y;
        if known {
            line(
                ui,
                status.word(),
                theme::semibold(13.0),
                p.text,
                if two { y - 8.0 } else { y },
            );
            if two {
                let color = if status.link == Link::None {
                    p.accent
                } else {
                    p.muted
                };
                line(ui, status.detail(), theme::regular(11.5), color, y + 8.0);
            }
        } else {
            let (text, color) = if status.down {
                (status.word(), p.warning)
            } else {
                (format!("{}…", status.word()), p.faint)
            };
            line(ui, text, theme::regular(12.5), color, y);
        }
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, status.word()));
    let tip = if known {
        format!("{}\nClick to connect or manage", status.hover())
    } else if status.down {
        "Assistants can't connect now: see the notice at the top of the window".to_string()
    } else {
        "Asking the books server who is connected".to_string()
    };
    resp.on_hover_text(tip).clicked()
}

// ---- "Ask your assistant" ---------------------------------------------------

/// What the pages need to know to offer "Ask your assistant", put where
/// they can read it for the company in front.
#[derive(Clone, Debug)]
pub struct AskContext {
    /// The company's name.
    pub company: String,
    /// Whether the window runs a server (without one, nothing is offered).
    pub serves: bool,
    /// The state.
    pub link: Link,
}

fn ask_key() -> egui::Id {
    egui::Id::new("wanderbooks-ask-context")
}

fn go_key() -> egui::Id {
    egui::Id::new("wanderbooks-ask-go")
}

/// Say, for this frame, what "Ask your assistant" is about.
pub fn set_context(ctx: &egui::Context, c: AskContext) {
    ctx.data_mut(|d| d.insert_temp(ask_key(), c));
}

/// Whether "Ask your assistant" is offered on the page now: only in a window
/// that runs a books server.
pub fn asking_is_offered(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<AskContext>(ask_key()))
        .is_some_and(|c| c.serves)
}

/// Whether "Connect an assistant" was clicked on a page this frame.
pub fn take_go(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(go_key()))
        .unwrap_or(false)
}

/// The sentence "Ask your assistant" copies (INVARIANT 3).
pub fn sentence(company: &str, what: &str) -> String {
    format!("In my Wanderbooks books for {company}, please {what}.")
}

/// The "Ask your assistant" button, where a button that edits the books
/// would be: it copies a ready sentence for `what` ("create a new
/// invoice") to paste into the assistant, and says so. Nothing is offered
/// in a window that runs no server. `id` tells apart two on one page.
pub fn ask_button(ui: &mut Ui, what: &str) {
    let Some(c) = ui.ctx().data(|d| d.get_temp::<AskContext>(ask_key())) else {
        return;
    };
    if !c.serves {
        return;
    }
    let p = pal(ui.ctx());
    let id = ui.id().with("ask-your-assistant");
    let button = egui::Button::new(
        RichText::new(format!("{}  Ask your assistant", icon::CHAT_CENTERED_TEXT))
            .font(theme::semibold(13.0))
            .color(p.accent),
    )
    .fill(p.accent_soft)
    .stroke(Stroke::new(1.0, p.accent))
    .corner_radius(CornerRadius::same(8))
    .min_size(vec2(0.0, 30.0));
    let resp = ui.add(button).on_hover_text(
        "Copies a sentence to paste into your assistant: this window only reads the books",
    );
    if resp.clicked() {
        ui.ctx().copy_text(sentence(&c.company, what));
        let now = ui.input(|i| i.time);
        ui.ctx().data_mut(|d| d.insert_temp(id, now));
    }
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Ask your assistant")
    });
    let said_at: Option<f64> = ui.ctx().data(|d| d.get_temp(id));
    let now = ui.input(|i| i.time);
    if let Some(at) = said_at
        && now - at < 8.0
    {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(500));
        ui.ctx()
            .data_mut(|d| d.insert_temp(ask_said_key(), (what.to_string(), at)));
    }
}

fn ask_said_key() -> egui::Id {
    egui::Id::new("wanderbooks-ask-said")
}

/// The line under a page's title that follows a click on "Ask your
/// assistant": what was copied, and, with no assistant connected, where to
/// connect one. Drawn by the page header; nothing while it has not been
/// clicked.
pub fn ask_feedback(ui: &mut Ui) {
    let Some(c) = ui.ctx().data(|d| d.get_temp::<AskContext>(ask_key())) else {
        return;
    };
    let said: Option<(String, f64)> = ui.ctx().data(|d| d.get_temp(ask_said_key()));
    let now = ui.input(|i| i.time);
    let Some((what, at)) = said else { return };
    if now - at >= 8.0 {
        ui.ctx()
            .data_mut(|d| d.remove_temp::<(String, f64)>(ask_said_key()));
        return;
    }
    let p = pal(ui.ctx());
    ui.add_space(6.0);
    ui.label(
        RichText::new(format!(
            "{} Copied: “{}” Paste it into your assistant.",
            icon::CHECK,
            sentence(&c.company, &what)
        ))
        .font(theme::regular(13.0))
        .color(p.muted),
    );
    if c.link == Link::None {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("No assistant is connected yet.")
                    .font(theme::regular(13.0))
                    .color(p.warning),
            );
            if ui.link("Connect an assistant").clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(go_key(), true));
            }
        });
    }
}

// ---- the Connect page -------------------------------------------------------

/// A tool the page has a row for.
struct Tool {
    /// Its name.
    name: &'static str,
    /// The agent the owner can connect with one button, if it can.
    agent: Option<crate::owner::Agent>,
    /// The words that add it to the tool by hand (before the command), if
    /// the repository documents them; None says: give the program and its
    /// arguments (INVARIANT 4).
    add_by_hand: Option<&'static str>,
}

/// The tools of "On this computer", in the order of plan 3.3.
const TOOLS: [Tool; 6] = [
    Tool {
        name: "Claude Code",
        agent: Some(crate::owner::Agent::Claude),
        add_by_hand: Some("claude mcp add --scope user"),
    },
    Tool {
        name: "Codex",
        agent: Some(crate::owner::Agent::Codex),
        add_by_hand: Some("codex mcp add"),
    },
    Tool {
        name: "Gemini CLI",
        agent: None,
        add_by_hand: None,
    },
    Tool {
        name: "Grok Build",
        agent: None,
        add_by_hand: None,
    },
    Tool {
        name: "Muse Code",
        agent: None,
        add_by_hand: None,
    },
    Tool {
        name: "Cursor",
        agent: None,
        add_by_hand: None,
    },
];

type Outcome = Result<String, String>;

/// The Connect page's memory: the command lines (found once), the job under
/// way and its result, and a revoke asked once.
#[derive(Default)]
pub struct State {
    /// The command lines for this company, found once: `Err` says why
    /// there are none.
    commands: Option<(String, Result<Commands, String>)>,
    /// A job under way: what it is, and its answer.
    job: Option<(String, Receiver<Outcome>)>,
    /// What the last job said.
    done: Option<(String, Outcome)>,
    /// The key whose Revoke was clicked once: the second click does it.
    confirm: Option<String>,
    /// A revoke ended: the state is read again.
    refresh: bool,
}

/// The two lines an assistant set up by hand needs.
#[derive(Clone)]
struct Commands {
    /// What the assistant runs: this program with `mcp --book ID`.
    bridge: String,
}

impl State {
    /// Whether a revoke just ended, so who is connected must be read again
    /// (taken once).
    pub fn take_refresh(&mut self) -> bool {
        std::mem::take(&mut self.refresh)
    }

    /// Whether a job of this page is running (the page is not left to
    /// start another).
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// What the last job said, `(what, result)` (tests).
    pub fn said(&self) -> Option<&(String, Outcome)> {
        self.done.as_ref()
    }

    /// Ask for the Revoke of `key` (the first click), or do it (the
    /// second): used by the page, and by tests.
    pub fn confirm(&self) -> Option<&str> {
        self.confirm.as_deref()
    }

    /// The web half of the page: "Allow web assistants", the address with
    /// Copy, and the steps for Claude and ChatGPT. Unit B3 draws it here;
    /// nothing until then (INVARIANT 7).
    fn web_section(&mut self, _ui: &mut Ui, _status: &Status) {}

    fn collect(&mut self) {
        if let Some((what, rx)) = &self.job
            && let Ok(result) = rx.try_recv()
        {
            if what.starts_with("Revoke") && result.is_ok() {
                self.refresh = true;
            }
            self.done = Some((what.clone(), result));
            self.job = None;
        }
    }

    fn start(
        &mut self,
        ui: &Ui,
        jobs: &crate::jobs::Spawner,
        what: String,
        keep: &str,
        work: Box<dyn FnOnce() -> Outcome + Send>,
    ) {
        let (tx, rx) = channel();
        let ctx = ui.ctx().clone();
        let started = jobs.spawn(
            &what.clone(),
            crate::jobs::Hold::Keep(format!("{keep}: wait for it to finish")),
            move |_| {
                let _ = tx.send(work());
                ctx.request_repaint();
            },
        );
        if started {
            self.job = Some((what, rx));
            self.done = None;
        } else {
            self.done = Some((what, Err("the system gave no thread for it".into())));
        }
    }

    /// The page. `book` is the company's id; `owner` does what is asked.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        status: &Status,
        book: &str,
        owner: Option<Arc<dyn Owner>>,
        jobs: &crate::jobs::Spawner,
    ) {
        self.collect();
        if self.job.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
        // The command lines, found once for this company.
        if self.commands.as_ref().is_none_or(|(b, _)| b != book) {
            let found = match &owner {
                Some(o) => o
                    .command_line(&["mcp", "--book", book])
                    .map(|bridge| Commands { bridge }),
                None => Err("this window has no books server to connect to".to_string()),
            };
            self.commands = Some((book.to_string(), found));
        }
        let commands = self.commands.as_ref().map(|(_, c)| c.clone());
        let p = pal(ui.ctx());
        crate::pages::page(ui, "connect", |ui| {
            crate::widgets::page_header(
                ui,
                "Assistants",
                &format!(
                    "You can read all of {}'s books in this window. To add or change anything, you ask an assistant (an AI program such as Claude Code or Codex) and this window shows what it did.",
                    status.company
                ),
            );
            section(ui, "On this computer");
            ui.label(
                RichText::new(
                    "Pick the assistant you use. It gets a key to these books, kept on this computer.",
                )
                .font(theme::regular(13.5))
                .color(p.muted),
            );
            ui.add_space(8.0);
            let busy = self.busy();
            let mut want: Want = None;
            for tool in &TOOLS {
                if let Some(w) = tool_row(ui, tool, status, book, &owner, &commands, busy) {
                    want = Some(w);
                }
            }
            if TOOLS.iter().any(|t| t.agent.is_none()) {
                key_row(ui, status, book, &owner, busy, &mut want);
            }
            if let Some((what, keep, work)) = want {
                self.start(ui, jobs, what, &keep, work);
            }
            if self.job.is_some() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new("Working…")
                            .font(theme::regular(13.0))
                            .color(p.muted),
                    );
                });
            } else if let Some((what, result)) = &self.done {
                ui.add_space(6.0);
                match result {
                    Ok(said) => ui.label(
                        RichText::new(format!("{what}: {said}"))
                            .font(theme::regular(13.0))
                            .color(p.muted),
                    ),
                    Err(why) => ui.label(
                        RichText::new(format!("{what}: {why}"))
                            .font(theme::regular(13.0))
                            .color(p.warning),
                    ),
                };
            }
            self.web_section(ui, status);
            ui.add_space(24.0);
            self.connections(ui, status, book, &owner, jobs);
        });
    }

    /// "Connections": every key to these books, who is connected on it,
    /// what it may do, when it last did anything, and Revoke.
    fn connections(
        &mut self,
        ui: &mut Ui,
        status: &Status,
        book: &str,
        owner: &Option<Arc<dyn Owner>>,
        jobs: &crate::jobs::Spawner,
    ) {
        let p = pal(ui.ctx());
        section(ui, "Who can reach these books");
        let rows = status.connections();
        if rows.is_empty() {
            ui.label(
                RichText::new(if status.link == Link::Unknown {
                    "Checking…"
                } else {
                    "No assistant has a key to these books yet. Connect one above."
                })
                .font(theme::regular(13.5))
                .color(p.muted),
            );
            return;
        }
        let mut revoke: Option<String> = None;
        for row in &rows {
            Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    connection_row(
                        ui,
                        row,
                        status,
                        &mut self.confirm,
                        &mut revoke,
                        self.job.is_some(),
                        owner.is_some(),
                    );
                });
            ui.add_space(6.0);
        }
        if let (Some(name), Some(owner)) = (revoke, owner.clone()) {
            self.confirm = None;
            let b = book.to_string();
            self.start(
                ui,
                jobs,
                format!("Revoke {name}"),
                "Ending an assistant's access",
                Box::new(move || owner.serve_revoke(&b, &name)),
            );
        }
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title).font(theme::semibold(17.0)));
    ui.add_space(4.0);
}

type Want = Option<(String, String, Box<dyn FnOnce() -> Outcome + Send>)>;

/// One tool's row: its name, its button when it has one, and the exact
/// instructions.
fn tool_row(
    ui: &mut Ui,
    tool: &Tool,
    status: &Status,
    book: &str,
    owner: &Option<Arc<dyn Owner>>,
    commands: &Option<Result<Commands, String>>,
    busy: bool,
) -> Want {
    let p = pal(ui.ctx());
    let mut want: Want = None;
    let connected = status
        .assistants
        .iter()
        .any(|a| program_name(a) == tool.name);
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(tool.name).font(theme::semibold(14.5)));
                if connected {
                    ui.label(
                        RichText::new(format!("{} connected now", icon::PLUGS_CONNECTED))
                            .font(theme::regular(12.5))
                            .color(p.positive),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let (Some(agent), Some(owner)) = (tool.agent, owner)
                        && ui
                            .add_enabled(!busy, egui::Button::new(format!("Connect {}", tool.name)))
                            .on_hover_text(format!(
                                "Makes a key for these books and adds them to {}",
                                tool.name
                            ))
                            .clicked()
                    {
                        let (o, b) = (owner.clone(), book.to_string());
                        let keep = match agent {
                            crate::owner::Agent::Claude => "Connecting Claude Code to these books",
                            crate::owner::Agent::Codex => "Connecting Codex to these books",
                        };
                        want = Some((
                            format!("Connect {}", tool.name),
                            keep.to_string(),
                            Box::new(move || o.connect(agent, &b).map(|lines| lines.join(" "))),
                        ));
                    }
                });
            });
            ui.add_space(4.0);
            match commands {
                Some(Ok(c)) => by_hand(ui, tool, book, &c.bridge),
                Some(Err(why)) => {
                    ui.label(
                        RichText::new(format!("The command could not be worked out: {why}"))
                            .font(theme::regular(12.5))
                            .color(p.warning),
                    );
                }
                None => {}
            }
        });
    ui.add_space(6.0);
    want
}

/// The exact text for a tool, copyable.
fn by_hand(ui: &mut Ui, tool: &Tool, book: &str, bridge: &str) {
    let p = pal(ui.ctx());
    let name = format!("wanderbooks-{book}");
    match tool.add_by_hand {
        Some(add) => {
            let line = format!("{add} {name} -- {bridge}");
            egui::CollapsingHeader::new(
                RichText::new("Or do it yourself, in a terminal")
                    .font(theme::regular(12.5))
                    .color(p.muted),
            )
            .id_salt(("by-hand", tool.name))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Run this once:")
                        .font(theme::regular(12.5))
                        .color(p.muted),
                );
                command_box(ui, &line);
                ui.label(
                    RichText::new(format!(
                        "Then restart {}. It reaches the books as “{name}”.",
                        tool.name
                    ))
                    .font(theme::regular(12.5))
                    .color(p.muted),
                );
            });
        }
        None => {
            ui.label(
                RichText::new(format!(
                    "In {}, add a local tool (a program it runs on this computer) named “{name}”, with this command:",
                    tool.name
                ))
                .font(theme::regular(13.0))
                .color(p.text),
            );
            command_box(ui, bridge);
            ui.label(
                RichText::new(format!(
                    "Then restart {}. If it asks for the program and its arguments separately, the program is the first word and the rest are its arguments. We haven't tested {} yet.",
                    tool.name, tool.name
                ))
                .font(theme::regular(12.5))
                .color(p.muted),
            );
        }
    }
}

/// The step the tools without a button share: the key they read, made once.
fn key_row(
    ui: &mut Ui,
    status: &Status,
    book: &str,
    owner: &Option<Arc<dyn Owner>>,
    busy: bool,
    want: &mut Want,
) {
    let p = pal(ui.ctx());
    ui.add_space(2.0);
    ui.label(
        RichText::new(format!(
            "For the others, make {}'s key first (once):",
            status.company
        ))
        .font(theme::regular(13.0))
        .color(p.muted),
    );
    if let Some(owner) = owner
        && ui
            .add_enabled(!busy, egui::Button::new("Make the key"))
            .on_hover_text(
                "Makes the key these books give an assistant, and keeps it on this computer",
            )
            .clicked()
    {
        let (o, b) = (owner.clone(), book.to_string());
        *want = Some((
            "Make the key".to_string(),
            "Making the key".to_string(),
            Box::new(move || {
                o.make_key(&b).map(|made| {
                    if made {
                        "made, and kept on this computer.".to_string()
                    } else {
                        "it was already made.".to_string()
                    }
                })
            }),
        ));
    }
}

/// Text in a box, with a Copy button.
fn command_box(ui: &mut Ui, text: &str) {
    let p = pal(ui.ctx());
    Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    if ui.button(format!("{} Copy", icon::COPY)).clicked() {
                        ui.ctx().copy_text(text.to_string());
                    }
                    ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(text).font(egui::FontId::monospace(12.0)),
                            )
                            .wrap()
                            .selectable(true),
                        );
                    });
                });
            });
        });
}

/// One key of these books: its name, what is connected on it, what it may
/// do, and Revoke (asked once).
fn connection_row(
    ui: &mut Ui,
    row: &Connection<'_>,
    status: &Status,
    confirm: &mut Option<String>,
    revoke: &mut Option<String>,
    busy: bool,
    can_revoke: bool,
) {
    let p = pal(ui.ctx());
    let working = row.sessions.iter().any(|a| a.running > 0);
    let link = if row.sessions.is_empty() {
        Link::None
    } else if working {
        Link::Working
    } else {
        Link::Connected
    };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        paint_mark(ui, rect.center(), 18.0, link);
        ui.vertical(|ui| {
            ui.label(
                RichText::new(format!("Key “{}”", row.name)).font(theme::semibold(13.5)),
            );
            let state = match link {
                Link::Working => "Working now",
                Link::Connected => "Connected",
                _ => "Not connected",
            };
            ui.label(
                RichText::new(format!("{state}. {}", row.may()))
                    .font(theme::regular(12.5))
                    .color(p.muted),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let Some(t) = row.token else { return };
            if !can_revoke {
                return;
            }
            if confirm.as_deref() == Some(t.name.as_str()) {
                if ui
                    .add_enabled(!busy, egui::Button::new("Yes, revoke"))
                    .clicked()
                {
                    *revoke = Some(t.name.clone());
                }
                if ui.button("Keep it").clicked() {
                    *confirm = None;
                }
            } else if ui
                .add_enabled(!busy, egui::Button::new("Revoke"))
                .on_hover_text("Ends this key: the assistant using it is cut off and cannot reach these books again")
                .clicked()
            {
                *confirm = Some(t.name.clone());
            }
        });
    });
    if confirm.as_deref() == Some(row.name.as_str()) {
        ui.label(
            RichText::new(format!(
                "Revoke the key “{}”? Anything using it is cut off now; {}'s books are not changed.",
                row.name, status.company
            ))
            .font(theme::regular(12.5))
            .color(p.warning),
        );
    }
    for a in &row.sessions {
        ui.label(
            RichText::new(describe(a, status.now))
                .font(theme::regular(12.5))
                .color(p.text),
        );
    }
}
