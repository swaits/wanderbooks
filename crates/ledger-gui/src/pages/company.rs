//! The company: who it is, who owns it, and the standing machinery of the
//! books — periods, fixed assets, amortization schedules, recurring
//! entries, vendors and customers.

use super::{columns, page};
use crate::app::Shell;
use crate::books::Books;
use crate::fmt;
use crate::nav::Route;
use crate::theme::{self, pal};
use crate::widgets;
use egui::{CornerRadius, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as icon;
use ledger_core::assets::{accumulated_depreciation, book_value};
use ledger_core::{PeriodStatus, vendor_1099_report};
use rust_decimal::Decimal;

fn kv(ui: &mut Ui, k: &str, v: &str) {
    let p = pal(ui.ctx());
    // Narrow, the label goes above its value.
    if ui.available_width() < 360.0 {
        ui.label(RichText::new(k).font(theme::regular(12.0)).color(p.muted));
        ui.add(egui::Label::new(RichText::new(v).font(theme::regular(13.0))).wrap());
        ui.add_space(4.0);
        return;
    }
    ui.horizontal(|ui| {
        let kw = (ui.available_width() * 0.4).min(180.0);
        let (r, _) = ui.allocate_exact_size(vec2(kw, 20.0), Sense::hover());
        ui.painter().text(
            r.left_center(),
            egui::Align2::LEFT_CENTER,
            k,
            theme::regular(13.0),
            p.muted,
        );
        ui.add(egui::Label::new(RichText::new(v).font(theme::regular(13.0))).wrap());
    });
}

pub fn show(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    page(ui, "company", |ui| {
        let c = &b.settings.company;
        widgets::page_header(
            ui,
            &c.name,
            "The company's profile and the standing parts of its books.",
        );
        columns(ui, 980.0, &[1.0, 1.0], |i, ui| match i {
            0 => {
                widgets::titled_card(
                    ui,
                    "Profile",
                    |_| {},
                    |ui| {
                        if c.profile.is_empty() {
                            ui.label(
                                RichText::new("No [company] settings in wanderbooks.toml.")
                                    .color(p.muted),
                            );
                        }
                        for (k, v) in &c.profile {
                            kv(ui, &fmt::label(k), v);
                        }
                        kv(ui, "Accounting basis", &c.basis);
                        kv(
                            ui,
                            "Tax return",
                            c.tax_form.map(fmt::tax_form).unwrap_or("not set"),
                        );
                        kv(ui, "Receipt threshold", &fmt::dollars(b.receipt_threshold));
                    },
                );
            }
            _ => {
                widgets::titled_card(
                    ui,
                    "Owners",
                    |_| {},
                    |ui| {
                        if c.members.is_empty() {
                            // A single owner or a corporation: equity by account.
                            for a in b
                                .ledger
                                .chart()
                                .all()
                                .filter(|a| a.account_type == ledger_core::AccountType::Equity)
                            {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new(format!("{} {}", a.code, a.name))
                                            .font(theme::regular(13.0)),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new(fmt::accounting(
                                                    b.balance_on(a.id, b.today),
                                                ))
                                                .font(theme::semibold(13.0)),
                                            );
                                        },
                                    );
                                });
                            }
                        }
                        for m in &c.members {
                            let cap = b.ledger.chart().find_by_code(&m.capital_account);
                            let bal = cap.map(|a| b.balance_on(a.id, b.today)).unwrap_or_default();
                            ui.horizontal_wrapped(|ui| {
                                ui.label(RichText::new(&m.name).font(theme::semibold(13.5)));
                                widgets::pill(
                                    ui,
                                    &format!("{}%", (m.share * Decimal::from(100)).normalize()),
                                    p.accent,
                                    p.accent_soft,
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(fmt::dollars(bal))
                                                .font(theme::semibold(13.5)),
                                        );
                                    },
                                );
                            });
                            if let Some(a) = cap {
                                ui.label(
                                    RichText::new(format!("Capital account {} {}", a.code, a.name))
                                        .font(theme::regular(12.0))
                                        .color(p.muted),
                                );
                            }
                            ui.add_space(6.0);
                        }
                    },
                );
                ui.add_space(12.0);
                widgets::titled_card(
                    ui,
                    "Periods",
                    |_| {},
                    |ui| {
                        let periods = b.ledger.list_periods();
                        if periods.is_empty() {
                            ui.label(RichText::new("No periods yet. Closing a period locks it and moves its earnings into equity.").color(p.muted));
                        }
                        for per in periods {
                            ui.horizontal_wrapped(|ui| {
                                let (g, color) = match per.status {
                                    PeriodStatus::Closed => (icon::LOCK, p.muted),
                                    PeriodStatus::Open => (icon::LOCK_OPEN, p.warning),
                                };
                                ui.label(RichText::new(g).color(color));
                                ui.label(RichText::new(&per.name).font(theme::semibold(13.0)));
                                ui.label(
                                    RichText::new(format!(
                                        "{} – {}",
                                        fmt::date(per.start),
                                        fmt::date(per.end)
                                    ))
                                    .color(p.muted),
                                );
                                if let Some(e) = per.closing_entry
                                    && widgets::link(
                                        ui,
                                        RichText::new(format!("closing entry #{e}"))
                                            .color(p.accent),
                                    )
                                    .clicked()
                                {
                                    shell.go(Route::Entry(e));
                                }
                            });
                        }
                    },
                );
            }
        });
        assets(ui, b, shell);
        schedules(ui, b);
        recurring(ui, b);
        parties(ui, b);
    });
}

fn assets(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let assets: Vec<_> = b.ledger.registered_assets().collect();
    if assets.is_empty() {
        return;
    }
    let mut go = None;
    widgets::titled_card(
        ui,
        "Fixed assets",
        |_| {},
        |ui| {
            for a in &assets {
                // What has actually been posted, so this ties to the balance
                // sheet; the schedule may run ahead of the books.
                let acc: Decimal = b
                    .journal
                    .iter()
                    .filter(|l| {
                        l.line_type == "asset.depreciation_posted"
                            && l.payload.get("asset_id").and_then(|v| v.as_u64()) == Some(a.id)
                    })
                    .filter_map(|l| {
                        l.payload
                            .get("amount")
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse::<Decimal>().ok())
                    })
                    .sum();
                let bv = a.cost.amount - acc;
                let scheduled = accumulated_depreciation(a, b.today);
                let _ = book_value;
                let dep = a.cost.amount - a.salvage_value.amount;
                let frac: f32 = if dep.is_zero() {
                    1.0
                } else {
                    (acc / dep).try_into().unwrap_or(0.0)
                };
                ui.horizontal_wrapped(|ui| {
                    if widgets::link(
                        ui,
                        RichText::new(&a.description).font(theme::semibold(13.5)),
                    )
                    .clicked()
                    {
                        go = Some(a.cost_account);
                    }
                    ui.label(
                        RichText::new(format!(
                            "bought {} · {} months · {}",
                            fmt::date(a.purchase_date),
                            a.useful_life_months,
                            fmt::words(&format!("{:?}", a.status)).to_lowercase()
                        ))
                        .font(theme::regular(12.0))
                        .color(p.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("book value {}", fmt::dollars(bv)))
                                .font(theme::semibold(13.0)),
                        );
                    });
                });
                let behind = scheduled - acc;
                let note = if behind > Decimal::ZERO {
                    format!(" · {} more scheduled through today", fmt::dollars(behind))
                } else {
                    String::new()
                };
                progress(
                    ui,
                    frac,
                    &format!(
                        "{} of {} depreciated{note}",
                        fmt::dollars(acc),
                        fmt::dollars(dep)
                    ),
                );
                ui.add_space(8.0);
            }
        },
    );
    if let Some(id) = go {
        shell.go(Route::Account(id));
    }
    ui.add_space(16.0);
}

fn progress(ui: &mut Ui, frac: f32, label: &str) {
    let p = pal(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(4), p.hover);
    let mut done = r;
    done.set_width(r.width() * frac.clamp(0.0, 1.0));
    ui.painter()
        .rect_filled(done, CornerRadius::same(4), p.accent);
    ui.label(
        RichText::new(label)
            .font(theme::regular(11.5))
            .color(p.faint),
    );
}

fn schedules(ui: &mut Ui, b: &Books) {
    let p = pal(ui.ctx());
    let list: Vec<_> = b.ledger.schedules().collect();
    if list.is_empty() {
        return;
    }
    widgets::titled_card(
        ui,
        "Amortization schedules",
        |_| {},
        |ui| {
            for s in list {
                let total = s.total.amount;
                let remaining = s.remaining();
                let frac: f32 = if total.is_zero() {
                    1.0
                } else {
                    ((total - remaining) / total).try_into().unwrap_or(0.0)
                };
                let due = s.due(b.today);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&s.description).font(theme::semibold(13.5)));
                    ui.label(
                        RichText::new(format!(
                            "{} · {} months from {}",
                            fmt::words(&format!("{:?}", s.kind)),
                            s.months,
                            fmt::month(s.start)
                        ))
                        .font(theme::regular(12.0))
                        .color(p.muted),
                    );
                    if !due.is_empty() {
                        widgets::pill(ui, &format!("{} due", due.len()), p.warning, p.warning_soft);
                    }
                });
                progress(
                    ui,
                    frac,
                    &format!(
                        "{} of {} recognized · {} left",
                        fmt::dollars(total - remaining),
                        fmt::dollars(total),
                        fmt::dollars(remaining)
                    ),
                );
                ui.add_space(8.0);
            }
        },
    );
    ui.add_space(16.0);
}

fn recurring(ui: &mut Ui, b: &Books) {
    let p = pal(ui.ctx());
    let list: Vec<_> = b.ledger.recurring_entries().collect();
    if list.is_empty() {
        return;
    }
    widgets::titled_card(
        ui,
        "Recurring entries",
        |_| {},
        |ui| {
            for r in list {
                let amount: Decimal = r
                    .lines
                    .iter()
                    .filter(|(_, s, _)| *s == ledger_core::Side::Debit)
                    .map(|(_, _, m)| m.amount)
                    .sum();
                let next = (0..)
                    .map_while(|k| {
                        r.frequency
                            .nth(r.start, k)
                            .filter(|d| r.end.is_none_or(|e| *d <= e))
                    })
                    .find(|d| !r.generated.contains(d));
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&r.description).font(theme::semibold(13.5)));
                    ui.label(
                        RichText::new(format!(
                            "{} · {}",
                            fmt::words(&format!("{:?}", r.frequency)),
                            fmt::dollars(amount)
                        ))
                        .font(theme::regular(12.0))
                        .color(p.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let text = match next {
                            Some(d) if d <= b.today => format!("due {}", fmt::date(d)),
                            Some(d) => format!("next {}", fmt::date(d)),
                            None => "ended".into(),
                        };
                        ui.label(RichText::new(text).font(theme::regular(12.5)).color(
                            if next.is_some_and(|d| d <= b.today) {
                                p.warning
                            } else {
                                p.muted
                            },
                        ));
                    });
                });
                ui.label(
                    RichText::new(format!(
                        "{} generated so far · drafts only; each one is posted with its receipt",
                        r.generated.len()
                    ))
                    .font(theme::regular(11.5))
                    .color(p.faint),
                );
                ui.add_space(6.0);
            }
        },
    );
    ui.add_space(16.0);
}

fn parties(ui: &mut Ui, b: &Books) {
    let p = pal(ui.ctx());
    let year = chrono::Datelike::year(&b.today);
    let r1099 = vendor_1099_report(&b.ledger, year, None);
    columns(ui, 980.0, &[1.0, 1.0], |i, ui| match i {
        0 => {
            widgets::titled_card(
                ui,
                "Vendors",
                |_| {},
                |ui| {
                    let vendors = b.ledger.list_vendors();
                    if vendors.is_empty() {
                        ui.label(RichText::new("No vendors yet.").color(p.muted));
                    }
                    for v in vendors {
                        let paid = r1099
                            .vendors
                            .iter()
                            .find(|x| x.vendor_id == v.id)
                            .map(|x| x.paid)
                            .unwrap_or_default();
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&v.name).font(theme::semibold(13.0)));
                            if let Some(f) = v.form_1099 {
                                widgets::pill(ui, f.label(), p.accent, p.accent_soft);
                                if v.tin_on_file {
                                    widgets::pill(ui, "W-9", p.positive, p.positive_soft);
                                } else {
                                    widgets::pill(ui, "no W-9", p.negative, p.negative_soft);
                                }
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if !paid.is_zero() {
                                        ui.label(
                                            RichText::new(format!(
                                                "{} paid in {year}",
                                                fmt::dollars(paid)
                                            ))
                                            .font(theme::regular(12.5))
                                            .color(p.muted),
                                        );
                                    }
                                },
                            );
                        });
                    }
                },
            );
        }
        _ => {
            widgets::titled_card(
                ui,
                "Customers",
                |_| {},
                |ui| {
                    let customers = b.ledger.list_customers();
                    if customers.is_empty() {
                        ui.label(RichText::new("No customers yet.").color(p.muted));
                    }
                    for c in customers {
                        let owed: Decimal = (1..b.ledger.next_invoice_id())
                            .filter_map(|i| b.ledger.get_invoice(i))
                            .filter(|i| i.customer_id == c.id)
                            .map(|i| i.balance())
                            .sum();
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&c.name).font(theme::semibold(13.0)));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if !owed.is_zero() {
                                        ui.label(
                                            RichText::new(format!("owes {}", fmt::dollars(owed)))
                                                .font(theme::regular(12.5))
                                                .color(p.muted),
                                        );
                                    }
                                },
                            );
                        });
                    }
                },
            );
        }
    });
}
