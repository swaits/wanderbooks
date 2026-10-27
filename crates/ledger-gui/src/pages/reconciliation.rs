//! One reconciliation: the account and statement it agrees with, the
//! balance, every line it found cleared (each opening its entry), and the
//! lines on the account through that date it did not clear. Shared with
//! the account page and "Needs attention": the prompt that asks the agent
//! to reconcile an account, and where a reconciliation stands.

use super::assets::{Notice, facts, notice_box, row};
use super::page;
use crate::app::Shell;
use crate::books::Books;
use crate::fmt;
use crate::nav::Route;
use crate::theme::{self, pal};
use crate::widgets;
use chrono::NaiveDate;
use egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use ledger_core::{AccountId, EntryId, Side};
use ledger_store::{Reconciliation, Subject};
use rust_decimal::Decimal;

/// The agent prompt that reconciles an account through a date.
///
/// ```text
/// GOAL: one paste gets the agent to do the monthly reconciliation the
///       way the books require it (docs/parties-and-attachments.md,
///       Reconciliations: a statement recorded and attached, the
///       difference exactly zero).
/// INVARIANTS:
///   1. Starts "In wanderbooks,"; names the account's code and name, the
///      date (YYYY-MM-DD), books.record_statement, books.mark_cleared,
///      and books.reconcile.
///   2. Says where it last stood: reconciled through a date, or never.
/// ```
pub fn reconcile_prompt(b: &Books, account: AccountId, through: NaiveDate) -> String {
    ledger_package::reconcile_prompt(
        &b.account_label(account),
        b.reconciled_through(account),
        through,
    )
}

/// Where a reconciliation stands among its account's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Standing {
    /// The account's newest reconciliation not voided.
    Current,
    /// A later reconciliation not voided supersedes it.
    Superseded,
    /// Voided, with the reason.
    Voided(String),
}

/// A reconciliation's [`Standing`].
pub fn standing(b: &Books, r: &Reconciliation) -> Standing {
    match &r.voided {
        Some(why) => Standing::Voided(why.clone()),
        None if b.reconciled_through(r.account_id) == Some(r.statement_date) => Standing::Current,
        None => Standing::Superseded,
    }
}

/// A line's amount in the account's normal direction (a debit raises an
/// asset, a credit raises a liability).
fn signed(b: &Books, account: AccountId, side: Side, amount: Decimal) -> Decimal {
    let normal = b
        .ledger
        .chart()
        .get(account)
        .map_or(Side::Debit, |a| a.account_type.normal_side());
    if side == normal { amount } else { -amount }
}

/// Every posted line on the reconciliation's account dated on or before
/// its statement date, as (entry, line index, date, description, signed
/// amount, cleared by it), by date then entry.
pub fn lines(
    b: &Books,
    r: &Reconciliation,
) -> Vec<(EntryId, usize, NaiveDate, String, Decimal, bool)> {
    let mut out = Vec::new();
    for e in b.ledger.posted_entries() {
        if e.date() > r.statement_date {
            continue;
        }
        for (i, l) in e.lines().iter().enumerate() {
            if l.account == r.account_id {
                out.push((
                    e.id(),
                    i,
                    e.date(),
                    e.description().to_string(),
                    signed(b, r.account_id, l.side, l.amount.amount),
                    r.cleared.contains(&(e.id(), i)),
                ));
            }
        }
    }
    out.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    out
}

pub fn show(ui: &mut Ui, b: &Books, id: u64, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let Some(r) = b.reconciliation(id) else {
        page(ui, "reconciliation", |ui| {
            widgets::empty(
                ui,
                icon::CHECK_SQUARE,
                "No such reconciliation",
                "It isn't in these books.",
            );
        });
        return;
    };
    let mut go: Option<Route> = None;
    page(ui, "reconciliation", |ui| {
        ui.horizontal(|ui| {
            if widgets::link(
                ui,
                RichText::new(format!(
                    "{} {}",
                    icon::ARROW_LEFT,
                    b.account_label(r.account_id)
                ))
                .font(theme::regular(12.5))
                .color(p.muted),
            )
            .clicked()
            {
                go = Some(Route::Account(r.account_id));
            }
            widgets::copyable(
                ui,
                RichText::new(format!("· Reconciliation {id}"))
                    .font(theme::regular(12.5))
                    .color(p.muted),
                &id.to_string(),
            );
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "{} reconciled through {}",
                b.account_name(r.account_id),
                fmt::date(r.statement_date)
            ))
            .font(theme::bold(26.0)),
        );
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| match standing(b, r) {
            Standing::Current => {
                widgets::pill(
                    ui,
                    &format!("{} Current", icon::CHECK),
                    p.positive,
                    p.positive_soft,
                );
            }
            Standing::Superseded => {
                widgets::pill(ui, "Superseded by a later reconciliation", p.muted, p.hover);
            }
            Standing::Voided(why) => {
                widgets::pill(ui, &format!("Voided: {why}"), p.warning, p.warning_soft);
            }
        });
        ui.add_space(14.0);
        let all = lines(b, r);
        let cleared: Vec<_> = all.iter().filter(|l| l.5).collect();
        let outstanding: Vec<_> = all.iter().filter(|l| !l.5).collect();
        let cleared_total: Decimal = cleared.iter().map(|l| l.4).sum();
        let st = b.statement(r.attachment_id);
        super::columns(ui, 860.0, &[1.0, 1.0], |col, ui| {
            if col == 0 {
                widgets::titled_card(
                    ui,
                    "Reconciliation",
                    |_| {},
                    |ui| {
                        facts(ui, |ui, k| {
                            k(ui, "Account");
                            if widgets::link_truncated(
                                ui,
                                RichText::new(b.account_label(r.account_id)).color(p.accent),
                            )
                            .clicked()
                            {
                                go = Some(Route::Account(r.account_id));
                            }
                            ui.end_row();
                            k(ui, "Through");
                            ui.label(fmt::date(r.statement_date));
                            ui.end_row();
                            k(ui, "Statement balance");
                            widgets::copyable(
                                ui,
                                RichText::new(fmt::dollars(r.statement_balance))
                                    .font(theme::semibold(14.0)),
                                &r.statement_balance.to_string(),
                            );
                            ui.end_row();
                            k(ui, "Cleared lines");
                            ui.label(format!(
                                "{} totaling {}",
                                fmt::count(r.cleared.len(), "line", "lines"),
                                fmt::dollars(cleared_total)
                            ));
                            ui.end_row();
                            k(ui, "Books' balance then");
                            ui.label(fmt::dollars(b.balance_on(r.account_id, r.statement_date)));
                            ui.end_row();
                        });
                        ui.add_space(6.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new("Recorded only when the cleared lines agree with the statement to the cent. A reconciliation is never edited: a mistake is voided and redone.")
                                    .font(theme::regular(11.5))
                                    .color(p.faint),
                            )
                            .wrap(),
                        );
                    },
                );
            } else {
                widgets::titled_card(
                    ui,
                    "Statement",
                    |_| {},
                    |ui| {
                        let file = b.attachment(r.attachment_id);
                        facts(ui, |ui, k| {
                            if let Some(st) = st {
                                k(ui, "Period");
                                ui.label(format!(
                                    "{} – {}",
                                    fmt::date(st.period_start),
                                    fmt::date(st.period_end)
                                ));
                                ui.end_row();
                                k(ui, "Opening");
                                ui.label(fmt::dollars(st.opening_balance));
                                ui.end_row();
                                k(ui, "Closing");
                                ui.label(fmt::dollars(st.closing_balance));
                                ui.end_row();
                            }
                            k(ui, "File");
                            let name = file.map_or_else(
                                || format!("attachment {}", r.attachment_id),
                                |a| a.file_name.clone(),
                            );
                            if widgets::link_truncated(
                                ui,
                                RichText::new(format!("{} {name}", icon::FILE_PDF)).color(p.accent),
                            )
                            .clicked()
                            {
                                go = Some(Route::Document(r.attachment_id));
                            }
                            ui.end_row();
                        });
                    },
                );
            }
        });
        lines_card(
            ui,
            b,
            "Cleared lines",
            &cleared,
            "It cleared no lines (the account had no activity through this date).",
            &mut go,
        );
        ui.add_space(14.0);
        if !outstanding.is_empty() {
            notice_box(
                ui,
                &Notice {
                    problem: false,
                    text: format!(
                        "{} on the account through {} had not cleared the bank by this statement (outstanding then).",
                        fmt::count(outstanding.len(), "line", "lines"),
                        fmt::date(r.statement_date)
                    ),
                },
            );
            lines_card(ui, b, "Outstanding then", &outstanding, "", &mut go);
            ui.add_space(14.0);
        }
        super::documents::documents_card(
            ui,
            b,
            "Documents",
            &[Subject::Reconciliation(id)],
            "No documents linked to it.",
            &mut go,
        );
        ui.add_space(14.0);
        widgets::titled_card(
            ui,
            "History",
            |_| {},
            |ui| {
                for line in b.journal.iter().filter(|l| {
                    matches!(
                        l.line_type.as_str(),
                        "reconciliation.completed" | "reconciliation.voided"
                    ) && l.payload.get("id").and_then(|v| v.as_u64()) == Some(id)
                }) {
                    let ev = super::journal::describe_any(b, line);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(ev.icon).color(p.muted));
                        if widgets::link_truncated(
                            ui,
                            RichText::new(&ev.text).font(theme::regular(12.5)),
                        )
                        .clicked()
                        {
                            go = Some(Route::JournalEvent(line.seq));
                        }
                        ui.label(
                            RichText::new(format!(
                                "event {} · {}",
                                line.seq,
                                fmt::timestamp(&line.ts)
                            ))
                            .font(theme::regular(11.0))
                            .color(p.faint),
                        );
                    });
                }
            },
        );
    });
    if let Some(to) = go {
        shell.go(to);
    }
}

type Line = (EntryId, usize, NaiveDate, String, Decimal, bool);

/// A table of account lines, each opening its entry.
fn lines_card(
    ui: &mut Ui,
    b: &Books,
    title: &str,
    rows: &[&Line],
    empty: &str,
    go: &mut Option<Route>,
) {
    let p = pal(ui.ctx());
    widgets::titled_card(
        ui,
        title,
        |ui| {
            if !rows.is_empty() {
                widgets::pill(ui, &rows.len().to_string(), p.muted, p.hover);
            }
        },
        |ui| {
            if rows.is_empty() {
                ui.label(RichText::new(empty).color(p.muted));
                return;
            }
            let amt = widgets::AMOUNT_MIN + 16.0;
            let w = ui.available_width().max(104.0 + 70.0 + 200.0 + amt);
            widgets::min_width(ui, title, w, |ui| {
                let head = |t: &str, cw: f32, right: bool| (t.to_string(), cw, right, p.muted);
                row(
                    ui,
                    w,
                    &[
                        head("Date", 104.0, false),
                        head("Entry", 70.0, false),
                        head("Description", 0.0, false),
                        head("Amount", amt, true),
                    ],
                    None,
                    true,
                    false,
                );
                widgets::rule(ui);
                for (e, _, date, desc, amount, _) in rows.iter().copied() {
                    let resp = row(
                        ui,
                        w,
                        &[
                            (fmt::date(*date), 104.0, false, p.muted),
                            (format!("#{e}"), 70.0, false, p.muted),
                            (desc.clone(), 0.0, false, p.text),
                            (
                                fmt::money(*amount),
                                amt,
                                true,
                                if *amount < Decimal::ZERO {
                                    p.negative
                                } else {
                                    p.text
                                },
                            ),
                        ],
                        None,
                        false,
                        true,
                    );
                    if resp.on_hover_text("Open its entry").clicked() {
                        *go = Some(Route::Entry(*e));
                    }
                }
            });
            let _ = b;
        },
    );
}
