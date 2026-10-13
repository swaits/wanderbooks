//! Profit: are we making money, how is it trending, and what moved.
//! Spending by category this month against the recent average, income by
//! customer and by class.

use super::{columns, page};
use crate::app::Shell;
use crate::books::Books;
use crate::charts::{self, BarGroup};
use crate::fmt;
use crate::nav::{ReportKind, Route};
use crate::theme::{self, pal};
use crate::widgets;
use chrono::{Days, Months};
use egui::{CornerRadius, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as icon;
use ledger_core::analysis::{Interval, interval_start, pnl_by_interval};
use ledger_core::reports::profit_and_loss_for_period;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

fn pct(part: Decimal, whole: Decimal) -> Option<Decimal> {
    (!whole.is_zero()).then(|| (part / whole * Decimal::from(100)).round_dp(1))
}

pub fn show(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    page(ui, "profit", |ui| {
        let (ys, ye) = b.fiscal_ytd();
        let ytd = profit_and_loss_for_period(&b.ledger, ys, ye, None).ok();
        let (rev, exp) = ytd
            .as_ref()
            .map(|p| (p.total_revenue, p.total_expenses))
            .unwrap_or_default();
        let net = rev - exp;
        let margin = pct(net, rev);
        let sentence = match margin {
            Some(m) => format!(
                "This year: {} of income, {} of spending, {} {} — a {}% margin.",
                fmt::dollars(rev),
                fmt::dollars(exp),
                fmt::dollars(net.abs()),
                if net >= Decimal::ZERO {
                    "profit"
                } else {
                    "loss"
                },
                m
            ),
            None if exp.is_zero() => "No income or spending recorded this year yet.".into(),
            None => format!(
                "This year: {} of spending and no income yet.",
                fmt::dollars(exp)
            ),
        };
        widgets::page_header(ui, "Profit", &sentence);

        let first = b.kpis.first_entry.unwrap_or(b.today);
        let (start, end) = b.trailing_months(12);
        let start = start.max(interval_start(first, Interval::Month));
        let months = pnl_by_interval(&b.ledger, start, end, Interval::Month).unwrap_or_default();
        widgets::titled_card(
            ui,
            "By month",
            |_| {},
            |ui| {
                if months.is_empty() {
                    widgets::empty(ui, icon::CHART_BAR, "No months yet", "");
                    return;
                }
                let groups: Vec<BarGroup> = months
                    .iter()
                    .map(|m| BarGroup {
                        label: m.start.format("%b").to_string(),
                        values: vec![m.revenue, m.expenses, m.net()],
                    })
                    .collect();
                if let Some(i) = charts::bar_chart(
                    ui,
                    &groups,
                    &[
                        ("Income", p.series[1]),
                        ("Spending", p.series[2]),
                        ("Net", p.series[0]),
                    ],
                    230.0,
                ) {
                    shell.state.reports.range =
                        crate::pages::reports::Range::Custom(months[i].start, months[i].end);
                    shell.go(Route::Reports(ReportKind::ProfitAndLoss));
                }
                ui.add_space(10.0);
                // The same numbers as a table, with the margin.
                let cw = widgets::amount_col_w(ui.available_width(), 4, 100.0, 130.0);
                let w = ui.available_width().max(100.0 + cw * 4.0);
                widgets::min_width(ui, "by_month_table", w, |ui| {
                    let header = |ui: &mut Ui, cells: [&str; 5]| {
                        let (r, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
                        ui.painter().text(
                            r.left_center() + vec2(4.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            cells[0],
                            theme::semibold(12.0),
                            p.muted,
                        );
                        for (i, c) in cells[1..].iter().enumerate() {
                            ui.painter().text(
                                egui::pos2(r.right() - cw * (3 - i) as f32 - 4.0, r.center().y),
                                egui::Align2::RIGHT_CENTER,
                                *c,
                                theme::semibold(12.0),
                                p.muted,
                            );
                        }
                    };
                    header(ui, ["Month", "Income", "Spending", "Net", "Margin"]);
                    for m in months.iter().rev() {
                        let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(r, CornerRadius::same(4), p.hover);
                        }
                        ui.painter().text(
                            r.left_center() + vec2(4.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            fmt::month(m.start),
                            theme::regular(13.0),
                            p.text,
                        );
                        let vals = [
                            fmt::money(m.revenue),
                            fmt::money(m.expenses),
                            fmt::accounting(m.net()),
                            pct(m.net(), m.revenue)
                                .map(|x| format!("{x}%"))
                                .unwrap_or_else(|| "—".into()),
                        ];
                        for (i, v) in vals.iter().enumerate() {
                            let color = if i == 2 && m.net() < Decimal::ZERO {
                                p.negative
                            } else {
                                p.text
                            };
                            ui.painter().text(
                                egui::pos2(r.right() - cw * (3 - i) as f32 - 4.0, r.center().y),
                                egui::Align2::RIGHT_CENTER,
                                v,
                                theme::regular(13.0),
                                color,
                            );
                        }
                        if resp.clicked() {
                            shell.state.reports.range =
                                crate::pages::reports::Range::Custom(m.start, m.end);
                            shell.go(Route::Reports(ReportKind::ProfitAndLoss));
                        }
                    }
                });
            },
        );
        ui.add_space(16.0);
        spending_changes(ui, b, shell);
        columns(ui, 980.0, &[1.0, 1.0], |i, ui| match i {
            0 => by_customer(ui, b),
            _ => by_class(ui, b),
        });
    });
}

/// Each expense account: this month so far, last month, the three-month
/// average, and how far this month is running from it.
fn spending_changes(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let ms = b.month_start();
    let prev =
        profit_and_loss_for_period(&b.ledger, ms - Months::new(1), ms - Days::new(1), None).ok();
    let three =
        profit_and_loss_for_period(&b.ledger, ms - Months::new(3), ms - Days::new(1), None).ok();
    let now = profit_and_loss_for_period(&b.ledger, ms, b.today, None).ok();
    let mut rows: BTreeMap<String, (String, Decimal, Decimal, Decimal)> = BTreeMap::new();
    let mut add = |s: &Option<ledger_core::ProfitAndLossStatement>, slot: usize| {
        for l in s.iter().flat_map(|s| &s.expenses) {
            let e = rows.entry(l.code.clone()).or_insert((
                l.name.clone(),
                Decimal::ZERO,
                Decimal::ZERO,
                Decimal::ZERO,
            ));
            match slot {
                0 => e.1 = l.amount,
                1 => e.2 = l.amount,
                _ => e.3 = l.amount / Decimal::from(3),
            }
        }
    };
    add(&now, 0);
    add(&prev, 1);
    add(&three, 2);
    let mut rows: Vec<_> = rows.into_iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1.3.max(r.1.1)));
    let mut go = None;
    widgets::titled_card(
        ui,
        "Spending by category",
        |ui| {
            ui.label(
                RichText::new("this month so far vs. the last three months")
                    .font(theme::regular(12.0))
                    .color(p.muted),
            );
        },
        |ui| {
            if rows.is_empty() {
                widgets::empty(ui, icon::RECEIPT, "No spending yet", "");
                return;
            }
            let label_min = 180.0;
            let cw = widgets::amount_col_w(ui.available_width(), 3, label_min, 130.0);
            let w = ui.available_width().max(label_min + cw * 3.0);
            widgets::min_width(ui, "spending_table", w, |ui| {
                let (r, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
                for (i, c) in ["This month", "Last month", "3-month avg"]
                    .iter()
                    .enumerate()
                {
                    ui.painter().text(
                        egui::pos2(r.right() - cw * (2 - i) as f32 - 4.0, r.center().y),
                        egui::Align2::RIGHT_CENTER,
                        *c,
                        theme::semibold(12.0),
                        p.muted,
                    );
                }
                ui.painter().text(
                    r.left_center() + vec2(4.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    "Account",
                    theme::semibold(12.0),
                    p.muted,
                );
                for (code, (name, now, last, avg)) in &rows {
                    let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(4), p.hover);
                    }
                    widgets::clipped_text(
                        ui.painter(),
                        r.left_center() + vec2(4.0, 0.0),
                        &format!("{code} {name}"),
                        theme::regular(13.0),
                        p.text,
                        w - cw * 3.0 - 8.0,
                    );
                    // Flag a category already running well above its average.
                    let hot = *avg > Decimal::ZERO && *now > *avg * Decimal::new(125, 2);
                    let vals = [(*now, hot), (*last, false), (*avg, false)];
                    for (i, (v, flag)) in vals.iter().enumerate() {
                        let x = r.right() - cw * (2 - i) as f32 - 4.0;
                        ui.painter().text(
                            egui::pos2(x, r.center().y),
                            egui::Align2::RIGHT_CENTER,
                            fmt::money(v.round_dp(2)),
                            theme::regular(13.0),
                            if *flag { p.warning } else { p.text },
                        );
                        if *flag {
                            ui.painter().text(
                                egui::pos2(x - 86.0, r.center().y),
                                egui::Align2::RIGHT_CENTER,
                                icon::ARROW_UP_RIGHT,
                                egui::FontId::proportional(13.0),
                                p.warning,
                            );
                        }
                    }
                    if resp.clicked() {
                        go = b.ledger.chart().find_by_code(code).map(|a| a.id);
                    }
                }
            });
        },
    );
    if let Some(id) = go {
        shell.state.accounts.range = Some((ms - Months::new(3), b.today));
        shell.go(Route::Account(id));
    }
    ui.add_space(16.0);
}

fn by_customer(ui: &mut Ui, b: &Books) {
    let p = pal(ui.ctx());
    let (ys, ye) = b.fiscal_ytd();
    let mut by: BTreeMap<String, Decimal> = BTreeMap::new();
    for id in 1..b.ledger.next_invoice_id() {
        if let Some(inv) = b.ledger.get_invoice(id)
            && inv.date >= ys
            && inv.date <= ye
        {
            let who = b
                .ledger
                .get_customer(inv.customer_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            *by.entry(who).or_default() += inv.total();
        }
    }
    let mut rows: Vec<(String, Decimal)> = by.into_iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    widgets::titled_card(
        ui,
        "Invoiced by customer, this year",
        |_| {},
        |ui| {
            if rows.is_empty() {
                widgets::empty(
                    ui,
                    icon::USERS,
                    "No invoices this year",
                    "Income recorded without invoices doesn't show a customer.",
                );
                return;
            }
            let total: Decimal = rows.iter().map(|r| r.1).sum();
            if let Some((top, v)) = rows.first() {
                ui.label(
                    RichText::new(format!(
                        "{top} is {}% of invoiced income.",
                        pct(*v, total).unwrap_or_default()
                    ))
                    .font(theme::regular(12.5))
                    .color(p.muted),
                );
                ui.add_space(4.0);
            }
            charts::hbars(ui, &rows, p.series[1]);
        },
    );
}

fn by_class(ui: &mut Ui, b: &Books) {
    let p = pal(ui.ctx());
    let (ys, ye) = b.fiscal_ytd();
    let classes = b.ledger.list_classes();
    widgets::titled_card(
        ui,
        "Net income by class, this year",
        |_| {},
        |ui| {
            if classes.is_empty() {
                widgets::empty(
                    ui,
                    icon::TAG,
                    "No classes",
                    "Tag lines with classes (departments, products) to see profit by each.",
                );
                return;
            }
            let mut rows: Vec<(String, Decimal)> = classes
                .iter()
                .filter_map(|c| {
                    let pl = profit_and_loss_for_period(&b.ledger, ys, ye, Some(c.id)).ok()?;
                    let net = pl.total_revenue - pl.total_expenses;
                    (!pl.total_revenue.is_zero() || !pl.total_expenses.is_zero())
                        .then(|| (c.name.clone(), net))
                })
                .collect();
            rows.sort_by_key(|r| std::cmp::Reverse(r.1));
            if rows.is_empty() {
                widgets::empty(
                    ui,
                    icon::TAG,
                    "No classified income or spending this year",
                    "",
                );
                return;
            }
            charts::hbars(ui, &rows, p.series[0]);
            ui.label(
                RichText::new("Only lines tagged with a class count here.")
                    .font(theme::regular(12.0))
                    .color(p.faint),
            );
        },
    );
}
