//! The chart of accounts with balances, and one account's register: every
//! posting with a running balance and the balance over time.

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
use ledger_core::AccountType;
use ledger_core::analysis::balance_history;
use ledger_core::reports::general_ledger;
use rust_decimal::Decimal;

#[derive(Default)]
pub struct State {
    /// Limit the register to these dates (set by drill-downs).
    pub range: Option<(NaiveDate, NaiveDate)>,
    pub show_empty: bool,
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
            let accounts: Vec<_> = b
                .ledger
                .chart()
                .all()
                .filter(|a| {
                    a.account_type == t && (shell.state.accounts.show_empty || used.contains(&a.id))
                })
                .collect();
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
                        painter.text(
                            r.left_center() + vec2(64.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            &a.name,
                            theme::regular(13.5),
                            p.text,
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
                        painter.text(
                            r.left_center() + vec2(340.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            tags.join(" · "),
                            theme::regular(12.0),
                            p.faint,
                        );
                        painter.text(
                            r.right_center() - vec2(8.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            fmt::accounting(bal),
                            theme::semibold(13.5),
                            p.text,
                        );
                        if resp.clicked() {
                            go = Some(a.id);
                        }
                    }
                },
            );
            if let Some(id) = go {
                shell.state.accounts.range = None;
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
    let first = b.kpis.first_entry.unwrap_or(b.today);
    let (s, e) = shell
        .state
        .accounts
        .range
        .unwrap_or((first.min(b.today), b.today));
    page(ui, "account", |ui| {
        ui.label(
            RichText::new(format!("{:?} account · {}", a.account_type, a.code))
                .font(theme::regular(13.0))
                .color(p.muted),
        );
        ui.label(RichText::new(&a.name).font(theme::bold(26.0)));
        ui.add_space(4.0);
        let bal = b.balance_on(id, b.today);
        ui.label(
            RichText::new(format!("Balance today {}", fmt::dollars(bal)))
                .font(theme::regular(15.0))
                .color(p.muted),
        );
        ui.add_space(14.0);
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
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} {} – {}",
                    icon::CALENDAR_BLANK,
                    fmt::date(s),
                    fmt::date(e)
                ))
                .font(theme::semibold(13.5)),
            );
            if shell.state.accounts.range.is_some() && ui.button("Show all dates").clicked() {
                shell.state.accounts.range = None;
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
        let Ok(gl) = general_ledger(&b.ledger, s, e) else {
            return;
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
            let w = ui.available_width();
            let amt = 120.0;
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
        if let Some(entry) = go {
            shell.go(Route::Entry(entry));
        }
    });
}
