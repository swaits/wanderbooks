//! The journal: every event, in order, as recorded. The audit trail an
//! accountant wants and the raw truth everything else is built from.

use super::page;
use crate::app::Shell;
use crate::books::{Books, entry_refs};
use crate::fmt;
use crate::nav::Route;
use crate::theme::{self, pal};
use crate::widgets;
use egui::{CornerRadius, Margin, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as icon;
use ledger_store::JournalLine;
use serde_json::Value;

#[derive(Default)]
pub struct State {
    pub query: String,
    pub hide_steps: bool,
}

pub struct Event {
    pub icon: &'static str,
    pub text: String,
    pub route: Option<Route>,
    /// A second place it's about (the asset of a depreciation entry).
    pub also: Option<Route>,
}

fn s<'a>(p: &'a Value, k: &str) -> &'a str {
    p.get(k).and_then(|v| v.as_str()).unwrap_or("")
}

fn amount(p: &Value) -> String {
    p.get("amount")
        .and_then(|v| v.as_str())
        .and_then(|a| a.parse().ok())
        .map(fmt::dollars)
        .unwrap_or_default()
}

fn entry_text(b: &Books, id: u64) -> String {
    b.entry(id)
        .map(|e| {
            format!(
                "#{} {} ({})",
                e.id,
                e.description,
                fmt::dollars_or(e.amount)
            )
        })
        .unwrap_or_else(|| format!("#{id}"))
}

fn attachment_name(b: &Books, id: u64) -> String {
    b.attachments
        .iter()
        .find(|a| a.id == id)
        .map(|a| a.file_name.clone())
        .unwrap_or_else(|| format!("attachment {id}"))
}

/// "Tax basis of FL-0001: personal-use conversion, contributor's basis
/// $2,000.00 → tax basis $1,700.00". The tax basis is ledger-core's
/// (`Asset::tax_basis`) for the event's rule and basis on the asset's
/// book cost, which never changes once registered.
fn tax_basis_text(b: &Books, asset_id: Option<u64>, p: &Value) -> String {
    let asset = asset_id.and_then(|id| b.ledger.get_asset(id));
    let name = match (asset, asset_id) {
        (Some(a), _) => a.tag.clone().unwrap_or_else(|| a.description.clone()),
        (None, Some(id)) => format!("asset {id}"),
        (None, None) => "an asset".to_string(),
    };
    let rule = ledger_core::TaxBasisRule::parse(s(p, "rule"));
    let contributor: Option<rust_decimal::Decimal> = p
        .get("contributor_basis")
        .and_then(|v| v.as_str())
        .and_then(|v| v.parse().ok());
    let Some(rule) = rule else {
        return format!("Tax basis of {name} recorded");
    };
    let tax = asset.and_then(|a| {
        let mut a = a.clone();
        a.tax = Some(ledger_core::TaxBasis {
            rule,
            contributor_basis: contributor,
        });
        a.tax_basis()
    });
    let mut text = format!("Tax basis of {name}: {}", super::assets::rule_words(rule));
    if let Some(c) = contributor {
        text.push_str(&format!(", contributor's basis {}", fmt::dollars(c)));
    }
    match (tax, contributor.is_some()) {
        (Some(t), true) => text.push_str(&format!(" → tax basis {}", fmt::dollars(t))),
        (Some(t), false) => text.push_str(&format!(", tax basis {}", fmt::dollars(t))),
        (None, _) => {}
    }
    text
}

/// What an attachment link's subject (`{"kind": .., "id": ..}`) is, in
/// words (Books::subject_label), and where it opens
/// (Books::subject_route).
fn subject_text(b: &Books, subject: Option<&Value>) -> (String, Option<Route>) {
    let parsed =
        subject.and_then(|v| serde_json::from_value::<ledger_store::Subject>(v.clone()).ok());
    match parsed {
        Some(ledger_store::Subject::Company) => ("the company".into(), Some(Route::Company)),
        Some(sub) => (b.subject_label(sub), b.subject_route(sub)),
        None => ("a subject".into(), None),
    }
}

/// A date from the payload, in words ("Sep 30, 2026"), or as written.
fn date_text(p: &Value, k: &str) -> String {
    let t = s(p, k);
    t.parse::<chrono::NaiveDate>()
        .map(fmt::date)
        .unwrap_or_else(|_| t.to_string())
}

/// A party's label, or "a party" when the event names none.
fn party_name(b: &Books, id: Option<u64>) -> String {
    match id {
        Some(id) => b.party_label(id),
        None => "a party".to_string(),
    }
}

/// "Display name of Gumroad, Inc. set to Gumroad", "Archived Etsy": one
/// `party.updated` event in words. Names are today's (the display name's
/// change names the legal name, so it reads the same afterwards).
fn party_update_text(b: &Books, id: Option<u64>, change: Option<&Value>) -> String {
    let name = party_name(b, id);
    let legal = id
        .and_then(|i| b.ledger.get_party(i))
        .map_or_else(|| name.clone(), |p| p.name.clone());
    let field = change
        .and_then(|c| c.get("field"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let value = change.and_then(|c| c.get("value"));
    let text = value.and_then(|v| v.as_str());
    let words = match field {
        "name" => "Name",
        "display_name" => "Display name",
        "website" => "Website",
        "address" => "Address",
        "notes" => "Notes",
        _ => "",
    };
    match (field, text) {
        ("archived", _) => {
            if value.and_then(|v| v.as_bool()).unwrap_or(false) {
                format!("Archived {name}")
            } else {
                format!("Unarchived {name}")
            }
        }
        ("name", Some(t)) => format!("Renamed a party to {t}"),
        ("display_name", Some(t)) => format!("Display name of {legal} set to {t}"),
        ("display_name", None) => format!("Display name of {legal} cleared"),
        ("notes", Some(_)) => format!("Notes of {name} updated"),
        ("", _) => format!("Updated {name}"),
        (_, Some(t)) => format!("{words} of {name} set to {t}"),
        (_, None) => format!("{words} of {name} cleared"),
    }
}

/// A sentence for a meaningful event; `None` for the intermediate steps of
/// building an entry (created, line added), which the posted event covers.
pub fn describe(b: &Books, line: &JournalLine) -> Option<Event> {
    let p = &line.payload;
    let id = p.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
    let entry_id = p.get("entry_id").and_then(|v| v.as_u64());
    let att = p.get("attachment_id").and_then(|v| v.as_u64()).unwrap_or(0);
    let ev = |icon, text: String, route: Option<Route>| {
        Some(Event {
            icon,
            text,
            route,
            also: None,
        })
    };
    let asset_id = p.get("asset_id").and_then(|v| v.as_u64());
    let asset = |id: Option<u64>| {
        id.map(|a| {
            b.ledger
                .get_asset(a)
                .map(|x| x.description.clone())
                .unwrap_or_else(|| format!("asset {a}"))
        })
        .unwrap_or_default()
    };
    let with_asset = |icon, text: String| {
        Some(Event {
            icon,
            text,
            route: entry_id.map(Route::Entry),
            also: asset_id.map(Route::Asset),
        })
    };
    match line.line_type.as_str() {
        "entry.created" | "entry.line_added" | "entry.line_class_set" => None,
        "store.created" => ev(icon::BOOK_OPEN, "Books created".into(), None),
        "entry.posted" => ev(
            icon::CHECK_CIRCLE,
            format!("Posted {}", entry_text(b, id)),
            Some(Route::Entry(id)),
        ),
        "entry.reversed" => ev(
            icon::ARROW_COUNTER_CLOCKWISE,
            format!(
                "Reversed #{} with #{id}",
                p.get("reverses").and_then(|v| v.as_u64()).unwrap_or(0)
            ),
            Some(Route::Entry(id)),
        ),
        "receipt.attached" => ev(
            icon::PAPERCLIP,
            match entry_id {
                Some(e) => format!("Attached {} to #{e}", s(p, "file_name")),
                None => format!("Filed receipt {}", s(p, "file_name")),
            },
            Some(Route::Document(att)),
        ),
        "document.filed" => ev(
            icon::FILE_ARROW_UP,
            format!(
                "Filed {} ({})",
                s(p, "file_name"),
                fmt::label(s(p, "category"))
            ),
            Some(Route::Document(att)),
        ),
        "receipt.linked" => ev(
            icon::LINK,
            format!(
                "Linked {} to #{}",
                attachment_name(b, att),
                entry_id.unwrap_or(0)
            ),
            entry_id.map(Route::Entry),
        ),
        "receipt.unlinked" => ev(
            icon::LINK_BREAK,
            format!(
                "Unlinked {} from #{}: {}",
                attachment_name(b, att),
                entry_id.unwrap_or(0),
                s(p, "reason")
            ),
            entry_id.map(Route::Entry),
        ),
        "attachment.linked" | "attachment.unlinked" => {
            let (what, route) = subject_text(b, p.get("subject"));
            let name = attachment_name(b, att);
            Some(Event {
                icon: if line.line_type == "attachment.linked" {
                    icon::LINK
                } else {
                    icon::LINK_BREAK
                },
                text: if line.line_type == "attachment.linked" {
                    format!("Linked {name} to {what}")
                } else {
                    format!("Unlinked {name} from {what}: {}", s(p, "reason"))
                },
                route: Some(Route::Document(att)),
                also: route,
            })
        }
        "statement.recorded" => {
            let account = p.get("account_id").and_then(|v| v.as_u64());
            let closing = s(p, "closing_balance")
                .parse()
                .map(fmt::dollars)
                .unwrap_or_default();
            Some(Event {
                icon: icon::FILE_TEXT,
                text: format!(
                    "Recorded {}'s statement, {} – {}: closing balance {closing}",
                    account.map_or("an account".into(), |a| b.account_label(a)),
                    date_text(p, "period_start"),
                    date_text(p, "period_end")
                ),
                route: Some(Route::Document(att)),
                also: account.map(Route::Account),
            })
        }
        "statement.voided" => Some(Event {
            icon: icon::PROHIBIT,
            text: format!(
                "Voided the statement recorded on {}: {}",
                attachment_name(b, att),
                s(p, "reason")
            ),
            route: Some(Route::Document(att)),
            also: None,
        }),
        "reconciliation.completed" => {
            let account = p.get("account_id").and_then(|v| v.as_u64());
            let balance = s(p, "statement_balance")
                .parse()
                .map(fmt::dollars)
                .unwrap_or_default();
            let lines = p
                .get("cleared")
                .and_then(|v| v.as_array())
                .map_or(0, |v| v.len());
            Some(Event {
                icon: icon::CHECK_SQUARE,
                text: format!(
                    "Reconciled {} through {} at {balance} ({} cleared)",
                    account.map_or("an account".into(), |a| b.account_label(a)),
                    date_text(p, "statement_date"),
                    fmt::count(lines, "line", "lines")
                ),
                route: p
                    .get("id")
                    .and_then(|v| v.as_u64())
                    .map(Route::Reconciliation),
                also: account.map(Route::Account),
            })
        }
        "reconciliation.voided" => {
            let r = p
                .get("id")
                .and_then(|v| v.as_u64())
                .and_then(|i| b.reconciliation(i));
            Some(Event {
                icon: icon::PROHIBIT,
                text: match r {
                    Some(r) => format!(
                        "Voided the reconciliation of {} through {}: {}",
                        b.account_label(r.account_id),
                        fmt::date(r.statement_date),
                        s(p, "reason")
                    ),
                    None => format!("Voided reconciliation {id}: {}", s(p, "reason")),
                },
                route: p
                    .get("id")
                    .and_then(|v| v.as_u64())
                    .map(Route::Reconciliation),
                also: r.map(|r| Route::Account(r.account_id)),
            })
        }
        "attachment.withdrawn" => ev(
            icon::ARCHIVE,
            format!("Withdrew {}: {}", attachment_name(b, att), s(p, "reason")),
            Some(Route::Document(att)),
        ),
        "account.added" => ev(
            icon::PLUS_CIRCLE,
            format!("Added account {} {}", s(p, "code"), s(p, "name")),
            Some(Route::Account(id)),
        ),
        "account.renumbered" => ev(
            icon::TAG,
            format!(
                "Renumbered {} from {}",
                b.account_label(p.get("account_id").and_then(|v| v.as_u64()).unwrap_or(0)),
                s(p, "previous_code")
            ),
            p.get("account_id")
                .and_then(|v| v.as_u64())
                .map(Route::Account),
        ),
        // Its code and both names as journaled: a later rename or
        // renumber never rewrites what this line said.
        "account.renamed" => ev(
            icon::TAG,
            format!(
                "Renamed {} from \u{201c}{}\u{201d} to \u{201c}{}\u{201d}",
                s(p, "code"),
                s(p, "previous"),
                s(p, "name")
            ),
            p.get("account_id")
                .and_then(|v| v.as_u64())
                .map(Route::Account),
        ),
        "role.assigned" => {
            let role = s(p, "role");
            let control = ledger_core::control_account(role);
            let purpose = control.map_or(role, |c| c.role);
            let mut chars = purpose.chars();
            let purpose: String = chars
                .next()
                .map(|f| f.to_uppercase().chain(chars).collect())
                .unwrap_or_default();
            let account = p.get("account_id").and_then(|v| v.as_u64());
            ev(
                icon::TAG,
                match account {
                    Some(id) => format!("{purpose} now posts to {}", b.account_label(id)),
                    None => format!(
                        "{purpose} back to its default code {}",
                        control.map_or("", |c| c.code)
                    ),
                },
                account.map(Route::Account),
            )
        }
        "account.tax_set" => ev(
            icon::TAG,
            format!(
                "Tax category of {}: {}",
                b.account_label(p.get("account_id").and_then(|v| v.as_u64()).unwrap_or(0)),
                p.get("tax_category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("none")
            ),
            p.get("account_id")
                .and_then(|v| v.as_u64())
                .map(Route::Account),
        ),
        "vendor.added" => ev(
            icon::STOREFRONT,
            format!("Added vendor {}", s(p, "name")),
            Some(
                p.get("id")
                    .and_then(|v| v.as_u64())
                    .and_then(|v| b.ledger.party_of_vendor(v))
                    .map_or(Route::Company, Route::Party),
            ),
        ),
        "customer.added" => ev(
            icon::USER_PLUS,
            format!("Added customer {}", s(p, "name")),
            Some(
                p.get("id")
                    .and_then(|v| v.as_u64())
                    .and_then(|c| b.ledger.party_of_customer(c))
                    .map_or(Route::Company, Route::Party),
            ),
        ),
        "class.added" => ev(icon::TAG, format!("Added class {}", s(p, "name")), None),
        "party.added" => {
            let roles: Vec<String> = p
                .get("roles")
                .and_then(|v| v.as_array())
                .map(|rs| {
                    rs.iter()
                        .filter_map(|r| r.as_str().and_then(ledger_core::Role::parse))
                        .map(|r| super::parties::role_label(r).to_lowercase())
                        .collect()
                })
                .unwrap_or_default();
            let name = match s(p, "name") {
                "" => party_name(b, p.get("id").and_then(|v| v.as_u64())),
                n => n.to_string(),
            };
            ev(
                icon::ADDRESS_BOOK,
                if roles.is_empty() {
                    format!("Added party {name}")
                } else {
                    format!("Added party {name} ({})", roles.join(", "))
                },
                p.get("id").and_then(|v| v.as_u64()).map(Route::Party),
            )
        }
        "party.updated" => {
            let pid = p.get("id").and_then(|v| v.as_u64());
            ev(
                icon::PENCIL_SIMPLE,
                party_update_text(b, pid, p.get("change")),
                pid.map(Route::Party),
            )
        }
        "party.role_set" => {
            let pid = p.get("id").and_then(|v| v.as_u64());
            let name = party_name(b, pid);
            let role = ledger_core::Role::parse(s(p, "role"));
            let on = p.get("on").and_then(|v| v.as_bool()).unwrap_or(true);
            let text = match (role, on) {
                (Some(r), true) => format!("{name} is now {}", super::parties::role_noun(r)),
                (Some(r), false) => format!("{name} is no longer {}", super::parties::role_noun(r)),
                (None, _) => format!("Changed the roles of {name}"),
            };
            ev(icon::USER_GEAR, text, pid.map(Route::Party))
        }
        "party.w9_set" => {
            let pid = p.get("id").and_then(|v| v.as_u64());
            let name = party_name(b, pid);
            let text = match p.get("facts").filter(|f| !f.is_null()) {
                Some(f) => {
                    let mut t = format!("W-9 for {name} recorded");
                    if let Some(d) = f.get("received").and_then(|v| v.as_str()) {
                        t.push_str(&format!(", received {d}"));
                    }
                    if let Some(e) = f.get("entity_type").and_then(|v| v.as_str()) {
                        t.push_str(&format!(": {e}"));
                    }
                    if let Some(l4) = f.get("tin_last4").and_then(|v| v.as_str()) {
                        t.push_str(&format!(", TIN ending {l4}"));
                    }
                    t
                }
                None => format!("W-9 facts for {name} cleared"),
            };
            ev(icon::IDENTIFICATION_CARD, text, pid.map(Route::Party))
        }
        "party.tin_set" => {
            let pid = p.get("id").and_then(|v| v.as_u64());
            let name = party_name(b, pid);
            let text = match p.get("facts").filter(|f| !f.is_null()) {
                Some(f) => {
                    let kind = match f.get("tin_type").and_then(|v| v.as_str()) {
                        Some("ssn") => "SSN",
                        Some("ein") => "EIN",
                        Some("itin") => "ITIN",
                        _ => "TIN",
                    };
                    let l4 = f.get("last4").and_then(|v| v.as_str()).unwrap_or("????");
                    if f.get("purged").is_some_and(|v| !v.is_null()) {
                        format!("Purged {name}'s {kind} (••••{l4}) from the vault")
                    } else {
                        format!(
                            "Taxpayer ID for {name} recorded: {kind} ending {l4}, kept in the vault"
                        )
                    }
                }
                None => format!("Taxpayer ID facts for {name} cleared"),
            };
            ev(icon::IDENTIFICATION_CARD, text, pid.map(Route::Party))
        }
        "party.w4_set" => {
            let pid = p.get("id").and_then(|v| v.as_u64());
            let name = party_name(b, pid);
            let text = match p.get("facts").filter(|f| !f.is_null()) {
                Some(f) => {
                    let mut t = format!("W-4 for {name} recorded");
                    if let Some(d) = f.get("received").and_then(|v| v.as_str()) {
                        t.push_str(&format!(", received {d}"));
                    }
                    if let Some(st) = f.get("state").and_then(|v| v.as_str()) {
                        t.push_str(&format!(" (works in {st})"));
                    }
                    t
                }
                None => format!("W-4 facts for {name} cleared"),
            };
            ev(icon::IDENTIFICATION_BADGE, text, pid.map(Route::Party))
        }
        "entry.parties_set" => {
            let e = entry_id.unwrap_or(0);
            let named: Vec<(u64, String)> = p
                .get("parties")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|x| {
                    let id = x.get("party_id").and_then(|v| v.as_u64())?;
                    let share = match x.get("share") {
                        Some(Value::Object(o)) => o
                            .get("amount")
                            .and_then(|v| v.as_str())
                            .map(|a| {
                                rust_decimal::Decimal::from_str_exact(a)
                                    .map(fmt::dollars)
                                    .unwrap_or_else(|_| a.to_string())
                            })
                            .or_else(|| {
                                o.get("percent")
                                    .and_then(|v| v.as_str())
                                    .map(|pc| format!("{pc}%"))
                            }),
                        _ => None,
                    };
                    let name = party_name(b, Some(id));
                    Some((
                        id,
                        match share {
                            Some(sh) => format!("{name} ({sh})"),
                            None => name,
                        },
                    ))
                })
                .collect();
            Some(Event {
                icon: icon::ADDRESS_BOOK,
                text: if named.is_empty() {
                    format!("Cleared the parties of #{e}")
                } else {
                    format!(
                        "Tagged #{e} with {}",
                        named
                            .iter()
                            .map(|(_, n)| n.as_str())
                            .collect::<Vec<_>>()
                            .join(" and ")
                    )
                },
                route: entry_id.map(Route::Entry),
                also: named.first().map(|(id, _)| Route::Party(*id)),
            })
        }
        "entry.party_set" => {
            let pid = p.get("party_id").and_then(|v| v.as_u64());
            let e = entry_id.unwrap_or(0);
            Some(Event {
                icon: icon::ADDRESS_BOOK,
                text: match pid {
                    Some(_) => format!("Tagged #{e} with {}", party_name(b, pid)),
                    None => format!("Cleared the party of #{e}"),
                },
                route: entry_id.map(Route::Entry),
                also: pid.map(Route::Party),
            })
        }
        "vendor.1099_set" => {
            let vendor = p.get("vendor_id").and_then(|v| v.as_u64());
            let name = vendor
                .and_then(|v| b.ledger.get_vendor(v))
                .map(|v| v.name.clone());
            let form = p
                .get("form_1099")
                .and_then(|v| v.as_str())
                .and_then(ledger_core::Form1099::parse)
                .map(|f| f.label());
            let w9 = p.get("tin_on_file").and_then(|v| v.as_bool());
            let text = match (name, form, w9) {
                (Some(n), Some(f), Some(w)) => format!(
                    "1099 details of {n}: {f}, W-9 {}",
                    if w { "on file" } else { "not on file" }
                ),
                (Some(n), None, Some(_)) => format!("{n} no longer gets a 1099"),
                (Some(n), _, None) => format!("Updated the 1099 details of {n}"),
                (None, ..) => "Updated a vendor's 1099 details".into(),
            };
            ev(
                icon::IDENTIFICATION_CARD,
                text,
                Some(
                    vendor
                        .and_then(|v| b.ledger.party_of_vendor(v))
                        .map_or(Route::Company, Route::Party),
                ),
            )
        }
        "entry.vendor_set" => {
            let vendor = p.get("vendor_id").and_then(|v| v.as_u64());
            let e = entry_id.unwrap_or(0);
            Some(Event {
                icon: icon::STOREFRONT,
                text: match vendor.and_then(|v| b.ledger.get_vendor(v)) {
                    Some(v) => format!("Tagged #{e} with vendor {}", v.name),
                    None if vendor.is_none() && p.get("vendor_id").is_some() => {
                        format!("Cleared the vendor of #{e}")
                    }
                    None => format!("Tagged #{e} with a vendor"),
                },
                route: entry_id.map(Route::Entry),
                also: vendor
                    .and_then(|v| b.ledger.party_of_vendor(v))
                    .map(Route::Party),
            })
        }
        "invoice.created" => ev(
            icon::INVOICE,
            format!("Invoice {id} created"),
            entry_id.map(Route::Entry),
        ),
        "bill.created" => ev(
            icon::RECEIPT,
            format!("Bill {id} recorded"),
            entry_id.map(Route::Entry),
        ),
        "invoice.payment_received" => ev(
            icon::HAND_COINS,
            format!("Payment received, {}", amount(p)),
            entry_id.map(Route::Entry),
        ),
        "bill.paid" => ev(
            icon::MONEY,
            format!("Bill paid, {}", amount(p)),
            entry_id.map(Route::Entry),
        ),
        "period.created" => ev(
            icon::CALENDAR_PLUS,
            format!("Period {} created", s(p, "name")),
            Some(Route::Company),
        ),
        "period.closed" => ev(
            icon::LOCK,
            "Period closed".into(),
            p.get("closing_entry")
                .and_then(|v| v.as_u64())
                .map(Route::Entry),
        ),
        "asset.registered" => ev(
            icon::PACKAGE,
            format!("Registered asset {}", s(p, "description")),
            Some(Route::Asset(id)),
        ),
        "asset.tagged" => {
            let tag = s(p, "tag");
            let text = match p.get("previous_tag").and_then(|v| v.as_str()) {
                Some(old) => format!("Retagged {} from {old} to {tag}", asset(asset_id)),
                None => format!("Tagged {} {tag}", asset(asset_id)),
            };
            ev(icon::TAG, text, asset_id.map(Route::Asset))
        }
        "asset.serial_set" => {
            let text = match p.get("serial").and_then(|v| v.as_str()) {
                Some(serial) => format!("Serial number of {} set to {serial}", asset(asset_id)),
                None => format!("Serial number of {} cleared", asset(asset_id)),
            };
            ev(icon::BARCODE, text, asset_id.map(Route::Asset))
        }
        "asset.tax_basis_set" => ev(
            icon::SCALES,
            tax_basis_text(b, asset_id, p),
            asset_id.map(Route::Asset),
        ),
        "asset.corrected" => ev(
            icon::PENCIL_SIMPLE,
            format!(
                "Corrected {}: cost {} (was {}), {} months from {}. {}",
                asset(asset_id),
                p.get("cost")
                    .and_then(|c| c.get("amount"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?"),
                p.get("previous")
                    .and_then(|c| c.get("cost"))
                    .and_then(|c| c.get("amount"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?"),
                p.get("useful_life_months")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                s(p, "purchase_date"),
                s(p, "reason")
            ),
            asset_id.map(Route::Asset),
        ),
        "asset.described" => ev(
            icon::PENCIL_SIMPLE,
            format!(
                "Renamed asset {} to {}",
                s(p, "previous_description"),
                s(p, "description")
            ),
            asset_id.map(Route::Asset),
        ),
        "asset.acquisition_linked" => {
            let entry = p.get("entry_id").and_then(|v| v.as_u64()).unwrap_or(0);
            Some(Event {
                icon: icon::LINK,
                text: format!(
                    "Linked {} to the entry that acquired it, #{entry}",
                    asset(asset_id)
                ),
                route: asset_id.map(Route::Asset),
                also: Some(Route::Entry(entry)),
            })
        }
        "asset.depreciation_posted" => with_asset(
            icon::TREND_DOWN,
            format!(
                "Depreciation posted: {} {} ({})",
                asset(asset_id),
                s(p, "period"),
                amount(p)
            ),
        ),
        "asset.disposed" => with_asset(
            icon::TRASH,
            format!(
                "Disposed {} on {} for {}",
                asset(asset_id),
                s(p, "date"),
                p.get("proceeds")
                    .and_then(|v| v.get("amount"))
                    .and_then(|v| v.as_str())
                    .and_then(|a| a.parse().ok())
                    .map(fmt::dollars)
                    .unwrap_or_default()
            ),
        ),
        "schedule.registered" => ev(
            icon::CALENDAR,
            format!("Schedule: {}", s(p, "description")),
            Some(Route::Company),
        ),
        "schedule.posted" => ev(
            icon::CALENDAR_CHECK,
            "Amortization posted".into(),
            p.get("entry")
                .and_then(|e| e.get("id"))
                .and_then(|v| v.as_u64())
                .map(Route::Entry),
        ),
        "recurring.registered" => ev(
            icon::REPEAT,
            format!("Recurring: {}", s(p, "description")),
            Some(Route::Company),
        ),
        "journal.tail_repaired" => ev(
            icon::WARNING,
            format!("Cut-off journal line set aside ({} bytes)", p["bytes"]),
            None,
        ),
        "journal.format_set" => {
            // The features in force with it (P3: account_renamed,
            // require_tax_category): what an earlier version can't read.
            let features: Vec<&str> = p["features"]
                .as_array()
                .map(|a| a.iter().filter_map(|f| f.as_str()).collect())
                .unwrap_or_default();
            ev(
                icon::FILE_TEXT,
                if features.is_empty() {
                    format!("Journal format {} declared", p["format"])
                } else {
                    format!(
                        "Journal format {} declared, with {}",
                        p["format"],
                        features.join(", ")
                    )
                },
                None,
            )
        }
        "recurring.generated" => ev(
            icon::REPEAT,
            "Recurring draft created".into(),
            entry_id.map(Route::Entry),
        ),
        "fx.rate_set" => ev(
            icon::CURRENCY_CIRCLE_DOLLAR,
            format!("{} rate set", s(p, "currency")),
            None,
        ),
        "payroll.run" => ev(
            icon::USERS,
            "Payroll run".into(),
            entry_id.map(Route::Entry),
        ),
        "payroll.taxes_remitted" => ev(
            icon::BANK,
            format!(
                "Remitted payroll taxes ({})",
                fmt::dollars(s(p, "total").parse().unwrap_or_default())
            ),
            entry_id.map(Route::Entry),
        ),
        "entry.no_party" => ev(
            icon::USER_MINUS,
            format!(
                "Entry #{} marked as having no counterparty: {}",
                entry_id.unwrap_or(0),
                s(p, "reason")
            ),
            entry_id.map(Route::Entry),
        ),
        "line.cleared" => ev(
            icon::CHECK_SQUARE,
            format!("Cleared a line of #{}", entry_id.unwrap_or(0)),
            entry_id.map(Route::Entry),
        ),
        // The books' record of how they are kept (journal format 2).
        "settings.set" => {
            let keys: Vec<&str> = p["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c.get("key").and_then(|k| k.as_str()))
                .collect();
            let by = s(&p["source"], "decided_by");
            ev(
                icon::GEAR_SIX,
                if keys.is_empty() {
                    format!("Settings moved into the books ({by})")
                } else {
                    format!("Settings changed: {} ({by})", keys.join(", "))
                },
                Some(Route::Company),
            )
        }
        "convention.set" => ev(
            icon::SCROLL,
            format!(
                "Convention: {} ({}, {})",
                s(p, "title"),
                s(p, "kind").replace('_', " "),
                s(&p["source"], "decided_by")
            ),
            Some(Route::Company),
        ),
        "convention.retired" => ev(
            icon::SCROLL,
            format!("Convention {} retired: {}", s(p, "id"), s(p, "reason")),
            Some(Route::Company),
        ),
        "decision.recorded" => ev(
            icon::GAVEL,
            format!(
                "Decision: {} ({}, {})",
                s(p, "subject"),
                p["decided_by"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" and "),
                s(p, "decided_on")
            ),
            Some(Route::Company),
        ),
        "open_item.raised" => ev(
            icon::FLAG,
            format!("Open item #{id}: {}", s(p, "title")),
            Some(Route::Company),
        ),
        "open_item.resolved" => ev(
            icon::CHECK_CIRCLE,
            format!("Open item #{id} resolved: {}", s(p, "resolution")),
            Some(Route::Company),
        ),
        "entry.approved" => ev(
            icon::SEAL_CHECK,
            format!(
                "Entry #{} approved by {}",
                entry_id.unwrap_or(0),
                s(p, "approver")
            ),
            entry_id.map(Route::Entry),
        ),
        other => ev(
            icon::DOT,
            other.to_string(),
            entry_refs(line).first().copied().map(Route::Entry),
        ),
    }
}

/// Like [`describe`], but every event gets a sentence.
pub fn describe_any(b: &Books, line: &JournalLine) -> Event {
    if let Some(e) = describe(b, line) {
        return e;
    }
    let p = &line.payload;
    let id = p
        .get("id")
        .and_then(|v| v.as_u64())
        .or_else(|| p.get("entry_id").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    match line.line_type.as_str() {
        "entry.created" => Event {
            icon: icon::PENCIL_SIMPLE,
            text: format!("Drafted #{id} {}", s(p, "description")),
            route: Some(Route::Entry(id)),
            also: None,
        },
        "entry.line_added" => {
            let amount = p
                .get("amount")
                .and_then(|a| a.get("amount").or(Some(a)))
                .and_then(|v| v.as_str())
                .and_then(|v| v.parse().ok())
                .map(fmt::money)
                .unwrap_or_default();
            Event {
                icon: icon::PLUS,
                text: format!(
                    "Line on #{id}: {} {} {amount}",
                    s(p, "side"),
                    b.account_label(
                        p.get("account_id")
                            .or_else(|| p.get("account"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0)
                    ),
                ),
                route: Some(Route::Entry(id)),
                also: None,
            }
        }
        "entry.line_class_set" => {
            let class = p
                .get("class_id")
                .and_then(|v| v.as_u64())
                .and_then(|c| b.ledger.get_class(c as u32))
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "no class".into());
            Event {
                icon: icon::TAG,
                text: format!(
                    "Line {} of #{id} tagged {class}",
                    p.get("line_index")
                        .and_then(|v| v.as_u64())
                        .map_or(0, |i| i + 1)
                ),
                route: Some(Route::Entry(id)),
                also: None,
            }
        }
        t => Event {
            icon: icon::DOT,
            text: t.to_string(),
            route: None,
            also: None,
        },
    }
}

pub fn show(ui: &mut Ui, b: &Books, selected: Option<u64>, shell: &mut Shell<'_>) {
    if let Some(seq) = selected
        && let Some(line) = b.journal.get(seq.saturating_sub(1) as usize)
    {
        super::split(
            ui,
            "journal",
            480.0,
            shell,
            |ui, shell| list(ui, b, selected, shell),
            |ui, shell| detail(ui, b, line, shell),
        );
    } else {
        list(ui, b, None, shell);
    }
}

fn list(ui: &mut Ui, b: &Books, selected: Option<u64>, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    page(ui, "journal", |ui| {
        widgets::page_header(
            ui,
            "Journal",
            &format!(
                "Every change to the books, in order: {} events, each hash-chained to the one before. Nothing is ever edited or deleted; corrections are new events.",
                b.journal.len()
            ),
        );
        let st = &mut shell.state.journal;
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut st.query)
                    .hint_text(format!("{} Filter events", icon::MAGNIFYING_GLASS))
                    .desired_width(ui.available_width().min(320.0)),
            );
            ui.checkbox(&mut st.hide_steps, "Hide entry-building steps");
        });
        ui.add_space(10.0);
        let q = st.query.to_lowercase();
        let hide = st.hide_steps;
        let rows: Vec<(&JournalLine, Event)> = b
            .journal
            .iter()
            .rev()
            .filter(|l| !(hide && describe(b, l).is_none()))
            .map(|l| (l, describe_any(b, l)))
            .filter(|(l, e)| {
                q.is_empty() || e.text.to_lowercase().contains(&q) || l.line_type.contains(&q)
            })
            .collect();
        widgets::card(ui, |ui| {
            let row_h = 34.0;
            egui::ScrollArea::vertical()
                .id_salt("journal_rows")
                .max_height(ui.available_height().max(400.0))
                .show_rows(ui, row_h, rows.len(), |ui, range| {
                    for (line, ev) in &rows[range] {
                        let (rect, resp) = ui
                            .allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                        let on = selected == Some(line.seq);
                        let fresh = line.seq > b.fresh_from_seq && shell.since_update < 8.0;
                        if on {
                            ui.painter()
                                .rect_filled(rect, CornerRadius::same(6), p.selected);
                        } else if fresh {
                            ui.painter()
                                .rect_filled(rect, CornerRadius::same(6), p.new_highlight);
                        } else if resp.hovered() {
                            ui.painter()
                                .rect_filled(rect, CornerRadius::same(6), p.hover);
                        }
                        let painter = ui.painter_at(rect);
                        let y = rect.center().y;
                        painter.text(
                            egui::pos2(rect.left() + 8.0, y),
                            egui::Align2::LEFT_CENTER,
                            format!("{}", line.seq),
                            theme::regular(12.0),
                            p.faint,
                        );
                        painter.text(
                            egui::pos2(rect.left() + 56.0, y),
                            egui::Align2::LEFT_CENTER,
                            ev.icon,
                            egui::FontId::proportional(15.0),
                            p.muted,
                        );
                        // The timestamp yields to the event when it's tight.
                        let ts = painter.layout_no_wrap(
                            fmt::timestamp(&line.ts),
                            theme::regular(12.0),
                            p.faint,
                        );
                        let ts_w = if rect.width() - 80.0 - ts.size().x - 24.0 >= 200.0 {
                            let w = ts.size().x;
                            painter.galley(
                                egui::pos2(rect.right() - 8.0 - w, y - ts.size().y / 2.0),
                                ts,
                                p.faint,
                            );
                            w + 24.0
                        } else {
                            8.0
                        };
                        widgets::clipped_text(
                            &painter,
                            egui::pos2(rect.left() + 80.0, y),
                            &ev.text,
                            theme::regular(13.0),
                            p.text,
                            rect.width() - 80.0 - ts_w,
                        );
                        if resp.clicked() {
                            shell.go(Route::JournalEvent(line.seq));
                        }
                    }
                });
        });
    });
}

fn detail(ui: &mut Ui, b: &Books, line: &JournalLine, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let ev = describe_any(b, line);
    egui::ScrollArea::vertical()
        .id_salt("event_detail")
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("Event {}", line.seq))
                    .font(theme::regular(12.5))
                    .color(p.muted),
            );
            ui.label(RichText::new(&ev.text).font(theme::semibold(17.0)));
            ui.add_space(10.0);
            let fact = |ui: &mut Ui, k: &str, v: &str| {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        vec2(90.0, 18.0),
                        egui::Label::new(
                            RichText::new(k).font(theme::regular(12.5)).color(p.muted),
                        ),
                    );
                    ui.add(egui::Label::new(RichText::new(v).font(theme::regular(12.5))).wrap());
                });
            };
            fact(ui, "Type", &line.line_type);
            fact(ui, "Recorded", &fmt::timestamp(&line.ts));
            fact(ui, "Previous", &line.prev);
            let own = ledger_store::journal::sha256_hex(line.to_canonical().as_bytes());
            fact(ui, "This hash", &own);
            for r in ev.route.iter().chain(&ev.also) {
                ui.add_space(6.0);
                if ui
                    .button(format!("{} Open {}", icon::ARROW_SQUARE_OUT, {
                        let d = crate::palette::describe(b, r);
                        if d.is_empty() {
                            r.section().title().to_string()
                        } else {
                            d
                        }
                    }))
                    .clicked()
                {
                    shell.go(r.clone());
                }
            }
            ui.add_space(12.0);
            ui.label(RichText::new("Payload").font(theme::semibold(13.0)));
            ui.add_space(4.0);
            let json = serde_json::to_string_pretty(&line.payload).unwrap_or_default();
            egui::Frame::new()
                .fill(p.hover)
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(json).monospace().size(12.0)).wrap());
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Build, d, find, render};

    /// The journal names an entry it posted by number, description and
    /// amount.
    #[test]
    fn the_journal_names_a_posted_entry() {
        let mut b = Build::new("journal-posted");
        let (cash, capital) = (b.cash, b.capital);
        let e = b.post_as("Opening deposit", d(2026, 1, 2), cash, capital, 100_000);
        let (_dir, books) = b.books();
        let (texts, _) = render(books, Route::Journal, |_| {});
        find(&texts, &format!("Posted #{e} Opening deposit ($1,000.00)"));
    }
}
