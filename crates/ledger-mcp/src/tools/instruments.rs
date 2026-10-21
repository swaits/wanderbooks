//! 1099 tracking through the server: payment methods
//! (`books.add_instrument`, `books.instruments`,
//! `books.set_entry_instrument`), and the 1099 decision's shape in and out
//! (`books.set_vendor_1099`, `books.list_vendors`, `books.vendor_1099`;
//! plan P3, defects 12, 18 and 23; D-40).
//!
//! ```text
//! GOAL: the agent records how each payment was made and why each vendor
//!       does or doesn't get a 1099, and the worksheet never guesses.
//! SOURCE: docs/spec/rules.md B-19 (Treas. Reg. §1.6041-1(a)(1)(iv),
//!         §1.6041-3; sources/irs-1099-payment-method.md); plan/served.md
//!         4.9; DECISION D-40 (owner, 2026-10-20).
//! INVARIANTS:
//!   1. A 1099 decision names who made it (decided_by; decided_on
//!      defaults to today in the company's time zone). "none" is refused:
//!      no 1099 is not_reportable with its reason, or undetermined.
//!   2. not_reportable takes a reason_kind and a reason, and other takes
//!      a citation too; no other decision takes either.
//!   3. tin_on_file omitted keeps the W-9 flag; form_1099 omitted changes
//!      only it, keeping the decision and who made it (P3 review R8).
//!   4. A change that reaches payments in a closed period, or in a year
//!      whose 1099s were due (January 31 of the next year), warns.
//!   5. A payment method is named by id or label; a number that is one
//!      method's id and another's label is refused.
//! ```

use super::*;
use ledger_core::form1099::{Applied, Dated1099, Decision1099, ExemptReason, Form1099};
use ledger_core::{InstrumentId, InstrumentKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ----------------------------------------------------------------------
// Parameters and results
// ----------------------------------------------------------------------

/// Parameters for `books.add_instrument`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddInstrumentParams {
    /// bank (the company's checks, ACH, wires, bill-pay), business_card (a
    /// company card the payee charged), member_card (a member's personal
    /// card the payee charged), member_cash_or_check (a member's own cash,
    /// check, ACH, Zelle, or a friends-and-family transfer), or platform (a
    /// payment the payee received as a seller through PayPal or Venmo goods
    /// and services).
    pub kind: String,
    /// Its name ("Harbor Bank checking", "Riley's Visa"), unique.
    pub label: String,
    /// The last four digits of its card or account number (never more).
    pub last4: Option<String>,
    /// Whose it is (a member's card): party id or name. Informational.
    pub owner_party: Option<String>,
    /// The account it pays through (id or code): the bank account, the
    /// card's liability, or the members' financing account (Due to
    /// Members) for a member's own card or cash.
    pub account: String,
}

/// A payment method.
#[derive(Debug, Serialize, JsonSchema)]
pub struct InstrumentOut {
    /// Its id.
    pub id: u64,
    /// Its kind (as books.add_instrument takes it).
    pub kind: String,
    /// Whether a payment made with it is a card or payment-network
    /// payment: excluded from the company's 1099s (Form 1099-K's).
    pub card_or_network: bool,
    /// Its name.
    pub label: String,
    /// Its last four digits, if recorded.
    pub last4: Option<String>,
    /// Whose it is: party id, if recorded.
    pub owner_party: Option<u64>,
    /// The account it pays through: id.
    pub account_id: u64,
    /// That account's code.
    pub account: String,
}

/// Result of `books.add_instrument`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AddInstrumentOut {
    /// The payment method.
    #[serde(flatten)]
    pub instrument: InstrumentOut,
    /// Things to check, if any.
    pub warnings: Vec<String>,
}

/// Parameters for `books.instruments`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct InstrumentsParams {}

/// Result of `books.instruments`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct InstrumentsOut {
    /// Every payment method, by id.
    pub instruments: Vec<InstrumentOut>,
}

/// Parameters for `books.set_entry_instrument`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetEntryInstrumentParams {
    /// Entry id (draft or posted).
    pub entry_id: u64,
    /// The payment method (id or label, books.instruments); omit to clear.
    pub instrument: Option<String>,
}

/// Result of `books.set_entry_instrument`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SetEntryInstrumentOut {
    /// The entry, as books.get_entry shows it.
    #[serde(flatten)]
    pub entry: GetEntryOut,
    /// Whether anything changed (false: the entry and its reversals were
    /// tagged so already).
    pub changed: bool,
    /// Payments the change reaches in a closed period, or in a year whose
    /// 1099s were due: tell the human.
    pub warnings: Vec<String>,
}

// ----------------------------------------------------------------------
// Shapes
// ----------------------------------------------------------------------

/// A dated decision as the server shows it (none: never decided).
pub(super) fn decision_out(d: Option<&Dated1099>) -> Decision1099Out {
    let undecided = Decision1099::Undetermined;
    let decision = d.map_or(&undecided, |d| &d.decision);
    let (reason_kind, reason, authority) = match decision {
        Decision1099::NotReportable { kind, reason } => (
            Some(kind.key().to_string()),
            Some(reason.clone()),
            kind.citation().map(str::to_string),
        ),
        _ => (None, None, None),
    };
    let source = d.and_then(|d| d.source.as_ref());
    Decision1099Out {
        value: decision.key().to_string(),
        reason_kind,
        reason,
        authority,
        effective: d.and_then(|d| d.effective).map(|e| e.to_string()),
        decided_by: source.map(|s| s.decided_by.clone()),
        decided_on: source.map(|s| s.decided_on.to_string()),
        citation: source.and_then(|s| s.citation.clone()),
        note: source.and_then(|s| s.note.clone()),
    }
}

/// A vendor as the server shows it: its decision in force today, and every
/// one recorded.
pub(super) fn vendor_out(store: &Store, v: &ledger_core::Vendor) -> VendorOut {
    let now = ledger_core::form1099::in_force(&v.decisions_1099, today());
    VendorOut {
        id: v.id,
        name: v.name.clone(),
        form_1099: form_1099_label(now.and_then(|d| d.decision.form())),
        decision_1099: decision_out(now),
        decisions_1099: v
            .decisions_1099
            .iter()
            .map(|d| decision_out(Some(d)))
            .collect(),
        tin_on_file: v.tin_on_file,
        party_id: store.ledger().party_of_vendor(v.id),
    }
}

fn instrument_out(store: &Store, i: &ledger_core::Instrument) -> InstrumentOut {
    InstrumentOut {
        id: i.id,
        kind: i.kind.key().to_string(),
        card_or_network: i.kind.is_card_or_network(),
        label: i.label.clone(),
        last4: i.last4.clone(),
        owner_party: i.owner_party,
        account_id: i.account,
        account: store
            .ledger()
            .chart()
            .get(i.account)
            .map(|a| a.code.clone())
            .unwrap_or_default(),
    }
}

/// Resolve a payment method by id or label (INVARIANT 5).
pub(super) fn resolve_instrument(store: &Store, r: &str) -> Result<InstrumentId, String> {
    let ledger = store.ledger();
    let by_id = r
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|id| ledger.get_instrument(*id).is_some());
    let by_label = ledger.instrument_by_label(r).map(|i| i.id);
    match (by_id, by_label) {
        (Some(i), Some(l)) if i != l => Err(format!(
            "payment method {r:?} is ambiguous: it is method {i}'s id and method {l}'s label; pass the id"
        )),
        (Some(i), _) | (None, Some(i)) => Ok(i),
        (None, None) => Err(format!(
            "unknown payment method {r:?}: use an id or label from books.instruments (books.add_instrument adds one)"
        )),
    }
}

/// The decision `books.set_vendor_1099` was given, or none when it
/// changes only the W-9 flag (INVARIANTS 1-3).
pub(super) fn decision_in(p: &SetVendor1099Params) -> Result<Option<Dated1099>, String> {
    let Some(value) = p.form_1099.as_deref() else {
        if p.reason_kind.is_some() || p.reason.is_some() || p.effective.is_some() {
            return Err(
                "reason_kind, reason and effective go with form_1099 (the decision they describe)"
                    .into(),
            );
        }
        if p.tin_on_file.is_none() {
            return Err(
                "nothing to change: pass form_1099 (the decision, with decided_by) or tin_on_file"
                    .into(),
            );
        }
        return Ok(None);
    };
    let value = value.trim().to_ascii_lowercase();
    let decision = match value.as_str() {
        "not_reportable" => {
            let kind = p.reason_kind.as_deref().ok_or(
                "not_reportable needs reason_kind: corporation, tax_exempt, united_states, state, foreign_government, foreign_person, merchandise, employee, or other",
            )?;
            let kind = ExemptReason::parse(kind).ok_or_else(|| {
                format!(
                    "reason_kind {kind:?}: expected corporation, tax_exempt, united_states, state, foreign_government, foreign_person, merchandise, employee, or other (paid by card is not a reason: card payments are excluded payment by payment)"
                )
            })?;
            let reason = p.reason.as_deref().filter(|r| !r.trim().is_empty()).ok_or(
                "not_reportable needs reason: the facts, in words (\"a corporation by name\")",
            )?;
            if kind == ExemptReason::Other
                && p.citation.as_deref().is_none_or(|c| c.trim().is_empty())
            {
                return Err(
                    "reason_kind other needs a citation: the rule or document that says no 1099 is due"
                        .into(),
                );
            }
            Decision1099::not_reportable(kind, reason)?
        }
        "none" | "" | "no" | "no_1099" => {
            return Err(format!(
                "form_1099 {value:?}: say not_reportable with its reason_kind and reason (a corporation, the United States or a state, a payee selling only merchandise, ...), or undetermined. Paid by card is not a reason: card payments are excluded payment by payment"
            ));
        }
        "undetermined" => Decision1099::Undetermined,
        other => Decision1099::Files(Form1099::parse(other).ok_or_else(|| {
            format!("form_1099 {other:?}: expected nec, misc, not_reportable, or undetermined")
        })?),
    };
    if !matches!(decision, Decision1099::NotReportable { .. })
        && (p.reason_kind.is_some() || p.reason.is_some())
    {
        return Err(format!(
            "reason_kind and reason go with not_reportable, not {value}"
        ));
    }
    let decided_by = p
        .decided_by
        .as_deref()
        .ok_or("decided_by is required with form_1099: who decided (a person's name)")?;
    let source = ledger_core::settings::Source {
        decided_by: decided_by.trim().to_string(),
        decided_on: match &p.decided_on {
            Some(d) => parse_date(d)?,
            None => today(),
        },
        // A blank citation or note is none.
        citation: p
            .citation
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string),
        note: p
            .note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string),
    };
    Ok(Some(Dated1099 {
        effective: p.effective.as_deref().map(parse_date).transpose()?,
        decision,
        source: Some(source),
    }))
}

/// What a change reaching `payments` (entry, date) warns of (INVARIANT 4):
/// one line per calendar year with a payment in a closed period or whose
/// 1099s were due by `today`.
pub(super) fn filed_year_warnings(
    ledger: &Ledger,
    what: &str,
    payments: &[(ledger_core::EntryId, NaiveDate)],
    today: NaiveDate,
) -> Vec<String> {
    let mut years: std::collections::BTreeMap<i32, (Vec<ledger_core::EntryId>, bool)> =
        Default::default();
    for &(e, d) in payments {
        let year = chrono::Datelike::year(&d);
        let closed = ledger.closed_period_containing(d).is_some();
        let due = NaiveDate::from_ymd_opt(year + 1, 1, 31).is_none_or(|due| today > due);
        if closed || due {
            let y = years.entry(year).or_default();
            y.0.push(e);
            y.1 |= closed;
        }
    }
    years
        .into_iter()
        .map(|(year, (mut ids, closed))| {
            ids.sort_unstable();
            ids.dedup();
            let ids = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ");
            let when = if closed {
                ", in a closed period".to_string()
            } else {
                format!(", a year whose 1099s were due January 31, {}", year + 1)
            };
            format!(
                "This changes {what} for {year} (entries {ids}){when}: if a {year} 1099 was filed, a corrected one may be needed. Tell the human and the CPA."
            )
        })
        .collect()
}

/// `books.set_vendor_1099`'s "changed".
pub(super) fn applied_key(a: Applied) -> &'static str {
    match a {
        Applied::Set => "set",
        Applied::Removed => "removed",
        Applied::Unchanged => "unchanged",
    }
}

#[tool_router(router = instruments_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.add_instrument",
        title = "Add a payment method",
        description = "Record a way the company pays, once, so the 1099 worksheet tells card payments (Form 1099-K's, never the company's 1099) from checks. kind: bank (company checks, ACH, wires, bill-pay); business_card or member_card (a company or a member's card the payee charged); member_cash_or_check (a member's own cash, check, ACH, Zelle, or friends-and-family transfer, even card-funded); platform (paid to the payee as a seller through PayPal or Venmo goods and services; never ACH, bill-pay, Zelle or friends-and-family). account: what it pays through (the bank, the card's liability, Due to Members for a member's own). last4 only. Then tag payments with books.set_entry_instrument.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_instrument(
        &self,
        params: Parameters<AddInstrumentParams>,
    ) -> Result<Json<AddInstrumentOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let kind = InstrumentKind::parse(&p.kind).ok_or_else(|| {
            format!(
                "kind {:?}: expected bank, business_card, member_card, member_cash_or_check, or platform",
                p.kind
            )
        })?;
        let account = resolve_account(&store, &p.account)?;
        let owner = p
            .owner_party
            .as_deref()
            .map(|r| resolve_party(&store, r))
            .transpose()?;
        let last4 = p.last4.as_deref().map(str::trim);
        let id = store
            .add_instrument(kind, &p.label, last4, owner, account)
            .map_err(|e| e.to_string())?;
        let settings = self.settings();
        let code = |a: AccountId| {
            store
                .ledger()
                .chart()
                .get(a)
                .map(|a| a.code.clone())
                .unwrap_or_default()
        };
        let mut warnings = Vec::new();
        if kind == InstrumentKind::BusinessCard && settings.roles.financing.contains(&code(account))
        {
            warnings.push(format!(
                "account {} is one of the books' financing accounts: a company card's liability usually isn't, and while it is, its payments with no method recorded stay undetermined. Check the financing accounts (books.settings).",
                code(account)
            ));
        }
        if kind.is_card_or_network() && settings.roles.cash.contains(&code(account)) {
            warnings.push(format!(
                "account {} is a cash account: only the payments tagged with this method are excluded as card payments; untagged ones from it still count (presumed check, ACH or transfer).",
                code(account)
            ));
        }
        let members = matches!(
            kind,
            InstrumentKind::MemberCard | InstrumentKind::MemberCashOrCheck
        );
        if members
            && !settings.roles.financing.contains(&code(account))
            && !settings.roles.cash.contains(&code(account))
        {
            warnings.push(format!(
                "account {} is not one of the books' financing accounts: the 1099 worksheet still holds what members paid through it undetermined until each payment's method is recorded, but name it in books.financing_accounts (books.set_setting) so the books read it as what members paid for the company.",
                code(account)
            ));
        }
        let i = store.ledger().get_instrument(id).expect("just added");
        Ok(Json(AddInstrumentOut {
            instrument: instrument_out(&store, i),
            warnings,
        }))
    }

    #[tool(
        name = "books.instruments",
        title = "Payment methods",
        description = "The company's payment methods (books.add_instrument), each with its kind, whether its payments are card or payment-network payments (excluded from the company's 1099s), its last four, and the account it pays through. Tag an entry with one by id or label (books.set_entry_instrument, books.draft_entry instrument).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn instruments(
        &self,
        _params: Parameters<InstrumentsParams>,
    ) -> Result<Json<InstrumentsOut>, String> {
        let store = self.lock()?;
        Ok(Json(InstrumentsOut {
            instruments: store
                .ledger()
                .instruments()
                .map(|i| instrument_out(&store, i))
                .collect(),
        }))
    }

    #[tool(
        name = "books.set_entry_instrument",
        title = "Tag an entry's payment method",
        description = "Tag an entry (draft or posted) with how it was paid (a payment method's id or label, books.instruments), or omit instrument to clear it, so the 1099 worksheet can classify it: a card or payment network's payment is excluded (Form 1099-K's), a bank or a member's own cash or check counts. Needed above all for what members paid for the company (their card or their check: undetermined until tagged). Reporting metadata only: amounts never change. An entry and its reversals are tagged together. Warns when it changes a year whose 1099s are due or a closed period.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_entry_instrument(
        &self,
        params: Parameters<SetEntryInstrumentParams>,
    ) -> Result<Json<SetEntryInstrumentOut>, String> {
        let p = params.0;
        let (changed, warnings) = {
            let mut store = self.lock()?;
            let instrument = p
                .instrument
                .as_deref()
                .map(|r| resolve_instrument(&store, r))
                .transpose()?;
            let ledger = store.ledger();
            // The chain's posted payments to a vendor: what the change
            // reaches on its 1099s.
            let reached: Vec<(ledger_core::EntryId, NaiveDate)> = ledger
                .reversal_chain(p.entry_id)
                .into_iter()
                .filter_map(|e| ledger.get_entry(e))
                .filter(|e| {
                    e.status() == ledger_core::EntryStatus::Posted
                        && ledger.entry_vendor(e.id()).is_some()
                        && ledger.entry_instrument(e.id()) != instrument
                })
                .map(|e| (e.id(), e.date()))
                .collect();
            let warnings =
                filed_year_warnings(ledger, "how a vendor's payment counts", &reached, today());
            let changed = store
                .set_entry_instrument(p.entry_id, instrument)
                .map_err(|e| e.to_string())?;
            (changed, if changed { warnings } else { Vec::new() })
        };
        let entry = self
            .get_entry(Parameters(GetEntryParams {
                entry_id: p.entry_id,
            }))
            .await?
            .0;
        Ok(Json(SetEntryInstrumentOut {
            entry,
            changed,
            warnings,
        }))
    }
}
