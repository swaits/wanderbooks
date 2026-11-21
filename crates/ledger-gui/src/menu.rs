//! The viewer's menu (plan P2 "Viewer menu", D-29): the files the books
//! make for the human, made by the viewer itself into the Downloads
//! folder, with their full TINs from the vault. Nothing here writes the
//! books.
//!
//! ```text
//! GOAL: the owner gets the CPA package and the 1099 worksheet from the
//!       viewer, the same files the agent would hand over.
//! SOURCE: plan P2 ("Export for CPA" (done by the viewer, into
//!         Downloads)); D-29 ("The viewer's menu makes the same files into
//!         Downloads"); ledger_package (the one builder).
//! INVARIANTS:
//!   1. Each is built from a fresh read-only snapshot of the books (its
//!      chain checked when it loads) by ledger_package, the builder the
//!      server uses: the same files, the same figures.
//!   2. The TINs come from the vault, read-only; a locked vault or a
//!      missing TIN never stops the file: the package lists the gaps.
//!   3. The file is saved as a new file in Downloads (export::save), never
//!      over one, off the UI thread; what came of it is shown.
//!   4. An export belongs to its tab: closing the tab, with no question,
//!      tells the job to stop (jobs.rs). It checks at its phase
//!      boundaries (before and after the books are read, after the file
//!      is built, and just before it is saved) and stops there, saving
//!      nothing: the save is one atomic write of a new file
//!      (`ledger_store::saving`: written aside whole and linked in), so a
//!      file is whole or is not there, and one that a stop reaches
//!      too late to prevent (it came after the last check) is removed
//!      again, being the job's own new file. What the builder
//!      (`ledger_package`) does inside one call is not interrupted.
//! ```

use crate::books::Books;
use chrono::Datelike;
use egui::{RichText, Ui, vec2};
use egui_phosphor::regular as icon;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

/// What a menu job came to: the file saved, and anything to say about it.
pub type Done = Result<(PathBuf, Option<String>), String>;

/// The menu's state: the job under way, and the last one's outcome.
#[derive(Default)]
pub struct Menu {
    job: Option<(String, Receiver<Done>)>,
    done: Option<(String, Done)>,
    /// The tab's threads: the job is one of them (INVARIANT 4).
    jobs: crate::jobs::Spawner,
}

impl Menu {
    /// A menu whose jobs are threads of a tab.
    pub fn in_tab(jobs: crate::jobs::Spawner) -> Self {
        Self {
            jobs,
            ..Default::default()
        }
    }

    /// The menu button, its items, and a note of the last job's outcome.
    pub fn ui(&mut self, ui: &mut Ui, books: &Arc<Books>, compact: bool) {
        let p = crate::theme::pal(ui.ctx());
        if let Some((what, rx)) = &self.job
            && let Ok(done) = rx.try_recv()
        {
            self.done = Some((what.clone(), done));
            self.job = None;
        }
        let text = if compact {
            RichText::new(icon::ARCHIVE).size(16.0).color(p.muted)
        } else {
            RichText::new(format!("{} Export", icon::ARCHIVE))
                .font(crate::theme::regular(12.5))
                .color(p.muted)
        };
        let mut button = egui::Button::new(text).frame(false);
        if compact {
            button = button.min_size(vec2(32.0, ui.spacing().interact_size.y + 4.0));
        }
        let busy = self.job.is_some();
        let year = books.today.year();
        let mut start: Option<Job> = None;
        let response = ui
            .add_enabled_ui(!busy, |ui| {
                egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                    ui.set_min_width(240.0);
                    if ui
                        .button(format!("{} Export for CPA", icon::PACKAGE))
                        .on_hover_text(
                            "The CPA package as of today, full TINs included, into your Downloads folder",
                        )
                        .clicked()
                    {
                        start = Some(Job::Package);
                    }
                    ui.separator();
                    for y in [year - 1, year] {
                        if ui
                            .button(format!("{} 1099 worksheet, {y}", icon::FILE_TEXT))
                            .on_hover_text(
                                "The year's 1099 worksheet as a CSV, full TINs included, into your Downloads folder",
                            )
                            .clicked()
                        {
                            start = Some(Job::Worksheet(y));
                        }
                    }
                })
            })
            .inner;
        if let Some(job) = start {
            self.start(ui.ctx(), books.clone(), job);
        }
        let said = if self.job.is_some() {
            Some(("Making the file…".to_string(), p.muted))
        } else {
            self.done.as_ref().map(|(what, done)| match done {
                Ok((path, None)) => (format!("{what}: saved {}", path.display()), p.muted),
                Ok((path, Some(note))) => (
                    format!("{what}: saved {}. {note}", path.display()),
                    p.warning,
                ),
                Err(why) => (format!("{what} couldn't be made: {why}"), p.warning),
            })
        };
        // Beside the button; in the narrow rail, on hovering it.
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

    fn start(&mut self, ctx: &egui::Context, books: Arc<Books>, job: Job) {
        let what = job.label();
        self.start_with(ctx, what, Box::new(move |stop| job.run_until(&books, stop)));
    }

    /// Run `work` as an export job: off the UI thread, one of the tab's
    /// threads, told to stop with the tab (INVARIANT 4).
    pub fn start_with(
        &mut self,
        ctx: &egui::Context,
        what: String,
        work: Box<dyn FnOnce(&crate::jobs::Stop) -> Done + Send>,
    ) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let started = self
            .jobs
            .spawn("export", crate::jobs::Hold::Free, move |stop| {
                let _ = tx.send(work(stop));
                ctx.request_repaint();
            });
        if !started {
            self.done = Some((what, Err("the system gave no thread for it".into())));
            return;
        }
        self.job = Some((what, rx));
        self.done = None;
    }
}

/// A file the menu makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    /// The CPA package as of the books' today.
    Package,
    /// The 1099 worksheet for a calendar year.
    Worksheet(i32),
}

impl Job {
    fn label(self) -> String {
        match self {
            Job::Package => "The CPA package".to_string(),
            Job::Worksheet(y) => format!("The {y} 1099 worksheet"),
        }
    }

    /// Make the file and save it (INVARIANTS 1-3).
    pub fn run(self, books: &Books) -> Done {
        self.run_until(books, &crate::jobs::Stop::never())
    }

    /// [`Self::run`], stopping at its phase boundaries when `stop` is
    /// given, saving nothing (INVARIANT 4).
    pub fn run_until(self, books: &Books, stop: &crate::jobs::Stop) -> Done {
        let stopped = || -> Done { Err("stopped: its tab was closed".into()) };
        if stop.requested() {
            return stopped();
        }
        let snap = ledger_store::Snapshot::load(&books.data_dir).map_err(|e| e.to_string())?;
        if stop.requested() {
            return stopped();
        }
        let ledger = snap.ledger();
        // The settings the rest of the viewer shows, read as the server
        // reads them (data::settings).
        let st = &crate::data::settings(&snap);
        let settings = ledger_package::Settings {
            company: &st.company,
            cash_accounts: &st.cash_accounts,
            financing_accounts: &st.financing_accounts,
            investing_accounts: &st.investing_accounts,
        };
        let vault = books.open_vault();
        let tin = |id| ledger_package::vault_tin(ledger, &vault, id);
        match self {
            Job::Package => {
                let p = ledger_package::build_package(
                    &snap,
                    &settings,
                    &ledger_package::PackageRequest {
                        as_of: books.today,
                        prepared_on: books.today,
                        cash_accounts: &[],
                        financing_accounts: &[],
                        investing_accounts: &[],
                        include_receipts: true,
                        tins: Some(&tin),
                    },
                )?;
                if stop.requested() {
                    return stopped();
                }
                let (stem, ext) = crate::export::split_name(&p.file_name);
                let path = crate::export::save(stem, ext, &p.zip)?;
                if stop.requested() {
                    // Reached too late to stop the write: the new file it
                    // made is the job's own, and goes.
                    let _ = std::fs::remove_file(&path);
                    return stopped();
                }
                let mut notes = Vec::new();
                if !p.tin_gaps.is_empty() {
                    notes.push(format!(
                        "{} TIN(s) aren't in it (the README says which): ask your assistant to collect them.",
                        p.tin_gaps.len()
                    ));
                }
                if p.iif_changes > 0 {
                    notes.push(format!(
                        "{} account or class name(s), number(s), place(s) or memo(s) are written differently in its QuickBooks file, to fit QuickBooks Desktop: the README says how.",
                        p.iif_changes
                    ));
                }
                let note = (!notes.is_empty()).then(|| notes.join(" "));
                Ok((path, note))
            }
            Job::Worksheet(year) => {
                let w = ledger_package::vendor_1099_worksheet(
                    ledger,
                    &settings,
                    year,
                    None,
                    None,
                    Some(&tin),
                )?;
                let csv = ledger_package::vendor_1099_csv(&w, true);
                let name = ledger_package::vendor_1099_csv_name(year, true);
                if stop.requested() {
                    return stopped();
                }
                let (stem, ext) = crate::export::split_name(&name);
                let path = crate::export::save(stem, ext, csv.as_bytes())?;
                if stop.requested() {
                    let _ = std::fs::remove_file(&path);
                    return stopped();
                }
                let gaps = w
                    .lines
                    .iter()
                    .filter(|l| l.may_file() && l.tin.is_none())
                    .count();
                let note = (gaps > 0).then(|| {
                    format!("{gaps} payee(s) a 1099 is or may be due for have no TIN in it: ask your assistant to collect them.")
                });
                Ok((path, note))
            }
        }
    }
}
