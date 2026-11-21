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
//!      Claude and ChatGPT) is unit B3's. Unit B1's part of it is here: a
//!      web assistant that asks to connect is shown to the person, in
//!      words, as a card that says who is asking, where it is sent back
//!      to, the address it asked from and what it will be able to do, with
//!      Allow and Don't allow; it appears over any page of the company
//!      (`State::web_prompt`) and on this page (`State::web_section`).
//!      Nothing on the web page can approve: only these buttons do, through
//!      the owner. What is known is said in neutral words, where it will be
//!      sent back to first ("An assistant that will be sent back to
//!      claude.ai"), never as a known or trusted product on the strength of
//!      that address: a host is public, and says nothing of who asks. Each
//!      card shows its own short code, the one its browser page shows, and
//!      says to allow only on a match; at most MOST_CARDS cards are drawn
//!      at once, the newest, and the rest are counted. What the assistant
//!      calls itself is shown in quotes, after the address. Posting is off
//!      unless the person ticks it, per assistant, and can be turned on or
//!      off later; every Allow is a new approval with only what is ticked.
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

/// Ask the window to open the Connect page (the Books menu's "Connect an
/// assistant…"; the same as clicking the link under "Ask your assistant").
pub fn open_page(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(go_key(), true));
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
        ui.ctx()
            .data_mut(|d| d.insert_temp(ask_said_key(), (what.to_string(), now, resp.rect)));
    }
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Ask your assistant")
    });
}

fn ask_said_key() -> egui::Id {
    egui::Id::new("wanderbooks-ask-said")
}

/// What follows a click on "Ask your assistant": a small popover under the
/// button (so nothing on the page moves) saying what was copied and, with
/// no assistant connected, where to connect one. It goes away after a few
/// seconds, or on the next click anywhere else. Drawn by the page header;
/// nothing while the button has not been clicked.
pub fn ask_feedback(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let Some(c) = ctx.data(|d| d.get_temp::<AskContext>(ask_key())) else {
        return;
    };
    let said: Option<(String, f64, egui::Rect)> = ctx.data(|d| d.get_temp(ask_said_key()));
    let now = ui.input(|i| i.time);
    let Some((what, at, button)) = said else {
        return;
    };
    let forget = || ctx.data_mut(|d| d.remove::<(String, f64, egui::Rect)>(ask_said_key()));
    if now - at >= 8.0 {
        forget();
        return;
    }
    let p = pal(&ctx);
    let shown = egui::Area::new(egui::Id::new("wanderbooks-ask-popover"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(button.right_bottom() + vec2(0.0, 6.0))
        .constrain(true)
        .show(&ctx, |ui| {
            Frame::popup(ui.style())
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .show(ui, |ui| {
                    ui.set_max_width(380.0);
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
                        ui.horizontal_wrapped(|ui| {
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
                });
        });
    // A click anywhere else puts it away; so does the click on its link.
    let rect = shown.response.rect;
    let (pressed, at_pos) = ctx.input(|i| (i.pointer.any_pressed(), i.pointer.interact_pos()));
    let outside = at_pos.is_some_and(|pos| !rect.contains(pos) && !button.contains(pos));
    if pressed && outside {
        forget();
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
}

// ---- the Connect page -------------------------------------------------------

/// A tool the page has a row for.
struct Tool {
    /// Its name.
    name: &'static str,
    /// The agent the owner can connect with one button.
    agent: crate::owner::Agent,
    /// The words that add it to the tool by hand (before the command), as
    /// the tool's own documentation gives them.
    add_by_hand: &'static str,
}

/// The tools of "On this computer" that have a button of their own; every
/// other assistant shares the "Another assistant" card.
const TOOLS: [Tool; 2] = [
    Tool {
        name: "Claude Code",
        agent: crate::owner::Agent::Claude,
        add_by_hand: "claude mcp add --scope user",
    },
    Tool {
        name: "Codex",
        agent: crate::owner::Agent::Codex,
        add_by_hand: "codex mcp add",
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
    /// The waiting web assistants the person has ticked "let it post
    /// entries too" for.
    post_asked: std::collections::BTreeSet<String>,
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

    /// What the person is asked of a web assistant, and what they may do
    /// about the web connections already allowed (INVARIANT 7). Unit B3
    /// draws the switch, the address and the steps above this.
    fn web_section(
        &mut self,
        ui: &mut Ui,
        status: &Status,
        owner: &Option<Arc<dyn Owner>>,
        jobs: &crate::jobs::Spawner,
    ) {
        if status.web_pending.is_empty() && status.web_approvals.is_empty() {
            return;
        }
        let p = pal(ui.ctx());
        ui.add_space(24.0);
        section(ui, "Assistants on the web");
        let mut want: Want = None;
        let (shown, more) = newest_waiting(&status.web_pending);
        for request in shown {
            if let Some(w) = self.pending_card(ui, status, request, owner) {
                want = Some(w);
            }
            ui.add_space(6.0);
        }
        say_more_waiting(ui, more);
        for a in &status.web_approvals {
            Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&a.label).font(theme::semibold(13.5)));
                            ui.label(
                                RichText::new(if a.may_post {
                                    "Can read, draft and post entries."
                                } else {
                                    "Can read and draft entries. It can't post them."
                                })
                                .font(theme::regular(12.5))
                                .color(p.muted),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if owner.is_none() || self.job.is_some() {
                                return;
                            }
                            let (label, to) = if a.may_post {
                                ("Stop letting it post", false)
                            } else {
                                ("Let it post entries", true)
                            };
                            if ui.button(label).clicked() {
                                let (o, id) = (owner.clone(), a.id.clone());
                                if let Some(o) = o {
                                    want = Some((
                                        format!("Posting for {}", a.label),
                                        "Changing what an assistant may do".to_string(),
                                        Box::new(move || o.web_set_post(&id, to)),
                                    ));
                                }
                            }
                        });
                    });
                });
            ui.add_space(6.0);
        }
        if let Some((what, keep, work)) = want {
            self.start(ui, jobs, what, &keep, work);
        }
    }

    /// One assistant asking to connect: who, from where, what it will be
    /// able to do, and Allow and Don't allow. The answer to ask for, if
    /// the person gave one.
    fn pending_card(
        &mut self,
        ui: &mut Ui,
        status: &Status,
        request: &crate::owner::WebRequest,
        owner: &Option<Arc<dyn Owner>>,
    ) -> Want {
        let can_answer = owner.is_some();
        let p = pal(ui.ctx());
        let mut answer: Option<(bool, bool)> = None;
        Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.5, p.accent))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // What is known is said in neutral words: where it will be
                // sent back to first, never a product's name made from it.
                ui.label(
                    RichText::new(format!(
                        "An assistant that will be sent back to {} wants to connect to {}. Allow?",
                        request.redirect_host, status.company
                    ))
                    .font(theme::semibold(15.0)),
                );
                if !request.client_name.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "It calls itself \u{201c}{}\u{201d}. That is only its own word for itself.",
                            request.client_name
                        ))
                        .font(theme::regular(13.0))
                        .color(p.muted),
                    );
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("Code {}", request.code)).font(theme::semibold(22.0)),
                );
                ui.label(
                    RichText::new(
                        "Allow only if the page in your browser shows this same code. If it shows another code, or you didn't just ask an assistant to connect, choose Don't allow.",
                    )
                    .font(theme::regular(13.0))
                    .color(p.muted),
                );
                if !request.known_host {
                    ui.label(
                        RichText::new(
                            "Wanderbooks doesn't know this address. Don't allow it unless you just set this assistant up yourself.",
                        )
                        .font(theme::regular(13.0))
                        .color(p.warning),
                    );
                }
                ui.label(
                    RichText::new(format!(
                        "After you answer, it is sent back to {}. The request came from the internet address {}.",
                        request.redirect_host, request.from
                    ))
                    .font(theme::regular(12.5))
                    .color(p.muted),
                );
                ui.label(
                    RichText::new(
                        "If you allow it, it can read these books and draft entries. It can't post entries unless you tick the box below.",
                    )
                    .font(theme::regular(12.5))
                    .color(p.muted),
                );
                let mut post = self.post_asked.contains(&request.id);
                if ui
                    .checkbox(
                        &mut post,
                        if request.wants_post {
                            "Also let it post entries (it asked to)"
                        } else {
                            "Also let it post entries"
                        },
                    )
                    .changed()
                {
                    if post {
                        self.post_asked.insert(request.id.clone());
                    } else {
                        self.post_asked.remove(&request.id);
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let enabled = can_answer && self.job.is_none();
                    if ui.add_enabled(enabled, egui::Button::new("Allow")).clicked() {
                        answer = Some((true, post));
                    }
                    if ui
                        .add_enabled(enabled, egui::Button::new("Don't allow"))
                        .clicked()
                    {
                        answer = Some((false, false));
                    }
                    ui.label(
                        RichText::new(format!(
                            "Waiting {} more minute(s)",
                            request.expires_in_secs.div_ceil(60)
                        ))
                        .font(theme::regular(12.0))
                        .color(p.muted),
                    );
                });
            });
        let (approve, post) = answer?;
        self.post_asked.remove(&request.id);
        let owner = owner.clone()?;
        let id = request.id.clone();
        let what = if approve {
            format!("Allow the assistant sent back to {}", request.redirect_host)
        } else {
            format!(
                "Don't allow the assistant sent back to {}",
                request.redirect_host
            )
        };
        Some((
            what,
            "Answering an assistant".to_string(),
            Box::new(move || owner.web_decide(&id, approve, post)),
        ))
    }

    /// The question over any page of the company: a web assistant asking
    /// to connect is not something to find by looking for it (INVARIANT 7).
    /// Not drawn on the Connect page, which has the same card.
    pub fn web_prompt(
        &mut self,
        ui: &Ui,
        status: &Status,
        owner: &Option<Arc<dyn Owner>>,
        jobs: &crate::jobs::Spawner,
    ) {
        self.collect();
        if status.web_pending.is_empty() {
            return;
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(500));
        let mut want: Want = None;
        egui::Window::new("An assistant wants to connect")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ui.ctx(), |win| {
                let (shown, more) = newest_waiting(&status.web_pending);
                for request in shown {
                    if let Some(w) = self.pending_card(win, status, request, owner) {
                        want = Some(w);
                    }
                    win.add_space(6.0);
                }
                say_more_waiting(win, more);
            });
        if let Some((what, keep, work)) = want {
            self.start(ui, jobs, what, &keep, work);
        }
    }

    fn collect(&mut self) {
        if let Some((what, rx)) = &self.job
            && let Ok(result) = rx.try_recv()
        {
            // Whatever came of it (a key already gone, a file left behind),
            // the list is read again.
            if what.starts_with("Revoke")
                || what.starts_with("Allow")
                || what.starts_with("Don't allow")
                || what.starts_with("Posting")
            {
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
            another_row(ui, status, book, &owner, &commands, busy, &mut want);
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
            self.web_section(ui, status, &owner, jobs);
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
        let mut revoke: Option<(String, String)> = None;
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
        if let (Some((name, id)), Some(owner)) = (revoke, owner.clone()) {
            self.confirm = None;
            let b = book.to_string();
            self.start(
                ui,
                jobs,
                format!("Revoke {name}"),
                "Ending an assistant's access",
                Box::new(move || owner.serve_revoke(&b, &name, &id)),
            );
        }
    }
}

/// The most requests drawn at once: a few addresses must not be able to
/// bury the one the person is waiting for under a screenful.
pub const MOST_CARDS: usize = 3;

/// The newest requests (the one just asked for is the one the person is
/// looking for), and how many more are waiting.
fn newest_waiting(all: &[crate::owner::WebRequest]) -> (Vec<&crate::owner::WebRequest>, usize) {
    let mut sorted: Vec<&crate::owner::WebRequest> = all.iter().collect();
    // Most time left is the newest.
    sorted.sort_by_key(|r| std::cmp::Reverse(r.expires_in_secs));
    let more = sorted.len().saturating_sub(MOST_CARDS);
    sorted.truncate(MOST_CARDS);
    (sorted, more)
}

fn say_more_waiting(ui: &mut Ui, more: usize) {
    if more == 0 {
        return;
    }
    let p = pal(ui.ctx());
    ui.label(
        RichText::new(format!(
            "{more} more {} waiting. Answer these first; the next ones then show.",
            if more == 1 { "is" } else { "are" }
        ))
        .font(theme::regular(12.5))
        .color(p.muted),
    );
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
                    if let (agent, Some(owner)) = (tool.agent, owner)
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
    let line = format!("{} {name} -- {bridge}", tool.add_by_hand);
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

/// The one card for every other assistant: the command, the key it reads
/// (made once), and the names we have not tried.
fn another_row(
    ui: &mut Ui,
    status: &Status,
    book: &str,
    owner: &Option<Arc<dyn Owner>>,
    commands: &Option<Result<Commands, String>>,
    busy: bool,
    want: &mut Want,
) {
    let p = pal(ui.ctx());
    let name = format!("wanderbooks-{book}");
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Another assistant").font(theme::semibold(14.5)));
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "Any assistant that can run a local tool (a program it runs on this computer) works. Add one named “{name}”, with this command:"
                ))
                .font(theme::regular(13.0))
                .color(p.text),
            );
            match commands {
                Some(Ok(c)) => command_box(ui, &c.bridge),
                Some(Err(why)) => {
                    ui.label(
                        RichText::new(format!("The command could not be worked out: {why}"))
                            .font(theme::regular(12.5))
                            .color(p.warning),
                    );
                }
                None => {}
            }
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!(
                    "It reads {}'s key, made once:",
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
                                "made, and kept on this computer. It can read the books, draft entries and post them; it cannot see the access record.".to_string()
                            } else {
                                "it was already made.".to_string()
                            }
                        })
                    }),
                ));
            }
            ui.add_space(2.0);
            ui.label(
                RichText::new(
                    "Then restart it. We haven't tested Gemini CLI, Grok Build, Muse Code or Cursor yet.",
                )
                .font(theme::regular(12.5))
                .color(p.muted),
            );
        });
    ui.add_space(6.0);
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
    revoke: &mut Option<(String, String)>,
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
            if confirm.as_deref() == Some(t.id.as_str()) {
                if ui
                    .add_enabled(!busy, egui::Button::new("Yes, revoke"))
                    .clicked()
                {
                    *revoke = Some((t.name.clone(), t.id.clone()));
                }
                if ui.button("Keep it").clicked() {
                    *confirm = None;
                }
            } else if ui
                .add_enabled(!busy, egui::Button::new("Revoke"))
                .on_hover_text("Ends this key: the assistant using it is cut off and cannot reach these books again")
                .clicked()
            {
                *confirm = Some(t.id.clone());
            }
        });
    });
    if row
        .token
        .is_some_and(|t| confirm.as_deref() == Some(t.id.as_str()))
    {
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
