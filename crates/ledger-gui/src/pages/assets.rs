//! Fixed assets: the register (everything the company owns as a fixed
//! asset, reconciled to the books) and one asset completely (its label,
//! its schedule month by month, the entries and documents behind it).
//! Every number comes from `ledger-core` (`fixed_asset_register`,
//! `depreciation_schedule`, `Asset::tax_basis`, `Asset::built_in_gain`);
//! what is posted comes from the journal. Book value and federal tax
//! basis sit side by side and are never confused: book cost is what was
//! paid or, for a member's contribution, its fair market value; tax basis
//! follows its rule (docs/spec/rules.md B-25).

use super::page;
use crate::app::Shell;
use crate::books::Books;
use crate::fmt;
use crate::nav::{ReportKind, Route};
use crate::theme::{self, pal};
use crate::widgets;
use chrono::NaiveDate;
use egui::{Color32, CornerRadius, Margin, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as icon;
use ledger_core::{Asset, AssetId, AssetRegister, AssetRegisterRow, AssetStatus, TaxBasisRule};
use rust_decimal::Decimal;

/// The register's date (None: today), kept across navigation.
#[derive(Default)]
pub struct State {
    pub as_of: Option<NaiveDate>,
}

/// Something the register says about the books.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    /// A problem (warning colors) or just so (muted).
    pub problem: bool,
    pub text: String,
}

/// What the register says about the books on its date: depreciation not
/// yet posted, cost the register doesn't account for, or that they agree.
///
/// ```text
/// GOAL: the register and the general ledger agree, and when they don't,
///       say how and who fixes it.
/// SOURCE: ledger_core::fixed_asset_register (unposted_depreciation,
///         unregistered_cost); Books::depreciation_due (a month's
///         depreciation is due when the month ends).
/// INVARIANTS:
///   1. unposted_depreciation() > 0: a problem naming the amount due
///      through the last finished month (when any) and asking the
///      bookkeeper to run depreciation; the rest of it (the month under
///      way) a muted note that it posts when the month ends.
///   2. unposted_depreciation() < 0, unregistered_cost() != 0: a problem
///      each, with the exact amount.
///   3. Both zero: one note that the register agrees with the books.
///   4. missing_tax_basis > 0: a problem, last, from
///      [`missing_tax_notice`].
/// ```
pub fn notices(b: &Books, r: &AssetRegister) -> Vec<Notice> {
    let mut out = Vec::new();
    let unposted = r.unposted_depreciation();
    if unposted > Decimal::ZERO {
        let due = match b.depreciation_due(r.as_of) {
            Ok(due) => due,
            Err(e) => {
                out.push(Notice {
                    problem: true,
                    text: format!("Depreciation due can't be computed: {e}."),
                });
                return out;
            }
        };
        let due_amount = due.amount.clamp(Decimal::ZERO, unposted);
        if due_amount > Decimal::ZERO {
            out.push(Notice {
                problem: true,
                text: format!(
                    "Depreciation not yet posted through {}: {} — ask the bookkeeper to run depreciation{}.",
                    fmt::date(due.through),
                    fmt::dollars(due_amount),
                    if due.periods.is_empty() {
                        String::new()
                    } else {
                        format!(" for {}", due.periods.join(", "))
                    }
                ),
            });
        }
        let accruing = unposted - due_amount;
        if accruing > Decimal::ZERO {
            out.push(Notice {
                problem: false,
                text: format!(
                    "{} more accrues in {} through {}; it's posted when the month ends.",
                    fmt::dollars(accruing),
                    r.as_of.format("%B"),
                    fmt::date(r.as_of)
                ),
            });
        }
    } else if unposted < Decimal::ZERO {
        out.push(Notice {
            problem: true,
            text: format!(
                "The books carry {} more accumulated depreciation than the schedules through {} — ask the bookkeeper to check the depreciation entries.",
                fmt::dollars(-unposted),
                fmt::date(r.as_of)
            ),
        });
    }
    let unregistered = r.unregistered_cost();
    if unregistered > Decimal::ZERO {
        out.push(Notice {
            problem: true,
            text: format!(
                "{} in the asset cost accounts isn't in the register — ask the bookkeeper to register it with books.register_asset.",
                fmt::dollars(unregistered)
            ),
        });
    } else if unregistered < Decimal::ZERO {
        out.push(Notice {
            problem: true,
            text: format!(
                "{} of registered cost isn't in the books' asset cost accounts — ask the bookkeeper to check the purchase entries.",
                fmt::dollars(-unregistered)
            ),
        });
    }
    if unposted.is_zero() && unregistered.is_zero() {
        out.push(Notice {
            problem: false,
            text: format!(
                "{} The register agrees with the books: cost and accumulated depreciation match the general ledger to the cent.",
                icon::CHECK
            ),
        });
    }
    if let Some(text) = missing_tax_notice(r.missing_tax_basis) {
        out.push(Notice {
            problem: true,
            text,
        });
    }
    out
}

/// "Tax basis not recorded for 2 assets — ask the bookkeeper to record it
/// (books.set_asset_tax_basis).", or `None` when every asset held has one.
pub fn missing_tax_notice(missing: usize) -> Option<String> {
    (missing > 0).then(|| {
        format!(
            "Tax basis not recorded for {} — ask the bookkeeper to record {} (books.set_asset_tax_basis): bought by the company, contributed by a member from business use, or converted from personal use.",
            fmt::count(missing, "asset", "assets"),
            if missing == 1 { "it" } else { "each" }
        )
    })
}

/// A tax basis rule for a sentence: "cost", "carryover", "personal-use
/// conversion".
pub fn rule_words(r: TaxBasisRule) -> &'static str {
    match r {
        TaxBasisRule::Cost => "cost",
        TaxBasisRule::Carryover => "carryover",
        TaxBasisRule::PersonalUseConversion => "personal-use conversion",
    }
}

/// What a rule means, in plain words, with the law it comes from.
pub fn rule_explained(r: TaxBasisRule) -> &'static str {
    match r {
        TaxBasisRule::Cost => {
            "Bought by the company: its tax basis is what it paid (IRC §1012(a))."
        }
        TaxBasisRule::Carryover => {
            "Contributed by a member from business use: it keeps the member's adjusted basis — what they paid less any depreciation they took (IRC §723)."
        }
        TaxBasisRule::PersonalUseConversion => {
            "Contributed by a member from personal use: it depreciates from the lesser of its fair market value and the member's adjusted basis (Treas. Reg. §1.168(i)-4(b))."
        }
    }
}

/// Book value against tax basis, in one sentence, for an asset under
/// `rule` (or none recorded).
pub fn book_vs_tax(rule: Option<TaxBasisRule>) -> &'static str {
    match rule {
        Some(TaxBasisRule::Cost) => {
            "Book cost and tax basis are both what the company paid; tax depreciation starts from the tax basis — the CPA's."
        }
        _ => {
            "Book value is the fair market value at contribution; tax depreciation starts from the tax basis — the CPA's."
        }
    }
}

/// An asset's federal tax basis as the viewer shows it: every figure is
/// ledger-core's (`Asset::tax_basis`, `Asset::built_in_gain`), never
/// re-derived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaxFacts {
    /// The rule, if recorded.
    pub rule: Option<TaxBasisRule>,
    /// The contributing member's adjusted basis (a contribution only).
    pub contributor_basis: Option<Decimal>,
    /// The depreciable tax basis, if recorded.
    pub tax_basis: Option<Decimal>,
    /// Built-in gain (+) or loss (−) at contribution, if any.
    pub built_in_gain: Option<Decimal>,
}

impl TaxFacts {
    /// An asset's.
    pub fn of(a: &Asset) -> Self {
        TaxFacts {
            rule: a.tax.map(|t| t.rule),
            contributor_basis: a.tax.and_then(|t| t.contributor_basis),
            tax_basis: a.tax_basis(),
            built_in_gain: a.built_in_gain(),
        }
    }

    /// A register row's.
    pub fn of_row(x: &AssetRegisterRow) -> Self {
        TaxFacts {
            rule: x.tax_rule,
            contributor_basis: x.contributor_basis,
            tax_basis: x.tax_basis,
            built_in_gain: x.built_in_gain,
        }
    }

    /// "Carryover · contributor's basis $900.00", or "Tax basis not
    /// recorded": the line under an asset in the register.
    pub fn summary(&self) -> String {
        match self.rule {
            None => "Tax basis not recorded".into(),
            Some(r) => {
                let mut s = ledger_import::asset_register::rule_label(r).to_string();
                if let Some(b) = self.contributor_basis {
                    s.push_str(&format!(" · contributor's basis {}", fmt::dollars(b)));
                }
                s
            }
        }
    }

    /// Everything, with the rule's citation, for a tooltip: "Tax basis
    /// $900.00: Carryover (IRC §723); contributor's basis $1,000.00;
    /// built-in gain $100.00." or "Tax basis not recorded."
    pub fn detail(&self) -> String {
        let (Some(r), Some(t)) = (self.rule, self.tax_basis) else {
            return "Tax basis not recorded.".into();
        };
        let mut s = format!(
            "Tax basis {}: {}",
            fmt::dollars(t),
            ledger_import::asset_register::rule_cited(r)
        );
        if let Some(b) = self.contributor_basis {
            s.push_str(&format!("; contributor's basis {}", fmt::dollars(b)));
        }
        if let Some(g) = self.built_in_gain {
            let (label, amount) = gain_or_loss(g);
            s.push_str(&format!(
                "; {} {}",
                label.to_lowercase(),
                fmt::dollars(amount)
            ));
        }
        s.push('.');
        s
    }
}

/// "Built-in gain" or "Built-in loss", and the amount as a positive
/// figure, for a built-in gain (+) or loss (−).
pub fn gain_or_loss(g: Decimal) -> (&'static str, Decimal) {
    if g < Decimal::ZERO {
        ("Built-in loss", -g)
    } else {
        ("Built-in gain", g)
    }
}

/// The status column: "Active", or "Disposed Mar 3, 2026 for $400.00".
pub fn status_text(disposed: Option<(NaiveDate, Decimal)>) -> String {
    match disposed {
        None => "Active".into(),
        Some((d, proceeds)) => format!("Disposed {} for {}", fmt::date(d), fmt::dollars(proceeds)),
    }
}

pub(crate) fn notice_box(ui: &mut Ui, n: &Notice) {
    let p = pal(ui.ctx());
    let (fg, bg) = if n.problem {
        (p.warning, p.warning_soft)
    } else {
        (p.muted, p.hover)
    };
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                if n.problem {
                    ui.label(RichText::new(icon::WARNING).color(fg));
                }
                ui.add(
                    egui::Label::new(RichText::new(&n.text).font(theme::regular(13.0)).color(fg))
                        .wrap(),
                );
            });
        });
    ui.add_space(6.0);
}

/// One painted table row: `cells` are (text, width, right-aligned,
/// color); the cell whose width is 0 takes what's left. A second line
/// (`sub`) goes under the flexible cell.
pub(crate) fn row(
    ui: &mut Ui,
    w: f32,
    cells: &[(String, f32, bool, Color32)],
    sub: Option<(&str, Color32)>,
    strong: bool,
    click: bool,
) -> egui::Response {
    let p = pal(ui.ctx());
    let h = if sub.is_some() { 40.0 } else { 28.0 };
    let (r, resp) = ui.allocate_exact_size(
        vec2(w, h),
        if click {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if click && resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), p.hover);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let fixed: f32 = cells.iter().map(|c| c.1).sum();
    let flex = (w - fixed).max(40.0);
    let painter = ui.painter_at(r);
    let font = if strong {
        theme::semibold(13.0)
    } else {
        theme::regular(13.0)
    };
    let y = if sub.is_some() {
        r.center().y - 7.0
    } else {
        r.center().y
    };
    let flex_at = cells.iter().position(|c| c.1 == 0.0);
    let mut x = r.left();
    for (i, (text, cw, right, color)) in cells.iter().enumerate() {
        let cw = if *cw == 0.0 { flex } else { *cw };
        if *right {
            painter.text(
                egui::pos2(x + cw - 6.0, y),
                egui::Align2::RIGHT_CENTER,
                text,
                font.clone(),
                *color,
            );
        } else {
            widgets::clipped_text(
                &painter,
                egui::pos2(x + 6.0, y),
                text,
                font.clone(),
                *color,
                cw - 12.0,
            );
        }
        if flex_at == Some(i)
            && let Some((s, c)) = sub
        {
            // The second line may run under the next cell when that one
            // is text (its own line is above).
            let room = cw
                + cells
                    .get(i + 1)
                    .filter(|n| !n.2 && n.1 > 0.0)
                    .map_or(0.0, |n| n.1);
            widgets::clipped_text(
                &painter,
                egui::pos2(x + 6.0, r.center().y + 9.0),
                s,
                theme::regular(11.5),
                c,
                room - 12.0,
            );
        }
        x += cw;
    }
    resp
}

fn date_controls(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) -> NaiveDate {
    let p = pal(ui.ctx());
    let as_of = shell.state.assets.as_of.unwrap_or(b.today);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!("{} As of", icon::CALENDAR_BLANK)).font(theme::semibold(13.5)),
        );
        let picked = widgets::date_picker(ui, "assets_as_of", as_of);
        if picked != as_of {
            shell.state.assets.as_of = Some(picked);
        }
        if shell.state.assets.as_of.is_some() && ui.button("Today").clicked() {
            shell.state.assets.as_of = None;
        }
        if ui
            .button(format!("{} As a report", icon::FILE_TEXT))
            .on_hover_text(
                "The same register on the Reports page, to save as a PDF or copy for a spreadsheet",
            )
            .clicked()
        {
            shell.state.reports.range = crate::pages::reports::Range::Custom(
                b.kpis.first_entry.unwrap_or(as_of).min(as_of),
                as_of,
            );
            shell.go(Route::Reports(ReportKind::FixedAssetRegister));
        }
        ui.label(
            RichText::new(format!("{} basis", b.settings.company.basis))
                .font(theme::regular(12.5))
                .color(p.muted),
        );
    });
    shell.state.assets.as_of.unwrap_or(b.today)
}

/// The fixed asset register page.
pub fn show_register(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    page(ui, "assets", |ui| {
        widgets::page_header(
            ui,
            "Fixed assets",
            "Everything the company owns as a fixed asset: its label, what it cost, how much has been depreciated, and what it's worth on the books, reconciled to the general ledger.",
        );
        if b.ledger.registered_assets().next().is_none() {
            widgets::card(ui, |ui| {
                widgets::empty(
                    ui,
                    icon::PACKAGE,
                    "No fixed assets",
                    "Equipment the company capitalizes appears here once the bookkeeper registers it (books.register_asset).",
                )
            });
            return;
        }
        let as_of = date_controls(ui, b, shell);
        ui.add_space(12.0);
        let r = match b.asset_register(as_of) {
            Ok(r) => r,
            Err(e) => {
                widgets::card(ui, |ui| {
                    widgets::empty(
                        ui,
                        icon::PACKAGE,
                        "The register can't be computed",
                        &format!("{e}. Ask the bookkeeper to find the record behind it."),
                    )
                });
                return;
            }
        };
        super::grid(ui, 4, 200.0, 4, |i, ui| {
            let (label, value, how, sub) = match i {
                0 => (
                    "Cost",
                    r.total_cost,
                    "What the assets still held cost (for a member's contribution, its fair market value then), together.",
                    None,
                ),
                1 => (
                    "Accumulated depreciation",
                    r.total_accumulated,
                    "Their book depreciation by schedule through the date.",
                    None,
                ),
                2 => (
                    "Book value",
                    r.total_book_value,
                    "Cost less accumulated depreciation.",
                    None,
                ),
                _ => (
                    "Tax basis",
                    r.total_tax_basis,
                    "The federal tax basis of the assets held that have one recorded: what tax depreciation (the CPA's) starts from.",
                    (r.missing_tax_basis > 0).then(|| {
                        (
                            format!(
                                "{} not recorded",
                                fmt::count(r.missing_tax_basis, "asset", "assets")
                            ),
                            p.warning,
                        )
                    }),
                ),
            };
            widgets::kpi(
                ui,
                widgets::Kpi {
                    label,
                    value: fmt::dollars(value),
                    sub,
                    spark: None,
                    how,
                },
            );
        });
        for n in notices(b, &r) {
            notice_box(ui, &n);
        }
        ui.add_space(8.0);
        let mut go = None;
        widgets::card(ui, |ui| {
            if r.rows.is_empty() {
                widgets::empty(
                    ui,
                    icon::PACKAGE,
                    "Nothing in service yet",
                    &format!("No asset was in service by {}.", fmt::date(as_of)),
                );
                return;
            }
            let amt = widgets::AMOUNT_MIN + 24.0;
            let fixed = 110.0 + 104.0 + amt * 5.0 + 190.0;
            let w = ui.available_width().max(fixed + 200.0);
            widgets::min_width(ui, "asset_register", w, |ui| {
                let head = |t: &str, cw: f32, right: bool| (t.to_string(), cw, right, p.muted);
                row(
                    ui,
                    w,
                    &[
                        head("Tag", 110.0, false),
                        head("Description", 0.0, false),
                        head("In service", 104.0, false),
                        head("Cost", amt, true),
                        head("Accumulated", amt, true),
                        head("Book value", amt, true),
                        head("Tax basis", amt, true),
                        head("Built-in gain", amt, true),
                        head("Status", 190.0, false),
                    ],
                    None,
                    true,
                    false,
                );
                widgets::rule(ui);
                for x in &r.rows {
                    let disposed = x.disposed.is_some();
                    let text = if disposed { p.muted } else { p.text };
                    let money = |d: Decimal| {
                        if disposed {
                            String::new()
                        } else {
                            fmt::money(d)
                        }
                    };
                    let tax = TaxFacts::of_row(x);
                    let sub = match &x.serial {
                        Some(s) => format!("Serial {s} · {}", tax.summary()),
                        None => tax.summary(),
                    };
                    let (tax_text, tax_color) = match tax.tax_basis {
                        _ if disposed => (String::new(), text),
                        Some(t) => (fmt::money(t), text),
                        None => ("not recorded".to_string(), p.warning),
                    };
                    let resp = row(
                        ui,
                        w,
                        &[
                            (
                                x.tag.clone().unwrap_or_else(|| "no tag".into()),
                                110.0,
                                false,
                                if x.tag.is_some() { text } else { p.warning },
                            ),
                            (x.description.clone(), 0.0, false, text),
                            (fmt::date(x.in_service), 104.0, false, text),
                            (money(x.cost), amt, true, text),
                            (money(x.accumulated), amt, true, text),
                            (money(x.book_value), amt, true, text),
                            (tax_text, amt, true, tax_color),
                            (
                                x.built_in_gain.map(money).unwrap_or_default(),
                                amt,
                                true,
                                text,
                            ),
                            (status_text(x.disposed), 190.0, false, p.muted),
                        ],
                        Some((sub.as_str(), p.faint)),
                        false,
                        true,
                    );
                    let resp = resp.on_hover_text(if disposed {
                        format!(
                            "Open this asset (it cost {}; it left the books when disposed). {}",
                            fmt::dollars(x.cost),
                            tax.detail()
                        )
                    } else {
                        format!("Open this asset. {}", tax.detail())
                    });
                    resp.context_menu(|ui| {
                        if let Some(t) = &x.tag {
                            widgets::copy_item(ui, "Copy tag", t);
                        }
                        if let Some(s) = &x.serial {
                            widgets::copy_item(ui, "Copy serial number", s);
                        }
                        widgets::copy_item(ui, "Copy book value", &x.book_value.to_string());
                        if let Some(t) = x.tax_basis {
                            widgets::copy_item(ui, "Copy tax basis", &t.to_string());
                        }
                    });
                    if resp.clicked() {
                        go = Some(x.asset_id);
                    }
                }
                widgets::rule(ui);
                row(
                    ui,
                    w,
                    &[
                        ("Total held".into(), 110.0, false, p.text),
                        (String::new(), 0.0, false, p.text),
                        (String::new(), 104.0, false, p.text),
                        (fmt::money(r.total_cost), amt, true, p.text),
                        (fmt::money(r.total_accumulated), amt, true, p.text),
                        (fmt::money(r.total_book_value), amt, true, p.text),
                        (fmt::money(r.total_tax_basis), amt, true, p.text),
                        (String::new(), amt, true, p.text),
                        (String::new(), 190.0, false, p.text),
                    ],
                    None,
                    true,
                    false,
                );
            });
        });
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Cost is what the company paid or, for a member's contribution, its fair market value then; the tax basis follows its rule (under each asset) and can differ. Tax depreciation starts from the tax basis — the CPA's.",
                )
                .font(theme::regular(11.5))
                .color(p.faint),
            )
            .wrap(),
        );
        ui.add_space(14.0);
        reconciliation(ui, b, &r, shell);
        if let Some(id) = go {
            shell.go(Route::Asset(id));
        }
    });
}

/// Register against the general ledger, line by line.
fn reconciliation(ui: &mut Ui, b: &Books, r: &AssetRegister, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let held: Vec<_> = r
        .rows
        .iter()
        .filter_map(|x| b.ledger.get_asset(x.asset_id))
        .collect();
    let mut cost_accts: Vec<u64> = held.iter().map(|a| a.cost_account).collect();
    cost_accts.sort_unstable();
    cost_accts.dedup();
    let mut accum_accts: Vec<u64> = held.iter().map(|a| a.accum_depr_account).collect();
    accum_accts.sort_unstable();
    accum_accts.dedup();
    let mut go = None;
    widgets::titled_card(
        ui,
        "Reconciliation to the books",
        |_| {},
        |ui| {
            let amt = widgets::AMOUNT_MIN + 40.0;
            let w = ui.available_width().max(260.0 + amt);
            widgets::min_width(ui, "asset_recon", w, |ui| {
                let line = |ui: &mut Ui, label: &str, v: Decimal, strong: bool, color| {
                    let resp = row(
                        ui,
                        w,
                        &[
                            (label.to_string(), 0.0, false, p.text),
                            (fmt::money(v), amt, true, color),
                        ],
                        None,
                        strong,
                        false,
                    );
                    resp.context_menu(|ui| widgets::copy_item(ui, "Copy amount", &v.to_string()));
                };
                let diff_color = |d: Decimal| if d.is_zero() { p.positive } else { p.warning };
                line(ui, "Cost in the register", r.total_cost, false, p.text);
                line(ui, "Cost in the books", r.ledger_cost, false, p.text);
                let u = r.unregistered_cost();
                line(
                    ui,
                    "Difference (not in the register)",
                    u,
                    true,
                    diff_color(u),
                );
                ui.add_space(8.0);
                line(
                    ui,
                    "Accumulated depreciation by schedule",
                    r.total_accumulated,
                    false,
                    p.text,
                );
                line(
                    ui,
                    "Accumulated depreciation in the books",
                    r.ledger_accumulated,
                    false,
                    p.text,
                );
                let d = r.unposted_depreciation();
                // Only the month under way: not a problem, just not yet.
                let accruing = d > Decimal::ZERO
                    && b.depreciation_due(r.as_of)
                        .is_ok_and(|due| due.amount <= Decimal::ZERO);
                let color = if accruing { p.muted } else { diff_color(d) };
                line(ui, "Difference (not yet posted)", d, true, color);
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("Accounts:")
                        .font(theme::regular(12.0))
                        .color(p.muted),
                );
                for id in cost_accts.iter().chain(&accum_accts) {
                    if widgets::link(
                        ui,
                        RichText::new(b.account_label(*id))
                            .font(theme::regular(12.0))
                            .color(p.accent),
                    )
                    .clicked()
                    {
                        go = Some(*id);
                    }
                }
            });
        },
    );
    if let Some(id) = go {
        shell.state.accounts.range = crate::pages::accounts::RegisterRange::Default;
        shell.go(Route::Account(id));
    }
}

/// How one month of an asset's schedule stands in the books.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonthStatus {
    /// Posted, by this entry.
    Posted(ledger_core::EntryId),
    /// The month has ended and nothing is posted for it.
    Due,
    /// The month under way.
    ThisMonth,
    /// A month still to come.
    Scheduled,
}

/// Each month of the asset's full schedule (to the end of its life, or
/// its disposal) with how it stands on `today`.
///
/// ```text
/// INVARIANTS:
///   1. The lines are ledger_core::depreciation_schedule(asset, the end of
///      time), unchanged and in order.
///   2. A month with depreciation posted (asset.depreciation_posted) is
///      Posted(its entry); otherwise Due when it ended before today,
///      ThisMonth when today is in it, Scheduled after.
/// ```
pub fn schedule(b: &Books, id: AssetId) -> Vec<(ledger_core::ScheduleLine, MonthStatus)> {
    let Some(a) = b.ledger.get_asset(id) else {
        return Vec::new();
    };
    let trail = b.asset_trail(id);
    let this_month = b.today.format("%Y-%m").to_string();
    ledger_core::depreciation_schedule(a, NaiveDate::MAX)
        .into_iter()
        .map(|l| {
            let st = match trail.depreciation.get(&l.period) {
                Some((e, _)) => MonthStatus::Posted(*e),
                None if l.period < this_month => MonthStatus::Due,
                None if l.period == this_month => MonthStatus::ThisMonth,
                None => MonthStatus::Scheduled,
            };
            (l, st)
        })
        .collect()
}

/// One asset, completely.
pub fn show_asset(ui: &mut Ui, b: &Books, id: AssetId, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let Some(a) = b.ledger.get_asset(id) else {
        page(ui, "asset", |ui| {
            widgets::empty(
                ui,
                icon::PACKAGE,
                "No such asset",
                "It isn't in these books.",
            );
        });
        return;
    };
    let trail = b.asset_trail(id);
    // Err: the register's totals can't be computed (a contributor basis an
    // earlier version journaled, past exact arithmetic); said, never guessed.
    let today = b
        .asset_register(b.today)
        .map(|r| r.rows.into_iter().find(|r| r.asset_id == id));
    let mut go: Option<Route> = None;
    page(ui, "asset", |ui| {
        ui.horizontal(|ui| {
            if widgets::link(
                ui,
                RichText::new(format!("{} Fixed assets", icon::ARROW_LEFT))
                    .font(theme::regular(12.5))
                    .color(p.muted),
            )
            .clicked()
            {
                go = Some(Route::Assets);
            }
            widgets::copyable(
                ui,
                RichText::new(format!("· Asset {id}"))
                    .font(theme::regular(12.5))
                    .color(p.muted),
                &id.to_string(),
            );
        });
        ui.add_space(4.0);
        match &a.tag {
            Some(tag) => {
                widgets::copyable(
                    ui,
                    RichText::new(tag).font(theme::bold(30.0)).monospace(),
                    tag,
                );
            }
            None => {
                widgets::pill(ui, "No tag yet", p.warning, p.warning_soft)
                    .on_hover_text("Ask the bookkeeper to label it (books.label_asset).");
            }
        }
        ui.label(RichText::new(&a.description).font(theme::bold(22.0)));
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            match &a.status {
                AssetStatus::Active => widgets::pill(ui, "Active", p.positive, p.positive_soft),
                AssetStatus::Disposed { date, proceeds } => widgets::pill(
                    ui,
                    &status_text(Some((*date, proceeds.amount))),
                    p.muted,
                    p.hover,
                ),
            };
            widgets::pill(ui, "Straight line", p.muted, p.hover);
        });
        ui.add_space(14.0);

        super::columns(ui, 820.0, &[1.0, 1.0], |col, ui| {
            if col == 0 {
                widgets::titled_card(
                    ui,
                    "Identity",
                    |_| {},
                    |ui| {
                        facts(ui, |ui, k| {
                            k(ui, "Tag");
                            match &a.tag {
                                Some(t) => {
                                    widgets::copyable(ui, RichText::new(t).monospace(), t);
                                }
                                None => {
                                    ui.label(RichText::new("none").color(p.warning));
                                }
                            }
                            ui.end_row();
                            k(ui, "Serial number");
                            match &a.serial {
                                Some(s) => {
                                    widgets::copyable(ui, RichText::new(s).monospace(), s);
                                }
                                None => {
                                    ui.label(RichText::new("not recorded").color(p.muted));
                                }
                            }
                            ui.end_row();
                            k(ui, "Asset id");
                            widgets::copyable(ui, RichText::new(id.to_string()), &id.to_string());
                            ui.end_row();
                            for (label, acct) in [
                                ("Cost account", a.cost_account),
                                ("Accumulated depreciation", a.accum_depr_account),
                                ("Depreciation expense", a.expense_account),
                            ] {
                                k(ui, label);
                                if widgets::link_truncated(
                                    ui,
                                    RichText::new(b.account_label(acct)).color(p.accent),
                                )
                                .clicked()
                                {
                                    shell.state.accounts.range =
                                        crate::pages::accounts::RegisterRange::Default;
                                    go = Some(Route::Account(acct));
                                }
                                ui.end_row();
                            }
                        });
                    },
                );
            } else {
                widgets::titled_card(
                    ui,
                    "Cost and depreciation",
                    |_| {},
                    |ui| {
                        let end = a
                            .purchase_date
                            .checked_add_months(chrono::Months::new(a.useful_life_months))
                            .and_then(|d| d.pred_opt());
                        facts(ui, |ui, k| {
                            let money = |ui: &mut Ui, d: Decimal, strong: bool| {
                                let t = fmt::dollars(d);
                                let rt = if strong {
                                    RichText::new(&t).font(theme::semibold(14.0))
                                } else {
                                    RichText::new(&t)
                                };
                                widgets::copyable(ui, rt, &d.to_string());
                            };
                            k(ui, "In service");
                            ui.label(fmt::date(a.purchase_date));
                            ui.end_row();
                            k(ui, "Cost");
                            money(ui, a.cost.amount, false);
                            ui.end_row();
                            k(ui, "Salvage value");
                            money(ui, a.salvage_value.amount, false);
                            ui.end_row();
                            k(ui, "Useful life");
                            ui.label(match end {
                                Some(e) => format!(
                                    "{} · through {}",
                                    fmt::count(a.useful_life_months as usize, "month", "months"),
                                    fmt::date(e)
                                ),
                                None => {
                                    fmt::count(a.useful_life_months as usize, "month", "months")
                                }
                            });
                            ui.end_row();
                            match &today {
                                Ok(Some(r)) => {
                                    k(ui, &format!("Accumulated, {}", fmt::date_short(b.today)));
                                    money(ui, r.accumulated, false);
                                    ui.end_row();
                                    k(ui, "Book value today");
                                    money(ui, r.book_value, true);
                                    ui.end_row();
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    k(ui, "Book value today");
                                    ui.label(format!("can't be computed: {e}"));
                                    ui.end_row();
                                }
                            }
                            k(ui, "Posted in the books");
                            money(ui, trail.posted(), false);
                            ui.end_row();
                            if let AssetStatus::Disposed { date, proceeds } = &a.status {
                                k(ui, "Disposed");
                                ui.label(fmt::date(*date));
                                ui.end_row();
                                k(ui, "Proceeds");
                                money(ui, proceeds.amount, false);
                                ui.end_row();
                            }
                        });
                    },
                );
            }
        });

        tax_card(ui, a);
        ui.add_space(14.0);

        // The schedule.
        let lines = schedule(b, id);
        let due: Decimal = lines
            .iter()
            .filter(|(_, s)| *s == MonthStatus::Due)
            .map(|(l, _)| l.amount)
            .sum();
        if due > Decimal::ZERO {
            notice_box(
                ui,
                &Notice {
                    problem: true,
                    text: format!(
                        "{} of depreciation for finished months isn't posted — ask the bookkeeper to run depreciation.",
                        fmt::dollars(due)
                    ),
                },
            );
        }
        widgets::titled_card(
            ui,
            "Depreciation schedule",
            |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} · {} posted",
                        fmt::count(lines.len(), "month", "months"),
                        lines
                            .iter()
                            .filter(|(_, s)| matches!(s, MonthStatus::Posted(_)))
                            .count()
                    ))
                    .font(theme::regular(12.0))
                    .color(p.muted),
                );
            },
            |ui| {
                if lines.is_empty() {
                    ui.label(
                        RichText::new("Nothing to depreciate: its salvage value is its cost.")
                            .color(p.muted),
                    );
                    return;
                }
                let amt = widgets::AMOUNT_MIN + 16.0;
                let w = ui.available_width().max(100.0 + amt * 3.0 + 200.0);
                widgets::min_width(ui, "asset_schedule", w, |ui| {
                    let head = |t: &str, cw: f32, right: bool| (t.to_string(), cw, right, p.muted);
                    row(
                        ui,
                        w,
                        &[
                            head("Month", 100.0, false),
                            head("Amount", amt, true),
                            head("Accumulated", amt, true),
                            head("Book value", amt, true),
                            head("", 24.0, false),
                            head("In the books", 0.0, false),
                        ],
                        None,
                        true,
                        false,
                    );
                    widgets::rule(ui);
                    for (l, st) in &lines {
                        let month =
                            NaiveDate::parse_from_str(&format!("{}-01", l.period), "%Y-%m-%d")
                                .map(fmt::month)
                                .unwrap_or_else(|_| l.period.clone());
                        let (status, color) = match st {
                            MonthStatus::Posted(e) => (format!("Posted · entry #{e}"), p.positive),
                            MonthStatus::Due => ("Not posted".to_string(), p.warning),
                            MonthStatus::ThisMonth => ("This month".to_string(), p.muted),
                            MonthStatus::Scheduled => ("Scheduled".to_string(), p.faint),
                        };
                        let text = if matches!(st, MonthStatus::Scheduled) {
                            p.muted
                        } else {
                            p.text
                        };
                        let resp = row(
                            ui,
                            w,
                            &[
                                (month, 100.0, false, text),
                                (fmt::money(l.amount), amt, true, text),
                                (fmt::money(l.accumulated), amt, true, text),
                                (fmt::money(l.book_value), amt, true, text),
                                (String::new(), 24.0, false, text),
                                (status, 0.0, false, color),
                            ],
                            None,
                            false,
                            matches!(st, MonthStatus::Posted(_)),
                        );
                        resp.context_menu(|ui| {
                            widgets::copy_item(ui, "Copy amount", &l.amount.to_string());
                            widgets::copy_item(ui, "Copy book value", &l.book_value.to_string());
                        });
                        if let MonthStatus::Posted(e) = st
                            && resp.on_hover_text("Open the depreciation entry").clicked()
                        {
                            go = Some(Route::Entry(*e));
                        }
                    }
                });
            },
        );
        ui.add_space(14.0);

        // Entries and documents.
        let entries = trail.entries();
        widgets::titled_card(
            ui,
            "Entries",
            |_| {},
            |ui| {
                if entries.is_empty() {
                    ui.label(
                        RichText::new("No entry in the books is about it yet.").color(p.muted),
                    );
                }
                for e in &entries {
                    let Some(info) = b.entry(*e) else {
                        continue;
                    };
                    let role = if Some(*e) == trail.purchase_entry {
                        "Purchase"
                    } else if Some(*e) == trail.disposal_entry {
                        "Disposal"
                    } else {
                        "Depreciation"
                    };
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            vec2(96.0, 18.0),
                            egui::Label::new(
                                RichText::new(role)
                                    .font(theme::regular(12.5))
                                    .color(p.muted),
                            ),
                        );
                        if widgets::link_truncated(
                            ui,
                            RichText::new(format!(
                                "{} · #{} {} · {}",
                                fmt::date(info.date),
                                info.id,
                                info.description,
                                fmt::dollars_or(info.amount)
                            ))
                            .font(theme::regular(13.0)),
                        )
                        .clicked()
                        {
                            go = Some(Route::Entry(*e));
                        }
                    });
                }
                if trail.purchase_entry.is_some() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(if a.acquired_by.is_some() {
                            "The purchase is the entry linked to this asset."
                        } else {
                            "The purchase is matched by its cost account, amount, and date."
                        })
                        .font(theme::regular(11.5))
                        .color(p.faint),
                    );
                }
            },
        );
        ui.add_space(14.0);
        let mut subjects = vec![ledger_store::Subject::Asset(id)];
        subjects.extend(entries.iter().map(|e| ledger_store::Subject::Entry(*e)));
        super::documents::documents_card_with(
            ui,
            b,
            "Receipts and documents",
            &subjects,
            &[ledger_store::Subject::Asset(id)],
            "None linked to it or its entries (photos, the warranty, the manual; depreciation needs none: the schedule is its evidence).",
            &mut go,
        );
        ui.add_space(14.0);

        // Its own events.
        widgets::titled_card(
            ui,
            "History",
            |_| {},
            |ui| {
                for &i in &trail.events {
                    let line = &b.journal[i];
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
    if let Some(r) = go {
        shell.go(r);
    }
}

/// The asset's federal tax basis beside its book figures.
fn tax_card(ui: &mut Ui, a: &Asset) {
    let p = pal(ui.ctx());
    let t = TaxFacts::of(a);
    widgets::titled_card(
        ui,
        "Tax basis",
        |ui| {
            ui.label(
                RichText::new("Federal")
                    .font(theme::regular(12.0))
                    .color(p.muted),
            );
        },
        |ui| {
            let Some(rule) = t.rule else {
                ui.label(
                    RichText::new("Not recorded")
                        .color(p.warning)
                        .font(theme::semibold(14.0)),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "Ask the bookkeeper to record it (books.set_asset_tax_basis): was it bought by the company, contributed by a member from business use, or converted from personal use? Book value is the fair market value at contribution; tax depreciation starts from the tax basis — the CPA's.",
                        )
                        .font(theme::regular(12.5))
                        .color(p.muted),
                    )
                    .wrap(),
                );
                return;
            };
            facts(ui, |ui, k| {
                let money = |ui: &mut Ui, d: Decimal, strong: bool| {
                    let s = fmt::dollars(d);
                    let rt = if strong {
                        RichText::new(&s).font(theme::semibold(14.0))
                    } else {
                        RichText::new(&s)
                    };
                    widgets::copyable(ui, rt, &d.to_string());
                };
                k(ui, "Rule");
                ui.label(ledger_import::asset_register::rule_label(rule));
                ui.end_row();
                k(ui, "Book cost");
                money(ui, a.cost.amount, false);
                ui.end_row();
                if let Some(c) = t.contributor_basis {
                    k(ui, "Contributor's basis");
                    money(ui, c, false);
                    ui.end_row();
                }
                if let Some(tb) = t.tax_basis {
                    k(ui, "Tax basis");
                    money(ui, tb, true);
                    ui.end_row();
                }
                if let Some(g) = t.built_in_gain {
                    let (label, amount) = gain_or_loss(g);
                    k(ui, label);
                    money(ui, amount, false);
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            let mut why = rule_explained(rule).to_string();
            if t.built_in_gain.is_some_and(|g| g < Decimal::ZERO) {
                why.push_str(" A built-in loss is taken into account only for the contributing member (IRC §704(c)(1)(C)).");
            } else if t.built_in_gain.is_some_and(|g| g > Decimal::ZERO) {
                why.push_str(" IRC §704(c) has the partnership take the built-in gain into account for the contributing member.");
            }
            for line in [why.as_str(), book_vs_tax(Some(rule))] {
                ui.add(
                    egui::Label::new(
                        RichText::new(line)
                            .font(theme::regular(12.5))
                            .color(p.muted),
                    )
                    .wrap(),
                );
            }
        },
    );
}

/// A two-column grid of facts; `add` gets the label painter.
pub(crate) fn facts(ui: &mut Ui, add: impl FnOnce(&mut Ui, &dyn Fn(&mut Ui, &str))) {
    let p = pal(ui.ctx());
    egui::Grid::new(ui.next_auto_id())
        .num_columns(2)
        .spacing(vec2(20.0, 6.0))
        .show(ui, |ui| {
            let k = |ui: &mut Ui, s: &str| {
                ui.label(RichText::new(s).font(theme::regular(12.5)).color(p.muted));
            };
            add(ui, &k);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Build, Painted, beside, cents, d, find, render, today};
    use ledger_core::{Currency, Money, NewAsset};

    /// Two assets bought 2026-01-01 for $1,200.00 over 12 months and
    /// $2,400.00 over 24 (both $100.00 a month), no depreciation posted.
    fn two_assets(tag: &str) -> (crate::test_support::TempDir, Books, Vec<AssetId>) {
        let mut b = Build::new(tag);
        let mut add =
            |code: &str, name: &str, ty| b.s.add_account(code, name, ty, None, false).unwrap();
        let equipment = add("1500", "Equipment", ledger_core::AccountType::Asset);
        let accum = add(
            "1510",
            "Accumulated Depreciation",
            ledger_core::AccountType::Asset,
        );
        let depreciation = add("6100", "Depreciation", ledger_core::AccountType::Expense);
        let (cash, capital) = (b.cash, b.capital);
        b.post(d(2026, 1, 1), cash, capital, 1_000_000);
        b.post(d(2026, 1, 1), equipment, cash, 360_000);
        let ids = [(120_000, 12), (240_000, 24)]
            .into_iter()
            .map(|(cost, months)| {
                b.s.register_asset(NewAsset {
                    description: format!("Thing {months}"),
                    purchase_date: d(2026, 1, 1),
                    cost: Money::new(cents(cost), Currency::USD),
                    salvage_value: Money::new(Decimal::ZERO, Currency::USD),
                    useful_life_months: months,
                    cost_account: equipment,
                    accum_depr_account: accum,
                    expense_account: depreciation,
                })
                .unwrap()
            })
            .collect();
        let (dir, books) = b.books();
        (dir, books, ids)
    }

    fn register_on(
        tag: &str,
        as_of: Option<NaiveDate>,
    ) -> (Vec<Painted>, Color32, Color32, Color32) {
        let (_dir, books, _) = two_assets(tag);
        let (texts, p) = render(books, Route::Assets, |s| s.assets.as_of = as_of);
        (texts, p.warning, p.muted, p.positive)
    }

    /// The register on its date, reconciled to the books: its cost, and
    /// the depreciation the schedules have accrued but the books don't
    /// carry: a problem when some is due through a finished month, just
    /// accruing when only the month under way has any, agreeing when none.
    #[test]
    fn the_register_reconciles_to_the_books_on_its_date() {
        let (texts, warning, muted, positive) = register_on("register-today", None);
        find(&texts, "Fixed assets");
        find(&texts, "Reconciliation to the books");
        assert_eq!(
            beside(&texts, "Cost in the register").text,
            fmt::money(cents(360_000))
        );
        // Due through September: a problem.
        assert_eq!(beside(&texts, "Difference (not yet posted)").color, warning);
        // January 15th: nothing due through December, half a month accruing.
        let (texts, _, muted2, _) = register_on("register-jan", Some(d(2026, 1, 15)));
        assert_eq!(muted2, muted);
        assert_eq!(beside(&texts, "Difference (not yet posted)").color, muted);
        // Before the purchase: nothing to post, the books agree.
        let (texts, _, _, _) = register_on("register-before", Some(d(2025, 12, 31)));
        let none = beside(&texts, "Difference (not yet posted)");
        assert_eq!(none.text, fmt::money(Decimal::ZERO));
        assert_eq!(none.color, positive);
    }

    /// One asset's page shows its own book value today, not another's.
    #[test]
    fn an_assets_page_shows_its_own_book_value() {
        for i in 0..2 {
            let (_dir, books, ids) = two_assets(&format!("asset-page-{i}"));
            let a = books.ledger.get_asset(ids[i]).unwrap().clone();
            let want = ledger_core::book_value(&a, today());
            let (texts, _) = render(books, Route::Asset(ids[i]), |_| {});
            assert_eq!(
                beside(&texts, "Book value today").text,
                fmt::dollars(want),
                "asset {i}"
            );
        }
    }
}
