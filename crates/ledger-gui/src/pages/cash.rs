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
use chrono::{Datelike, Days, NaiveDate};
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
                    "Name an asset account \"checking\", \"savings\", or \"cash\", or ask the agent to set the books' cash accounts.",
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
                    // Checked: a sub-cent average (lines an earlier version
                    // booked) makes a runway past any calendar.
                    let months_left = b.kpis.cash.checked_div(-net).map(|m| m.round_dp(1));
                    let out = months_left
                        .and_then(|m| m.checked_mul(Decimal::from(30)))
                        .and_then(|d| u64::try_from(d).ok())
                        .and_then(|d| b.today.checked_add_days(Days::new(d)));
                    big(
                        ui,
                        "Runway",
                        months_left
                            .map_or_else(|| fmt::CANT_COMPUTE.into(), |m| format!("{m} months")),
                        p.text,
                    );
                    ui.add_space(32.0);
                    big(
                        ui,
                        "Cash runs out around",
                        out.map_or_else(|| "beyond any calendar date".into(), fmt::month),
                        p.warning,
                    );
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

/// A recurring entry's ungenerated occurrences dated `from..=to` (and on
/// or before its end), found from the occurrence just before `from`
/// rather than walking from its start: a weekly entry an earlier version
/// started at chrono's first day has millions of occurrences before today,
/// and this runs every frame.
fn occurrences_from(
    r: &ledger_core::RecurringEntry,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<NaiveDate> {
    use ledger_core::Frequency;
    let last = r.end.map_or(to, |e| e.min(to));
    // How many periods from the start to `from`, rounded down, less one
    // for a month-end clamp: never past the first occurrence on or after
    // `from`.
    let months = |d: NaiveDate| {
        (i64::from(d.year()) - i64::from(r.start.year())) * 12 + i64::from(d.month())
            - i64::from(r.start.month())
    };
    let k0 = match r.frequency {
        Frequency::Weekly => (from - r.start).num_days() / 7,
        Frequency::Monthly => months(from),
        Frequency::Quarterly => months(from) / 3,
        Frequency::Yearly => months(from) / 12,
    } - 1;
    let Ok(k0) = u32::try_from(k0.max(0)) else {
        return Vec::new();
    };
    (k0..)
        .map_while(|k| r.frequency.nth(r.start, k).filter(|d| *d <= last))
        .filter(|d| *d >= from && !r.generated.contains(d))
        .collect()
}

struct Upcoming {
    date: NaiveDate,
    label: String,
    /// None: it can't be computed exactly (a recurring template or a
    /// document an earlier version journaled, past exact arithmetic).
    amount: Option<Decimal>,
    route: Option<Route>,
}

fn coming_up(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let horizon = b.today + Days::new(45);
    let mut items: Vec<Upcoming> = Vec::new();
    for r in b.ledger.recurring_entries() {
        let cash = r
            .lines
            .iter()
            .filter(|(a, _, _)| b.cash_accounts.contains(a))
            .try_fold(Decimal::ZERO, |sum, (_, s, m)| {
                if *s == ledger_core::Side::Debit {
                    sum.checked_add(m.amount)
                } else {
                    sum.checked_sub(m.amount)
                }
            });
        for d in occurrences_from(r, b.today, horizon) {
            items.push(Upcoming {
                date: d,
                label: format!("{} (recurring)", r.description),
                amount: cash,
                route: Some(Route::Company),
            });
        }
    }
    // An open document: its balance, or None when it can't be computed
    // (shown as such, never left out).
    for id in 1..b.ledger.next_invoice_id() {
        let Some(inv) = b.ledger.get_invoice(id) else {
            continue;
        };
        let balance = inv.balance().ok();
        if balance.is_some_and(|x| x.is_zero()) {
            continue;
        }
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
            amount: balance,
            route: Some(Route::Entry(inv.entry_id)),
        });
    }
    for id in 1..b.ledger.next_bill_id() {
        let Some(bill) = b.ledger.get_bill(id) else {
            continue;
        };
        let balance = bill.balance().ok();
        if balance.is_some_and(|x| x.is_zero()) {
            continue;
        }
        let who = b
            .ledger
            .get_vendor(bill.vendor_id)
            .map(|v| v.name.clone())
            .unwrap_or_default();
        items.push(Upcoming {
            date: bill.due_date,
            label: format!("Bill {id} to {who}"),
            amount: balance.map(|x| -x),
            route: Some(Route::Entry(bill.entry_id)),
        });
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
            // None: an item, or the sum, can't be computed exactly.
            let (inflow, outflow) = items.iter().fold(
                (Some(Decimal::ZERO), Some(Decimal::ZERO)),
                |(inflow, outflow), i| match i.amount {
                    Some(a) if a > Decimal::ZERO => {
                        (inflow.and_then(|x| x.checked_add(a)), outflow)
                    }
                    Some(a) => (inflow, outflow.and_then(|x| x.checked_sub(a))),
                    None => (None, None),
                },
            );
            ui.label(
                RichText::new(format!(
                    "Expected in {} · expected out {}",
                    fmt::dollars_or(inflow),
                    fmt::dollars_or(outflow)
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
                        let (txt, c) = match it.amount {
                            Some(a) if a > Decimal::ZERO => {
                                (format!("+{}", fmt::money(a)), p.positive)
                            }
                            a => (fmt::money_or(a), p.text),
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
                && e.cash_effect.is_some_and(|c| c < Decimal::ZERO)
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
                    fmt::money_or(e.cash_effect),
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

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::{Frequency, RecurringEntry};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// occurrences_from is the recurring entry's due dates in the window,
    /// exactly, without walking from the start: month ends clamped, ends
    /// and generated dates honored, a start after the window, and a start
    /// at chrono's first day.
    #[test]
    fn occurrences_in_a_window_are_the_due_dates_in_it() {
        for frequency in [
            Frequency::Weekly,
            Frequency::Monthly,
            Frequency::Quarterly,
            Frequency::Yearly,
        ] {
            for start in [
                d(2020, 1, 31),
                d(2024, 2, 29),
                d(2026, 1, 10),
                d(2026, 10, 20),
                d(2027, 1, 1),
                d(2027, 1, 15),
            ] {
                for (end, (from, to)) in [None, Some(d(2026, 11, 2))].into_iter().flat_map(|end| {
                    // Windows a few weeks long, years long, and one
                    // opening before the entry starts.
                    [
                        (d(2026, 10, 17), d(2026, 12, 1)),
                        (d(2026, 6, 1), d(2028, 12, 31)),
                        (d(2025, 12, 20), d(2027, 12, 31)),
                    ]
                    .map(|w| (end, w))
                }) {
                    let mut r = RecurringEntry {
                        id: 1,
                        description: "x".into(),
                        lines: vec![],
                        frequency,
                        start,
                        end,
                        generated: Default::default(),
                    };
                    if let Some(first) = frequency.nth(start, 3) {
                        r.generated.insert(first);
                    }
                    let want: Vec<NaiveDate> =
                        r.due(to).into_iter().filter(|x| *x >= from).collect();
                    assert_eq!(
                        occurrences_from(&r, from, to),
                        want,
                        "{frequency:?} {start} {end:?} {from}..{to}"
                    );
                }
            }
        }
        let far = RecurringEntry {
            id: 1,
            description: "weekly since chrono's first day".into(),
            lines: vec![],
            frequency: Frequency::Weekly,
            start: NaiveDate::MIN,
            end: None,
            generated: Default::default(),
        };
        let got = occurrences_from(&far, d(2026, 10, 17), d(2026, 11, 30));
        assert!((6..=7).contains(&got.len()), "{got:?}");
        assert!(got.iter().all(|x| x.weekday() == NaiveDate::MIN.weekday()));
    }

    /// The cash page's burn and runway, from the last three full months:
    /// burning $1,000.00 a month on $7,000.00 is a 7-month runway, out
    /// around May 2027 (210 days from Oct 15); an average of zero is cash
    /// growing, not a runway past any date.
    #[test]
    fn runway_is_cash_over_the_average_burn() {
        use crate::test_support::{Build, find, render};
        let mut b = Build::new("runway");
        let (cash, capital, rent) = (b.cash, b.capital, b.rent);
        b.post(d(2026, 1, 2), cash, capital, 1_000_000);
        for m in [7, 8, 9] {
            b.post(d(2026, m, 10), rent, cash, 100_000);
        }
        let (_dir, books) = b.books();
        let (texts, p) = render(books, Route::Cash, |_| {});
        let net = find(&texts, &fmt::dollars(Decimal::new(-100_000, 2)));
        assert_eq!(net.color, p.negative);
        find(&texts, "Runway");
        find(&texts, "7 months");
        find(&texts, "May 2027");
        // Zero on average: growing (July out, August in, September quiet).
        let mut b = Build::new("runway-flat");
        let (cash, capital, rent) = (b.cash, b.capital, b.rent);
        b.post(d(2026, 1, 2), cash, capital, 1_000_000);
        b.post(d(2026, 7, 10), rent, cash, 100_000);
        b.post(d(2026, 8, 10), cash, capital, 100_000);
        let (_dir, books) = b.books();
        let (texts, p) = render(books, Route::Cash, |_| {});
        assert_eq!(find(&texts, "Cash is growing").color, p.positive);
        assert!(!crate::test_support::has(
            &texts,
            "beyond any calendar date"
        ));
    }

    /// "Coming up" lists recurring cash (its cash lines' sign: rent paid
    /// out is negative, a recurring entry moving no cash zero, never
    /// "+0.00"), open invoices (overdue only when due before today) and
    /// open bills (negative), sorted by date, with the expected in and out.
    #[test]
    fn coming_up_is_what_cash_expects() {
        use crate::test_support::{Build, cents, find, has, render};
        use ledger_core::arap::DocumentLine;
        use ledger_core::{AccountType, Currency, Frequency, Money, NewRecurring, Side};
        let mut b = Build::new("coming-up");
        let (cash, capital, rent) = (b.cash, b.capital, b.rent);
        let accrued =
            b.s.add_account("2000", "Accrued", AccountType::Liability, None, false)
                .unwrap();
        b.post(d(2026, 1, 2), cash, capital, 1_000_000);
        let usd = |c: i64| Money::new(cents(c), Currency::USD);
        for (what, cr, c, day) in [("Rent", cash, 50_000, 20), ("Accrual", accrued, 5_000, 25)] {
            b.s.register_recurring(NewRecurring {
                description: what.into(),
                lines: vec![(rent, Side::Debit, usd(c)), (cr, Side::Credit, usd(c))],
                frequency: Frequency::Monthly,
                start: d(2026, 1, day),
                end: None,
            })
            .unwrap();
        }
        let line = |c: i64| {
            vec![DocumentLine {
                description: "work".into(),
                amount: cents(c),
            }]
        };
        let acme = b.s.add_customer("Acme").unwrap();
        let beta = b.s.add_customer("Beta").unwrap();
        b.s.create_invoice(acme, d(2026, 9, 1), d(2026, 10, 1), line(120_000))
            .unwrap();
        b.s.create_invoice(beta, d(2026, 10, 1), d(2026, 10, 15), line(20_000))
            .unwrap();
        let supplier = b.s.add_vendor("Supplier").unwrap();
        b.s.create_bill(supplier, d(2026, 10, 1), d(2026, 11, 5), line(30_000))
            .unwrap();
        let (_dir, books) = b.books();
        let (texts, p) = render(books, Route::Cash, |_| {});
        find(&texts, "Coming up");
        find(&texts, "Invoice 1 from Acme \u{2014} overdue");
        find(&texts, "Invoice 2 from Beta");
        find(&texts, "Bill 1 to Supplier");
        find(
            &texts,
            "Expected in $1,400.00 \u{b7} expected out $1,300.00",
        );
        let plus = |c: i64| format!("+{}", fmt::money(cents(c)));
        assert_eq!(find(&texts, &plus(120_000)).color, p.positive);
        assert_eq!(find(&texts, &plus(20_000)).color, p.positive);
        assert_eq!(find(&texts, &fmt::money(cents(-50_000))).color, p.text);
        assert_eq!(find(&texts, &fmt::money(cents(-30_000))).color, p.text);
        assert_eq!(find(&texts, &fmt::money(Decimal::ZERO)).color, p.text);
        assert!(!has(&texts, &plus(0)));
        assert!(!has(&texts, &plus(50_000)));
        assert!(!has(&texts, &plus(30_000)));
    }

    /// "Biggest payments": posted entries of the last 30 days that took
    /// cash out, never older ones, deposits, or drafts.
    #[test]
    fn biggest_payments_are_recent_posted_outflows() {
        use crate::test_support::{Build, find, has, render};
        let mut b = Build::new("outflows");
        let (cash, capital, rent) = (b.cash, b.capital, b.rent);
        b.post_as("Opening", d(2026, 1, 2), cash, capital, 1_000_000);
        b.post_as("Old rent", d(2026, 9, 10), rent, cash, 25_000);
        b.post_as("October rent", d(2026, 10, 1), rent, cash, 25_000);
        b.post_as("Deposit", d(2026, 10, 5), cash, capital, 5_000);
        b.draft("Draft pay", d(2026, 10, 10), rent, cash, 7_000);
        // Thirty days before today exactly: not in the last 30 days.
        b.post_as("Edge payment", d(2026, 9, 15), rent, cash, 9_000);
        // An expense the owner paid: no cash out.
        b.post_as("Owner paid", d(2026, 10, 12), rent, capital, 4_000);
        let (_dir, books) = b.books();
        let (texts, _) = render(books, Route::Cash, |_| {});
        find(&texts, "Biggest payments, last 30 days");
        find(&texts, "October rent");
        for gone in [
            "Old rent",
            "Deposit",
            "Draft pay",
            "Edge payment",
            "Owner paid",
        ] {
            assert!(!has(&texts, gone), "{gone}");
        }
    }
}
