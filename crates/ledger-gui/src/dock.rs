//! The agent dock (plan P11b, D-27): a terminal along the bottom of the
//! viewer's window that runs Claude Code (the default) or Codex in the
//! book's books home.
//!
//! ```text
//! GOAL: the owner talks to the agent that keeps the books without leaving
//!       the app, and never has to: any other agent, anywhere, reaches the
//!       same books through the MCP.
//! SOURCE: D-27 (the dock, the books home, "never required"); D-32 (no
//!         helper, no confinement: the agent is an ordinary child process
//!         of the viewer's program, started through the [`Owner`]); D-54;
//!         Q-7 (Codex asks about trusting the folder itself).
//! INVARIANTS:
//!   1. The dock writes nothing of the books. The agent it runs writes
//!      through the books server like any other agent; the viewer stays
//!      read-only (tests/read_only.rs).
//!   2. The agent runs in the book's books home, a folder holding only its
//!      pointer and settings; the dock never runs anything in the data
//!      root and never marks a folder trusted.
//!   3. An agent that isn't installed is offered through its vendor's
//!      installer (a command to copy and a page); nothing is installed by
//!      the app and no agent is bundled.
//!   4. Keystrokes go to the agent only while the dock has the keyboard
//!      (a click gives it, a click elsewhere takes it away): the viewer's
//!      own shortcuts work everywhere else. Ctrl-C is the agent's.
//!   5. The terminal is as large as the panel: resizing the panel resizes
//!      the agent's terminal, which redraws.
//!   6. The agent chosen and whether the dock is open are the viewer's
//!      preferences, remembered; they are not the books'.
//!   7. Offered only where it has been checked to work (Linux and macOS):
//!      a ConPTY agent on Windows must pass the four checks (it starts,
//!      `/login` works, resize redraws, Ctrl-C reaches it) before it is.
//! ```

use crate::owner::{Agent, AgentStart, Installer, Owner};
use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode, point_to_viewport, viewport_to_point};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color as TermColor, NamedColor, Rgb};
use egui::{Color32, Event, FontId, Key, Modifiers, Pos2, Rect, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as icon;
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

/// Whether the dock is offered on this system (INVARIANT 7).
pub fn offered() -> bool {
    cfg!(unix)
}

/// A size in character cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cells {
    /// Columns.
    pub cols: u16,
    /// Rows.
    pub rows: u16,
}

impl Dimensions for Cells {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }
    fn screen_lines(&self) -> usize {
        self.rows as usize
    }
    fn columns(&self) -> usize {
        self.cols as usize
    }
}

/// What the terminal tells the dock: it listens for the emulator's events.
#[derive(Clone)]
struct Proxy {
    wake: Arc<dyn Fn() + Send + Sync>,
    to_pty: Arc<Mutex<Option<EventLoopSender>>>,
    size: Arc<Mutex<WindowSize>>,
    exited: Arc<AtomicBool>,
    title: Arc<Mutex<String>>,
}

impl Proxy {
    fn write(&self, text: String) {
        if let Some(tx) = self
            .to_pty
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = tx.send(Msg::Input(Cow::Owned(text.into_bytes())));
        }
    }
}

impl EventListener for Proxy {
    fn send_event(&self, event: TermEvent) {
        match event {
            TermEvent::Wakeup | TermEvent::Bell => (self.wake)(),
            // The terminal's own answers to the program's questions.
            TermEvent::PtyWrite(text) => self.write(text),
            TermEvent::TextAreaSizeRequest(format) => {
                let size = *self.size.lock().unwrap_or_else(|e| e.into_inner());
                self.write(format(size));
            }
            TermEvent::Title(t) => {
                *self.title.lock().unwrap_or_else(|e| e.into_inner()) = t;
                (self.wake)();
            }
            TermEvent::ResetTitle => {
                self.title.lock().unwrap_or_else(|e| e.into_inner()).clear();
            }
            TermEvent::ChildExit(_) | TermEvent::Exit => {
                self.exited.store(true, Ordering::SeqCst);
                (self.wake)();
            }
            _ => {}
        }
    }
}

/// An agent running on a terminal of its own.
pub struct Session {
    term: Arc<FairMutex<Term<Proxy>>>,
    to_pty: EventLoopSender,
    size: Arc<Mutex<WindowSize>>,
    cells: Cells,
    exited: Arc<AtomicBool>,
    title: Arc<Mutex<String>>,
}

impl Session {
    /// Run `program args` in `dir` with `env` added, on a terminal of `cells`;
    /// `wake` is called when there is something new to draw.
    pub fn spawn(
        program: &std::path::Path,
        args: &[String],
        dir: &std::path::Path,
        env: &[(String, String)],
        cells: Cells,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Session, String> {
        let window = WindowSize {
            num_lines: cells.rows,
            num_cols: cells.cols,
            cell_width: 1,
            cell_height: 1,
        };
        let options = tty::Options {
            shell: Some(tty::Shell::new(
                program.to_string_lossy().into_owned(),
                args.to_vec(),
            )),
            working_directory: Some(dir.to_path_buf()),
            drain_on_exit: true,
            env: env.iter().cloned().collect(),
            #[cfg(windows)]
            escape_args: true,
        };
        let pty = tty::new(&options, window, 0).map_err(|e| e.to_string())?;
        let exited = Arc::new(AtomicBool::new(false));
        let to_pty = Arc::new(Mutex::new(None));
        let size = Arc::new(Mutex::new(window));
        let title = Arc::new(Mutex::new(String::new()));
        let proxy = Proxy {
            wake,
            to_pty: to_pty.clone(),
            size: size.clone(),
            exited: exited.clone(),
            title: title.clone(),
        };
        let term = Arc::new(FairMutex::new(Term::new(
            Config::default(),
            &cells,
            proxy.clone(),
        )));
        let event_loop =
            EventLoop::new(term.clone(), proxy, pty, true, false).map_err(|e| e.to_string())?;
        let sender = event_loop.channel();
        *to_pty.lock().unwrap_or_else(|e| e.into_inner()) = Some(sender.clone());
        // Its thread ends on Shutdown, and its terminal's child with it.
        drop(event_loop.spawn());
        Ok(Session {
            term,
            to_pty: sender,
            size,
            cells,
            exited,
            title,
        })
    }

    /// Send keystrokes (or anything else the program is to read).
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let bytes = bytes.into();
        if !bytes.is_empty() {
            let _ = self.to_pty.send(Msg::Input(bytes));
        }
    }

    /// Make the terminal `cells` large (the program is told, and redraws).
    pub fn resize(&mut self, cells: Cells) {
        if cells == self.cells || cells.cols == 0 || cells.rows == 0 {
            return;
        }
        self.cells = cells;
        let window = WindowSize {
            num_lines: cells.rows,
            num_cols: cells.cols,
            cell_width: 1,
            cell_height: 1,
        };
        *self.size.lock().unwrap_or_else(|e| e.into_inner()) = window;
        self.term.lock().resize(cells);
        let _ = self.to_pty.send(Msg::Resize(window));
    }

    /// The terminal's size now.
    pub fn cells(&self) -> Cells {
        self.cells
    }

    /// Whether the program has ended.
    pub fn exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    /// The window title the program set, if any.
    pub fn title(&self) -> String {
        self.title.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The screen as text, a line each, trailing blanks cut (tests and
    /// accessibility).
    pub fn text(&self) -> String {
        let term = self.term.lock();
        let (rows, cols) = (term.screen_lines(), term.columns());
        let mut out = Vec::new();
        for line in 0..rows {
            let start = Point::new(Line(line as i32), Column(0));
            let end = Point::new(Line(line as i32), Column(cols - 1));
            out.push(term.bounds_to_string(start, end).trim_end().to_string());
        }
        out.join("\n")
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.to_pty.send(Msg::Shutdown);
    }
}

/// The bytes a key sends to a program on a terminal (None: nothing, or the
/// key's text comes separately as typed text). `app_cursor` is the
/// program's application-cursor mode.
pub fn key_bytes(key: Key, mods: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    // xterm's modifier parameter: 1 + shift 1 + alt 2 + ctrl 4.
    let param = 1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.ctrl);
    let csi = |final_byte: char| -> Vec<u8> {
        if param == 1 {
            format!("\x1b[{final_byte}").into_bytes()
        } else {
            format!("\x1b[1;{param}{final_byte}").into_bytes()
        }
    };
    let arrow = |final_byte: char| -> Vec<u8> {
        if param == 1 && app_cursor {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            csi(final_byte)
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if param == 1 {
            format!("\x1b[{n}~").into_bytes()
        } else {
            format!("\x1b[{n};{param}~").into_bytes()
        }
    };
    let with_alt = |mut b: Vec<u8>| {
        if mods.alt {
            b.insert(0, 0x1b);
        }
        b
    };
    Some(match key {
        Key::Enter if mods.shift => b"\n".to_vec(),
        Key::Enter => with_alt(b"\r".to_vec()),
        Key::Backspace if mods.ctrl => vec![0x08],
        Key::Backspace => with_alt(vec![0x7f]),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Escape => vec![0x1b],
        Key::ArrowUp => arrow('A'),
        Key::ArrowDown => arrow('B'),
        Key::ArrowRight => arrow('C'),
        Key::ArrowLeft => arrow('D'),
        Key::Home => arrow('H'),
        Key::End => arrow('F'),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::F1 => b"\x1bOP".to_vec(),
        Key::F2 => b"\x1bOQ".to_vec(),
        Key::F3 => b"\x1bOR".to_vec(),
        Key::F4 => b"\x1bOS".to_vec(),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        Key::Space if mods.ctrl => vec![0],
        // Ctrl-A to Ctrl-Z (Ctrl-C included: it is the agent's, INVARIANT 4),
        // Alt adding the escape; text typed with Alt is the key's too.
        k if mods.ctrl || mods.alt => {
            let name = k.name();
            let c = name.chars().next().filter(|_| name.chars().count() == 1)?;
            if mods.ctrl && c.is_ascii_alphabetic() {
                with_alt(vec![c.to_ascii_lowercase() as u8 - b'a' + 1])
            } else if mods.ctrl {
                match c {
                    '[' => vec![0x1b],
                    '\\' => vec![0x1c],
                    ']' => vec![0x1d],
                    '/' => vec![0x1f],
                    _ => return None,
                }
            } else {
                let c = if mods.shift {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                };
                with_alt(c.to_string().into_bytes())
            }
        }
        _ => return None,
    })
}

/// The 16 colors of the terminal's palette, dark or light.
fn ansi16(dark: bool) -> [Color32; 16] {
    let c = |r, g, b| Color32::from_rgb(r, g, b);
    if dark {
        [
            c(0x1c, 0x1c, 0x22),
            c(0xe0, 0x6c, 0x75),
            c(0x7f, 0xc0, 0x6e),
            c(0xe5, 0xc0, 0x7b),
            c(0x61, 0xaf, 0xef),
            c(0xc6, 0x78, 0xdd),
            c(0x56, 0xb6, 0xc2),
            c(0xc8, 0xcc, 0xd4),
            c(0x5c, 0x63, 0x70),
            c(0xf0, 0x8b, 0x93),
            c(0x98, 0xd3, 0x87),
            c(0xf0, 0xd0, 0x8f),
            c(0x7b, 0xc0, 0xf8),
            c(0xd8, 0x92, 0xea),
            c(0x73, 0xcb, 0xd6),
            c(0xf2, 0xf4, 0xf8),
        ]
    } else {
        [
            c(0x1b, 0x1b, 0x1f),
            c(0xb9, 0x1c, 0x1c),
            c(0x15, 0x80, 0x3d),
            c(0x8a, 0x5a, 0x00),
            c(0x1d, 0x4e, 0xd8),
            c(0x86, 0x25, 0xa8),
            c(0x0e, 0x74, 0x90),
            c(0x6b, 0x6b, 0x74),
            c(0x4a, 0x4a, 0x52),
            c(0xdc, 0x26, 0x26),
            c(0x16, 0xa3, 0x4a),
            c(0xa1, 0x62, 0x07),
            c(0x25, 0x63, 0xeb),
            c(0xa8, 0x55, 0xf7),
            c(0x08, 0x91, 0xb2),
            c(0x9c, 0x9c, 0xa4),
        ]
    }
}

/// A color the terminal asks for, as the dock draws it.
fn color(c: TermColor, dark: bool, fg: Color32, bg: Color32, bold: bool) -> Color32 {
    let table = ansi16(dark);
    match c {
        TermColor::Spec(Rgb { r, g, b }) => Color32::from_rgb(r, g, b),
        TermColor::Indexed(i) => match i {
            0..=7 if bold => table[i as usize + 8],
            0..=15 => table[i as usize],
            16..=231 => {
                let n = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
                Color32::from_rgb(level(n / 36), level(n % 36 / 6), level(n % 6))
            }
            _ => {
                let v = 8 + 10 * (i - 232);
                Color32::from_rgb(v, v, v)
            }
        },
        TermColor::Named(n) => match n {
            NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => fg,
            NamedColor::Background => bg,
            NamedColor::DimForeground => fg.gamma_multiply(0.6),
            n if (n as usize) < 8 && bold => table[n as usize + 8],
            n if (n as usize) < 16 => table[n as usize],
            n => {
                // Dim black .. dim white follow the cursor color.
                let i = (n as usize).saturating_sub(NamedColor::DimBlack as usize);
                table[i.min(7)].gamma_multiply(0.7)
            }
        },
    }
}

/// What the dock is doing.
enum State {
    /// Closed or not started: nothing runs.
    Idle,
    /// The owner is finding the agent and its books home.
    Starting(Receiver<Result<AgentStart, String>>),
    /// The agent runs.
    Running(Box<Session>),
    /// The agent isn't installed.
    Missing(Installer),
    /// It couldn't be started.
    Failed(String),
}

/// The dock's state and look: one per window.
pub struct Dock {
    /// Whether the panel is open (INVARIANT 6).
    pub open: bool,
    /// The agent chosen (INVARIANT 6).
    pub agent: Agent,
    state: State,
    /// The book it was started for.
    book: Option<String>,
    /// Has the keyboard (INVARIANT 4).
    focused: bool,
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            open: false,
            agent: Agent::Claude,
            state: State::Idle,
            book: None,
            focused: false,
        }
    }
}

impl Dock {
    /// The dock as kept: the `agent` key and the `open` flag.
    pub fn restored(get: impl Fn(&str) -> Option<String>) -> Dock {
        Dock {
            open: get("dock_open").is_some_and(|v| v == "true"),
            agent: get("dock_agent")
                .and_then(|k| Agent::from_key(&k))
                .unwrap_or(Agent::Claude),
            ..Default::default()
        }
    }

    /// Keep the preferences.
    pub fn keep(&self, set: &mut dyn FnMut(&str, String)) {
        set("dock_open", self.open.to_string());
        set("dock_agent", self.agent.key().to_string());
    }

    /// Whether the dock has the keyboard: the viewer's own shortcuts wait.
    pub fn has_keyboard(&self) -> bool {
        self.focused && self.open
    }

    /// The screen of the running agent, as text (tests).
    pub fn screen(&self) -> Option<String> {
        match &self.state {
            State::Running(s) => Some(s.text()),
            _ => None,
        }
    }

    /// The running agent's terminal size (tests).
    pub fn cells(&self) -> Option<Cells> {
        match &self.state {
            State::Running(s) => Some(s.cells()),
            _ => None,
        }
    }

    /// Send keystrokes to the running agent (tests).
    pub fn send(&self, bytes: &'static [u8]) {
        if let State::Running(s) = &self.state {
            s.write(bytes);
        }
    }

    /// Start (or restart) the agent for `book`: found and its books home
    /// made off the UI thread, then run.
    fn start(&mut self, ctx: &egui::Context, owner: &Arc<dyn Owner>, book: &str) {
        let (tx, rx) = channel();
        let (o, agent, b, ctx) = (owner.clone(), self.agent, book.to_string(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(o.agent_start(agent, &b));
            ctx.request_repaint();
        });
        self.state = State::Starting(rx);
        self.book = Some(book.to_string());
    }

    /// The panel along the bottom of the window, for `book`'s agent.
    pub fn show(&mut self, ui: &mut Ui, owner: &Arc<dyn Owner>, book: &str) {
        let p = crate::theme::pal(ui.ctx());
        // Another book is shown: its own agent, not the last one's.
        if self.book.as_deref().is_some_and(|b| b != book) {
            self.state = State::Idle;
            self.book = None;
        }
        // Starting finished?
        if let State::Starting(rx) = &self.state
            && let Ok(done) = rx.try_recv()
        {
            let ctx = ui.ctx().clone();
            self.state = match done {
                Ok(AgentStart::Ready {
                    program,
                    args,
                    dir,
                    env,
                    ..
                }) => {
                    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
                    match Session::spawn(
                        &program,
                        &args,
                        &dir,
                        &env,
                        Cells {
                            cols: 100,
                            rows: 24,
                        },
                        wake,
                    ) {
                        Ok(s) => {
                            self.focused = true;
                            State::Running(Box::new(s))
                        }
                        Err(why) => State::Failed(why),
                    }
                }
                Ok(AgentStart::Missing(i)) => State::Missing(i),
                Err(why) => State::Failed(why),
            };
        }
        if self.open && matches!(self.state, State::Idle) {
            self.start(ui.ctx(), owner, book);
        }
        let header = 34.0;
        let panel = // Its own id open and closed: a size is remembered for each.
        egui::Panel::bottom(if self.open { "dock-open" } else { "dock-closed" }).frame(
            egui::Frame::new()
                .fill(p.surface)
                .stroke(egui::Stroke::new(1.0, p.border)),
        );
        let panel = if self.open {
            panel
                .resizable(true)
                .default_size(280.0)
                .size_range(120.0..=2000.0)
        } else {
            panel.resizable(false).exact_size(header)
        };
        panel.show(ui, |ui| {
            // The panel keeps the size it has, whatever it holds just now.
            if self.open {
                ui.set_min_height(ui.available_height());
            }
            self.header(ui, owner, book);
            if !self.open {
                return;
            }
            ui.add_space(0.0);
            self.body(ui, &p);
        });
    }

    fn header(&mut self, ui: &mut Ui, owner: &Arc<dyn Owner>, book: &str) {
        let p = crate::theme::pal(ui.ctx());
        let mut restart = false;
        ui.horizontal(|ui| {
            ui.set_height(30.0);
            ui.add_space(8.0);
            let chevron = if self.open {
                icon::CARET_DOWN
            } else {
                icon::CARET_UP
            };
            if ui
                .add(egui::Button::new(RichText::new(chevron).size(14.0)).frame(false))
                .on_hover_text(if self.open {
                    "Collapse the agent dock"
                } else {
                    "Open the agent dock"
                })
                .clicked()
            {
                self.open = !self.open;
            }
            ui.label(
                RichText::new(format!("{} Agent", icon::TERMINAL_WINDOW))
                    .font(crate::theme::semibold(12.5))
                    .color(p.text),
            );
            for agent in [Agent::Claude, Agent::Codex] {
                if ui
                    .selectable_label(self.agent == agent, agent.label())
                    .on_hover_text(format!("Run {} in this book's books home", agent.label()))
                    .clicked()
                    && self.agent != agent
                {
                    self.agent = agent;
                    self.open = true;
                    restart = true;
                }
            }
            if self.open {
                let running = matches!(self.state, State::Running(_));
                let ended = matches!(&self.state, State::Running(s) if s.exited());
                if (ended || !running)
                    && !matches!(self.state, State::Starting(_))
                    && ui.button("Start").clicked()
                {
                    restart = true;
                }
                if running
                    && !ended
                    && ui
                        .button(icon::ARROW_CLOCKWISE)
                        .on_hover_text("Restart the agent")
                        .clicked()
                {
                    restart = true;
                }
                if let State::Running(s) = &self.state {
                    let title = s.title();
                    if !title.is_empty() {
                        ui.label(RichText::new(title).size(12.0).color(p.faint));
                    }
                }
            } else {
                ui.label(
                    RichText::new("Optional: any agent can use these books")
                        .size(12.0)
                        .color(p.faint),
                );
            }
        });
        if restart {
            self.state = State::Idle;
            self.start(ui.ctx(), owner, book);
        }
    }

    fn body(&mut self, ui: &mut Ui, p: &crate::theme::Palette) {
        match &mut self.state {
            State::Idle => {}
            State::Starting(_) => {
                ui.label(RichText::new("Starting the agent…").color(p.muted));
            }
            State::Failed(why) => {
                ui.add_space(6.0);
                ui.colored_label(p.warning, format!("{} {why}", icon::WARNING));
            }
            State::Missing(i) => {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("{} isn't installed here.", i.label))
                        .font(crate::theme::semibold(13.0)),
                );
                ui.label(
                    "Install it with its vendor's installer, in a terminal, then press Start:",
                );
                ui.horizontal(|ui| {
                    ui.code(&i.command);
                    if ui.button(format!("{} Copy", icon::COPY)).clicked() {
                        ui.ctx().copy_text(i.command.clone());
                    }
                });
                ui.hyperlink_to(format!("{} instructions", i.label), &i.page);
                ui.label(
                    RichText::new(
                        "You never need this: any agent you run yourself reaches the same books.",
                    )
                    .color(p.faint),
                );
            }
            State::Running(session) => {
                let (focused, _) = terminal(ui, session, p, self.focused);
                self.focused = focused;
            }
        }
    }
}

/// The terminal's cell, in points.
fn cell_size(ui: &Ui, font: &FontId) -> egui::Vec2 {
    let w = ui.ctx().fonts_mut(|f| f.glyph_width(font, 'M'));
    let h = ui.ctx().fonts_mut(|f| f.row_height(font));
    vec2(w.max(1.0), h.max(1.0))
}

/// Draw the terminal in the space left, feed it the keyboard while it has
/// it, and keep its size the panel's (INVARIANT 5). Whether it has the
/// keyboard now.
fn terminal(
    ui: &mut Ui,
    session: &mut Session,
    p: &crate::theme::Palette,
    had: bool,
) -> (bool, Rect) {
    let font = FontId::monospace(13.0);
    let cell = cell_size(ui, &font);
    let avail = ui.available_size();
    let (rect, response) = ui.allocate_exact_size(avail, Sense::click_and_drag());
    let id = response.id;
    let dark = ui.visuals().dark_mode;
    // The keyboard: given by a click in the terminal, kept until a click
    // outside it (INVARIANT 4).
    let mut focused = had;
    if response.clicked() || response.drag_started() {
        focused = true;
    } else if ui.input(|i| i.pointer.any_pressed())
        && !ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|pos| rect.contains(pos))
    {
        focused = false;
    }
    if focused {
        ui.memory_mut(|m| {
            m.request_focus(id);
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
    }
    // Its size: the panel's, in cells.
    let cols = ((rect.width() / cell.x).floor() as i64).clamp(2, 1000) as u16;
    let rows = ((rect.height() / cell.y).floor() as i64).clamp(1, 500) as u16;
    session.resize(Cells { cols, rows });

    // The mouse: select with a drag, scroll with the wheel.
    let point_at = |pos: Pos2, offset: usize| -> Point {
        let col =
            (((pos.x - rect.min.x) / cell.x).floor().max(0.0) as usize).min(cols as usize - 1);
        let line =
            (((pos.y - rect.min.y) / cell.y).floor().max(0.0) as usize).min(rows as usize - 1);
        viewport_to_point(offset, Point::new(line, Column(col)))
    };
    {
        let mut term = session.term.lock();
        let offset = term.grid().display_offset();
        if let Some(pos) = response.interact_pointer_pos() {
            if response.drag_started() {
                term.selection = Some(Selection::new(
                    SelectionType::Simple,
                    point_at(pos, offset),
                    Side::Left,
                ));
            } else if response.dragged()
                && let Some(sel) = term.selection.as_mut()
            {
                sel.update(point_at(pos, offset), Side::Right);
            } else if response.clicked() {
                term.selection = None;
            }
        }
        if response.hovered() {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            let lines = (wheel / cell.y).round() as i32;
            if lines != 0 {
                term.scroll_display(Scroll::Delta(lines));
            }
        }
    }

    // The keyboard.
    if focused {
        let app_cursor = session.term.lock().mode().contains(TermMode::APP_CURSOR);
        let bracketed = session
            .term
            .lock()
            .mode()
            .contains(TermMode::BRACKETED_PASTE);
        let events = ui.input(|i| i.events.clone());
        let mods = ui.input(|i| i.modifiers);
        for ev in events {
            match ev {
                Event::Text(t) if !mods.alt && !mods.ctrl => {
                    session.write(t.into_bytes());
                }
                Event::Paste(t) => {
                    let t = t.replace("\r\n", "\r").replace('\n', "\r");
                    if bracketed {
                        session.write(format!("\x1b[200~{t}\x1b[201~").into_bytes());
                    } else {
                        session.write(t.into_bytes());
                    }
                }
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    // Ctrl-Shift-C copies the selection; Ctrl-V is a paste (its
                    // own event), anything else a program's.
                    if modifiers.ctrl && modifiers.shift && key == Key::C {
                        if let Some(text) = session.term.lock().selection_to_string() {
                            ui.ctx().copy_text(text);
                        }
                    } else if modifiers.ctrl && !modifiers.shift && key == Key::V {
                    } else if let Some(bytes) = key_bytes(key, modifiers, app_cursor) {
                        // Typing back to the bottom.
                        session.term.lock().scroll_display(Scroll::Bottom);
                        session.write(bytes);
                    }
                }
                _ => {}
            }
        }
    }

    // The picture.
    let painter = ui.painter_at(rect);
    let (fg, bg) = (p.text, p.surface);
    painter.rect_filled(rect, 0.0, bg);
    let term = session.term.lock();
    let content = term.renderable_content();
    let offset = content.display_offset;
    for cell_at in content.display_iter {
        let Some(vp) = point_to_viewport(offset, cell_at.point) else {
            continue;
        };
        let flags = cell_at.cell.flags;
        if flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let bold = flags.contains(Flags::BOLD);
        let mut cfg = color(cell_at.cell.fg, dark, fg, bg, bold);
        let mut cbg = color(cell_at.cell.bg, dark, fg, bg, false);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut cfg, &mut cbg);
        }
        if flags.contains(Flags::DIM) {
            cfg = cfg.gamma_multiply(0.7);
        }
        let selected = content
            .selection
            .is_some_and(|sel| sel.contains(cell_at.point));
        if selected {
            cbg = p.selected;
            cfg = p.text;
        }
        let width = if flags.contains(Flags::WIDE_CHAR) {
            2.0
        } else {
            1.0
        };
        let top_left = rect.min + vec2(vp.column.0 as f32 * cell.x, vp.line as f32 * cell.y);
        let r = Rect::from_min_size(top_left, vec2(cell.x * width, cell.y));
        if cbg != bg {
            painter.rect_filled(r, 0.0, cbg);
        }
        let c = cell_at.cell.c;
        if c != ' ' && c != '\0' && !flags.contains(Flags::HIDDEN) {
            painter.text(top_left, egui::Align2::LEFT_TOP, c, font.clone(), cfg);
        }
        if flags.intersects(Flags::ALL_UNDERLINES) {
            let y = r.bottom() - 1.0;
            painter.line_segment(
                [Pos2::new(r.left(), y), Pos2::new(r.right(), y)],
                (1.0, cfg),
            );
        }
    }
    // The cursor, a block that is hollow when the keyboard is elsewhere.
    if !content.mode.contains(TermMode::SHOW_CURSOR) {
        return (focused, rect);
    }
    if let Some(vp) = point_to_viewport(offset, content.cursor.point) {
        let top_left = rect.min + vec2(vp.column.0 as f32 * cell.x, vp.line as f32 * cell.y);
        let r = Rect::from_min_size(top_left, cell);
        if focused {
            painter.rect_filled(r, 0.0, p.accent.gamma_multiply(0.8));
        } else {
            painter.rect_stroke(r, 0.0, (1.0, p.accent), egui::StrokeKind::Inside);
        }
    }
    (focused, rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_become_what_a_terminal_sends() {
        let none = Modifiers::NONE;
        let ctrl = Modifiers::CTRL;
        assert_eq!(key_bytes(Key::C, ctrl, false), Some(vec![3]));
        assert_eq!(key_bytes(Key::D, ctrl, false), Some(vec![4]));
        assert_eq!(key_bytes(Key::A, ctrl, false), Some(vec![1]));
        assert_eq!(key_bytes(Key::Enter, none, false), Some(b"\r".to_vec()));
        assert_eq!(key_bytes(Key::Backspace, none, false), Some(vec![0x7f]));
        assert_eq!(key_bytes(Key::Escape, none, false), Some(vec![0x1b]));
        assert_eq!(
            key_bytes(Key::ArrowUp, none, false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_bytes(Key::ArrowUp, none, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            key_bytes(Key::ArrowLeft, ctrl, true),
            Some(b"\x1b[1;5D".to_vec()),
            "a modifier takes the arrow out of application mode"
        );
        assert_eq!(
            key_bytes(Key::Delete, none, false),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(
            key_bytes(Key::Tab, Modifiers::SHIFT, false),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            key_bytes(Key::B, Modifiers::ALT, false),
            Some(b"\x1bb".to_vec())
        );
        // Plain letters are typed text, not keys.
        assert_eq!(key_bytes(Key::A, none, false), None);
        assert_eq!(key_bytes(Key::Space, none, false), None);
    }

    #[test]
    fn colors_follow_the_terminals_palette() {
        let (fg, bg) = (Color32::WHITE, Color32::BLACK);
        let red = color(TermColor::Named(NamedColor::Red), true, fg, bg, false);
        assert_eq!(red, ansi16(true)[1]);
        let bold_red = color(TermColor::Named(NamedColor::Red), true, fg, bg, true);
        assert_eq!(bold_red, ansi16(true)[9]);
        assert_eq!(
            color(
                TermColor::Named(NamedColor::Foreground),
                true,
                fg,
                bg,
                false
            ),
            fg
        );
        assert_eq!(
            color(
                TermColor::Spec(Rgb { r: 1, g: 2, b: 3 }),
                true,
                fg,
                bg,
                false
            ),
            Color32::from_rgb(1, 2, 3)
        );
        // The 6x6x6 cube and the grays.
        assert_eq!(
            color(TermColor::Indexed(16), true, fg, bg, false),
            Color32::from_rgb(0, 0, 0)
        );
        assert_eq!(
            color(TermColor::Indexed(231), true, fg, bg, false),
            Color32::from_rgb(255, 255, 255)
        );
        assert_eq!(
            color(TermColor::Indexed(232), true, fg, bg, false),
            Color32::from_rgb(8, 8, 8)
        );
    }
}
