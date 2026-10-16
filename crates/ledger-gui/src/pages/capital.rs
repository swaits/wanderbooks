//! A member's capital account statement, on their party page: the partner
//! capital statement (opening balance, contributions of cash and of
//! property, share of net income, distributions, closing balance), their
//! share, and their part of what the company owes the members jointly.
//! Every number is [`crate::books::CapitalStatement`]'s (which reads
//! ledger_core::k1, the CPA package's K-1 worksheet); this module only
//! lays it out.

use crate::books::{Books, CapitalKind, CapitalStatement};
use crate::fmt;
use crate::nav::Route;
use crate::theme::{self, pal};
use crate::widgets;
use egui::{Align, Layout, RichText, Sense, Ui, vec2};
use rust_decimal::Decimal;

/// What a capital line is, in words.
pub fn kind_label(k: CapitalKind) -> &'static str {
    match k {
        CapitalKind::CashContribution => "Contribution — cash",
        CapitalKind::PropertyContribution => "Contribution — property",
        CapitalKind::IncomeShare => "Share of net income",
        CapitalKind::Distribution => "Distribution",
    }
}

/// A share as a percent: 0.5 → "50%", 0.3333 → "33.33%".
pub fn percent(share: Decimal) -> String {
    format!("{}%", (share * Decimal::ONE_HUNDRED).normalize())
}

/// One statement row: an indented label, an amount column, and an
/// optional total column; `strong` for the balances.
fn line(
    ui: &mut Ui,
    label: &str,
    indent: f32,
    amount: Option<String>,
    total: Option<String>,
    strong: bool,
) {
    let p = pal(ui.ctx());
    let col = 130.0;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        let font = if strong {
            theme::semibold(14.0)
        } else {
            theme::regular(14.0)
        };
        ui.label(RichText::new(label).font(font));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let t = total.unwrap_or_default();
            widgets::amount_cell(ui, &t, col, p.text, strong, false);
            let a = amount.unwrap_or_default();
            widgets::amount_cell(ui, &a, col, p.text, false, false);
        });
    });
}

/// A short rule over the amount column (`double`: two, under a total).
fn underline(ui: &mut Ui, total_col: bool, double: bool) {
    let p = pal(ui.ctx());
    let col = 130.0;
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, if double { 5.0 } else { 3.0 }), Sense::hover());
    let right = if total_col {
        rect.right()
    } else {
        rect.right() - col - ui.spacing().item_spacing.x
    };
    let left = right - col + 30.0;
    let stroke = egui::Stroke::new(1.0, p.text);
    let y = rect.top() + 1.0;
    ui.painter()
        .line_segment([egui::pos2(left, y), egui::pos2(right, y)], stroke);
    if double {
        ui.painter().line_segment(
            [egui::pos2(left, y + 3.0), egui::pos2(right, y + 3.0)],
            stroke,
        );
    }
}

/// The member's capital account card.
pub fn capital_card(
    ui: &mut Ui,
    b: &Books,
    st: &CapitalStatement,
    name: &str,
    go: &mut Option<Route>,
) {
    let p = pal(ui.ctx());
    let account = b.account_name(st.capital);
    let (open_account, ()) = widgets::titled_card(
        ui,
        "Capital account",
        |ui| {
            widgets::link(
                ui,
                RichText::new(&account)
                    .font(theme::regular(12.0))
                    .color(p.accent),
            )
            .on_hover_text("Open the capital account's register")
            .clicked()
        },
        |ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(name).font(theme::semibold(15.0)));
                ui.label(
                    RichText::new(format!(
                        "Statement of partner's capital · {} – {}",
                        fmt::date(st.span.0),
                        fmt::date(st.span.1)
                    ))
                    .font(theme::regular(12.5))
                    .color(p.muted),
                );
                if let Some(j) = &st.files_jointly_with {
                    ui.label(
                        RichText::new(format!(
                            "Files a joint return with {j}: a K-1 each, both on one Form 1040"
                        ))
                        .font(theme::regular(12.0))
                        .color(p.muted),
                    );
                }
            });
            ui.add_space(10.0);
            let amt = |d: Decimal| Some(fmt::accounting(d));
            line(
                ui,
                &format!("Capital, {}", fmt::date(st.span.0)),
                0.0,
                None,
                Some(format!("${}", fmt::accounting(st.opening))),
                true,
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("Add")
                    .font(theme::regular(12.0))
                    .color(p.muted),
            );
            line(
                ui,
                "Contributions of cash",
                16.0,
                amt(st.contributions_cash),
                None,
                false,
            );
            line(
                ui,
                "Contributions of property (at fair market value)",
                16.0,
                amt(st.contributions_property),
                None,
                false,
            );
            line(
                ui,
                &format!("Share of net income ({})", percent(st.share)),
                16.0,
                amt(st.income_share),
                None,
                false,
            );
            ui.label(
                RichText::new("Less")
                    .font(theme::regular(12.0))
                    .color(p.muted),
            );
            line(
                ui,
                "Distributions",
                16.0,
                Some(fmt::accounting(-st.distributions_taken)),
                None,
                false,
            );
            if !st.other.is_zero() {
                line(ui, "Other changes", 16.0, amt(st.other), None, false);
            }
            underline(ui, false, false);
            let change = st.contributions_cash + st.contributions_property + st.income_share
                - st.distributions_taken
                + st.other;
            line(
                ui,
                "Net change in capital",
                0.0,
                None,
                Some(fmt::accounting(change)),
                false,
            );
            underline(ui, true, false);
            line(
                ui,
                &format!("Capital, {}", fmt::date(st.span.1)),
                0.0,
                None,
                Some(format!("${}", fmt::accounting(st.closing))),
                true,
            );
            underline(ui, true, true);
            if !st.income_share.is_zero()
                && !st.lines.iter().any(|l| l.kind == CapitalKind::IncomeShare)
            {
                ui.label(
                    RichText::new(
                        "No period closed in this span: the share of net income is earned but not yet closed into the capital account.",
                    )
                    .font(theme::regular(11.5))
                    .color(p.faint),
                );
            }
            ui.add_space(10.0);
            widgets::rule(ui);
            ui.add_space(6.0);
            line(
                ui,
                "Share of profit, loss, and capital",
                0.0,
                None,
                Some(percent(st.share)),
                false,
            );
            let owed = format!(
                "Owed to them on Due to Shareholders ({} of {})",
                percent(st.share),
                fmt::dollars(st.due_to_members)
            );
            line(
                ui,
                &owed,
                0.0,
                None,
                Some(fmt::dollars(st.owed_to_member)),
                false,
            );
            if !st.lines.is_empty() {
                ui.add_space(10.0);
                ui.collapsing(
                    RichText::new(format!(
                        "{} in this span",
                        fmt::count(st.lines.len(), "capital entry", "capital entries")
                    ))
                    .font(theme::regular(12.5)),
                    |ui| {
                        for l in &st.lines {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(fmt::date(l.date))
                                        .font(theme::regular(12.5))
                                        .color(p.muted),
                                );
                                if widgets::link(
                                    ui,
                                    RichText::new(format!("#{} {}", l.entry, l.description))
                                        .font(theme::regular(12.5)),
                                )
                                .clicked()
                                {
                                    *go = Some(Route::Entry(l.entry));
                                }
                                ui.label(
                                    RichText::new(kind_label(l.kind))
                                        .font(theme::regular(12.0))
                                        .color(p.faint),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    widgets::amount_cell(
                                        ui,
                                        &fmt::accounting(l.amount),
                                        130.0,
                                        p.text,
                                        false,
                                        false,
                                    );
                                });
                            });
                        }
                    },
                );
            }
        },
    );
    if open_account {
        *go = Some(Route::Account(st.capital));
    }
}
