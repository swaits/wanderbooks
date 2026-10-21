//! The chart of accounts with balances, and one account's register: every
//! posting with a running balance and the balance over time, its bank or
//! card statements, its reconciliation history, and its documents.

use super::assets::row;
use super::page;
use crate::app::Shell;
use crate::books::Books;
use crate::fmt;
use crate::nav::Route;
use crate::theme::{self, pal};
use crate::widgets;
use chrono::NaiveDate;
use egui::{CornerRadius, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as icon;
use ledger_core::AccountId;
use ledger_core::AccountType;
use ledger_core::analysis::balance_history;
use ledger_core::reports::general_ledger;
use ledger_store::Subject;
use rust_decimal::Decimal;

#[derive(Default)]
pub struct State {
    /// Which dates the register shows.
    pub range: RegisterRange,
    pub show_empty: bool,
}

/// Which dates an account's register shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RegisterRange {
    /// What a register opens on when nobody chose dates: the fiscal year
    /// to date (rules.md B-30, `Books::default_period`), its opening
    /// balance carried in.
    #[default]
    Default,
    /// Every date: the books' first entry through the later of today
    /// and the last ("Show all dates", `Books::all_dates`).
    All,
    /// These dates: a drill-down's, or a `books.show` target's.
    Dates(NaiveDate, NaiveDate),
}

impl RegisterRange {
    /// The first and last day shown.
    pub fn dates(self, b: &Books) -> (NaiveDate, NaiveDate) {
        match self {
            RegisterRange::Default => b.default_period(),
            RegisterRange::All => b.all_dates(),
            RegisterRange::Dates(s, e) => (s, e),
        }
    }
}

pub fn show_chart(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    page(ui, "accounts", |ui| {
        widgets::page_header(
            ui,
            "Accounts",
            "Every account in the chart with today's balance. Click one for its register.",
        );
        ui.checkbox(
            &mut shell.state.accounts.show_empty,
            "Show accounts with no activity",
        );
        ui.add_space(10.0);
        let used: std::collections::HashSet<u64> = b
            .entries
            .iter()
            .flat_map(|e| e.accounts.iter().copied())
            .collect();
        for (t, title) in [
            (AccountType::Asset, "Assets"),
            (AccountType::Liability, "Liabilities"),
            (AccountType::Equity, "Equity"),
            (AccountType::Revenue, "Income"),
            (AccountType::Expense, "Expenses"),
        ] {
            let mut accounts: Vec<_> = b
                .ledger
                .chart()
                .all()
                .filter(|a| {
                    a.account_type == t && (shell.state.accounts.show_empty || used.contains(&a.id))
                })
                .collect();
            // In the chart's order: by code, as every report lists them.
            accounts.sort_by(|x, y| x.code.cmp(&y.code));
            if accounts.is_empty() {
                continue;
            }
            let mut go = None;
            widgets::titled_card(
                ui,
                title,
                |_| {},
                |ui| {
                    for a in &accounts {
                        let bal = b.balance_on(a.id, b.today);
                        let (r, resp) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(r, CornerRadius::same(6), p.hover);
                        }
                        let painter = ui.painter_at(r);
                        painter.text(
                            r.left_center() + vec2(8.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            &a.code,
                            theme::regular(13.0),
                            p.muted,
                        );
                        let bal_text = fmt::accounting(bal);
                        let bal_w = painter
                            .layout_no_wrap(bal_text.clone(), theme::semibold(13.5), p.text)
                            .size()
                            .x;
                        // Tags sit in their own column when there's room for
                        // it; narrower, they move to the row's tooltip.
                        let tags_x = 340.0;
                        let tags_room = r.width() - tags_x - bal_w - 24.0;
                        let name_room = if tags_room >= 120.0 {
                            tags_x - 64.0 - 12.0
                        } else {
                            r.width() - 64.0 - bal_w - 24.0
                        };
                        widgets::clipped_text(
                            &painter,
                            r.left_center() + vec2(64.0, 0.0),
                            &a.name,
                            theme::regular(13.5),
                            p.text,
                            name_room,
                        );
                        let mut tags = Vec::new();
                        if b.cash_accounts.contains(&a.id) {
                            tags.push("cash".to_string());
                        }
                        if let Some(c) = &a.tax_category {
                            let pct = a
                                .deductible_percent
                                .map(|p| format!(" {}%", p.normalize()))
                                .unwrap_or_default();
                            tags.push(format!("tax: {}{pct}", c.replace('_', " ")));
                        } else if matches!(t, AccountType::Revenue | AccountType::Expense) {
                            tags.push("no tax category".into());
                        }
                        if a.placeholder {
                            tags.push("header".into());
                        }
                        let tags = tags.join(" · ");
                        if tags_room >= 120.0 {
                            widgets::clipped_text(
                                &painter,
                                r.left_center() + vec2(tags_x, 0.0),
                                &tags,
                                theme::regular(12.0),
                                p.faint,
                                tags_room,
                            );
                        }
                        painter.text(
                            r.right_center() - vec2(8.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            bal_text,
                            theme::semibold(13.5),
                            p.text,
                        );
                        let resp = if tags_room < 120.0 && !tags.is_empty() {
                            resp.on_hover_text(format!("{} {} · {tags}", a.code, a.name))
                        } else {
                            resp
                        };
                        if resp.clicked() {
                            go = Some(a.id);
                        }
                    }
                },
            );
            if let Some(id) = go {
                shell.state.accounts.range = RegisterRange::Default;
                shell.go(Route::Account(id));
            }
            ui.add_space(12.0);
        }
    });
}

pub fn show_account(ui: &mut Ui, b: &Books, id: u64, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let Some(a) = b.ledger.chart().get(id) else {
        return;
    };
    let (s, e) = shell.state.accounts.range.dates(b);
    page(ui, "account", |ui| {
        widgets::copyable(
            ui,
            RichText::new(format!("{:?} account · {}", a.account_type, a.code))
                .font(theme::regular(13.0))
                .color(p.muted),
            &a.code,
        );
        ui.label(RichText::new(&a.name).font(theme::bold(26.0)));
        ui.add_space(4.0);
        let bal = b.balance_on(id, b.today);
        widgets::copyable(
            ui,
            RichText::new(format!("Balance today {}", fmt::dollars(bal)))
                .font(theme::regular(15.0))
                .color(p.muted),
            &fmt::plain(&fmt::dollars(bal)).unwrap_or_default(),
        );
        ui.add_space(6.0);
        let mut go_to: Option<Route> = None;
        reconciled_badge(ui, b, id);
        ui.add_space(8.0);
        let hist = balance_history(&b.ledger, &[id]);
        if hist.len() > 1 && a.account_type.is_balance_sheet() {
            widgets::titled_card(
                ui,
                "Balance over time",
                |_| {},
                |ui| crate::charts::balance_line(ui, &hist, b.today, 180.0),
            );
            ui.add_space(12.0);
        }
        statements_and_reconciliations(ui, b, id, &mut go_to);
        super::documents::documents_card(
            ui,
            b,
            "Documents",
            &[Subject::Account(id)],
            "No documents linked to this account (statements, the agreement that opened it).",
            &mut go_to,
        );
        ui.add_space(14.0);
        if let Some(r) = go_to.take() {
            shell.go(r);
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} {} – {}",
                    icon::CALENDAR_BLANK,
                    fmt::date(s),
                    fmt::date(e)
                ))
                .font(theme::semibold(13.5)),
            );
            // "Show all dates" one click away from any other range; from
            // all dates, back to the fiscal year to date.
            if shell.state.accounts.range == RegisterRange::All {
                if ui.button("Fiscal year to date").clicked() {
                    shell.state.accounts.range = RegisterRange::Default;
                }
            } else if ui.button("Show all dates").clicked() {
                shell.state.accounts.range = RegisterRange::All;
            }
            if ui
                .button(format!("{} Find in transactions", icon::LIST_BULLETS))
                .clicked()
            {
                shell.state.transactions.account = Some(id);
                shell.go(Route::Transactions);
            }
        });
        ui.add_space(8.0);
        let gl = match general_ledger(&b.ledger, s, e) {
            Ok(gl) => gl,
            Err(why) => {
                // Never a blank page: say why these dates can't be shown.
                widgets::card(ui, |ui| {
                    widgets::empty(
                        ui,
                        icon::WARNING,
                        "These dates can't be shown",
                        &format!("{why}. Choose other dates."),
                    )
                });
                return;
            }
        };
        let Some(acct) = gl.accounts.iter().find(|x| x.code == a.code) else {
            widgets::card(ui, |ui| {
                widgets::empty(
                    ui,
                    icon::EMPTY,
                    "No activity",
                    "Nothing posted to this account in this range.",
                )
            });
            return;
        };
        let mut go = None;
        widgets::card(ui, |ui| {
            let amt = widgets::amount_col_w(ui.available_width(), 3, 90.0 + 120.0, 120.0);
            let w = ui.available_width().max(90.0 + 120.0 + amt * 3.0);
            widgets::min_width(ui, "register", w, |ui| {
                let header = |ui: &mut Ui, vals: [&str; 5]| {
                    let (r, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
                    ui.painter().text(
                        r.left_center() + vec2(4.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        vals[0],
                        theme::semibold(12.0),
                        p.muted,
                    );
                    ui.painter().text(
                        r.left_center() + vec2(90.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        vals[1],
                        theme::semibold(12.0),
                        p.muted,
                    );
                    for (i, v) in vals[2..].iter().enumerate() {
                        ui.painter().text(
                            egui::pos2(r.right() - amt * (2 - i) as f32 - 4.0, r.center().y),
                            egui::Align2::RIGHT_CENTER,
                            *v,
                            theme::semibold(12.0),
                            p.muted,
                        );
                    }
                };
                header(ui, ["Date", "Description", "Debit", "Credit", "Balance"]);
                widgets::rule(ui);
                let line = |ui: &mut Ui,
                            date: String,
                            desc: String,
                            sub: String,
                            d: Decimal,
                            c: Decimal,
                            bal: Decimal,
                            strong: bool|
                 -> egui::Response {
                    let (r, resp) = ui.allocate_exact_size(
                        vec2(w, if sub.is_empty() { 30.0 } else { 42.0 }),
                        Sense::click(),
                    );
                    resp.context_menu(|ui| {
                        for (label, v) in [("Copy debit", d), ("Copy credit", c)] {
                            if !v.is_zero() {
                                widgets::copy_item(
                                    ui,
                                    label,
                                    &fmt::plain(&fmt::money(v)).unwrap_or_default(),
                                );
                            }
                        }
                        widgets::copy_item(
                            ui,
                            "Copy balance",
                            &fmt::plain(&fmt::money(bal)).unwrap_or_default(),
                        );
                        if let Some(id) = sub.strip_prefix('#').and_then(|t| t.split(' ').next()) {
                            widgets::copy_item(ui, "Copy entry number", &format!("#{id}"));
                        }
                    });
                    if resp.hovered() && !strong {
                        ui.painter().rect_filled(r, CornerRadius::same(6), p.hover);
                    }
                    let painter = ui.painter_at(r);
                    let font = if strong {
                        theme::semibold(13.0)
                    } else {
                        theme::regular(13.0)
                    };
                    let y = if sub.is_empty() {
                        r.center().y
                    } else {
                        r.center().y - 7.0
                    };
                    painter.text(
                        egui::pos2(r.left() + 4.0, y),
                        egui::Align2::LEFT_CENTER,
                        date,
                        theme::regular(12.5),
                        p.muted,
                    );
                    // Leave room for the debit, credit, and balance columns.
                    let text_w = r.width() - 90.0 - amt * 3.0 - 12.0;
                    widgets::clipped_text(
                        &painter,
                        egui::pos2(r.left() + 90.0, y),
                        &desc,
                        font.clone(),
                        p.text,
                        text_w,
                    );
                    if !sub.is_empty() {
                        widgets::clipped_text(
                            &painter,
                            egui::pos2(r.left() + 90.0, r.center().y + 9.0),
                            &sub,
                            theme::regular(11.5),
                            p.faint,
                            text_w,
                        );
                    }
                    for (i, v) in [d, c].iter().enumerate() {
                        if !v.is_zero() {
                            painter.text(
                                egui::pos2(r.right() - amt * (2 - i) as f32 - 4.0, y),
                                egui::Align2::RIGHT_CENTER,
                                fmt::money(*v),
                                font.clone(),
                                p.text,
                            );
                        }
                    }
                    painter.text(
                        egui::pos2(r.right() - 4.0, y),
                        egui::Align2::RIGHT_CENTER,
                        fmt::accounting(bal),
                        font,
                        p.text,
                    );
                    resp
                };
                line(
                    ui,
                    fmt::date_short(s),
                    "Beginning balance".into(),
                    String::new(),
                    Decimal::ZERO,
                    Decimal::ZERO,
                    acct.beginning_balance,
                    true,
                );
                for post in acct.postings.iter().rev() {
                    let r = line(
                        ui,
                        fmt::date_short(post.date),
                        post.description.clone(),
                        format!("#{} · {}", post.entry_id, post.split),
                        post.debit,
                        post.credit,
                        post.balance,
                        false,
                    );
                    if r.clicked() {
                        go = Some(post.entry_id);
                    }
                }
                widgets::rule(ui);
                line(
                    ui,
                    fmt::date_short(e),
                    "Ending balance".into(),
                    String::new(),
                    acct.total_debits,
                    acct.total_credits,
                    acct.ending_balance,
                    true,
                );
            });
        });
        if let Some(entry) = go {
            shell.go(Route::Entry(entry));
        }
    });
}

/// Where the account stands on reconciliation: "Reconciled through
/// <date>" (in green when that covers last month, else with a notice and
/// the agent prompt), or "Never reconciled" for an account the books
/// expect to reconcile or one with statements. Nothing for other
/// accounts.
fn reconciled_badge(ui: &mut Ui, b: &Books, id: AccountId) {
    let p = pal(ui.ctx());
    let expected = b.reconcilable_accounts().contains(&id);
    let through = b.reconciled_through(id);
    if !expected && through.is_none() && b.statements_for(id).is_empty() {
        return;
    }
    let due = b.reconciliation_due(id);
    ui.horizontal_wrapped(|ui| {
        match through {
            Some(d) => {
                let (fg, bg) = if due.is_some() {
                    (p.warning, p.warning_soft)
                } else {
                    (p.positive, p.positive_soft)
                };
                widgets::pill(
                    ui,
                    &format!("{} Reconciled through {}", icon::CHECK_SQUARE, fmt::date(d)),
                    fg,
                    bg,
                );
            }
            None => {
                widgets::pill(ui, "Never reconciled", p.warning, p.warning_soft);
            }
        }
        if let Some(d) = due {
            ui.label(
                RichText::new(format!(
                    "Due through {}: the monthly close reconciles every bank and card account to its statement.",
                    fmt::date(d)
                ))
                .font(theme::regular(12.5))
                .color(p.muted),
            );
            widgets::copy_prompt(ui, &super::reconciliation::reconcile_prompt(b, id, d));
        }
    });
}

/// The account's statements (period, opening, closing, the file) and its
/// reconciliations (date, statement balance, cleared lines, standing),
/// side by side when there's room. Nothing when it has neither.
fn statements_and_reconciliations(ui: &mut Ui, b: &Books, id: AccountId, go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    let statements = b.statements_for(id);
    let recs = b.reconciliations_for(id);
    if statements.is_empty() && recs.is_empty() {
        return;
    }
    super::columns(ui, 1100.0, &[1.0, 1.0], |col, ui| {
        if col == 0 {
            widgets::titled_card(
                ui,
                "Statements",
                |_| {},
                |ui| {
                    if statements.is_empty() {
                        ui.label(RichText::new("No statements recorded.").color(p.muted));
                        return;
                    }
                    let amt = widgets::AMOUNT_MIN + 8.0;
                    let w = ui.available_width().max(180.0 + amt * 2.0 + 120.0);
                    widgets::min_width(ui, "statements", w, |ui| {
                        let head =
                            |t: &str, cw: f32, right: bool| (t.to_string(), cw, right, p.muted);
                        row(
                            ui,
                            w,
                            &[
                                head("Period", 0.0, false),
                                head("Opening", amt, true),
                                head("Closing", amt, true),
                            ],
                            None,
                            true,
                            false,
                        );
                        widgets::rule(ui);
                        for st in statements.iter().rev() {
                            let file = b
                                .attachment(st.attachment_id)
                                .map_or_else(String::new, |a| a.file_name.clone());
                            let resp = row(
                                ui,
                                w,
                                &[
                                    (
                                        format!(
                                            "{} – {}",
                                            fmt::date_short(st.period_start),
                                            fmt::date(st.period_end)
                                        ),
                                        0.0,
                                        false,
                                        p.text,
                                    ),
                                    (fmt::money(st.opening_balance), amt, true, p.muted),
                                    (fmt::money(st.closing_balance), amt, true, p.text),
                                ],
                                Some((&format!("{} {file}", icon::FILE_PDF), p.faint)),
                                false,
                                true,
                            );
                            if resp.on_hover_text("Open the statement").clicked() {
                                *go = Some(Route::Document(st.attachment_id));
                            }
                        }
                    });
                },
            );
        } else {
            widgets::titled_card(
                ui,
                "Reconciliations",
                |_| {},
                |ui| {
                    if recs.is_empty() {
                        ui.label(RichText::new("Never reconciled.").color(p.muted));
                        return;
                    }
                    let amt = widgets::AMOUNT_MIN + 8.0;
                    let w = ui.available_width().max(130.0 + amt + 70.0 + 110.0);
                    widgets::min_width(ui, "reconciliations", w, |ui| {
                        let head =
                            |t: &str, cw: f32, right: bool| (t.to_string(), cw, right, p.muted);
                        row(
                            ui,
                            w,
                            &[
                                head("Through", 0.0, false),
                                head("Balance", amt, true),
                                head("Lines", 70.0, true),
                            ],
                            None,
                            true,
                            false,
                        );
                        widgets::rule(ui);
                        for r in recs.iter().rev() {
                            use super::reconciliation::Standing;
                            let (sub, color) = match super::reconciliation::standing(b, r) {
                                Standing::Current => ("Current".to_string(), p.positive),
                                Standing::Superseded => ("Superseded".to_string(), p.faint),
                                Standing::Voided(why) => (format!("Voided: {why}"), p.warning),
                            };
                            let text = if r.voided.is_some() { p.muted } else { p.text };
                            let resp = row(
                                ui,
                                w,
                                &[
                                    (fmt::date(r.statement_date), 0.0, false, text),
                                    (fmt::money(r.statement_balance), amt, true, text),
                                    (r.cleared.len().to_string(), 70.0, true, p.muted),
                                ],
                                Some((&sub, color)),
                                false,
                                true,
                            );
                            if resp.on_hover_text("Open this reconciliation").clicked() {
                                *go = Some(Route::Reconciliation(r.id));
                            }
                        }
                    });
                },
            );
        }
    });
}
