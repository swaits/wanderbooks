//! Parties: everyone the company deals with (vendors, contractors,
//! customers, employees, members, payout platforms, tax agencies, banks,
//! landlords), and one party completely: who they are, their W-9 or W-4
//! facts (never more of a TIN than its last four digits), and everything
//! the company did with them. Every number is [`crate::books::PartyInfo`]'s
//! (built once per refresh from ledger-core) or ledger-core's own 1099
//! worksheet; this module only arranges them.

use super::assets::{Notice, facts, notice_box, row};
use super::page;
use crate::app::Shell;
use crate::books::{Books, PartyFlow, PartyInfo, PartyPeriod};
use crate::fmt;
use crate::nav::{ReportKind, Route};
use crate::theme::{self, pal};
use crate::widgets;
use chrono::{Datelike, NaiveDate};
use egui::{CornerRadius, RichText, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use ledger_core::{Party, PartyId, Role};
use rust_decimal::Decimal;

/// The directory's filters, kept across navigation.
#[derive(Default)]
pub struct State {
    /// Only parties with this role (None: every role).
    pub role: Option<Role>,
    /// Archived parties listed too.
    pub show_archived: bool,
    /// Search text: name or display name.
    pub query: String,
    /// The span the money columns and a party's page sum over.
    pub period: PartyPeriod,
    /// How the directory is ordered.
    pub sort: Sort,
}

/// The directory's order: by name, or by an amount, largest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Name,
    PaidOut,
    ReceivedIn,
    Net,
}

/// The directory's rows for `st` in its order: [`listed`] by name, or by
/// the chosen amount over `st.period` largest first (ties by name), each
/// with its [`PartyFlow`] (totals only).
pub fn sorted<'a>(b: &'a Books, st: &State) -> Vec<(&'a Party, PartyFlow)> {
    let span = b.party_span(st.period);
    let mut rows: Vec<(&Party, PartyFlow)> = listed(b, st)
        .into_iter()
        .map(|p| (p, b.party_flow(p.id, span, false)))
        .collect();
    let key = |f: &PartyFlow| match st.sort {
        Sort::Name => Decimal::ZERO,
        Sort::PaidOut => f.paid,
        Sort::ReceivedIn => f.received,
        Sort::Net => f.net(),
    };
    // Stable: equal amounts keep the by-name order.
    rows.sort_by_key(|x| std::cmp::Reverse(key(&x.1)));
    rows
}

/// The period chips: each of [`Books::party_periods`], the chosen one
/// lit.
fn period_chips(ui: &mut Ui, b: &Books, party: Option<PartyId>, period: &mut PartyPeriod) {
    let p = pal(ui.ctx());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        ui.label(
            RichText::new(format!("{} Period", icon::CALENDAR_BLANK))
                .font(theme::regular(12.5))
                .color(p.muted),
        );
        for x in b.party_periods(party) {
            if chip(ui, &b.party_period_label(x), *period == x) {
                *period = x;
            }
        }
    });
}

/// A role in words, capitalized: "Payout platform".
pub fn role_label(r: Role) -> &'static str {
    match r {
        Role::Vendor => "Vendor",
        Role::Contractor => "Contractor",
        Role::Customer => "Customer",
        Role::Employee => "Employee",
        Role::Member => "Member",
        Role::PayoutPlatform => "Payout platform",
        Role::TaxAgency => "Tax agency",
        Role::FinancialInstitution => "Bank or card issuer",
        Role::Landlord => "Landlord",
    }
}

/// A role with its article, for a sentence: "a contractor", "an employee".
pub fn role_noun(r: Role) -> &'static str {
    match r {
        Role::Vendor => "a vendor",
        Role::Contractor => "a contractor",
        Role::Customer => "a customer",
        Role::Employee => "an employee",
        Role::Member => "a member",
        Role::PayoutPlatform => "a payout platform",
        Role::TaxAgency => "a tax agency",
        Role::FinancialInstitution => "a bank or card issuer",
        Role::Landlord => "a landlord",
    }
}

/// Every role a party has ([`Party::all_roles`]), in display order:
/// "Vendor, contractor"; "No role yet" when none.
pub fn roles_text(p: &Party) -> String {
    let roles: Vec<&str> = p.all_roles().into_iter().map(role_label).collect();
    if roles.is_empty() {
        return "No role yet".into();
    }
    let mut out = roles.join(", ").to_lowercase();
    if let Some(first) = out.get(..1) {
        out = first.to_uppercase() + &out[1..];
    }
    out
}

/// The icon for a party, by its most telling role.
pub fn party_icon(p: &Party) -> &'static str {
    let r = p.all_roles();
    let has = |x| r.contains(&x);
    if has(Role::TaxAgency) {
        icon::BANK
    } else if has(Role::FinancialInstitution) {
        icon::BUILDING_OFFICE
    } else if has(Role::PayoutPlatform) {
        icon::DEVICE_MOBILE
    } else if has(Role::Landlord) {
        icon::HOUSE_LINE
    } else if has(Role::Member) {
        icon::CROWN
    } else if has(Role::Employee) || has(Role::Contractor) {
        icon::USER
    } else if has(Role::Customer) {
        icon::HANDSHAKE
    } else if has(Role::Vendor) {
        icon::STOREFRONT
    } else {
        icon::ADDRESS_BOOK
    }
}

/// The last four of a TIN, masked the way a truncated TIN is printed:
/// "•••-••-1234". The books never hold more than these four digits.
pub fn masked_tin(last4: &str) -> String {
    format!("•••-••-{last4}")
}

/// A party's taxpayer-ID facts as the viewer shows them: "TIN on file
/// ••••1234 (SSN)", never more than the last four; once purged from the
/// vault, "TIN ••••1234 (SSN), purged from the vault".
pub fn tin_on_file(t: &ledger_core::TinFacts) -> String {
    if t.vault_record.is_none() && t.purged.is_some() {
        format!(
            "TIN ••••{} ({}), purged from the vault",
            t.last4,
            t.tin_type.label()
        )
    } else {
        format!("TIN on file ••••{} ({})", t.last4, t.tin_type.label())
    }
}

/// Where a party stands on its W-9.
///
/// ```text
/// SOURCE: ledger_core::parties (W9Facts; a 1099 role carries them);
///         docs/parties-and-attachments.md (a contractor paid without
///         W-9 facts needs attention).
/// INVARIANTS:
///   1. OnFile when the party has W-9 facts (whatever its roles).
///   2. Otherwise NotNeeded when it doesn't file a 1099
///      (Books::files_1099 is false).
///   3. Otherwise MarkedOnFile when its vendor record says a W-9 is on
///      file (books from before the directory), else Missing.
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum W9Status {
    NotNeeded,
    OnFile { last4: String, received: NaiveDate },
    MarkedOnFile,
    Missing,
}

impl W9Status {
    /// For the directory's W-9 column: "On file · •••-••-1234",
    /// "Missing", "Marked on file", or empty.
    pub fn text(&self) -> String {
        match self {
            W9Status::NotNeeded => String::new(),
            W9Status::OnFile { last4, .. } => format!("On file · {}", masked_tin(last4)),
            W9Status::MarkedOnFile => "Marked on file, facts not recorded".into(),
            W9Status::Missing => "Missing".into(),
        }
    }
}

/// A party's [`W9Status`].
pub fn w9_status(b: &Books, p: &Party) -> W9Status {
    if let Some(w) = &p.w9 {
        return W9Status::OnFile {
            last4: w.tin_last4.clone(),
            received: w.received,
        };
    }
    if !b.files_1099(p) {
        return W9Status::NotNeeded;
    }
    let marked = p
        .vendor
        .and_then(|v| b.ledger.get_vendor(v))
        .is_some_and(|v| v.tin_on_file);
    if marked {
        W9Status::MarkedOnFile
    } else {
        W9Status::Missing
    }
}

/// The 1099 role a party files under, in words ("contractor",
/// "landlord"), for prompts.
fn form_role(p: &Party) -> &'static str {
    if p.roles.contains(&Role::Landlord) && !p.roles.contains(&Role::Contractor) {
        "landlord"
    } else {
        "contractor"
    }
}

/// The agent prompt that collects and records a party's W-9.
pub fn collect_w9_prompt(b: &Books, p: &Party) -> String {
    let paid = b.party(p.id).map(|i| i.paid_ytd).unwrap_or_default();
    let paid = if paid > Decimal::ZERO {
        format!(", paid {} so far in {}", fmt::dollars(paid), b.today.year())
    } else {
        String::new()
    };
    format!(
        "In wanderbooks, {} (party {}) is a {}{paid} with no W-9 facts recorded. Remind me to collect a W-9 before the next payment. Once I have it, record what it says with books.set_w9: the date received, the entity type (line 3), any exempt payee or FATCA codes (line 4), the last four digits of the TIN only (the full TIN goes in the vault, never the books), and where the form is kept. Then show me the party.",
        p.label(),
        p.id,
        form_role(p)
    )
}

/// The parties the directory lists for `st`, by label (ignoring case):
/// archived ones only when shown, those with the chosen role (any of
/// [`Party::all_roles`]), and those whose name or display name holds
/// every word of the query (ignoring case).
pub fn listed<'a>(b: &'a Books, st: &State) -> Vec<&'a Party> {
    let q = st.query.trim().to_lowercase();
    let mut out: Vec<&Party> = b
        .ledger
        .parties()
        .filter(|p| st.show_archived || !p.archived)
        .filter(|p| st.role.is_none_or(|r| p.all_roles().contains(&r)))
        .filter(|p| {
            let hay = format!(
                "{} {}",
                p.name.to_lowercase(),
                p.display_name.as_deref().unwrap_or("").to_lowercase()
            );
            q.split_whitespace().all(|w| hay.contains(w))
        })
        .collect();
    out.sort_by(|a, b| {
        a.label()
            .to_lowercase()
            .cmp(&b.label().to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    out
}

/// Each role some party (archived ones when shown) has, with how many.
pub fn role_counts(b: &Books, show_archived: bool) -> Vec<(Role, usize)> {
    Role::ALL
        .into_iter()
        .map(|r| {
            (
                r,
                b.ledger
                    .parties()
                    .filter(|p| (show_archived || !p.archived) && p.all_roles().contains(&r))
                    .count(),
            )
        })
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// "Paid this year" in words and color: paid out plain, received net
/// with a plus (in green), as the transactions list shows cash.
fn paid_cell(paid: Decimal, p: &theme::Palette) -> (String, egui::Color32) {
    if paid < Decimal::ZERO {
        (format!("+{}", fmt::money(-paid)), p.positive)
    } else if paid.is_zero() {
        ("—".into(), p.faint)
    } else {
        (fmt::money(paid), p.text)
    }
}

/// A filter chip.
fn chip(ui: &mut Ui, text: &str, on: bool) -> bool {
    let p = pal(ui.ctx());
    ui.add(
        egui::Button::new(
            RichText::new(text)
                .font(if on {
                    theme::semibold(12.5)
                } else {
                    theme::regular(12.5)
                })
                .color(if on { p.accent } else { p.muted }),
        )
        .fill(if on { p.accent_soft } else { p.surface })
        .stroke(Stroke::new(1.0, if on { p.accent_soft } else { p.border }))
        .corner_radius(CornerRadius::same(255))
        .min_size(vec2(0.0, 26.0)),
    )
    .clicked()
}

/// The party directory.
pub fn show_list(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    page(ui, "parties", |ui| {
        widgets::page_header(
            ui,
            "Parties",
            "Everyone the company deals with — vendors, contractors, customers, platforms, agencies — and what the company paid each and received from each. Click one for everything the books hold about them.",
        );
        if b.ledger.parties().next().is_none() {
            widgets::card(ui, |ui| {
                widgets::empty(
                    ui,
                    icon::ADDRESS_BOOK,
                    "No parties yet",
                    "Vendors, customers, contractors, and anyone else the company deals with appear here once the bookkeeper adds them.",
                )
            });
            return;
        }
        let st = &mut shell.state.parties;
        let narrow = ui.available_width() < 720.0;
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut st.query)
                    .hint_text(format!("{} Search names", icon::MAGNIFYING_GLASS))
                    .desired_width(if narrow { ui.available_width() } else { 280.0 }),
            );
            let archived = b.ledger.parties().filter(|x| x.archived).count();
            if archived > 0 {
                ui.add_space(8.0);
                ui.checkbox(&mut st.show_archived, format!("Show archived ({archived})"));
            }
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            let all = b
                .ledger
                .parties()
                .filter(|x| st.show_archived || !x.archived)
                .count();
            if chip(ui, &format!("All {all}"), st.role.is_none()) {
                st.role = None;
            }
            for (r, n) in role_counts(b, st.show_archived) {
                let on = st.role == Some(r);
                if chip(ui, &format!("{} {n}", role_label(r)), on) {
                    st.role = if on { None } else { Some(r) };
                }
            }
        });
        ui.add_space(8.0);
        period_chips(ui, b, None, &mut st.period);
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            ui.label(
                RichText::new(format!("{} Sort", icon::SORT_DESCENDING))
                    .font(theme::regular(12.5))
                    .color(p.muted),
            );
            for (x, label) in [
                (Sort::Name, "Name"),
                (Sort::PaidOut, "Paid out"),
                (Sort::ReceivedIn, "Received in"),
                (Sort::Net, "Net"),
            ] {
                if chip(ui, label, st.sort == x) {
                    st.sort = x;
                }
            }
        });
        ui.add_space(12.0);
        let period_label = b.party_period_label(st.period);
        let rows = sorted(b, st);
        let missing: Vec<&Party> = b
            .ledger
            .parties()
            .filter(|x| !x.archived && w9_status(b, x) == W9Status::Missing)
            .collect();
        if !missing.is_empty() {
            let names: Vec<&str> = missing.iter().map(|x| x.label()).collect();
            notice_box(
                ui,
                &Notice {
                    problem: true,
                    text: format!(
                        "No W-9 on file for {}: collect one before the next payment.",
                        names.join(", ")
                    ),
                },
            );
            ui.add_space(4.0);
        }
        if rows.is_empty() {
            widgets::card(ui, |ui| {
                widgets::empty(
                    ui,
                    icon::MAGNIFYING_GLASS,
                    "No matching parties",
                    "Try a different search or role.",
                )
            });
            return;
        }
        let mut go = None;
        widgets::card(ui, |ui| {
            let amt = widgets::AMOUNT_MIN + 16.0;
            // Narrow, the entry count and then the roles go (the role
            // chips above still filter), so the amounts stay in view.
            let avail = ui.available_width();
            let show_roles = avail >= 900.0;
            let roles_w = if show_roles {
                (avail * 0.16).clamp(120.0, 200.0)
            } else {
                0.0
            };
            let w9_w = (avail * 0.16).clamp(140.0, 210.0);
            let n_w = if avail >= 1000.0 { 70.0 } else { 0.0 };
            let fixed = roles_w + w9_w + n_w + amt * 3.0;
            let w = avail.max(fixed + 180.0);
            // Cells without the entry count when it has no room.
            let cells = |c: [(String, f32, bool, egui::Color32); 7]| -> Vec<_> {
                c.into_iter()
                    .enumerate()
                    .filter(|(i, _)| (*i != 3 || n_w > 0.0) && (*i != 1 || show_roles))
                    .map(|(_, c)| c)
                    .collect()
            };
            widgets::min_width(ui, "party_list", w, |ui| {
                let head = |t: &str, cw: f32, right: bool, on: bool| {
                    (
                        t.to_string(),
                        cw,
                        right,
                        if on { p.accent } else { p.muted },
                    )
                };
                let sort = st.sort;
                row(
                    ui,
                    w,
                    &cells([
                        head("Name", 0.0, false, sort == Sort::Name),
                        head("Roles", roles_w, false, false),
                        head("W-9", w9_w, false, false),
                        head("Entries", n_w, true, false),
                        head("Paid out", amt, true, sort == Sort::PaidOut),
                        head("Received in", amt, true, sort == Sort::ReceivedIn),
                        head("Net", amt, true, sort == Sort::Net),
                    ]),
                    None,
                    true,
                    false,
                );
                widgets::rule(ui);
                for (x, flow) in &rows {
                    let text = if x.archived { p.muted } else { p.text };
                    let status = w9_status(b, x);
                    let w9_color = match status {
                        W9Status::Missing => p.warning,
                        W9Status::MarkedOnFile => p.muted,
                        _ => text,
                    };
                    let mut sub = Vec::new();
                    if x.display_name.is_some() {
                        sub.push(x.name.clone());
                    }
                    if let Some(site) = &x.website {
                        sub.push(site.clone());
                    }
                    if x.archived {
                        sub.push("archived".into());
                    }
                    if sub.is_empty() {
                        sub.push(format!("Party {}", x.id));
                    }
                    let sub = sub.join(" · ");
                    let money = |d: Decimal, c| {
                        if d.is_zero() {
                            ("—".to_string(), p.faint)
                        } else {
                            (fmt::money(d), c)
                        }
                    };
                    let (out_t, out_c) = money(flow.paid, p.text);
                    let (in_t, in_c) = money(flow.received, p.positive);
                    let (net_t, net_c) = paid_cell(flow.net(), p);
                    let resp = row(
                        ui,
                        w,
                        &cells([
                            (x.label().to_string(), 0.0, false, text),
                            (roles_text(x), roles_w, false, p.muted),
                            (status.text(), w9_w, false, w9_color),
                            (flow.entries.to_string(), n_w, true, p.muted),
                            (out_t, amt, true, out_c),
                            (in_t, amt, true, in_c),
                            (net_t, amt, true, net_c),
                        ]),
                        Some((sub.as_str(), p.faint)),
                        false,
                        true,
                    );
                    let resp = resp.on_hover_text("Open this party");
                    resp.context_menu(|ui| {
                        widgets::copy_item(ui, "Copy name", &x.name);
                        widgets::copy_item(ui, "Copy party id", &x.id.to_string());
                        widgets::copy_item(ui, "Copy paid out", &flow.paid.to_string());
                        widgets::copy_item(ui, "Copy received in", &flow.received.to_string());
                    });
                    if resp.clicked() {
                        go = Some(x.id);
                    }
                }
            });
        });
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "{period_label}: Paid out is what each party's posted entries moved out of cash, cards, and what's owed back to whoever paid for the company; Received in is what came back in; Net is paid out less received (a plus means more came in). Entries counts the posted ones in the period. The 1099 amount, on each party's page, is the 1099 worksheet's."
                ))
                .font(theme::regular(11.5))
                .color(p.faint),
            )
            .wrap(),
        );
        if let Some(id) = go {
            shell.go(Route::Party(id));
        }
    });
}

/// A number tile on a party's page: label, amount, how it's computed,
/// and the line under it.
type Tile = (String, Decimal, String, Option<(String, egui::Color32)>);

/// One party, completely.
pub fn show_party(ui: &mut Ui, b: &Books, id: PartyId, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let Some(party) = b.ledger.get_party(id) else {
        page(ui, "party", |ui| {
            widgets::empty(
                ui,
                icon::ADDRESS_BOOK,
                "No such party",
                "It isn't in these books.",
            );
        });
        return;
    };
    let info = b.party(id).cloned().unwrap_or_default();
    let year = b.today.year();
    let roles = party.all_roles();
    let mut go: Option<Route> = None;
    page(ui, "party", |ui| {
        ui.horizontal(|ui| {
            if widgets::link(
                ui,
                RichText::new(format!("{} Parties", icon::ARROW_LEFT))
                    .font(theme::regular(12.5))
                    .color(p.muted),
            )
            .clicked()
            {
                go = Some(Route::Parties);
            }
            widgets::copyable(
                ui,
                RichText::new(format!("· Party {id}"))
                    .font(theme::regular(12.5))
                    .color(p.muted),
                &id.to_string(),
            );
        });
        ui.add_space(4.0);
        widgets::copyable(
            ui,
            RichText::new(party.label()).font(theme::bold(26.0)),
            party.label(),
        );
        if party.display_name.is_some() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Legal name")
                        .font(theme::regular(12.5))
                        .color(p.muted),
                );
                widgets::copyable(ui, RichText::new(&party.name), &party.name);
            });
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if roles.is_empty() {
                widgets::pill(ui, "No role yet", p.muted, p.hover);
            }
            for r in &roles {
                if r.gets_1099() {
                    widgets::pill(ui, role_label(*r), p.accent, p.accent_soft);
                } else {
                    widgets::pill(ui, role_label(*r), p.muted, p.hover);
                }
            }
            if party.archived {
                widgets::pill(ui, "Archived", p.warning, p.warning_soft)
                    .on_hover_text("No longer dealt with; its history stays.");
            }
        });
        ui.add_space(14.0);

        // The money over the chosen period: out, in, and the net.
        let period = shell.state.parties.period;
        period_chips(ui, b, Some(id), &mut shell.state.parties.period);
        ui.add_space(10.0);
        let span = b.party_span(period);
        let flow = b.party_flow(id, span, true);
        let label = b.party_period_label(period);
        let span_words = format!("{} – {}", fmt::date(span.0), fmt::date(span.1));
        let mut tiles: Vec<Tile> = Vec::new();
        tiles.push((
            "Paid out".into(),
            flow.paid,
            format!("{label}, {span_words}: what this party's posted entries moved out of cash, cards, and what's owed back to whoever paid for the company (net of refunds and reversals of payments)."),
            Some((label.clone(), p.muted)),
        ));
        tiles.push((
            "Received in".into(),
            flow.received,
            format!("{label}, {span_words}: what this party's posted entries brought into cash and those accounts (net of reversals)."),
            Some((label.clone(), p.muted)),
        ));
        let net = flow.net();
        tiles.push((
            "Net".into(),
            net.abs(),
            "Paid out less received in.".into(),
            Some((
                if net < Decimal::ZERO {
                    "more received than paid".to_string()
                } else if net.is_zero() {
                    "even".to_string()
                } else {
                    "more paid than received".to_string()
                },
                if net < Decimal::ZERO {
                    p.positive
                } else {
                    p.muted
                },
            )),
        ));
        if party.customer.is_some() {
            tiles.push((
                format!("Invoiced in {year}"),
                info.invoiced_ytd,
                "The totals of its invoices dated since January 1.".into(),
                None,
            ));
            tiles.push((
                "Open invoices".into(),
                info.open_invoices,
                "What its invoices still have owing to the company today.".into(),
                None,
            ));
        }
        if party.vendor.is_some() {
            tiles.push((
                "Open bills".into(),
                info.open_bills,
                "What the company still owes on its bills today.".into(),
                (!info.bills.is_empty()).then(|| {
                    (
                        format!("{} billed in {year}", fmt::dollars(info.billed_ytd)),
                        p.muted,
                    )
                }),
            ));
        }
        let tax_year = b.party_period_year(period);
        let form = if tax_year == year {
            info.form_1099.clone().zip(info.threshold_1099)
        } else {
            b.party_1099(id, tax_year)
        };
        if let Some((l, threshold)) = &form {
            let sub = match l.form {
                Some(_) if l.reportable => (
                    format!("Reportable · threshold {}", fmt::dollars(*threshold)),
                    p.warning,
                ),
                Some(_) => (
                    format!("Under the {} threshold", fmt::dollars(*threshold)),
                    p.muted,
                ),
                None => ("Not marked for a 1099".into(), p.muted),
            };
            tiles.push((
                format!("1099 amount, {tax_year}"),
                l.paid,
                "Cash paid in the calendar year, from the 1099 worksheet (ledger-core): posted entries tagged with its vendor record, net of refunds and reversals. Card payments aren't counted (the card issuer reports them).".into(),
                Some(sub),
            ));
        }
        let n = tiles.len();
        super::grid(ui, n, 170.0, 6, |i, ui| {
            let (label, value, how, sub) = &tiles[i];
            let r = widgets::kpi(
                ui,
                widgets::Kpi {
                    label,
                    value: fmt::dollars(*value),
                    sub: sub.clone(),
                    spark: None,
                    how,
                },
            );
            r.context_menu(|ui| widgets::copy_item(ui, "Copy amount", &value.to_string()));
        });

        // A member's page is their capital account statement too.
        if let Some(st) = b.capital_statement(id, span) {
            super::capital::capital_card(ui, b, &st, party.label(), &mut go);
            ui.add_space(14.0);
        }
        super::columns(ui, 860.0, &[1.0, 1.0], |col, ui| {
            if col == 0 {
                what_for_card(ui, b, &flow, &label, &mut go);
            } else {
                months_card(ui, b, id, span, &label);
            }
        });
        super::columns(ui, 860.0, &[1.0, 1.0], |col, ui| {
            if col == 0 {
                facts_card(ui, party, &mut go);
            } else {
                let mut any = false;
                if b.files_1099(party) || party.w9.is_some() {
                    w9_card(ui, b, party);
                    ui.add_space(16.0);
                    any = true;
                }
                if roles.contains(&Role::Employee) || party.w4.is_some() {
                    w4_card(ui, party);
                    ui.add_space(16.0);
                    any = true;
                }
                if !any || party.vendor.is_some() || party.customer.is_some() {
                    records_card(ui, b, party, &info, &mut go);
                }
            }
        });

        entries_card(ui, b, &info, &mut go);
        ui.add_space(14.0);
        if !info.invoices.is_empty() || !info.bills.is_empty() {
            documents_card(ui, b, &info, &mut go);
            ui.add_space(14.0);
        }
        if !info.payroll_runs.is_empty() {
            payroll_card(ui, b, party, &info, &mut go);
            ui.add_space(14.0);
        }
        let mut subjects = vec![ledger_store::Subject::Party(id)];
        subjects.extend(
            info.invoices
                .iter()
                .map(|i| ledger_store::Subject::Invoice(*i)),
        );
        subjects.extend(info.bills.iter().map(|i| ledger_store::Subject::Bill(*i)));
        super::documents::documents_card_with(
            ui,
            b,
            "Documents",
            &subjects,
            &[ledger_store::Subject::Party(id)],
            "No documents linked to them (contracts, a W-9 is kept in the vault, correspondence).",
            &mut go,
        );
        ui.add_space(14.0);
        widgets::titled_card(
            ui,
            "History",
            |_| {},
            |ui| {
                if info.events.is_empty() {
                    ui.label(RichText::new("No journal events name it.").color(p.muted));
                }
                for &i in &info.events {
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

/// Who they are: names, website, address, notes, ids.
fn facts_card(ui: &mut Ui, party: &Party, go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    widgets::titled_card(
        ui,
        "Facts",
        |_| {},
        |ui| {
            facts(ui, |ui, k| {
                let text = |ui: &mut Ui, v: &Option<String>| match v {
                    Some(t) => {
                        widgets::copyable(ui, RichText::new(t), t);
                    }
                    None => {
                        ui.label(RichText::new("not recorded").color(p.faint));
                    }
                };
                k(ui, "Name");
                widgets::copyable(ui, RichText::new(&party.name), &party.name);
                ui.end_row();
                k(ui, "Display name");
                text(ui, &party.display_name);
                ui.end_row();
                k(ui, "Website");
                text(ui, &party.website);
                ui.end_row();
                k(ui, "Address");
                match &party.address {
                    Some(a) => {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            widgets::copyable(ui, RichText::new(a), a);
                        });
                    }
                    None => {
                        ui.label(RichText::new("not recorded").color(p.faint));
                    }
                }
                ui.end_row();
                k(ui, "Party id");
                widgets::copyable(
                    ui,
                    RichText::new(party.id.to_string()),
                    &party.id.to_string(),
                );
                ui.end_row();
                k(ui, "Taxpayer ID");
                match &party.tin {
                    Some(t) => {
                        ui.label(RichText::new(tin_on_file(t)).monospace())
                            .on_hover_text(
                                "Only the last four digits are in the books; the full number is kept in the encrypted TIN vault, for the CPA package.",
                            );
                    }
                    None => {
                        ui.label(RichText::new("not recorded").color(p.faint));
                    }
                }
                ui.end_row();
                if let Some(v) = party.vendor {
                    k(ui, "Vendor record");
                    if widgets::link(ui, RichText::new(format!("vendor {v}")).color(p.accent))
                        .on_hover_text("Vendors and customers are listed on the Company page")
                        .clicked()
                    {
                        *go = Some(Route::Company);
                    }
                    ui.end_row();
                }
                if let Some(c) = party.customer {
                    k(ui, "Customer record");
                    if widgets::link(ui, RichText::new(format!("customer {c}")).color(p.accent))
                        .on_hover_text("Vendors and customers are listed on the Company page")
                        .clicked()
                    {
                        *go = Some(Route::Company);
                    }
                    ui.end_row();
                }
            });
            if let Some(n) = &party.notes {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Notes")
                        .font(theme::regular(12.5))
                        .color(p.muted),
                );
                ui.add(egui::Label::new(RichText::new(n).font(theme::regular(13.0))).wrap());
            }
        },
    );
}

/// The W-9: what it said (never more of the TIN than its last four), or
/// that it's missing.
fn w9_card(ui: &mut Ui, b: &Books, party: &Party) {
    let p = pal(ui.ctx());
    let status = w9_status(b, party);
    widgets::titled_card(
        ui,
        "W-9",
        |ui| match &status {
            W9Status::OnFile { .. } => {
                widgets::pill(ui, "On file", p.positive, p.positive_soft);
            }
            W9Status::Missing => {
                widgets::pill(ui, "Missing", p.warning, p.warning_soft);
            }
            W9Status::MarkedOnFile => {
                widgets::pill(ui, "Facts not recorded", p.muted, p.hover);
            }
            W9Status::NotNeeded => {}
        },
        |ui| {
            let Some(w) = &party.w9 else {
                let text = if status == W9Status::MarkedOnFile {
                    "Its vendor record says a W-9 is on file, but what the form says isn't recorded here. Record its facts so the 1099 worksheet can be checked against it."
                } else {
                    "No W-9 on file. Collect one before the next payment: the 1099 the company files for them needs their taxpayer identification number."
                };
                ui.add(
                    egui::Label::new(RichText::new(text).font(theme::regular(12.5)).color(
                        if status == W9Status::Missing {
                            p.warning
                        } else {
                            p.muted
                        },
                    ))
                    .wrap(),
                );
                ui.add_space(6.0);
                widgets::copy_prompt(ui, &collect_w9_prompt(b, party));
                return;
            };
            facts(ui, |ui, k| {
                k(ui, "Received");
                ui.label(fmt::date(w.received));
                ui.end_row();
                k(ui, "Entity type");
                ui.label(&w.entity_type);
                ui.end_row();
                k(ui, "Exempt payee code");
                ui.label(w.exempt_payee_code.as_deref().unwrap_or("none"));
                ui.end_row();
                k(ui, "FATCA code");
                ui.label(w.fatca_code.as_deref().unwrap_or("none"));
                ui.end_row();
                k(ui, "TIN");
                ui.label(RichText::new(masked_tin(&w.tin_last4)).monospace())
                    .on_hover_text(
                        "Only the last four digits are in the books; the full number is kept in the vault.",
                    );
                ui.end_row();
                k(ui, "Kept at");
                ui.label(&w.kept_at);
                ui.end_row();
                if let Some(h) = &w.vault_record {
                    k(ui, "Vault record");
                    widgets::copyable(
                        ui,
                        RichText::new(format!("sha256 {}…", &h[..h.len().min(12)])).monospace(),
                        h,
                    );
                    ui.end_row();
                }
            });
        },
    );
}

/// The W-4: what it said (never the SSN), or that it's missing.
fn w4_card(ui: &mut Ui, party: &Party) {
    let p = pal(ui.ctx());
    widgets::titled_card(
        ui,
        "W-4",
        |ui| {
            if party.w4.is_some() {
                widgets::pill(ui, "On file", p.positive, p.positive_soft);
            } else {
                widgets::pill(ui, "Missing", p.warning, p.warning_soft);
            }
        },
        |ui| match &party.w4 {
            Some(w) => facts(ui, |ui, k| {
                k(ui, "Received");
                ui.label(fmt::date(w.received));
                ui.end_row();
                k(ui, "Work state");
                ui.label(&w.state);
                ui.end_row();
                k(ui, "Kept at");
                ui.label(&w.kept_at);
                ui.end_row();
            }),
            None => {
                ui.add(
                    egui::Label::new(
                        RichText::new("No W-4 facts recorded for this employee.")
                            .font(theme::regular(12.5))
                            .color(p.warning),
                    )
                    .wrap(),
                );
            }
        },
    );
}

/// Its vendor and customer records, and the 1099 worksheet's line.
fn records_card(ui: &mut Ui, b: &Books, party: &Party, info: &PartyInfo, go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    widgets::titled_card(
        ui,
        "Linked records",
        |_| {},
        |ui| {
            let vendor = party.vendor.and_then(|v| b.ledger.get_vendor(v));
            let customer = party.customer.and_then(|c| b.ledger.get_customer(c));
            if vendor.is_none() && customer.is_none() {
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "No vendor or customer record: it isn't billed or invoiced, only tagged on entries.",
                        )
                        .font(theme::regular(12.5))
                        .color(p.muted),
                    )
                    .wrap(),
                );
                return;
            }
            facts(ui, |ui, k| {
                if let Some(v) = vendor {
                    k(ui, "Vendor");
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{} · vendor {}", v.name, v.id));
                        if let Some(f) = v.form_1099 {
                            widgets::pill(ui, f.label(), p.accent, p.accent_soft);
                        }
                    });
                    ui.end_row();
                    k(ui, "Bills");
                    ui.label(fmt::count(info.bills.len(), "bill", "bills"));
                    ui.end_row();
                }
                if let Some(c) = customer {
                    k(ui, "Customer");
                    ui.label(format!("{} · customer {}", c.name, c.id));
                    ui.end_row();
                    k(ui, "Invoices");
                    ui.label(fmt::count(info.invoices.len(), "invoice", "invoices"));
                    ui.end_row();
                }
            });
            if info.form_1099.is_some() {
                ui.add_space(6.0);
                if widgets::link(
                    ui,
                    RichText::new(format!("{} The 1099 worksheet", icon::FILE_TEXT))
                        .color(p.accent),
                )
                .clicked()
                {
                    *go = Some(Route::Reports(ReportKind::Vendor1099));
                }
            }
        },
    );
}

/// Every entry tagged with the party, newest first.
fn entries_card(ui: &mut Ui, b: &Books, info: &PartyInfo, go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    widgets::titled_card(
        ui,
        "Entries",
        |ui| {
            ui.label(
                RichText::new(fmt::count(info.entries.len(), "entry", "entries"))
                    .font(theme::regular(12.0))
                    .color(p.muted),
            );
        },
        |ui| {
            if info.entries.is_empty() {
                ui.label(
                    RichText::new("No entry in the books is tagged with them yet.").color(p.muted),
                );
                return;
            }
            let amt = widgets::AMOUNT_MIN + 16.0;
            let w = ui.available_width().max(104.0 + 64.0 + 150.0 + amt + 200.0);
            widgets::min_width(ui, "party_entries", w, |ui| {
                let head = |t: &str, cw: f32, right: bool| (t.to_string(), cw, right, p.muted);
                row(
                    ui,
                    w,
                    &[
                        head("Date", 104.0, false),
                        head("Entry", 64.0, false),
                        head("Description", 0.0, false),
                        head("Kind", 150.0, false),
                        head("Amount", amt, true),
                    ],
                    None,
                    true,
                    false,
                );
                widgets::rule(ui);
                for id in &info.entries {
                    let Some(e) = b.entry(*id) else {
                        continue;
                    };
                    let draft = e.status != ledger_core::EntryStatus::Posted;
                    let text = if draft { p.muted } else { p.text };
                    let accounts: Vec<String> =
                        e.accounts.iter().map(|a| b.account_name(*a)).collect();
                    let mut sub = accounts.join(" → ");
                    let kind = if draft {
                        format!("{} · draft", e.origin)
                    } else {
                        e.origin.to_string()
                    };
                    // Shared with other parties: show their part.
                    let mut amount = e.amount;
                    if e.parties.len() > 1
                        && let Some(mine) = e.parties.iter().find(|x| x.id == info.id)
                    {
                        let others: Vec<&str> = e
                            .parties
                            .iter()
                            .filter(|x| x.id != info.id)
                            .map(|x| x.label.as_str())
                            .collect();
                        sub = format!(
                            "Their share, {} of {}, with {} · {sub}",
                            super::entry::share_text(mine),
                            fmt::dollars(e.amount),
                            others.join(" & ")
                        );
                        amount = mine.resolved.map(|v| v.abs()).unwrap_or(e.amount);
                    }
                    let resp = row(
                        ui,
                        w,
                        &[
                            (fmt::date(e.date), 104.0, false, text),
                            (format!("#{}", e.id), 64.0, false, p.muted),
                            (e.description.clone(), 0.0, false, text),
                            (kind, 150.0, false, if draft { p.warning } else { p.muted }),
                            (fmt::money(amount), amt, true, text),
                        ],
                        Some((sub.as_str(), p.faint)),
                        false,
                        true,
                    );
                    resp.context_menu(|ui| {
                        widgets::copy_item(ui, "Copy entry number", &format!("#{}", e.id));
                        widgets::copy_item(ui, "Copy amount", &e.amount.to_string());
                    });
                    if resp.on_hover_text("Open this entry").clicked() {
                        *go = Some(Route::Entry(e.id));
                    }
                }
            });
        },
    );
}

/// Its invoices and bills.
fn documents_card(ui: &mut Ui, b: &Books, info: &PartyInfo, go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    widgets::titled_card(
        ui,
        "Invoices and bills",
        |_| {},
        |ui| {
            let amt = widgets::AMOUNT_MIN + 16.0;
            let w = ui
                .available_width()
                .max(110.0 + 104.0 * 2.0 + 90.0 + amt * 2.0 + 60.0);
            widgets::min_width(ui, "party_docs", w, |ui| {
                let head = |t: &str, cw: f32, right: bool| (t.to_string(), cw, right, p.muted);
                row(
                    ui,
                    w,
                    &[
                        head("Document", 0.0, false),
                        head("Date", 104.0, false),
                        head("Due", 104.0, false),
                        head("Status", 90.0, false),
                        head("Total", amt, true),
                        head("Still owed", amt, true),
                    ],
                    None,
                    true,
                    false,
                );
                widgets::rule(ui);
                let invoices = info
                    .invoices
                    .iter()
                    .filter_map(|i| b.ledger.get_invoice(*i))
                    .map(|i| {
                        (
                            format!("Invoice {}", i.id),
                            i.date,
                            i.due_date,
                            i.status,
                            i.total(),
                            i.balance(),
                            i.entry_id,
                        )
                    });
                let bills = info
                    .bills
                    .iter()
                    .filter_map(|i| b.ledger.get_bill(*i))
                    .map(|i| {
                        (
                            format!("Bill {}", i.id),
                            i.date,
                            i.due_date,
                            i.status,
                            i.total(),
                            i.balance(),
                            i.entry_id,
                        )
                    });
                let mut docs: Vec<_> = invoices.chain(bills).collect();
                docs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                for (name, date, due, status, total, balance, entry) in docs {
                    let late = !balance.is_zero() && due < b.today;
                    let resp = row(
                        ui,
                        w,
                        &[
                            (name, 0.0, false, p.text),
                            (fmt::date(date), 104.0, false, p.text),
                            (
                                fmt::date(due),
                                104.0,
                                false,
                                if late { p.warning } else { p.text },
                            ),
                            (fmt::words(&format!("{status:?}")), 90.0, false, p.muted),
                            (fmt::money(total), amt, true, p.text),
                            (fmt::money(balance), amt, true, p.text),
                        ],
                        None,
                        false,
                        true,
                    );
                    if resp.on_hover_text("Open its entry").clicked() {
                        *go = Some(Route::Entry(entry));
                    }
                }
            });
        },
    );
}

/// The payroll runs that name them, with their line of each.
fn payroll_card(ui: &mut Ui, b: &Books, party: &Party, info: &PartyInfo, go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    widgets::titled_card(
        ui,
        "Payroll runs",
        |_| {},
        |ui| {
            let amt = widgets::AMOUNT_MIN + 16.0;
            let w = ui.available_width().max(104.0 + 80.0 + amt * 2.0 + 160.0);
            widgets::min_width(ui, "party_payroll", w, |ui| {
                let head = |t: &str, cw: f32, right: bool| (t.to_string(), cw, right, p.muted);
                row(
                    ui,
                    w,
                    &[
                        head("Pay date", 104.0, false),
                        head("Run", 80.0, false),
                        head("Employee", 0.0, false),
                        head("Gross", amt, true),
                        head("Net", amt, true),
                    ],
                    None,
                    true,
                    false,
                );
                widgets::rule(ui);
                let key = ledger_core::parties::name_key(&party.name);
                for run in b
                    .ledger
                    .list_payroll_runs()
                    .into_iter()
                    .filter(|r| info.payroll_runs.contains(&r.id))
                {
                    for l in run.lines.iter().filter(|l| match l.employee {
                        Some(e) => e == party.id,
                        None => ledger_core::parties::name_key(&l.employee_name) == key,
                    }) {
                        let resp = row(
                            ui,
                            w,
                            &[
                                (fmt::date(run.date), 104.0, false, p.text),
                                (format!("Run {}", run.id), 80.0, false, p.muted),
                                (l.employee_name.clone(), 0.0, false, p.text),
                                (fmt::money(l.gross_pay), amt, true, p.text),
                                (fmt::money(l.net_pay()), amt, true, p.text),
                            ],
                            None,
                            false,
                            true,
                        );
                        if resp.on_hover_text("Open the payroll entry").clicked() {
                            *go = Some(Route::Entry(run.entry_id));
                        }
                    }
                }
            });
        },
    );
}

/// "What for": the period's money by the account on the other side of
/// each entry ([`PartyFlow::what_for`]), which sums to the net exactly.
fn what_for_card(ui: &mut Ui, b: &Books, flow: &PartyFlow, label: &str, go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    widgets::titled_card(
        ui,
        "What for",
        |ui| {
            ui.label(
                RichText::new(label)
                    .font(theme::regular(12.0))
                    .color(p.muted),
            );
        },
        |ui| {
            if flow.what_for.is_empty() {
                ui.label(
                    RichText::new("No posted entries with them in this period.").color(p.muted),
                );
                return;
            }
            let amt = widgets::AMOUNT_MIN + 16.0;
            let w = ui.available_width().max(200.0 + amt);
            let roles = [
                (ledger_core::PAYABLES, "billed, not yet paid"),
                (ledger_core::RECEIVABLES, "invoiced, not yet collected"),
            ];
            widgets::min_width(ui, "what_for", w, |ui| {
                for (account, amount) in &flow.what_for {
                    let note = roles
                        .iter()
                        .find(|(r, _)| b.ledger.role_account(r) == Some(*account))
                        .map(|(_, n)| *n);
                    let (text, color) = if *amount < Decimal::ZERO {
                        (format!("+{}", fmt::money(-*amount)), p.positive)
                    } else {
                        (fmt::money(*amount), p.text)
                    };
                    let resp = row(
                        ui,
                        w,
                        &[
                            (b.account_label(*account), 0.0, false, p.text),
                            (text, amt, true, color),
                        ],
                        note.map(|n| (n, p.faint)),
                        false,
                        true,
                    );
                    if resp.on_hover_text("Open the account's register").clicked() {
                        *go = Some(Route::Account(*account));
                    }
                }
                widgets::rule(ui);
                let net = flow.net();
                row(
                    ui,
                    w,
                    &[
                        ("Net".into(), 0.0, false, p.text),
                        (
                            if net < Decimal::ZERO {
                                format!("+{}", fmt::money(-net))
                            } else {
                                fmt::money(net)
                            },
                            amt,
                            true,
                            if net < Decimal::ZERO {
                                p.positive
                            } else {
                                p.text
                            },
                        ),
                    ],
                    None,
                    true,
                    false,
                );
            });
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new("Each entry's other side: what the money paid for, or (with a plus) where money received came from. Sums to the net exactly.")
                        .font(theme::regular(11.5))
                        .color(p.faint),
                )
                .wrap(),
            );
        },
    );
}

/// Out versus in, month by month over the period ([`Books::party_months`]).
fn months_card(ui: &mut Ui, b: &Books, id: PartyId, span: (NaiveDate, NaiveDate), label: &str) {
    let p = pal(ui.ctx());
    let months = b.party_months(id, span);
    // All time can be long: the chart shows the last 24 months of it.
    let shown: Vec<_> = months.iter().rev().take(24).rev().collect();
    widgets::titled_card(
        ui,
        "By month",
        |ui| {
            ui.label(
                RichText::new(if months.len() > shown.len() {
                    format!("{label}, last {} months", shown.len())
                } else {
                    label.to_string()
                })
                .font(theme::regular(12.0))
                .color(p.muted),
            );
        },
        |ui| {
            if shown
                .iter()
                .all(|m| m.paid.is_zero() && m.received.is_zero())
            {
                widgets::empty(ui, icon::CHART_BAR, "No money moved", "");
                return;
            }
            let multi_year =
                shown.first().map(|m| m.month.year()) != shown.last().map(|m| m.month.year());
            let groups: Vec<crate::charts::BarGroup> = shown
                .iter()
                .map(|m| crate::charts::BarGroup {
                    label: if multi_year && m.month.month() == 1 {
                        m.month.format("%b %y").to_string()
                    } else {
                        m.month.format("%b").to_string()
                    },
                    values: vec![m.paid, m.received],
                })
                .collect();
            crate::charts::bar_chart(
                ui,
                &groups,
                &[("Paid out", p.series[2]), ("Received in", p.series[1])],
                200.0,
            );
        },
    );
}
