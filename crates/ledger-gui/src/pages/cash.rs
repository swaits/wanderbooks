//! Cash: what's in the bank, where it's been going, how long it lasts, and
//! what's coming in and out next.

use super::{columns, page};
use crate::app::Shell;
use crate::books::Books;
use crate::charts::{self, BarGroup};
use crate::fmt;
use crate::nav::Route;
use crate::theme::{self, pal};
use crate::widgets;
use chrono::{Days, NaiveDate};
use egui::{CornerRadius, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as icon;
use ledger_core::analysis::{Interval, balance_history, flows_by_interval, interval_start};
use rust_decimal::Decimal;

pub fn show(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    page(ui, "cash", |ui| {
        let k = &b.kpis;
        let since = b.today - Days::new(30);
        let flows = flows_by_interval(
            &b.ledger,
            &b.cash_accounts,
            since + Days::new(1),
            b.today,
            Interval::Year,
        );
        let (cin, cout): (Decimal, Decimal) =
            flows.iter().fold((Decimal::ZERO, Decimal::ZERO), |a, f| {
                (a.0 + f.2, a.1 + f.3)
            });
        widgets::page_header(
            ui,
            "Cash",
            &format!(
                "{} across {}. In the last 30 days {} came in and {} went out.",
                fmt::dollars(k.cash),
                fmt::count(b.cash_accounts.len(), "account", "accounts"),
                fmt::dollars(cin),
                fmt::dollars(cout)
            ),
        );
        if b.cash_accounts.is_empty() {
            widgets::card(ui, |ui| {
                widgets::empty(
                    ui,
                    icon::BANK,
                    "No cash accounts found",
                    "Name an asset account \"checking\", \"savings\", or \"cash\", or set [books] cash_accounts in wanderbooks.toml.",
                )
            });
            return;
        }
        // One tile per cash and card account.
        let mut tiles: Vec<(u64, bool)> = b.cash_accounts.iter().map(|a| (*a, false)).collect();
        tiles.extend(b.card_accounts.iter().map(|a| (*a, true)));
        // Up to four across, fewer when the window is narrow.
        let n = tiles.len().clamp(1, 4);
        let n = widgets::fit_columns(ui.available_width(), 200.0, 16.0, n);
        for row in tiles.chunks(n) {
            columns(ui, 0.0, &vec![1.0; n], |i, ui| {
                let Some((id, is_card)) = row.get(i) else {
                    return;
                };
                let hist = balance_history(&b.ledger, &[*id]);
                let spark: Vec<f64> = hist
                    .iter()
                    .rev()
                    .take(40)
                    .rev()
                    .map(|(_, v)| (*v).try_into().unwrap_or(0.0))
                    .collect();
                let bal = b.balance_on(*id, b.today);
                let ago = b.balance_on(*id, since);
                let delta = bal - ago;
                let r = widgets::kpi(
                    ui,
                    widgets::Kpi {
                        label: &b.account_label(*id),
                        value: fmt::dollars(bal),
                        sub: Some((
                            if *is_card {
                                "owed on the card".to_string()
                            } else if delta.is_zero() {
                                "no change in 30 days".to_string()
                            } else {
                                format!(
                                    "{}{} in 30 days",
                                    if delta > Decimal::ZERO { "+" } else { "" },
                                    fmt::dollars(delta)
                                )
                            },
                            if *is_card || delta.is_zero() {
                                p.muted
                            } else if delta > Decimal::ZERO {
                                p.positive
                            } else {
                                p.negative
                            },
                        )),
                        spark: (spark.len() > 1).then_some(&spark[..]),
                        how: "",
                    },
                );
                if r.interact(Sense::click()).clicked() {
                    shell.state.accounts.range = None;
                    shell.go(Route::Account(*id));
                }
            });
        }
        let first = k.first_entry.unwrap_or(b.today);
        let (start, end) = b.trailing_months(12);
        let start = start.max(interval_start(first, Interval::Month));
        columns(ui, 980.0, &[3.0, 2.0], |i, ui| match i {
            0 => {
                widgets::titled_card(
                    ui,
                    "Cash balance",
                    |_| {},
                    |ui| charts::balance_line(ui, &b.cash_history(), b.today, 240.0),
                );
            }
            _ => {
                let months =
                    flows_by_interval(&b.ledger, &b.cash_accounts, start, end, Interval::Month);
                widgets::titled_card(
                    ui,
                    "Money in and out by month",
                    |_| {},
                    |ui| {
                        let groups: Vec<BarGroup> = months
                            .iter()
                            .map(|(s, _, i, o)| BarGroup {
                                label: s.format("%b").to_string(),
                                values: vec![*i, *o],
                            })
                            .collect();
                        charts::bar_chart(
                            ui,
                            &groups,
                            &[("In", p.series[1]), ("Out", p.series[2])],
                            210.0,
                        );
                    },
                );
            }
        });
        columns(ui, 980.0, &[1.0, 1.0], |i, ui| match i {
            0 => runway(ui, b),
            _ => coming_up(ui, b, shell),
        });
        biggest_outflows(ui, b, shell);
    });
}

fn runway(ui: &mut Ui, b: &Books) {
    let p = pal(ui.ctx());
    let ms = b.month_start();
    let three = ms - chrono::Months::new(3);
    let start = three.max(
        b.kpis
            .first_entry
            .map_or(three, |f| interval_start(f, Interval::Month)),
    );
    let months = flows_by_interval(
        &b.ledger,
        &b.cash_accounts,
        start,
        ms - Days::new(1),
        Interval::Month,
    );
    widgets::titled_card(
        ui,
        "Burn and runway",
        |_| {},
        |ui| {
            if months.is_empty() {
                ui.label(RichText::new("Needs at least one full month of books.").color(p.muted));
                return;
            }
            let net: Decimal = months.iter().map(|(_, _, i, o)| i - o).sum::<Decimal>()
                / Decimal::from(months.len() as u64);
            let net = net.round_dp(2);
            let big = |ui: &mut Ui, label: &str, value: String, color| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(label)
                            .font(theme::regular(12.5))
                            .color(p.muted),
                    );
                    ui.label(
                        RichText::new(value)
                            .font(theme::semibold(22.0))
                            .color(color),
                    );
                });
            };
            ui.horizontal(|ui| {
                big(
                    ui,
                    "Average monthly net cash flow",
                    format!(
                        "{}{}",
                        if net > Decimal::ZERO { "+" } else { "" },
                        fmt::dollars(net)
                    ),
                    if net >= Decimal::ZERO {
                        p.positive
                    } else {
                        p.negative
                    },
                );
                ui.add_space(32.0);
                if net < Decimal::ZERO {
                    let months_left = (b.kpis.cash / -net).round_dp(1);
                    let out = b.today
                        + Days::new((months_left * Decimal::from(30)).try_into().unwrap_or(0u64));
                    big(ui, "Runway", format!("{months_left} months"), p.text);
                    ui.add_space(32.0);
                    big(ui, "Cash runs out around", fmt::month(out), p.warning);
                } else {
                    big(ui, "Runway", "Cash is growing".into(), p.positive);
                }
            });
            ui.add_space(8.0);
            ui.label(
            RichText::new(format!(
                "Cash in minus cash out of {}, averaged over the last {} (owner contributions and draws included: this is what the bank sees).",
                b.cash_accounts.iter().map(|a| b.account_name(*a)).collect::<Vec<_>>().join(" and "),
                fmt::count(months.len(), "full month", "full months")
            ))
            .font(theme::regular(12.0))
            .color(p.faint),
        );
        },
    );
}

struct Upcoming {
    date: NaiveDate,
    label: String,
    amount: Decimal,
    route: Option<Route>,
}

fn coming_up(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let horizon = b.today + Days::new(45);
    let mut items: Vec<Upcoming> = Vec::new();
    for r in b.ledger.recurring_entries() {
        let cash: Decimal = r
            .lines
            .iter()
            .filter(|(a, _, _)| b.cash_accounts.contains(a))
            .map(|(_, s, m)| {
                if *s == ledger_core::Side::Debit {
                    m.amount
                } else {
                    -m.amount
                }
            })
            .sum();
        for d in r
            .due(horizon)
            .into_iter()
            .filter(|d| *d > b.today - Days::new(1))
        {
            items.push(Upcoming {
                date: d,
                label: format!("{} (recurring)", r.description),
                amount: cash,
                route: Some(Route::Company),
            });
        }
    }
    for id in 1..b.ledger.next_invoice_id() {
        if let Some(inv) = b.ledger.get_invoice(id)
            && !inv.balance().is_zero()
        {
            let who = b
                .ledger
                .get_customer(inv.customer_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            let late = inv.due_date < b.today;
            items.push(Upcoming {
                date: inv.due_date,
                label: format!(
                    "Invoice {id} from {who}{}",
                    if late { " — overdue" } else { "" }
                ),
                amount: inv.balance(),
                route: Some(Route::Entry(inv.entry_id)),
            });
        }
    }
    for id in 1..b.ledger.next_bill_id() {
        if let Some(bill) = b.ledger.get_bill(id)
            && !bill.balance().is_zero()
        {
            let who = b
                .ledger
                .get_vendor(bill.vendor_id)
                .map(|v| v.name.clone())
                .unwrap_or_default();
            items.push(Upcoming {
                date: bill.due_date,
                label: format!("Bill {id} to {who}"),
                amount: -bill.balance(),
                route: Some(Route::Entry(bill.entry_id)),
            });
        }
    }
    items.sort_by_key(|i| i.date);
    let mut go = None;
    widgets::titled_card(
        ui,
        "Coming up",
        |ui| {
            ui.label(
                RichText::new("open invoices, bills, recurring")
                    .font(theme::regular(12.0))
                    .color(p.muted),
            );
        },
        |ui| {
            if items.is_empty() {
                widgets::empty(
                    ui,
                    icon::CALENDAR_CHECK,
                    "Nothing scheduled",
                    "No open invoices, bills, or recurring entries due soon.",
                );
                return;
            }
            let (inflow, outflow): (Decimal, Decimal) =
                items.iter().fold((Decimal::ZERO, Decimal::ZERO), |a, i| {
                    if i.amount > Decimal::ZERO {
                        (a.0 + i.amount, a.1)
                    } else {
                        (a.0, a.1 - i.amount)
                    }
                });
            ui.label(
                RichText::new(format!(
                    "Expected in {} · expected out {}",
                    fmt::dollars(inflow),
                    fmt::dollars(outflow)
                ))
                .font(theme::semibold(13.0)),
            );
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .id_salt("upcoming")
                .max_height(220.0)
                .show(ui, |ui| {
                    for it in &items {
                        let (r, resp) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(r, CornerRadius::same(6), p.hover);
                        }
                        let late = it.date < b.today;
                        ui.painter().text(
                            r.left_center() + vec2(6.0, 0.0),
                            egui::Align2::LEFT_CENTER,
                            fmt::date_short(it.date),
                            theme::regular(12.5),
                            if late { p.negative } else { p.muted },
                        );
                        widgets::clipped_text(
                            ui.painter(),
                            r.left_center() + vec2(70.0, 0.0),
                            &it.label,
                            theme::regular(13.0),
                            p.text,
                            r.width() - 70.0 - 130.0,
                        );
                        let (txt, c) = if it.amount > Decimal::ZERO {
                            (format!("+{}", fmt::money(it.amount)), p.positive)
                        } else {
                            (fmt::money(it.amount), p.text)
                        };
                        ui.painter().text(
                            r.right_center() - vec2(6.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            txt,
                            theme::semibold(13.0),
                            c,
                        );
                        if resp.clicked() {
                            go = it.route.clone();
                        }
                    }
                });
        },
    );
    if let Some(r) = go {
        shell.go(r);
    }
}

fn biggest_outflows(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let since = b.today - Days::new(30);
    let mut out: Vec<&crate::books::EntryInfo> = b
        .entries
        .iter()
        .filter(|e| {
            e.status == ledger_core::EntryStatus::Posted
                && e.date > since
                && e.cash_effect < Decimal::ZERO
        })
        .collect();
    out.sort_by_key(|e| e.cash_effect);
    out.truncate(8);
    let mut go = None;
    widgets::titled_card(
        ui,
        "Biggest payments, last 30 days",
        |_| {},
        |ui| {
            if out.is_empty() {
                widgets::empty(ui, icon::ARROW_UP_RIGHT, "No payments", "");
            }
            for e in &out {
                let (r, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, CornerRadius::same(6), p.hover);
                }
                ui.painter().text(
                    r.left_center() + vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    fmt::date_short(e.date),
                    theme::regular(12.5),
                    p.muted,
                );
                widgets::clipped_text(
                    ui.painter(),
                    r.left_center() + vec2(70.0, 0.0),
                    &e.description,
                    theme::regular(13.0),
                    p.text,
                    r.width() - 70.0 - 130.0,
                );
                ui.painter().text(
                    r.right_center() - vec2(6.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    fmt::money(e.cash_effect),
                    theme::semibold(13.0),
                    p.text,
                );
                if resp.clicked() {
                    go = Some(e.id);
                }
            }
        },
    );
    if let Some(id) = go {
        shell.go(Route::Entry(id));
    }
}
