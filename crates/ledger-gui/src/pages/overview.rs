//! Home: the state of the business in one sentence, the numbers behind
//! it, what needs doing, and what just happened.

use super::{columns, page};
use crate::app::Shell;
use crate::books::{Books, Severity};
use crate::charts::{self, BarGroup};
use crate::fmt;
use crate::nav::{ReportKind, Route};
use crate::theme::{self, pal};
use crate::widgets::{self, Kpi};
use egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use ledger_core::analysis::{Interval, pnl_by_interval};
use ledger_core::reports::profit_and_loss_for_period;
use rust_decimal::Decimal;

pub fn show(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    page(ui, "overview", |ui| {
        let k = &b.kpis;
        widgets::page_header(ui, b.company_name(), &headline(b));
        kpis(ui, b, shell);
        let _ = k;
        // What needs doing gets the page's full width, under the numbers:
        // long items wrap in their own room instead of squeezing the chart.
        attention(ui, b, shell);
        ui.add_space(16.0);
        let (more, _) = widgets::titled_card(
            ui,
            "Cash over time",
            |ui| {
                widgets::link(
                    ui,
                    RichText::new("Cash details")
                        .font(theme::regular(12.5))
                        .color(pal(ui.ctx()).accent),
                )
                .clicked()
            },
            |ui| charts::balance_line(ui, &b.cash_history(), b.today, 230.0),
        );
        if more {
            shell.go(Route::Cash);
        }
        ui.add_space(16.0);
        columns(ui, 980.0, &[1.0, 1.0], |i, ui| match i {
            0 => income_vs_spending(ui, b, shell),
            _ => where_it_went(ui, b, shell),
        });
        recent(ui, b, shell);
    });
}

/// The answer, in words.
fn headline(b: &Books) -> String {
    let k = &b.kpis;
    let Some(first) = k.first_entry else {
        return "No transactions yet. Once your agent records some, this page fills in on its own."
            .into();
    };
    let mut s = format!(
        "You have {} in cash and owe {}.",
        fmt::dollars(k.cash),
        fmt::dollars(k.liabilities)
    );
    if let Some(burn) = k.monthly_burn {
        if burn > Decimal::ZERO && k.cash > Decimal::ZERO {
            let months = (k.cash / burn).round_dp(1);
            s.push_str(&format!(
                " Lately you spend {} a month more than you earn: about {} months of runway.",
                fmt::dollars(burn),
                months
            ));
        }
    } else if k.net_ytd > Decimal::ZERO {
        s.push_str(&format!(
            " You've earned {} more than you've spent this year.",
            fmt::dollars(k.net_ytd)
        ));
    } else if k.net_ytd < Decimal::ZERO {
        s.push_str(&format!(
            " You've spent {} more than you've earned this year.",
            fmt::dollars(-k.net_ytd)
        ));
    }
    if (b.today - first).num_days() < 60 {
        s.push_str(&format!(
            " Your books start {}; trends fill in as the months go by.",
            fmt::date(first)
        ));
    }
    s
}

fn change(
    now: Decimal,
    prev: Decimal,
    up_is_good: bool,
    what: &str,
    p: &theme::Palette,
) -> Option<(String, egui::Color32)> {
    let pct = fmt::pct_change(prev, now)?;
    let good = (pct >= 0.0) == up_is_good;
    let arrow = if pct >= 0.0 { "▲" } else { "▼" };
    Some((
        format!("{arrow} {:.0}% vs {what}", pct.abs()),
        if pct.abs() < 0.5 {
            p.muted
        } else if good {
            p.positive
        } else {
            p.negative
        },
    ))
}

fn kpis(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let k = &b.kpis;
    let p = pal(ui.ctx());
    // Last 12 months of month-end cash for the sparkline.
    let (start, end) = b.trailing_months(12);
    let start = k.first_entry.map_or(start, |f| {
        start.max(ledger_core::analysis::interval_start(f, Interval::Month))
    });
    let month_ends: Vec<chrono::NaiveDate> =
        ledger_core::analysis::intervals(start, end, Interval::Month)
            .into_iter()
            .map(|(_, e)| e)
            .collect();
    let cash_spark: Vec<f64> =
        ledger_core::analysis::balance_series(&b.ledger, &b.cash_accounts, &month_ends)
            .into_iter()
            .map(|d| d.try_into().unwrap_or(0.0))
            .collect();
    let runway = match k.monthly_burn {
        Some(burn) if burn > Decimal::ZERO && k.cash > Decimal::ZERO => (
            format!("{} mo", (k.cash / burn).round_dp(1)),
            Some((format!("burning {}/mo", fmt::compact(burn)), p.warning)),
        ),
        _ if k.first_entry.is_none() => ("—".to_string(), None),
        _ if k.first_entry.is_some_and(|f| f >= b.month_start()) => (
            "Too early".to_string(),
            Some(("needs a full month of books".to_string(), p.muted)),
        ),
        _ => (
            "Not burning".to_string(),
            Some(("earning ≥ spending".to_string(), p.positive)),
        ),
    };
    let ytd = b.fiscal_ytd();
    let tiles: Vec<(Kpi, Route)> = vec![
        (
            Kpi {
                label: "Cash on hand",
                value: fmt::dollars(k.cash),
                sub: change(k.cash, k.cash_30d_ago, true, "30 days ago", &p),
                spark: Some(&cash_spark),
                how: "The balance of every cash account (bank, savings, cash on hand) at the end of today.",
            },
            Route::Cash,
        ),
        (
            Kpi {
                label: "Owed by the business",
                value: fmt::dollars(k.liabilities),
                sub: Some(("bills, cards, owners, taxes".into(), p.muted)),
                spark: None,
                how: "Total liabilities on today's balance sheet.",
            },
            Route::Reports(ReportKind::BalanceSheet),
        ),
        (
            Kpi {
                label: "Spending this month",
                value: fmt::dollars(k.expenses_mtd),
                sub: change(
                    k.expenses_mtd,
                    k.expenses_prev_month,
                    false,
                    "same days last month",
                    &p,
                ),
                spark: None,
                how: "Expenses dated this month (accrual basis: when incurred, not when paid).",
            },
            Route::Profit,
        ),
        (
            Kpi {
                label: "Revenue this month",
                value: fmt::dollars(k.revenue_mtd),
                sub: change(
                    k.revenue_mtd,
                    k.revenue_prev_month,
                    true,
                    "same days last month",
                    &p,
                ),
                spark: None,
                how: "Revenue dated this month.",
            },
            Route::Profit,
        ),
        (
            Kpi {
                label: "Net income, year to date",
                value: fmt::dollars(k.net_ytd),
                sub: Some((format!("since {}", fmt::date_short(ytd.0)), p.muted)),
                spark: None,
                how: "Revenue minus expenses for the fiscal year so far.",
            },
            Route::Reports(ReportKind::ProfitAndLoss),
        ),
        (
            Kpi {
                label: "Runway",
                value: runway.0,
                sub: runway.1,
                spark: None,
                how: "Cash on hand divided by the average monthly net loss over the last three full months the books cover.",
            },
            Route::Cash,
        ),
    ];
    super::grid(ui, tiles.len(), 150.0, 6, |i, ui| {
        let (tile, route) = &tiles[i];
        let r = widgets::kpi(
            ui,
            Kpi {
                label: tile.label,
                value: tile.value.clone(),
                sub: tile.sub.clone(),
                spark: tile.spark,
                how: tile.how,
            },
        );
        if r.interact(egui::Sense::click()).clicked() {
            shell.go(route.clone());
        }
    });
}

/// "Needs attention", full width: a summary of how many problems, to-dos,
/// and notes there are, then each item as a row (its severity, the title
/// that opens where to look, what's wrong, and the prompt for the agent at
/// the right; stacked under the text when narrow). The first few show;
/// the rest wait behind a button that says how many.
fn attention(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let count = |s: Severity| b.attention.iter().filter(|a| a.severity == s).count();
    widgets::titled_card(
        ui,
        "Needs attention",
        |ui| {
            // Laid out right to left: problems end up first, reading on.
            for (sev, one, many, fg, bg) in [
                (Severity::Info, "note", "notes", p.muted, p.hover),
                (Severity::Todo, "to do", "to do", p.warning, p.warning_soft),
                (
                    Severity::Problem,
                    "problem",
                    "problems",
                    p.negative,
                    p.negative_soft,
                ),
            ] {
                let n = count(sev);
                if n > 0 {
                    widgets::pill(ui, &fmt::count(n, one, many), fg, bg);
                }
            }
        },
        |ui| {
            if b.attention.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(icon::CHECK_CIRCLE)
                            .size(18.0)
                            .color(p.positive),
                    );
                    ui.label(
                        RichText::new("All clear: nothing is missing or overdue.")
                            .font(theme::regular(13.5))
                            .color(p.muted),
                    );
                });
                return;
            }
            const FIRST: usize = 6;
            let id = ui.id().with("attention-expanded");
            let mut expanded: bool = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(false);
            let shown = if expanded {
                b.attention.len()
            } else {
                b.attention.len().min(FIRST)
            };
            let wide = ui.available_width() >= 720.0;
            for (i, a) in b.attention.iter().take(shown).enumerate() {
                if i > 0 {
                    ui.add_space(6.0);
                    widgets::rule(ui);
                    ui.add_space(6.0);
                }
                let (glyph, color, word) = match a.severity {
                    Severity::Problem => (icon::WARNING_CIRCLE, p.negative, "Problem"),
                    Severity::Todo => (icon::CIRCLE_DASHED, p.warning, "To do"),
                    Severity::Info => (icon::INFO, p.muted, "Note"),
                };
                let full = ui.available_width();
                let button_w = if wide { 150.0 } else { 0.0 };
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new(glyph).size(18.0).color(color))
                        .on_hover_text(word);
                    let text_w = (full - 30.0 - button_w).max(120.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(text_w, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(text_w);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            let t = RichText::new(&a.title).font(theme::semibold(13.5));
                            let r = ui.add(egui::Label::new(t).wrap().sense(egui::Sense::click()));
                            if a.target.is_some() && r.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if r.clicked()
                                && let Some(to) = &a.target
                            {
                                shell.go(to.clone());
                            }
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&a.detail)
                                        .font(theme::regular(12.5))
                                        .color(p.muted),
                                )
                                .wrap(),
                            );
                            if !wide {
                                ui.add_space(4.0);
                                widgets::copy_prompt(ui, &a.prompt);
                            }
                        },
                    );
                    if wide {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            widgets::copy_prompt(ui, &a.prompt);
                        });
                    }
                });
            }
            let hidden = b.attention.len() - shown;
            if hidden > 0 || expanded && b.attention.len() > FIRST {
                ui.add_space(8.0);
                let label = if hidden > 0 {
                    format!("{} Show {} more", icon::CARET_DOWN, hidden)
                } else {
                    format!("{} Show fewer", icon::CARET_UP)
                };
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(label)
                                .font(theme::semibold(12.5))
                                .color(p.accent),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    expanded = !expanded;
                    ui.ctx().data_mut(|d| d.insert_temp(id, expanded));
                }
            }
        },
    );
}

fn income_vs_spending(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let (start, end) = b.trailing_months(12);
    let start = b.kpis.first_entry.map_or(start, |f| {
        start.max(ledger_core::analysis::interval_start(f, Interval::Month))
    });
    let months = pnl_by_interval(&b.ledger, start, end, Interval::Month).unwrap_or_default();
    widgets::titled_card(
        ui,
        "Income and spending by month",
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
            let clicked = charts::bar_chart(
                ui,
                &groups,
                &[
                    ("Income", p.series[1]),
                    ("Spending", p.series[2]),
                    ("Net", p.series[0]),
                ],
                220.0,
            );
            if let Some(i) = clicked {
                shell.state.reports.range =
                    crate::pages::reports::Range::Custom(months[i].start, months[i].end);
                shell.go(Route::Reports(ReportKind::ProfitAndLoss));
            }
        },
    );
}

fn where_it_went(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let (start, end) = b.fiscal_ytd();
    widgets::titled_card(
        ui,
        "Where the money went, this year",
        |ui| {
            ui.label(
                RichText::new(format!(
                    "{} – {}",
                    fmt::date_short(start),
                    fmt::date_short(end)
                ))
                .font(theme::regular(12.0))
                .color(p.muted),
            );
        },
        |ui| {
            let Ok(pl) = profit_and_loss_for_period(&b.ledger, start, end, None) else {
                return;
            };
            let mut rows: Vec<(String, Decimal, String)> = pl
                .expenses
                .iter()
                .map(|l| (l.name.clone(), l.amount, l.code.clone()))
                .collect();
            rows.sort_by_key(|r| std::cmp::Reverse(r.1));
            rows.truncate(8);
            if rows.is_empty() {
                widgets::empty(ui, icon::CHART_BAR_HORIZONTAL, "No spending yet", "");
                return;
            }
            let bars: Vec<(String, Decimal)> = rows.iter().map(|r| (r.0.clone(), r.1)).collect();
            if let Some(i) = charts::hbars(ui, &bars, p.series[2])
                && let Some(a) = b.ledger.chart().find_by_code(&rows[i].2)
            {
                shell.go(Route::Account(a.id));
            }
        },
    );
}

fn recent(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let since_update = shell.since_update;
    let mut go = None;
    let (full, _) = widgets::titled_card(
        ui,
        "Recent activity",
        |ui| {
            widgets::link(
                ui,
                RichText::new("Full journal")
                    .font(theme::regular(12.5))
                    .color(p.accent),
            )
            .clicked()
        },
        |ui| {
            let mut shown = 0;
            for (i, line) in b.journal.iter().enumerate().rev() {
                let Some(ev) = super::journal::describe(b, line) else {
                    continue;
                };
                if shown == 12 {
                    break;
                }
                shown += 1;
                let fresh = line.seq > b.fresh_from_seq && since_update < 8.0;
                // One painted row: icon, the event (cut with an ellipsis
                // before the timestamp column), the timestamp.
                let (r, resp) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 30.0),
                    egui::Sense::click(),
                );
                if fresh {
                    ui.painter()
                        .rect_filled(r, egui::CornerRadius::same(6), p.new_highlight);
                } else if resp.hovered() {
                    ui.painter()
                        .rect_filled(r, egui::CornerRadius::same(6), p.hover);
                }
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let painter = ui.painter_at(r);
                let y = r.center().y;
                painter.text(
                    egui::pos2(r.left() + 8.0, y),
                    egui::Align2::LEFT_CENTER,
                    ev.icon,
                    egui::FontId::proportional(15.0),
                    p.muted,
                );
                let ts =
                    painter.layout_no_wrap(fmt::timestamp(&line.ts), theme::regular(11.5), p.faint);
                let ts_w = ts.size().x;
                painter.galley(
                    egui::pos2(r.right() - 8.0 - ts_w, y - ts.size().y / 2.0),
                    ts,
                    p.faint,
                );
                widgets::clipped_text(
                    &painter,
                    egui::pos2(r.left() + 32.0, y),
                    &ev.text,
                    theme::regular(13.0),
                    p.text,
                    r.width() - 32.0 - ts_w - 28.0,
                );
                if resp.clicked() {
                    go = Some(ev.route.clone().unwrap_or(Route::JournalEvent(line.seq)));
                }
                let _ = i;
            }
            if shown == 0 {
                widgets::empty(ui, icon::CLOCK, "Nothing yet", "");
            }
        },
    );
    if full {
        go = Some(Route::Journal);
    }
    if let Some(r) = go {
        shell.go(r);
    }
}
