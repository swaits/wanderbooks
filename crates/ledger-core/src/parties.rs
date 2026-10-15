//! Parties: everyone the company deals with, in one directory.
//!
//! A party is a vendor, a contractor, a customer, an employee, a member, a
//! payout platform, a tax agency, a bank or card issuer, a landlord, or
//! several of these. Vendor and customer *records* (bills, invoices,
//! payments, aging) stay where they are and link to their party; a party
//! adds who they are (names, facts) and links entries to them.
//!
//! ```text
//! GOAL: answer "who is this, and what has the company done with them?"
//!       for anyone the books touch, without a CRM's sprawl.
//! SOURCE: docs/parties-and-attachments.md; DECISION (Stephen,
//!         2026-10-15); IRS truncated TINs: Treas. Reg. §301.6109-4.
//! INVARIANTS:
//!   1. No party field holds a full taxpayer identification number: any
//!      run of nine digits (single spaces or hyphens between them allowed)
//!      is refused. A W-9's TIN is kept as its last four digits only.
//!   2. W-9 facts need a 1099 role (contractor or landlord); W-4 facts need
//!      the employee role.
//!   3. Names are unique, ignoring case and surrounding space, among parties
//!      not archived.
//!   4. Every vendor and customer record links to exactly one party.
//! ```

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Opaque party handle.
pub type PartyId = u64;

/// What a party is to the company.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Sells to the company (has a vendor record: bills, payments).
    Vendor,
    /// A vendor paid for services who gets a 1099-NEC.
    Contractor,
    /// Buys from the company (has a customer record: invoices).
    Customer,
    /// Works for the company on payroll.
    Employee,
    /// A member (owner) of the company, as in wanderbooks.toml.
    Member,
    /// Pays the company out of sales (an app store, a payment processor).
    PayoutPlatform,
    /// Taxes the company (the IRS, a state).
    TaxAgency,
    /// Holds the company's money or credit (a bank, a card issuer).
    FinancialInstitution,
    /// Rents property to the company (gets a 1099-MISC for rent).
    Landlord,
}

impl Role {
    /// Every role, in display order.
    pub const ALL: [Role; 9] = [
        Role::Vendor,
        Role::Contractor,
        Role::Customer,
        Role::Employee,
        Role::Member,
        Role::PayoutPlatform,
        Role::TaxAgency,
        Role::FinancialInstitution,
        Role::Landlord,
    ];

    /// Its wire name, e.g. `payout_platform`.
    pub fn key(self) -> &'static str {
        match self {
            Role::Vendor => "vendor",
            Role::Contractor => "contractor",
            Role::Customer => "customer",
            Role::Employee => "employee",
            Role::Member => "member",
            Role::PayoutPlatform => "payout_platform",
            Role::TaxAgency => "tax_agency",
            Role::FinancialInstitution => "financial_institution",
            Role::Landlord => "landlord",
        }
    }

    /// The role named `key`.
    pub fn parse(key: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.key() == key)
    }

    /// Whether this role files a 1099 (and so may carry W-9 facts).
    pub fn gets_1099(self) -> bool {
        matches!(self, Role::Contractor | Role::Landlord)
    }
}

/// What a W-9 said, without the number itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct W9Facts {
    /// When it was received.
    pub received: NaiveDate,
    /// Line 3 federal tax classification, e.g. "individual/sole proprietor".
    pub entity_type: String,
    /// Line 4 exempt payee code, if any.
    #[serde(default)]
    pub exempt_payee_code: Option<String>,
    /// Line 4 FATCA exemption code, if any.
    #[serde(default)]
    pub fatca_code: Option<String>,
    /// The TIN's last four digits (exactly four digits).
    pub tin_last4: String,
    /// Where the form itself is kept, e.g. "wanderbooks vault" or "CPA portal".
    pub kept_at: String,
    /// SHA-256 of the vault record holding the full TIN, when one exists:
    /// proves the vault held it without the books holding it.
    #[serde(default)]
    pub vault_record: Option<String>,
}

/// What a W-4 said, without the SSN.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct W4Facts {
    /// When it was received.
    pub received: NaiveDate,
    /// The work state, two letters.
    pub state: String,
    /// Where the form itself is kept.
    pub kept_at: String,
}

/// One party.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Party {
    /// Opaque handle.
    pub id: PartyId,
    /// Legal or trade name, as on documents.
    pub name: String,
    /// A shorter name to show, if any.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Roles set by hand (vendor and customer also follow their records).
    pub roles: BTreeSet<Role>,
    /// Website.
    #[serde(default)]
    pub website: Option<String>,
    /// Business address.
    #[serde(default)]
    pub address: Option<String>,
    /// Notes.
    #[serde(default)]
    pub notes: Option<String>,
    /// No longer dealt with; history stays.
    #[serde(default)]
    pub archived: bool,
    /// W-9 facts (contractor, landlord).
    #[serde(default)]
    pub w9: Option<W9Facts>,
    /// W-4 facts (employee).
    #[serde(default)]
    pub w4: Option<W4Facts>,
    /// Its vendor record, if it bills the company.
    #[serde(default)]
    pub vendor: Option<crate::VendorId>,
    /// Its customer record, if the company invoices it.
    #[serde(default)]
    pub customer: Option<crate::CustomerId>,
}

impl Party {
    /// The name to show: the display name, else the name.
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }

    /// Its roles: the ones set by hand, plus vendor and customer when it
    /// has those records.
    pub fn all_roles(&self) -> BTreeSet<Role> {
        let mut r = self.roles.clone();
        if self.vendor.is_some() {
            r.insert(Role::Vendor);
        }
        if self.customer.is_some() {
            r.insert(Role::Customer);
        }
        r
    }
}

/// The key names are compared by: case and surrounding space ignored.
pub fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Whether `text` holds a run of nine digits, with at most one space or
/// hyphen between neighbors (an SSN, EIN, or ITIN however it's written).
/// Any Unicode numeral counts as a digit (full-width, Arabic-Indic, ...),
/// so a number can't slip through in another script.
pub fn has_full_tin(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_numeric() {
            let (mut digits, mut j) = (1, i + 1);
            while j < chars.len() && digits < 9 {
                if chars[j].is_numeric() {
                    digits += 1;
                    j += 1;
                } else if matches!(chars[j], ' ' | '-')
                    && chars.get(j + 1).is_some_and(|c| c.is_numeric())
                {
                    j += 1;
                } else {
                    break;
                }
            }
            if digits >= 9 {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Check one party text field: not a full TIN, at most 500 characters, no
/// control characters except newline (addresses and notes run several
/// lines). Returns why not.
pub fn check_text(field: &str, text: &str) -> Result<(), String> {
    if has_full_tin(text) {
        return Err(format!(
            "{field} looks like it holds a full taxpayer identification number; \
             the books keep only its last four digits (the full number belongs in the vault)"
        ));
    }
    if text.chars().count() > 500 {
        return Err(format!("{field} is at most 500 characters"));
    }
    if text.chars().any(|c| c.is_control() && c != '\n') {
        return Err(format!("{field} can't contain control characters"));
    }
    Ok(())
}

impl W9Facts {
    /// Check the facts' form. Returns why not.
    pub fn check(&self) -> Result<(), String> {
        if self.tin_last4.len() != 4 || !self.tin_last4.chars().all(|c| c.is_ascii_digit()) {
            return Err("tin_last4 is exactly four digits".into());
        }
        if self.entity_type.trim().is_empty() {
            return Err("a W-9's entity type (line 3) is required".into());
        }
        if self.kept_at.trim().is_empty() {
            return Err("say where the W-9 itself is kept".into());
        }
        for (f, t) in [
            ("entity_type", Some(self.entity_type.as_str())),
            ("exempt_payee_code", self.exempt_payee_code.as_deref()),
            ("fatca_code", self.fatca_code.as_deref()),
            ("kept_at", Some(self.kept_at.as_str())),
        ] {
            if let Some(t) = t {
                check_text(f, t)?;
            }
        }
        if let Some(h) = &self.vault_record
            && (h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()))
        {
            return Err("vault_record is a SHA-256 in hex".into());
        }
        Ok(())
    }
}

impl W4Facts {
    /// Check the facts' form. Returns why not.
    pub fn check(&self) -> Result<(), String> {
        if self.state.len() != 2 || !self.state.chars().all(|c| c.is_ascii_uppercase()) {
            return Err("state is two capital letters, e.g. UT".into());
        }
        if self.kept_at.trim().is_empty() {
            return Err("say where the W-4 itself is kept".into());
        }
        check_text("kept_at", &self.kept_at)
    }
}

/// One fact of a party to change ([`crate::Ledger::update_party`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "field", content = "value")]
pub enum PartyField {
    /// The legal or trade name.
    Name(String),
    /// The display name (`None` clears it).
    DisplayName(Option<String>),
    /// The website.
    Website(Option<String>),
    /// The business address.
    Address(Option<String>),
    /// Notes.
    Notes(Option<String>),
    /// Archived (no longer dealt with) or not.
    Archived(bool),
}

/// What the books did with one party over a span of dates
/// ([`activity`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    /// The party's entries, drafts included, by id.
    pub entries: BTreeSet<crate::EntryId>,
    /// Paid to it in the span (net of refunds and reversals of payments).
    pub paid: rust_decimal::Decimal,
    /// Received from it in the span (net of reversals of receipts).
    pub received: rust_decimal::Decimal,
}

/// The accounts [`entry_flow`] reads money from: the cash accounts (every
/// asset account but receivables when `cash` is empty), and the
/// liabilities other than payables and the payroll withholdings (cards,
/// what members paid for the company and are owed back, loans).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoneyAccounts {
    /// Cash: bank, savings, cash on hand.
    pub cash: std::collections::HashSet<crate::AccountId>,
    /// Liabilities that pay for the company or are paid down.
    pub liabilities: std::collections::HashSet<crate::AccountId>,
}

/// The [`MoneyAccounts`] of `ledger`, with `cash` the books' cash accounts.
pub fn money_accounts(ledger: &crate::Ledger, cash: &[crate::AccountId]) -> MoneyAccounts {
    use crate::AccountType;
    let not_money: std::collections::HashSet<crate::AccountId> = crate::PAYROLL[2..]
        .iter()
        .chain([&crate::PAYABLES, &crate::RECEIVABLES])
        .filter_map(|c| ledger.role_account(c))
        .collect();
    let mut out = MoneyAccounts::default();
    for a in ledger.chart().all().filter(|a| !not_money.contains(&a.id)) {
        match a.account_type {
            AccountType::Asset if cash.is_empty() || cash.contains(&a.id) => {
                out.cash.insert(a.id);
            }
            AccountType::Liability => {
                out.liabilities.insert(a.id);
            }
            _ => {}
        }
    }
    out
}

/// What one entry moved with its counterparty ([`entry_flow`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryFlow {
    /// Paid to the counterparty (positive) or received from it (negative).
    pub net: rust_decimal::Decimal,
    /// What for, by account (debits − credits): positive is what the money
    /// paid for, negative where money received came from. Sums to `net`.
    pub what_for: Vec<(crate::AccountId, rust_decimal::Decimal)>,
}

/// The money one entry moved with its counterparty, and what for.
///
/// ```text
/// GOAL: "how much did we pay them" right however it was paid: from the
///       bank, on a card, by a member for the company, or by paying down
///       what the company owes them (a reimbursement, a card payment).
/// SOURCE: DECISION (Stephen, 2026-10-15), after a reimbursement to the
///         members netted to zero under the earlier rule (cash and
///         liabilities alike as money).
/// DEFINITIONS (over the entry's lines, in `money`):
///   cash_cr, cash_dr: credits and debits on cash accounts;
///   liab_cr, liab_dr: credits and debits on money liabilities;
///   down = min(liab_dr, cash_cr): cash paying down a liability (its
///     creditor is paid; the liability is what for);
///   up = min(liab_cr, cash_dr): a liability raising cash (borrowing: the
///     lender pays the company).
/// INVARIANTS:
///   1. net = (cash_cr − cash_dr) + (liab_cr − liab_dr) + down − up.
///   2. what_for: every line not on a money account (debits − credits),
///      plus `down` on the debited liabilities and −`up` on the credited
///      ones (in line order), zero sums left out, accounts merged; its sum
///      is net exactly, because the entry balances.
/// SCENARIOS:
///   1. Expense from the bank, on a card, or paid by a member for the
///      company (Dr expense / Cr 1000, 2050, or 2100): net is the amount.
///   2. Reimbursing members (Dr 2100 / Cr 1000) or paying a card (Dr 2050 /
///      Cr 1000): net is the amount, what for is the liability.
///   3. A loan (Dr 1000 / Cr loan): net is minus the amount (received).
///   4. A refund (Dr 1000 or Dr card / Cr expense): negative net.
///   5. A transfer between the company's own cash accounts: zero.
///   6. A member contributing property (Dr equipment / Cr capital): zero.
/// ```
pub fn entry_flow(entry: &crate::JournalEntry, money: &MoneyAccounts) -> EntryFlow {
    use crate::Side;
    use rust_decimal::Decimal;
    let (mut cash_cr, mut cash_dr, mut liab_cr, mut liab_dr) =
        (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    for l in entry.lines() {
        let a = l.amount.amount;
        match (
            money.cash.contains(&l.account),
            money.liabilities.contains(&l.account),
            l.side,
        ) {
            (true, _, Side::Credit) => cash_cr += a,
            (true, _, Side::Debit) => cash_dr += a,
            (false, true, Side::Credit) => liab_cr += a,
            (false, true, Side::Debit) => liab_dr += a,
            _ => {}
        }
    }
    let down = liab_dr.min(cash_cr);
    let up = liab_cr.min(cash_dr);
    let net = (cash_cr - cash_dr) + (liab_cr - liab_dr) + down - up;
    let mut by: std::collections::BTreeMap<crate::AccountId, Decimal> = Default::default();
    let (mut down_left, mut up_left) = (down, up);
    let mut order: Vec<crate::AccountId> = Vec::new();
    for l in entry.lines() {
        let a = l.amount.amount;
        let part = if money.cash.contains(&l.account) {
            Decimal::ZERO
        } else if money.liabilities.contains(&l.account) {
            match l.side {
                Side::Debit => {
                    let take = a.min(down_left);
                    down_left -= take;
                    take
                }
                Side::Credit => {
                    let take = a.min(up_left);
                    up_left -= take;
                    -take
                }
            }
        } else {
            match l.side {
                Side::Debit => a,
                Side::Credit => -a,
            }
        };
        if !part.is_zero() {
            if !by.contains_key(&l.account) {
                order.push(l.account);
            }
            *by.entry(l.account).or_default() += part;
        }
    }
    EntryFlow {
        net,
        what_for: order
            .into_iter()
            .map(|a| (a, by[&a]))
            .filter(|(_, v)| !v.is_zero())
            .collect(),
    }
}

/// One party's entries, and the money paid to and received from it in
/// `start..=end`.
///
/// ```text
/// GOAL: one answer to "what did we pay them this year?" for the MCP
///       server and the viewer alike.
/// SOURCE: docs/parties-and-attachments.md (Parties); DECISION (Stephen,
///         2026-10-15).
/// INVARIANTS:
///   1. entries: every entry Ledger::entry_party names the party
///      (hand tags, vendor tags, bills and their payments, invoices and
///      their payments, and reversals of any of these carry it).
///   2. Each posted entry dated in the span counts its entry_flow net:
///      positive is a payment, negative a receipt. A reversal counts on
///      its original's side, so it subtracts.
///   3. A card charge counts as paid (the company paid them, by card); a
///      bill doesn't until it is paid. Not the 1099 amount: that is
///      form1099::vendor_1099_report_with.
/// ```
pub fn activity(
    ledger: &crate::Ledger,
    party: PartyId,
    (start, end): (NaiveDate, NaiveDate),
    cash: &[crate::AccountId],
) -> Activity {
    use rust_decimal::Decimal;
    let entries: BTreeSet<crate::EntryId> = (0..ledger.next_entry_id())
        .filter(|&id| ledger.entry_party(id) == Some(party))
        .collect();
    let money = money_accounts(ledger, cash);
    let net_out = |e: &crate::JournalEntry| entry_flow(e, &money).net;
    let (mut paid, mut received) = (Decimal::ZERO, Decimal::ZERO);
    for &id in &entries {
        let Some(e) = ledger.get_entry(id) else {
            continue;
        };
        if e.status() != crate::EntryStatus::Posted || e.date() < start || e.date() > end {
            continue;
        }
        let net = net_out(e);
        let side = match e.reversal_of().and_then(|o| ledger.get_entry(o)) {
            Some(original) => net_out(original),
            None => net,
        };
        if side > Decimal::ZERO {
            paid += net;
        } else {
            received -= net;
        }
    }
    Activity {
        entries,
        paid,
        received,
    }
}
