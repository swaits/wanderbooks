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
use ledger_core::assets::accumulated_scheduled_through;
use ledger_core::{AssetStatus, PeriodStatus};
use rust_decimal::Decimal;

fn kv(ui: &mut Ui, k: &str, v: &str) {
    let p = pal(ui.ctx());
    // Narrow, the label goes above its value.
    if ui.available_width() < 360.0 {
        ui.label(RichText::new(k).font(theme::regular(12.0)).color(p.muted));
        widgets::copyable(ui, RichText::new(v).font(theme::regular(13.0)), v);
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
        widgets::copyable(ui, RichText::new(v).font(theme::regular(13.0)), v);
    });
}

/// Who decided a setting and when, under its value.
fn provenance(ui: &mut Ui, by: &str, on: chrono::NaiveDate) {
    let p = pal(ui.ctx());
    ui.label(
        RichText::new(format!("{by} · {}", fmt::date(on)))
            .font(theme::regular(11.0))
            .color(p.muted),
    );
}

/// How the books are kept, as the owners decided: conventions, decisions,
/// and what is open (each only when there is something to show).
fn kept_cards(ui: &mut Ui, b: &Books, go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    let kept = b.ledger.conventions();
    let conventions: Vec<_> = kept.conventions().filter(|c| c.retired.is_none()).collect();
    if !conventions.is_empty() {
        ui.add_space(12.0);
        widgets::titled_card(
            ui,
            "How the books are kept",
            |_| {},
            |ui| {
                for c in &conventions {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&c.title).font(theme::semibold(13.0)));
                        widgets::pill(ui, &c.kind.key().replace('_', " "), p.accent, p.accent_soft);
                    });
                    widgets::copyable(
                        ui,
                        RichText::new(c.summary()).font(theme::regular(12.5)),
                        &c.body_md,
                    );
                    provenance(ui, &c.source.decided_by, c.source.decided_on);
                    ui.add_space(6.0);
                }
            },
        );
    }
    let decisions: Vec<_> = kept
        .decisions()
        .filter(|d| d.superseded_by.is_none())
        .collect();
    if !decisions.is_empty() {
        ui.add_space(12.0);
        widgets::titled_card(
            ui,
            "Decisions",
            |_| {},
            |ui| {
                for d in decisions.iter().rev() {
                    ui.label(RichText::new(&d.new.subject).font(theme::semibold(13.0)));
                    widgets::copyable(
                        ui,
                        RichText::new(&d.new.decision_md).font(theme::regular(12.5)),
                        &d.new.decision_md,
                    );
                    if !d.new.projects.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            for &c in &d.new.projects {
                                if widgets::link(
                                    ui,
                                    RichText::new(format!(
                                        "{} {}",
                                        icon::BRIEFCASE,
                                        b.project_label(c)
                                    ))
                                    .font(theme::regular(12.0))
                                    .color(p.accent),
                                )
                                .on_hover_text("Open this project")
                                .clicked()
                                {
                                    *go = Some(Route::Project(c));
                                }
                            }
                        });
                    }
                    provenance(ui, &d.new.decided_by.join(" and "), d.new.decided_on);
                    ui.add_space(6.0);
                }
            },
        );
    }
    let open: Vec<_> = kept.open_items().filter(|i| i.resolved.is_none()).collect();
    if !open.is_empty() {
        ui.add_space(12.0);
        widgets::titled_card(
            ui,
            "Open items",
            |_| {},
            |ui| {
                for i in open {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(icon::FLAG).color(p.warning));
                        ui.label(RichText::new(&i.new.title).font(theme::regular(13.0)));
                        if let Some(due) = i.new.due {
                            // Late says so in words, not only in colour.
                            let late = due < b.today;
                            let when = if late { "overdue since" } else { "due" };
                            ui.label(
                                RichText::new(format!("{when} {}", fmt::date(due)))
                                    .font(theme::regular(12.0))
                                    .color(if late { p.warning } else { p.muted }),
                            );
                        }
                    });
                }
            },
        );
    }
}

pub fn show(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let mut kept_go: Option<Route> = None;
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
                        let kept = b.ledger.settings();
                        // Where the settings shown come from: a settings
                        // file only for an older books dir that keeps none
                        // in its journal and has one (data::settings).
                        let from_file =
                            !kept.is_journaled() && crate::data::legacy_toml(&b.data_dir).is_some();
                        // The currency is always known: facts are the rest.
                        if c.profile.iter().all(|(k, _)| k == "base_currency") {
                            ui.label(
                                RichText::new(if from_file {
                                    "No company facts in the settings file yet: the agent records them as you give them."
                                } else {
                                    "No company facts set yet: the agent records them as you give them."
                                })
                                .color(p.muted),
                            );
                        }
                        for (k, v) in &c.profile {
                            kv(ui, &fmt::label(k), v);
                            // Who set it, and when (settings kept in the
                            // books carry their source).
                            if let Some(r) = kept.get(&format!("company.{k}")) {
                                provenance(ui, &r.source.decided_by, r.source.decided_on);
                            }
                        }
                        kv(ui, "Accounting basis", &c.basis);
                        kv(
                            ui,
                            "Tax return",
                            c.tax_form.map(fmt::tax_form).unwrap_or("not set"),
                        );
                        kv(ui, "Receipt threshold", &fmt::dollars(b.receipt_threshold));
                        if let Some(z) = &b.settings.timezone {
                            kv(ui, "Time zone", z);
                        }
                        ui.label(
                            RichText::new(if from_file {
                                "Read from the settings file: the agent can move it into the books."
                            } else {
                                "Kept in the books: the agent changes settings, with who decided and when."
                            })
                            .font(theme::regular(12.0))
                            .color(p.muted),
                        );
                    },
                );
                kept_cards(ui, b, &mut kept_go);
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
        let mut subjects = vec![ledger_store::Subject::Company];
        subjects.extend(
            b.ledger
                .list_periods()
                .iter()
                .map(|x| ledger_store::Subject::Period(x.id)),
        );
        subjects.extend(
            b.ledger
                .schedules()
                .map(|x| ledger_store::Subject::Schedule(x.id)),
        );
        let mut go = None;
        super::documents::documents_card_with(
            ui,
            b,
            "Company documents",
            &subjects,
            &[ledger_store::Subject::Company],
            "No documents linked to the company, its periods, or its schedules yet (formation papers, the operating agreement, tax filings).",
            &mut go,
        );
        ui.add_space(16.0);
        if let Some(r) = go {
            shell.go(r);
        }
        assets(ui, b, shell);
        schedules(ui, b);
        recurring(ui, b);
        parties(ui, b, shell);
    });
    if let Some(r) = kept_go {
        shell.go(r);
    }
}

fn assets(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let assets: Vec<_> = b.ledger.registered_assets().collect();
    if assets.is_empty() {
        return;
    }
    let mut go = None;
    let (register, _) = widgets::titled_card(
        ui,
        "Fixed assets",
        |ui| {
            widgets::link(
                ui,
                RichText::new(format!("Asset register {}", icon::ARROW_RIGHT))
                    .font(theme::regular(12.5))
                    .color(p.accent),
            )
            .clicked()
        },
        |ui| {
            for a in &assets {
                // What the books carry (its runs, less reversals:
                // Ledger::accumulated_posted; zero once disposed, when the
                // asset left the books), so this ties to the balance
                // sheet; the schedule may run ahead of the books.
                let disposed = match &a.status {
                    AssetStatus::Disposed { date, .. } if *date <= b.today => Some(*date),
                    _ => None,
                };
                // Not on the books until its acquired date: no book value
                // yet (the register leaves it out the same way).
                let later = b.ledger.asset_acquired_date(a.id).filter(|d| *d > b.today);
                let acc = b.ledger.accumulated_posted(a.id, b.today);
                let scheduled = accumulated_scheduled_through(a, b.today);
                let dep = a.cost.amount - a.salvage_value.amount;
                ui.horizontal_wrapped(|ui| {
                    if widgets::link(
                        ui,
                        RichText::new(b.asset_label(a.id)).font(theme::semibold(13.5)),
                    )
                    .clicked()
                    {
                        go = Some(a.id);
                    }
                    ui.label(
                        RichText::new(format!(
                            "in service {} · {} months · {}",
                            fmt::date(a.in_service_date),
                            a.useful_life_months,
                            match &a.status {
                                AssetStatus::Active => "active".to_string(),
                                AssetStatus::Disposed { date, .. } =>
                                    format!("disposed {}", fmt::date(*date)),
                            }
                        ))
                        .font(theme::regular(12.0))
                        .color(p.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let text = match (&acc, disposed) {
                            _ if later.is_some() => format!(
                                "not on the books until {}",
                                fmt::date(later.expect("checked"))
                            ),
                            (_, Some(_)) => format!("book value {}", fmt::dollars(Decimal::ZERO)),
                            (Ok(acc), None) => {
                                format!("book value {}", fmt::dollars(a.cost.amount - *acc))
                            }
                            (Err(e), None) => format!("book value can't be computed: {e}"),
                        };
                        ui.label(RichText::new(text).font(theme::semibold(13.0)));
                    });
                });
                match (&acc, disposed) {
                    (_, Some(date)) => progress(
                        ui,
                        1.0,
                        &format!("disposed {}: it left the books", fmt::date(date)),
                    ),
                    (Ok(acc), None) => {
                        let frac: f32 = if dep.is_zero() {
                            1.0
                        } else {
                            (*acc / dep).try_into().unwrap_or(0.0)
                        };
                        let note = match scheduled {
                            Ok(s) if s > *acc => {
                                format!(
                                    " · {} more scheduled through today",
                                    fmt::dollars(s - *acc)
                                )
                            }
                            Ok(_) => String::new(),
                            // An earlier version's asset: said, never guessed.
                            Err(_) => " · its schedule can't be computed".to_string(),
                        };
                        progress(
                            ui,
                            frac,
                            &format!(
                                "{} of {} depreciated{note}",
                                fmt::dollars(*acc),
                                fmt::dollars(dep)
                            ),
                        );
                    }
                    (Err(_), None) => {}
                }
                ui.add_space(8.0);
            }
        },
    );
    if let Some(id) = go {
        shell.go(Route::Asset(id));
    }
    if register {
        shell.go(Route::Assets);
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
                // A schedule an earlier version journaled may not compute
                // (months past chrono's last day): said, never guessed.
                let (remaining, due) = match (s.remaining(), s.due(b.today)) {
                    (Ok(r), Ok(d)) => (r, d),
                    (Err(e), _) | (_, Err(e)) => {
                        ui.label(RichText::new(&s.description).font(theme::semibold(13.5)));
                        ui.label(
                            RichText::new(format!("Its plan can't be computed: {e}."))
                                .font(theme::regular(12.0))
                                .color(p.negative),
                        );
                        ui.add_space(8.0);
                        continue;
                    }
                };
                let recognized = total.checked_sub(remaining);
                let frac: f32 = match recognized {
                    _ if total.is_zero() => 1.0,
                    Some(r) => r
                        .checked_div(total)
                        .and_then(|x| x.try_into().ok())
                        .unwrap_or(0.0),
                    None => 0.0,
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&s.description).font(theme::semibold(13.5)));
                    ui.label(
                        RichText::new(match s.convention {
                            ledger_core::ScheduleConvention::Daily => format!(
                                "{} · {} months from {}, by the day",
                                fmt::words(&format!("{:?}", s.kind)),
                                s.months,
                                fmt::date(s.start)
                            ),
                            ledger_core::ScheduleConvention::MonthlyEven => format!(
                                "{} · {} months from {}",
                                fmt::words(&format!("{:?}", s.kind)),
                                s.months,
                                fmt::month(s.start)
                            ),
                        })
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
                        fmt::dollars_or(recognized),
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
                let amount = r
                    .lines
                    .iter()
                    .filter(|(_, s, _)| *s == ledger_core::Side::Debit)
                    .try_fold(Decimal::ZERO, |t, (_, _, m)| t.checked_add(m.amount));
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
                            fmt::dollars_or(amount)
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

fn parties(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let mut go: Option<crate::nav::Route> = None;
    // A record's name opens its party.
    let mut name = |ui: &mut Ui, text: &str, party: Option<u64>| {
        let r = widgets::link(ui, RichText::new(text).font(theme::semibold(13.0)));
        if let Some(id) = party
            && r.on_hover_text("Open this party").clicked()
        {
            go = Some(crate::nav::Route::Party(id));
        }
    };
    let year = chrono::Datelike::year(&b.today);
    let r1099 = b.vendor_1099(year).ok();
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
                            .iter()
                            .flat_map(|r| &r.vendors)
                            .find(|x| x.vendor_id == v.id)
                            .map(|x| x.paid)
                            .unwrap_or_default();
                        ui.horizontal_wrapped(|ui| {
                            name(ui, &v.name, b.ledger.party_of_vendor(v.id));
                            match v.decision_on(b.today) {
                                Some(ledger_core::Decision1099::Files(f)) => {
                                    widgets::pill(ui, f.label(), p.accent, p.accent_soft);
                                    if v.tin_on_file {
                                        widgets::pill(ui, "W-9", p.positive, p.positive_soft);
                                    } else {
                                        widgets::pill(ui, "no W-9", p.negative, p.negative_soft);
                                    }
                                }
                                Some(ledger_core::Decision1099::NotReportable { .. }) => {
                                    widgets::pill(ui, "No 1099", p.muted, p.hover);
                                }
                                // Undecided: nothing to show (the 1099
                                // worksheet and Needs attention ask).
                                _ => {}
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
                        // None: a balance, or their sum, can't be computed.
                        let owed = (1..b.ledger.next_invoice_id())
                            .filter_map(|i| b.ledger.get_invoice(i))
                            .filter(|i| i.customer_id == c.id)
                            .try_fold(Decimal::ZERO, |t, i| t.checked_add(i.balance().ok()?));
                        ui.horizontal_wrapped(|ui| {
                            name(ui, &c.name, b.ledger.party_of_customer(c.id));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if !owed.is_some_and(|o| o.is_zero()) {
                                        ui.label(
                                            RichText::new(format!(
                                                "owes {}",
                                                fmt::dollars_or(owed)
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
    });
    if let Some(r) = go {
        shell.go(r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Build, beside, cents, d, find, has, render_all};
    use ledger_core::arap::DocumentLine;
    use ledger_core::{AccountType, Currency, Frequency, Money, NewRecurring, NewSchedule, Side};

    /// The company page: each amortization schedule's recognized share as
    /// a bar (three of twelve months posted: a quarter), each recurring
    /// entry, and each customer with what they owe, theirs alone.
    #[test]
    fn the_company_page_says_what_each_record_holds() {
        let mut b = Build::new("company");
        let (cash, capital, rent) = (b.cash, b.capital, b.rent);
        let prepaid =
            b.s.add_account("1300", "Prepaid", AccountType::Asset, None, false)
                .unwrap();
        let insurance =
            b.s.add_account("6300", "Insurance", AccountType::Expense, None, false)
                .unwrap();
        b.post(d(2026, 1, 2), cash, capital, 1_000_000);
        b.post(d(2026, 1, 2), prepaid, cash, 120_000);
        let usd = |c: i64| Money::new(cents(c), Currency::USD);
        b.s.register_schedule(NewSchedule {
            description: "Insurance plan".into(),
            kind: ledger_core::ScheduleKind::PrepaidExpense,
            total: usd(120_000),
            start: d(2026, 1, 1),
            months: 12,
            balance_account: prepaid,
            pl_account: insurance,
            convention: ledger_core::ScheduleConvention::MonthlyEven,
        })
        .unwrap();
        b.s.run_schedules(d(2026, 3, 31)).unwrap();
        b.s.register_recurring(NewRecurring {
            description: "Office rent".into(),
            lines: vec![
                (rent, Side::Debit, usd(50_000)),
                (cash, Side::Credit, usd(50_000)),
            ],
            frequency: Frequency::Monthly,
            start: d(2026, 1, 20),
            end: None,
        })
        .unwrap();
        let line = |c: i64| {
            vec![DocumentLine {
                description: "work".into(),
                amount: cents(c),
                class_id: None,
            }]
        };
        let acme = b.s.add_customer("Acme").unwrap();
        let beta = b.s.add_customer("Beta").unwrap();
        b.s.add_customer("Gamma").unwrap();
        b.s.create_invoice(acme, d(2026, 9, 1), d(2026, 10, 1), line(120_000))
            .unwrap();
        b.s.create_invoice(beta, d(2026, 9, 1), d(2026, 10, 1), line(20_000))
            .unwrap();
        let (_dir, books) = b.books();
        let (texts, rects, p) = render_all(books, Route::Company, |_| {});
        find(&texts, "Amortization schedules");
        let label = find(
            &texts,
            "$300.00 of $1,200.00 recognized \u{b7} $900.00 left",
        );
        // The bar just above it: the track, and the part done.
        let near = |fill: egui::Color32| {
            rects
                .iter()
                .filter(|(r, f)| {
                    *f == fill
                        && r.height() <= 8.5
                        && (label.pos.y - r.max.y) >= 0.0
                        && (label.pos.y - r.max.y) < 12.0
                })
                .map(|(r, _)| r.width())
                .fold(0.0f32, f32::max)
        };
        let (track, done) = (near(p.hover), near(p.accent));
        assert!(track > 100.0, "track {track}");
        assert!((done / track - 0.25).abs() < 0.01, "{done} of {track}");
        find(&texts, "Recurring entries");
        find(&texts, "Office rent");
        find(&texts, "Vendors");
        find(&texts, "Customers");
        assert_eq!(beside(&texts, "Acme").text, "owes $1,200.00");
        assert_eq!(beside(&texts, "Beta").text, "owes $200.00");
        assert!(!has(&texts, "owes $0.00"));
    }
}
