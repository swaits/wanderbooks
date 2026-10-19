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
}

impl Menu {
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
            button = button.min_size(vec2(32.0, 32.0));
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
        let (tx, rx) = channel();
        let what = job.label();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job.run(&books));
            ctx.request_repaint();
        });
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
        let snap = ledger_store::Snapshot::load(&books.data_dir).map_err(|e| e.to_string())?;
        let ledger = snap.ledger();
        // The settings the rest of the viewer shows, read as the server
        // reads them (data::settings: an older books dir that keeps none
        // in its journal reads its settings file).
        let st = &crate::data::settings_of(&books.data_dir, &snap)?;
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
                let (stem, ext) = crate::export::split_name(&p.file_name);
                let path = crate::export::save(stem, ext, &p.zip)?;
                let note = (!p.tin_gaps.is_empty()).then(|| {
                    format!(
                        "{} TIN(s) aren't in it (the README says which): ask your agent to collect them.",
                        p.tin_gaps.len()
                    )
                });
                Ok((path, note))
            }
            Job::Worksheet(year) => {
                let w = ledger_package::vendor_1099_worksheet(
                    ledger,
                    &settings,
                    year,
                    None,
                    Some(&tin),
                )?;
                let csv = ledger_package::vendor_1099_csv(&w, true);
                let name = ledger_package::vendor_1099_csv_name(year, true);
                let (stem, ext) = crate::export::split_name(&name);
                let path = crate::export::save(stem, ext, csv.as_bytes())?;
                let gaps = w
                    .lines
                    .iter()
                    .filter(|l| l.reportable && l.tin.is_none())
                    .count();
                let note = (gaps > 0).then(|| {
                    format!("{gaps} reportable payee(s) have no TIN in it: ask your agent to collect them.")
                });
                Ok((path, note))
            }
        }
    }
}
