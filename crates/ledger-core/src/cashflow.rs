//! The statement of cash flows (rules.md B-14), by the direct method: each
//! entry that moved cash, in the section of the accounts it settled, gross;
//! the reconciliation of net income to operating cash as a separate
//! schedule; and the investing and financing activities no cash settled,
//! disclosed apart from the body.
//!
//! The arithmetic is the kernel's ([`kernel::entry_flows`],
//! [`kernel::apply_cash`], [`kernel::constructive`],
//! [`kernel::exchanged`]), proved by Kani at `i64` and modelled in Lean
//! (`cashflow_sections_sum_to_cash_change`, `constructive_nets`,
//! `operating_reconciliation_ties`). This module decides where each line
//! falls, what a member or lender paid for the company, pairs reversals,
//! names the kinds and checks the result foots.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::kernel::{self, EntryFlows, Flow, OnBehalf};
use crate::reports::MemberCapital;
use crate::{Account, AccountId, AccountType, EntryId, JournalEntry, Ledger, LedgerError, Side};

/// A section of the statement of cash flows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CashFlowSection {
    /// Operating activities.
    Operating,
    /// Investing activities.
    Investing,
    /// Financing activities.
    Financing,
}

impl CashFlowSection {
    /// The three, in the statement's order.
    pub const ALL: [CashFlowSection; 3] = [
        CashFlowSection::Operating,
        CashFlowSection::Investing,
        CashFlowSection::Financing,
    ];

    /// "Operating activities", ...
    pub fn label(self) -> &'static str {
        match self {
            CashFlowSection::Operating => "Operating activities",
            CashFlowSection::Investing => "Investing activities",
            CashFlowSection::Financing => "Financing activities",
        }
    }

    /// `"operating"`, as the exports spell it.
    pub fn key(self) -> &'static str {
        match self {
            CashFlowSection::Operating => "operating",
            CashFlowSection::Investing => "investing",
            CashFlowSection::Financing => "financing",
        }
    }
}

/// The classes of operating cash ASC 230-10-45-25 (a)-(g) requires, at a
/// minimum, of a direct-method statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatingClass {
    /// (a) Cash collected from customers.
    FromCustomers,
    /// (b) Interest and dividends received.
    InterestAndDividendsReceived,
    /// (c) Other operating cash receipts.
    OtherReceipts,
    /// (d) Cash paid to employees and other suppliers.
    ToSuppliersAndEmployees,
    /// (e) Interest paid.
    InterestPaid,
    /// (f) Income taxes paid. No tax category marks an entity-level income
    /// tax yet, so nothing is filed here (rules.md B-14 rule 5).
    IncomeTaxesPaid,
    /// (g) Other operating cash payments.
    OtherPayments,
}

impl OperatingClass {
    /// Every class, (a) through (g).
    pub const ALL: [OperatingClass; 7] = [
        OperatingClass::FromCustomers,
        OperatingClass::InterestAndDividendsReceived,
        OperatingClass::OtherReceipts,
        OperatingClass::ToSuppliersAndEmployees,
        OperatingClass::InterestPaid,
        OperatingClass::IncomeTaxesPaid,
        OperatingClass::OtherPayments,
    ];

    /// The caption a printed statement uses.
    pub fn label(self) -> &'static str {
        match self {
            OperatingClass::FromCustomers => "Cash received from customers",
            OperatingClass::InterestAndDividendsReceived => "Interest and dividends received",
            OperatingClass::OtherReceipts => "Other operating cash receipts",
            OperatingClass::ToSuppliersAndEmployees => "Cash paid to suppliers and employees",
            OperatingClass::InterestPaid => "Interest paid",
            OperatingClass::IncomeTaxesPaid => "Income taxes paid",
            OperatingClass::OtherPayments => "Other operating cash payments",
        }
    }

    /// `"from_customers"`, as the exports spell it.
    pub fn key(self) -> &'static str {
        match self {
            OperatingClass::FromCustomers => "from_customers",
            OperatingClass::InterestAndDividendsReceived => "interest_and_dividends_received",
            OperatingClass::OtherReceipts => "other_receipts",
            OperatingClass::ToSuppliersAndEmployees => "to_suppliers_and_employees",
            OperatingClass::InterestPaid => "interest_paid",
            OperatingClass::IncomeTaxesPaid => "income_taxes_paid",
            OperatingClass::OtherPayments => "other_payments",
        }
    }

    fn order(self) -> u8 {
        match self {
            OperatingClass::FromCustomers => 0,
            OperatingClass::InterestAndDividendsReceived => 1,
            OperatingClass::OtherReceipts => 2,
            OperatingClass::ToSuppliersAndEmployees => 3,
            OperatingClass::InterestPaid => 4,
            OperatingClass::IncomeTaxesPaid => 5,
            OperatingClass::OtherPayments => 6,
        }
    }
}

/// Parameters for [`cash_flow`].
#[derive(Clone, Debug)]
pub struct CashFlowParams {
    /// Period start, inclusive.
    pub start: NaiveDate,
    /// Period end, inclusive.
    pub end: NaiveDate,
    /// Accounts that hold cash (asset accounts). Their net change is what
    /// the statement explains.
    pub cash_accounts: Vec<AccountId>,
    /// Liability accounts treated as financing (debt, including what the
    /// owners advanced: ASC 230-10-45-14(b) "other short- or long-term
    /// borrowing"). Equity is always financing, listed or not. Default
    /// empty: other liabilities are operating.
    pub financing_accounts: Vec<AccountId>,
    /// Asset accounts treated as investing: loans the entity makes and
    /// investments it buys (ASC 230-10-20; 230-10-45-13). Registered fixed
    /// assets are investing without being listed. Default empty: other
    /// assets are operating.
    pub investing_accounts: Vec<AccountId>,
    /// The members of a multi-member LLC, so financing names each member's
    /// contributions and distributions (D-35). Empty: equity is named by
    /// account. Shares are not used here.
    pub members: Vec<MemberCapital>,
}

/// One entry's cash in one kind of one section: a line of the statement's
/// body. Positive is cash in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CashFlowLine {
    /// The entry that moved the cash.
    pub entry: EntryId,
    /// Its date.
    pub date: NaiveDate,
    /// `"<description> (#N)"`, `"Reversal of #N — <N's description>"`, or
    /// `"Reinstatement of #A (reverses #B)"`: never derived from the sign.
    pub label: String,
    /// The kind the line is listed under: an operating class's caption
    /// ("Cash paid to suppliers and employees"), or an investing or
    /// financing caption ("Contributions from members — Member A").
    pub kind: String,
    /// The cash, + in.
    pub amount: Decimal,
    /// Of `amount`, what a member or a lender paid or received for the
    /// company with no cash account moving (rules.md B-14 rule 11, D-42):
    /// a constructive receipt or payment. Zero for most lines.
    pub on_behalf: Decimal,
    /// Codes of this kind's accounts the entry moved, sorted.
    pub accounts: Vec<String>,
    /// The entry this one reverses, if any.
    pub reverses: Option<EntryId>,
    /// The operating class (operating lines only).
    pub class: Option<OperatingClass>,
}

/// A kind's total in one section, with the entries listed under it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CashFlowKind {
    /// The caption.
    pub kind: String,
    /// The operating class, for an operating kind.
    pub class: Option<OperatingClass>,
    /// The kind's cash, + in.
    pub amount: Decimal,
    /// Its entries, in the order listed.
    pub entries: Vec<EntryId>,
}

/// An investing or financing activity no cash settled (ASC 230-10-50-3,
/// 50-4), or the part of one cash didn't (50-5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoncashDisclosure {
    /// The entry.
    pub entry: EntryId,
    /// Its date.
    pub date: NaiveDate,
    /// Its label, as a line's.
    pub label: String,
    /// The amount disclosed: the larger of what moved into sections
    /// without cash (`Σ max(part, 0)`) and the exchange inside investing
    /// and financing.
    pub amount: Decimal,
    /// The operating part no cash settled, debit-positive.
    pub operating: Decimal,
    /// The investing part no cash settled, debit-positive.
    pub investing: Decimal,
    /// The financing part no cash settled, debit-positive.
    pub financing: Decimal,
    /// What was exchanged inside investing and financing without cash
    /// (debt converted to equity, one asset for another).
    pub exchanged: Decimal,
    /// The entry it reverses, if any.
    pub reverses: Option<EntryId>,
}

/// What a reconciling item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcilingKind {
    /// Depreciation: an operating expense no cash paid, against
    /// accumulated depreciation.
    Depreciation,
    /// The revenue and expense lines of entries that dispose of or write
    /// down investing assets (gains, losses), whose cash is investing.
    InvestingGainLoss,
    /// Operating movements no cash settled, through one investing or
    /// financing account (interest charged into a loan; a prepaid a member
    /// contributed).
    NoncashThrough,
    /// Other operating movements no cash settled.
    OtherNoncash,
    /// The change in an operating asset or liability.
    OperatingBalanceChange,
}

impl ReconcilingKind {
    /// `"noncash_through"`, as the exports spell it.
    pub fn key(self) -> &'static str {
        match self {
            ReconcilingKind::Depreciation => "depreciation",
            ReconcilingKind::InvestingGainLoss => "investing_gain_loss",
            ReconcilingKind::NoncashThrough => "noncash_through",
            ReconcilingKind::OtherNoncash => "other_noncash",
            ReconcilingKind::OperatingBalanceChange => "operating_balance_change",
        }
    }
}

/// One line of the reconciliation of net income to operating cash.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReconcilingLine {
    /// What it is.
    pub kind: ReconcilingKind,
    /// "Depreciation", "Noncash: through 2500 Bank loan", "Increase
    /// in 1300 Prepaid Expenses", ...
    pub label: String,
    /// Its amount, added to net income.
    pub amount: Decimal,
    /// The account it is about (`NoncashThrough`,
    /// `OperatingBalanceChange`).
    pub account: Option<String>,
    /// The entries behind it, for the entry-based kinds (an entry and its
    /// reversal in the period are left out).
    pub entries: Vec<EntryId>,
}

/// Net income reconciled to net cash from operating activities, the
/// separate schedule a direct-method statement carries (ASC
/// 230-10-45-30).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperatingReconciliation {
    /// The period's net income (closing entries excluded).
    pub net_income: Decimal,
    /// The reconciling items: depreciation, gains and losses on investing
    /// assets, noncash items by account, other noncash items, then the
    /// changes in operating assets and liabilities by code.
    pub lines: Vec<ReconcilingLine>,
    /// Net income plus every line: equal to the operating section.
    pub net_cash_from_operating: Decimal,
}

/// The footing check (rules.md B-14 rule 9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CashFlowFooting {
    /// Operating + investing + financing.
    pub sections_sum: Decimal,
    /// The cash accounts' ending balance less their beginning balance.
    pub net_change: Decimal,
    /// `sections_sum - net_change`: always zero on output.
    pub difference: Decimal,
}

/// Why the statement flags an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CashFlowWarningKind {
    /// A fixed asset or investment bought on an operating payable: its
    /// later payment shows as operating, following how the payable is
    /// classified (D-35).
    AssetOnOperatingPayable,
    /// A fixed asset or investment sold for an amount owed on an operating
    /// receivable: its later collection would show as operating, though
    /// ASC 230-10-45-12(c) makes it investing (D-35: flagged with a fix).
    AssetSoldOnOperatingReceivable,
}

impl CashFlowWarningKind {
    /// `"asset_on_operating_payable"`, as the exports spell it.
    pub fn key(self) -> &'static str {
        match self {
            CashFlowWarningKind::AssetOnOperatingPayable => "asset_on_operating_payable",
            CashFlowWarningKind::AssetSoldOnOperatingReceivable => {
                "asset_sold_on_operating_receivable"
            }
        }
    }
}

/// A note on an entry whose later settlement the books can't classify by
/// themselves. Figures are unaffected.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CashFlowWarning {
    /// The entry.
    pub entry: EntryId,
    /// What it is.
    pub kind: CashFlowWarningKind,
    /// What happened and what to change.
    pub message: String,
}

/// The statement of cash flows, direct method (rules.md B-14).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CashFlowStatement {
    /// Period start, inclusive.
    pub start: NaiveDate,
    /// Period end, inclusive.
    pub end: NaiveDate,
    /// Operating lines, by class, then date and entry.
    pub operating: Vec<CashFlowLine>,
    /// Net cash from operating activities.
    pub cash_from_operations: Decimal,
    /// Investing lines, by kind, then date and entry.
    pub investing: Vec<CashFlowLine>,
    /// Net cash from investing activities.
    pub cash_from_investing: Decimal,
    /// Financing lines, by kind, then date and entry.
    pub financing: Vec<CashFlowLine>,
    /// Net cash from financing activities.
    pub cash_from_financing: Decimal,
    /// Net change in cash: the three sections summed.
    pub net_change_in_cash: Decimal,
    /// The cash accounts' balance at the start.
    pub cash_beginning: Decimal,
    /// Their balance at the end.
    pub cash_ending: Decimal,
    /// Noncash investing and financing activities, by date and entry.
    pub noncash_disclosures: Vec<NoncashDisclosure>,
    /// Net income reconciled to operating cash.
    pub operating_reconciliation: OperatingReconciliation,
    /// The footing check.
    pub reconciles: CashFlowFooting,
    /// Entries whose later settlement the books can't classify.
    pub warnings: Vec<CashFlowWarning>,
}

impl CashFlowStatement {
    /// A section's lines.
    pub fn lines(&self, section: CashFlowSection) -> &[CashFlowLine] {
        match section {
            CashFlowSection::Operating => &self.operating,
            CashFlowSection::Investing => &self.investing,
            CashFlowSection::Financing => &self.financing,
        }
    }

    /// A section's total.
    pub fn total(&self, section: CashFlowSection) -> Decimal {
        match section {
            CashFlowSection::Operating => self.cash_from_operations,
            CashFlowSection::Investing => self.cash_from_investing,
            CashFlowSection::Financing => self.cash_from_financing,
        }
    }

    /// A section's kinds, in the order listed, each with its total and
    /// entries. They sum to the section's total.
    pub fn kinds(&self, section: CashFlowSection) -> Vec<CashFlowKind> {
        let mut out: Vec<CashFlowKind> = Vec::new();
        for l in self.lines(section) {
            match out.last_mut() {
                Some(k) if k.kind == l.kind => {
                    k.amount += l.amount;
                    k.entries.push(l.entry);
                }
                _ => out.push(CashFlowKind {
                    kind: l.kind.clone(),
                    class: l.class,
                    amount: l.amount,
                    entries: vec![l.entry],
                }),
            }
        }
        out
    }

    /// Operating cash by class, (a) through (g), classes with no line left
    /// out.
    pub fn operating_by_class(&self) -> Vec<(OperatingClass, Decimal)> {
        self.kinds(CashFlowSection::Operating)
            .into_iter()
            .filter_map(|k| Some((k.class?, k.amount)))
            .collect()
    }
}

/// Which role list a code comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleList {
    /// `cash_accounts`.
    Cash,
    /// `financing_accounts`.
    Financing,
    /// `investing_accounts`.
    Investing,
}

/// A contradiction among the cash flow account roles (rules.md B-14 rule
/// 2): the statement refuses rather than guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoleProblem {
    /// An account of the wrong type in a list.
    WrongType {
        /// The list.
        list: RoleList,
        /// Its code.
        code: String,
        /// Its name.
        name: String,
        /// Its type.
        found: AccountType,
    },
    /// A code listed twice in one list.
    Repeated {
        /// The list.
        list: RoleList,
        /// The code.
        code: String,
    },
    /// A code in two lists.
    InTwoLists {
        /// The code.
        code: String,
        /// Its name.
        name: String,
        /// One list.
        first: RoleList,
        /// The other.
        second: RoleList,
    },
    /// A registered fixed asset's cost or accumulated-depreciation account
    /// listed as cash.
    RegisteredAssetIsCash {
        /// The code.
        code: String,
        /// Its name.
        name: String,
    },
}

fn an(t: AccountType) -> &'static str {
    match t {
        AccountType::Asset => "an asset",
        AccountType::Liability => "a liability",
        AccountType::Equity => "an equity",
        AccountType::Revenue => "a revenue",
        AccountType::Expense => "an expense",
    }
}

impl RoleProblem {
    /// What is wrong and what to change. `setting` names each list as the
    /// caller knows it ("books.financing_accounts", or "financing_accounts
    /// (from the call or books.financing_accounts)").
    pub fn message(&self, setting: impl Fn(RoleList) -> String) -> String {
        match self {
            RoleProblem::WrongType {
                list,
                code,
                name,
                found,
            } => {
                let want = match list {
                    RoleList::Cash => {
                        "cash accounts are asset accounts (bank, savings, cash on hand)"
                    }
                    RoleList::Investing => {
                        "investing accounts are asset accounts (loans the company makes, investments it buys)"
                    }
                    RoleList::Financing => {
                        "financing accounts are liabilities (equity is always financing)"
                    }
                };
                format!(
                    "{} lists {code} \"{name}\", {} account: {want}; remove it from {}",
                    setting(*list),
                    an(*found),
                    setting(*list)
                )
            }
            RoleProblem::Repeated { list, code } => format!(
                "{} lists {code} twice: list each account once",
                setting(*list)
            ),
            RoleProblem::InTwoLists {
                code,
                name,
                first,
                second,
            } => format!(
                "{code} \"{name}\" is in both {} and {}: an account belongs to one section; remove it from one",
                setting(*first),
                setting(*second)
            ),
            RoleProblem::RegisteredAssetIsCash { code, name } => format!(
                "{} lists {code} \"{name}\", a registered fixed asset's account: equipment is investing, not cash; remove it from {}",
                setting(RoleList::Cash),
                setting(RoleList::Cash)
            ),
        }
    }
}

/// Every contradiction among the cash flow account roles (rules.md B-14
/// rule 2), in a fixed order. Codes the chart lacks are the caller's to
/// report. The one check behind both the statement's refusal and the
/// settings' problems, so the two never drift apart.
///
/// ```text
/// GOAL: the statement's account roles partition the chart, so every
///       account is in exactly one section.
/// SOURCE: rules.md B-14 rule 2 (ASC 230-10-45-10: every cash receipt and
///         payment is investing, financing or operating).
/// INVARIANTS:
///   1. Cash and investing accounts MUST be assets; financing accounts
///      liabilities or equity (equity is financing anyway).
///   2. NEVER a code twice in one list, or in two lists.
///   3. NEVER a registered fixed asset's cost or accumulated-depreciation
///      account as cash.
/// ```
pub fn role_problems(
    ledger: &Ledger,
    cash: &[AccountId],
    financing: &[AccountId],
    investing: &[AccountId],
) -> Vec<RoleProblem> {
    let chart = ledger.chart();
    let mut out = Vec::new();
    let lists = [
        (RoleList::Cash, cash),
        (RoleList::Financing, financing),
        (RoleList::Investing, investing),
    ];
    for (list, ids) in lists {
        let mut seen = BTreeSet::new();
        for &id in ids {
            let Some(a) = chart.get(id) else {
                continue;
            };
            if !seen.insert(id) {
                out.push(RoleProblem::Repeated {
                    list,
                    code: a.code.clone(),
                });
                continue;
            }
            let ok = match list {
                RoleList::Cash | RoleList::Investing => a.account_type == AccountType::Asset,
                RoleList::Financing => {
                    matches!(a.account_type, AccountType::Liability | AccountType::Equity)
                }
            };
            if !ok {
                out.push(RoleProblem::WrongType {
                    list,
                    code: a.code.clone(),
                    name: a.name.clone(),
                    found: a.account_type,
                });
            }
        }
    }
    for (i, &(first, xs)) in lists.iter().enumerate() {
        for &(second, ys) in &lists[i + 1..] {
            let both: BTreeSet<&str> = xs
                .iter()
                .filter(|x| ys.contains(x))
                .filter_map(|&x| chart.get(x).map(|a| a.code.as_str()))
                .collect();
            for code in both {
                let a = chart.find_by_code(code).expect("just found");
                out.push(RoleProblem::InTwoLists {
                    code: a.code.clone(),
                    name: a.name.clone(),
                    first,
                    second,
                });
            }
        }
    }
    let mut asset_accounts = BTreeSet::new();
    for asset in ledger.registered_assets() {
        for id in [asset.cost_account, asset.accum_depr_account] {
            if cash.contains(&id)
                && let Some(a) = chart.get(id)
            {
                asset_accounts.insert((a.code.clone(), a.name.clone()));
            }
        }
    }
    for (code, name) in asset_accounts {
        out.push(RoleProblem::RegisteredAssetIsCash { code, name });
    }
    out
}

/// rules.md B-14 rule 9, on its own so the check is tested directly (it
/// cannot fail on posted entries): the sections sum to the change in
/// cash, or nothing is produced.
pub fn foot(sections_sum: Decimal, net_change: Decimal) -> Result<CashFlowFooting, LedgerError> {
    let difference = sections_sum
        .checked_sub(net_change)
        .ok_or_else(|| too_large("the cash flow statement's footing"))?;
    if !difference.is_zero() {
        return Err(LedgerError::CashFlowDoesNotFoot {
            what: "sections".to_string(),
            expected: net_change,
            found: sections_sum,
        });
    }
    Ok(CashFlowFooting {
        sections_sum,
        net_change,
        difference,
    })
}

fn too_large(what: &str) -> LedgerError {
    LedgerError::TooLargeToCompute(what.to_string())
}

fn add(a: Decimal, b: Decimal, what: &str) -> Result<Decimal, LedgerError> {
    a.checked_add(b).ok_or_else(|| too_large(what))
}

fn signed(side: Side, amount: Decimal) -> Decimal {
    match side {
        Side::Debit => amount,
        Side::Credit => -amount,
    }
}

/// What a line part belongs to within its section: the kind its cash is
/// listed under. Order matters: kinds are split in this order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Bucket {
    /// Operating balance-sheet lines (prepaids, payables, ...), the
    /// receivables control aside: classed per entry by the structure of
    /// the entry's operating lines (rule 5), with `Receivables`, `Income`
    /// and `GuaranteedPayments`.
    Operating,
    /// The receivables control account: classed with `Operating`. Apart
    /// because what a member or lender collects of it for the company is
    /// a receipt on its behalf (rule 11), and another operating asset
    /// handed over is not.
    Receivables,
    /// Operating revenue and expense lines other than interest and
    /// guaranteed payments: classed with `Operating`. Apart from it
    /// because only these can a member pay for through equity (rule 11),
    /// and after it so cash and lenders settle balance-sheet lines first.
    Income,
    /// Guaranteed payments to members (tax category
    /// `guaranteed_payments`): classed with `Operating`. Apart because a
    /// member never pays their own guaranteed payment for the company:
    /// one charged to what the company owes them is accrued, not paid
    /// (rule 11).
    GuaranteedPayments,
    /// Operating: a line on an `interest_income` account, class (b) (ASC
    /// 230-10-45-25(b)), whatever else the entry settles.
    InterestReceived,
    /// Operating: a line on an `interest_expense` account, class (e)
    /// (45-25(e)), whatever else the entry settles.
    InterestPaid,
    /// Registered fixed assets (cost, accumulated depreciation, and the
    /// gains and losses of entries disposing of them).
    Property,
    /// An `investing_accounts` account, by code.
    Investment(String),
    /// A member's capital or distributions account, by member.
    Member(usize),
    /// Another equity account, by code.
    Equity(String),
    /// A `financing_accounts` liability, by code.
    Borrowing(String),
}

/// One line of an entry, or one part of a line split between sections
/// (the bill split, rule 12), tagged with its place on the statement.
#[derive(Clone, Debug)]
struct Part {
    account: AccountId,
    flow: Flow,
    side: Side,
    amount: Decimal,
    /// A revenue or expense line.
    pl: bool,
    /// A line on an accumulated-depreciation account.
    accum: bool,
    bucket: Bucket,
}

impl Part {
    fn signed(&self) -> Decimal {
        signed(self.side, self.amount)
    }
}

/// The account roles, checked.
struct Roles<'a> {
    ledger: &'a Ledger,
    cash: BTreeSet<AccountId>,
    financing: BTreeSet<AccountId>,
    investing: BTreeSet<AccountId>,
    cost: BTreeSet<AccountId>,
    accum: BTreeSet<AccountId>,
    receivables: Option<AccountId>,
    member_of: HashMap<AccountId, usize>,
    members: Vec<String>,
}

impl<'a> Roles<'a> {
    fn checked(ledger: &'a Ledger, params: &CashFlowParams) -> Result<Self, LedgerError> {
        let chart = ledger.chart();
        for &id in params
            .cash_accounts
            .iter()
            .chain(&params.financing_accounts)
            .chain(&params.investing_accounts)
        {
            if chart.get(id).is_none() {
                return Err(LedgerError::UnknownAccount(id));
            }
        }
        if params.cash_accounts.is_empty() {
            return Err(LedgerError::CashFlowRoles(
                "no cash accounts: pass cash_accounts, or set books.cash_accounts (the bank, savings and cash-on-hand accounts the statement explains)".to_string(),
            ));
        }
        let problems = role_problems(
            ledger,
            &params.cash_accounts,
            &params.financing_accounts,
            &params.investing_accounts,
        );
        if !problems.is_empty() {
            let name = |l: RoleList| {
                let key = match l {
                    RoleList::Cash => "cash_accounts",
                    RoleList::Financing => "financing_accounts",
                    RoleList::Investing => "investing_accounts",
                };
                format!("{key} (from the call or books.{key})")
            };
            let text: Vec<String> = problems.iter().map(|p| p.message(name)).collect();
            return Err(LedgerError::CashFlowRoles(text.join("; ")));
        }
        let mut cost = BTreeSet::new();
        let mut accum = BTreeSet::new();
        for asset in ledger.registered_assets() {
            cost.insert(asset.cost_account);
            accum.insert(asset.accum_depr_account);
        }
        let mut member_of = HashMap::new();
        for (i, m) in params.members.iter().enumerate() {
            member_of.insert(m.capital_account, i);
            if let Some(d) = m.distribution_account {
                member_of.insert(d, i);
            }
        }
        Ok(Roles {
            ledger,
            cash: params.cash_accounts.iter().copied().collect(),
            financing: params.financing_accounts.iter().copied().collect(),
            investing: params.investing_accounts.iter().copied().collect(),
            cost,
            accum,
            receivables: ledger.role_account(&crate::RECEIVABLES),
            member_of,
            members: params.members.iter().map(|m| m.name.clone()).collect(),
        })
    }

    fn account(&self, id: AccountId) -> Result<&'a Account, LedgerError> {
        self.ledger
            .chart()
            .get(id)
            .ok_or(LedgerError::UnknownAccount(id))
    }

    /// An investing asset other than accumulated depreciation: a
    /// registered cost account or an `investing_accounts` account.
    fn investing_asset(&self, id: AccountId) -> bool {
        self.cost.contains(&id) || (self.investing.contains(&id) && !self.accum.contains(&id))
    }

    /// Where a balance-sheet account falls (rule 2): its section and kind.
    fn place(&self, a: &Account) -> (Flow, Bucket) {
        if self.cash.contains(&a.id) {
            (Flow::Cash, Bucket::Operating)
        } else if self.cost.contains(&a.id) || self.accum.contains(&a.id) {
            (Flow::Investing, Bucket::Property)
        } else if self.investing.contains(&a.id) {
            (Flow::Investing, Bucket::Investment(a.code.clone()))
        } else if a.account_type == AccountType::Equity {
            match self.member_of.get(&a.id) {
                Some(&i) => (Flow::Financing, Bucket::Member(i)),
                None => (Flow::Financing, Bucket::Equity(a.code.clone())),
            }
        } else if self.financing.contains(&a.id) {
            (Flow::Financing, Bucket::Borrowing(a.code.clone()))
        } else if Some(a.id) == self.receivables {
            (Flow::Operating, Bucket::Receivables)
        } else {
            (Flow::Operating, Bucket::Operating)
        }
    }

    /// Rule 4: the kind an entry's revenue and expense lines go to when it
    /// credits a registered cost account (property) or an investing
    /// account (the lowest code it credits); `None` otherwise.
    fn disposal(&self, basis: &JournalEntry) -> Result<Option<Bucket>, LedgerError> {
        let mut investment: Option<String> = None;
        for l in basis.lines() {
            if l.side != Side::Credit {
                continue;
            }
            if self.cost.contains(&l.account) {
                return Ok(Some(Bucket::Property));
            }
            if self.investing_asset(l.account) {
                let code = self.account(l.account)?.code.clone();
                if investment.as_ref().is_none_or(|c| code < *c) {
                    investment = Some(code);
                }
            }
        }
        Ok(investment.map(Bucket::Investment))
    }

    /// The caption a kind is listed under, and its place in the section's
    /// order. `inflow` is the root's direction: a reversal is listed
    /// under its original's kind.
    fn kind(&self, bucket: &Bucket, inflow: bool) -> (KindOrder, String) {
        let named = |code: &str| {
            self.ledger
                .chart()
                .find_by_code(code)
                .map_or(code.to_string(), |a| format!("{} {}", a.code, a.name))
        };
        let owners = if self.members.is_empty() {
            "owners"
        } else {
            "members"
        };
        match (bucket, inflow) {
            // Operating kinds are classes (`class_of`); no operating part
            // is ever in another section.
            (
                Bucket::Operating
                | Bucket::Receivables
                | Bucket::Income
                | Bucket::GuaranteedPayments
                | Bucket::InterestReceived
                | Bucket::InterestPaid,
                _,
            ) => ((9, 0, String::new()), "Other activities".to_string()),
            (Bucket::Property, false) => (
                (0, 0, String::new()),
                "Purchases of property and equipment".to_string(),
            ),
            (Bucket::Property, true) => (
                (1, 0, String::new()),
                "Proceeds from sales of property and equipment".to_string(),
            ),
            (Bucket::Investment(c), false) => {
                ((2, 0, c.clone()), format!("Payments for {}", named(c)))
            }
            (Bucket::Investment(c), true) => {
                ((3, 0, c.clone()), format!("Collections on {}", named(c)))
            }
            (Bucket::Member(i), true) => (
                (0, 0, format!("{i:08}")),
                format!("Contributions from members — {}", self.members[*i]),
            ),
            (Bucket::Member(i), false) => (
                (1, 0, format!("{i:08}")),
                format!("Distributions to members — {}", self.members[*i]),
            ),
            (Bucket::Equity(c), true) => (
                (0, 1, c.clone()),
                format!("Contributions from {owners} — {}", named(c)),
            ),
            (Bucket::Equity(c), false) => (
                (1, 1, c.clone()),
                format!("Distributions to {owners} — {}", named(c)),
            ),
            (Bucket::Borrowing(c), true) => (
                (2, 0, c.clone()),
                format!("Proceeds from borrowing — {}", named(c)),
            ),
            (Bucket::Borrowing(c), false) => (
                (3, 0, c.clone()),
                format!("Repayments of borrowing — {}", named(c)),
            ),
        }
    }
}

/// Where a kind sits in its section: its group (contributions before
/// distributions, purchases before sales, ...), then its member or
/// account.
type KindOrder = (u8, u8, String);

/// Rule 12 (D-35): a bill whose entry debits an investing asset carries
/// that part, equipment first, to its payables lines and to its payments,
/// so the payment is an investing outflow (ASC 230-10-45-13(c)). A bill
/// posts to the general-expenses control today, so this is empty until a
/// bill can carry an equipment line.
#[derive(Default)]
struct BillSplits {
    /// By (entry, line index): the investing part of that payables line,
    /// and the kind it goes to.
    parts: HashMap<(EntryId, usize), (Decimal, Bucket)>,
    /// By bill entry: the equipment part each payment paid, and its date;
    /// a payment's reversal unpays it again (a negative part, on the
    /// reversal's date). The disclosure of a purchase on account is what
    /// is still unpaid at the period's end (EY FRD 5.3.3; DART 6.1.3).
    paid: HashMap<EntryId, Vec<(NaiveDate, Decimal)>>,
}

fn bill_splits(roles: &Roles<'_>, chains: &Chains) -> Result<BillSplits, LedgerError> {
    let ledger = roles.ledger;
    let mut out = BillSplits::default();
    let operating_liability = |id: AccountId| -> Result<bool, LedgerError> {
        let a = roles.account(id)?;
        Ok(a.account_type == AccountType::Liability && roles.place(a).0 == Flow::Operating)
    };
    for bill in ledger.bills() {
        let Some(b) = ledger.get_entry(bill.entry_id) else {
            continue;
        };
        let mut equipment = Decimal::ZERO;
        let mut bucket: Option<Bucket> = None;
        for l in b.lines() {
            if l.side == Side::Debit && roles.investing_asset(l.account) {
                equipment = add(equipment, l.amount.amount, "a bill's equipment part")?;
                let kind = roles.place(roles.account(l.account)?).1;
                if bucket.as_ref().is_none_or(|k| kind < *k) {
                    bucket = Some(kind);
                }
            }
        }
        let Some(bucket) = bucket else {
            continue;
        };
        // The bill's payables credits carry the equipment part first.
        let mut left = equipment;
        for (i, l) in b.lines().iter().enumerate() {
            if l.side == Side::Credit && left > Decimal::ZERO && operating_liability(l.account)? {
                let part = left.min(l.amount.amount);
                out.parts.insert((b.id(), i), (part, bucket.clone()));
                left -= part;
            }
        }
        // Each payment pays the equipment part first, as the bill records
        // them, in order: the part its payables carried (none when the
        // payable is financing: its payments are repayments already).
        // A payment's own amount bounds what it paid of this bill. A
        // reversed payment still counts against what later payments carry
        // (the bill keeps it as paid), but not against what is unpaid at a
        // period's end after the reversal.
        let mut owed = equipment - left;
        for &(pay, amount) in ledger.bill_payments(bill.id) {
            let Some(p) = ledger.get_entry(pay) else {
                continue;
            };
            let mut this = amount;
            for (i, l) in p.lines().iter().enumerate() {
                if l.side == Side::Debit
                    && owed > Decimal::ZERO
                    && this > Decimal::ZERO
                    && operating_liability(l.account)?
                {
                    let part = owed.min(l.amount.amount).min(this);
                    out.parts.insert((p.id(), i), (part, bucket.clone()));
                    let paid = out.paid.entry(b.id()).or_default();
                    paid.push((p.date(), part));
                    // Its reversal unpays it; a reinstatement pays it again.
                    let mut undo = -part;
                    let mut cur = p.id();
                    while let Some(&r) = chains.reversed_by.get(&cur) {
                        let Some(re) = ledger.get_entry(r).filter(|_| r > cur) else {
                            break;
                        };
                        paid.push((re.date(), undo));
                        undo = -undo;
                        cur = r;
                    }
                    owed -= part;
                    this -= part;
                }
            }
        }
    }
    Ok(out)
}

/// One kept entry, analysed.
struct Analysed<'a> {
    entry: &'a JournalEntry,
    /// Its reversal chain, root first.
    chain: Vec<EntryId>,
    /// Its place in the chain: 0 for the root.
    depth: usize,
    /// It mirrors its root at an odd depth: its kinds and class are its
    /// root's, its direction the opposite.
    flip: bool,
    /// The entry whose structure decides its sections and class: its
    /// root, when it mirrors the root line for line; else itself.
    basis: &'a JournalEntry,
    parts: Vec<Part>,
    /// Its cash flows on the statement: each section's cash (constructive
    /// cash included, rule 11; it nets to zero within the entry, so they
    /// still sum to the entry's cash), and what no cash settled.
    flows: EntryFlows<Decimal>,
    /// Per section, the kinds with cash, in kind order.
    kinds: [Vec<KindCash>; 3],
    exchanged: Decimal,
}

/// One kind's cash in one entry.
#[derive(Clone, Debug, PartialEq)]
struct KindCash {
    bucket: Bucket,
    /// Its cash, + in: what cash settled plus what was settled on the
    /// company's behalf.
    cash: Decimal,
    /// Of `cash`, what a member or lender paid or received for the company
    /// (rule 11).
    on_behalf: Decimal,
}

/// One kind's movement in one entry while its cash is worked out.
struct KindWork {
    bucket: Bucket,
    /// Its net movement, debit-positive.
    moved: Decimal,
    /// The entry's cash it took (rule 3).
    cash: Decimal,
    /// What a member or lender settled of it for the company (rule 11).
    on_behalf: Decimal,
}

impl KindWork {
    /// What no cash, real or constructive, has settled yet.
    fn left(&self) -> Result<Decimal, LedgerError> {
        add(
            add(self.moved, self.cash, "a kind's movement")?,
            self.on_behalf,
            "a kind's movement",
        )
    }
}

/// Rule 11 (D-42), one pass: the kinds `source` picks pay for, or receive
/// for, the kinds `uses` picks, by `kernel::constructive`, each in kind
/// order (sections in rule 3's order, operating first).
fn on_behalf(
    kinds: &mut [Vec<KindWork>; 3],
    way: OnBehalf,
    source: impl Fn(usize, &Bucket) -> bool,
    uses: impl Fn(usize, &Bucket) -> bool,
) -> Result<(), LedgerError> {
    let pick = |f: &dyn Fn(usize, &Bucket) -> bool| -> Vec<(usize, usize)> {
        kinds
            .iter()
            .enumerate()
            .flat_map(|(k, ks)| {
                ks.iter()
                    .enumerate()
                    .filter(move |(_, w)| f(k, &w.bucket))
                    .map(move |(i, _)| (k, i))
            })
            .collect()
    };
    let (src, dst) = (pick(&source), pick(&uses));
    if src.is_empty() || dst.is_empty() {
        return Ok(());
    }
    let left = |at: &[(usize, usize)]| -> Result<Vec<Decimal>, LedgerError> {
        at.iter().map(|&(k, i)| kinds[k][i].left()).collect()
    };
    let (s, u) = (left(&src)?, left(&dst)?);
    let mut from = vec![Decimal::ZERO; s.len()];
    let mut to = vec![Decimal::ZERO; u.len()];
    kernel::constructive(way, &s, &u, &mut from, &mut to)
        .ok_or_else(|| too_large("cash paid on the company's behalf"))?;
    for (&(k, i), x) in src.iter().chain(&dst).zip(from.into_iter().chain(to)) {
        let w = &mut kinds[k][i];
        w.on_behalf = add(w.on_behalf, x, "cash paid on the company's behalf")?;
    }
    Ok(())
}

/// Whether `entry`'s lines are `root`'s, line for line, sides flipped when
/// `flipped`.
fn mirrors(root: &JournalEntry, entry: &JournalEntry, flipped: bool) -> bool {
    root.lines().len() == entry.lines().len()
        && root.lines().iter().zip(entry.lines()).all(|(r, e)| {
            let side = if flipped {
                kernel::flip(r.side)
            } else {
                r.side
            };
            r.account == e.account && e.side == side && r.amount.amount == e.amount.amount
        })
}

fn analyse<'a>(
    roles: &Roles<'a>,
    splits: &BillSplits,
    chains: &Chains,
    entry: &'a JournalEntry,
) -> Result<Analysed<'a>, LedgerError> {
    let ledger = roles.ledger;
    let chain = chains.chain(ledger, entry.id());
    let depth = chain.iter().position(|&e| e == entry.id()).unwrap_or(0);
    let (basis, flip) = match ledger.get_entry(chain[0]) {
        Some(root) if depth > 0 && mirrors(root, entry, depth % 2 == 1) => (root, depth % 2 == 1),
        _ => (entry, false),
    };
    let disposal = roles.disposal(basis)?;
    let mut parts = Vec::with_capacity(entry.lines().len());
    for (i, line) in entry.lines().iter().enumerate() {
        let a = roles.account(line.account)?;
        let amount = line.amount.amount;
        let pl = !a.account_type.is_balance_sheet();
        let (flow, bucket) = if pl {
            // Rule 4: a disposal's gains and losses are investing, except
            // interest, received or paid, which is operating in any entry
            // (ASC 230-10-45-16(b), 45-17(d)) and its own class (rule 5).
            let interest = match a.tax_category.as_deref() {
                Some("interest_income") => Some(Bucket::InterestReceived),
                Some("interest_expense") => Some(Bucket::InterestPaid),
                _ => None,
            };
            match (interest, &disposal) {
                (Some(i), _) => (Flow::Operating, i),
                (None, Some(b)) => (Flow::Investing, b.clone()),
                (None, None) if a.tax_category.as_deref() == Some("guaranteed_payments") => {
                    (Flow::Operating, Bucket::GuaranteedPayments)
                }
                (None, None) => (Flow::Operating, Bucket::Income),
            }
        } else {
            roles.place(a)
        };
        let mut part = Part {
            account: line.account,
            flow,
            side: line.side,
            amount,
            pl,
            accum: roles.accum.contains(&line.account),
            bucket,
        };
        // Rule 12: the investing part of a bill's payables line.
        if !pl
            && flow == Flow::Operating
            && let Some((inv, kind)) = splits.parts.get(&(basis.id(), i))
        {
            let inv = (*inv).min(amount);
            if inv > Decimal::ZERO {
                parts.push(Part {
                    flow: Flow::Investing,
                    amount: inv,
                    bucket: kind.clone(),
                    ..part.clone()
                });
            }
            part.amount = amount - inv;
            if part.amount.is_zero() {
                continue;
            }
        }
        parts.push(part);
    }
    let tagged: Vec<(Flow, Side, Decimal)> =
        parts.iter().map(|p| (p.flow, p.side, p.amount)).collect();
    let Some(cash) = kernel::entry_flows(&tagged) else {
        let net = kernel::net(tagged.iter().map(|&(_, s, a)| (s, a)))
            .ok_or_else(|| too_large("an entry's cash flows"))?;
        return Err(if net.is_zero() {
            too_large("an entry's cash flows")
        } else {
            LedgerError::CashFlowDoesNotFoot {
                what: format!("entry #{}", entry.id()),
                expected: Decimal::ZERO,
                found: net,
            }
        });
    };
    // Each section's movement by kind, in kind order.
    let mut work: [Vec<KindWork>; 3] = Default::default();
    for (k, section) in Flow::SECTIONS.iter().enumerate() {
        let mut moved: BTreeMap<Bucket, Decimal> = BTreeMap::new();
        for p in parts.iter().filter(|p| p.flow == *section) {
            let m = moved.entry(p.bucket.clone()).or_default();
            *m = add(*m, p.signed(), "a kind's movement")?;
        }
        work[k] = moved
            .into_iter()
            .map(|(bucket, moved)| KindWork {
                bucket,
                moved,
                cash: Decimal::ZERO,
                on_behalf: Decimal::ZERO,
            })
            .collect();
        // Rule 3: the section's cash among its kinds, in kind order (the
        // convention again, within the section).
        if !cash.flows[k].is_zero() {
            let movements: Vec<Decimal> = work[k].iter().map(|w| w.moved).collect();
            let mut applied = vec![Decimal::ZERO; movements.len()];
            kernel::apply_cash(cash.flows[k], &movements, &mut applied).ok_or_else(|| {
                LedgerError::CashFlowDoesNotFoot {
                    what: format!("entry #{} kinds", entry.id()),
                    expected: cash.flows[k],
                    found: Decimal::ZERO,
                }
            })?;
            for (w, a) in work[k].iter_mut().zip(applied) {
                w.cash = a;
            }
        }
    }
    // Rule 11 (D-42): what no cash settled, a member or a lender paid (or
    // received) for the company. A financing liability pays for operating
    // and investing kinds; a member's equity only for revenue and expense
    // (property a member contributes is noncash, ASC 230-10-50-4). Either
    // receives for the company only what a third party pays cash for:
    // revenue, a refund of an expense, a receivable collected (property
    // handed over in kind is noncash). Never interest or guaranteed
    // payments: those are the lender's or the member's own charge, accrued
    // into what the company owes them, paid to no one. A reversal mirrors
    // its root (rule 6): what the root paid, it receives back.
    let lender = |k: usize, b: &Bucket| k == 2 && matches!(b, Bucket::Borrowing(_));
    let member =
        |k: usize, b: &Bucket| k == 2 && matches!(b, Bucket::Member(_) | Bucket::Equity(_));
    let lender_pays_for = |k: usize, b: &Bucket| {
        (k == 0 && matches!(b, Bucket::Operating | Bucket::Receivables | Bucket::Income)) || k == 1
    };
    let member_pays_for = |k: usize, b: &Bucket| k == 0 && *b == Bucket::Income;
    let receives_for =
        |k: usize, b: &Bucket| k == 0 && matches!(b, Bucket::Receivables | Bucket::Income);
    let way = |w: OnBehalf| if flip { w.mirror() } else { w };
    on_behalf(&mut work, way(OnBehalf::Paid), lender, lender_pays_for)?;
    on_behalf(&mut work, way(OnBehalf::Received), lender, receives_for)?;
    on_behalf(&mut work, way(OnBehalf::Paid), member, member_pays_for)?;
    on_behalf(&mut work, way(OnBehalf::Received), member, receives_for)?;
    // The entry's flows: each section's cash, real and constructive (the
    // latter nets to zero within the entry, so they still sum to its
    // cash), and what neither settled.
    let mut flows = EntryFlows {
        cash: cash.cash,
        flows: [Decimal::ZERO; 3],
        noncash: [Decimal::ZERO; 3],
    };
    let mut kinds: [Vec<KindCash>; 3] = Default::default();
    // Cash in and out of investing and financing, for rule 7.
    let (mut inflow, mut outflow) = (Decimal::ZERO, Decimal::ZERO);
    for (k, ws) in work.into_iter().enumerate() {
        for w in ws {
            let total = add(w.cash, w.on_behalf, "a kind's cash")?;
            flows.flows[k] = add(flows.flows[k], total, "a section's cash")?;
            flows.noncash[k] = add(flows.noncash[k], w.left()?, "a section's noncash part")?;
            if k > 0 {
                if total > Decimal::ZERO {
                    inflow = add(inflow, total, "an entry's exchange")?;
                } else {
                    outflow = add(outflow, -total, "an entry's exchange")?;
                }
            }
            if !total.is_zero() {
                kinds[k].push(KindCash {
                    bucket: w.bucket,
                    cash: total,
                    on_behalf: w.on_behalf,
                });
            }
        }
    }
    // Rule 7: the exchange inside investing and financing, account by
    // account (accumulated depreciation and gains and losses left out).
    let mut net: BTreeMap<AccountId, Decimal> = BTreeMap::new();
    for p in &parts {
        if matches!(p.flow, Flow::Investing | Flow::Financing) && !p.pl && !p.accum {
            let m = net.entry(p.account).or_default();
            *m = add(*m, p.signed(), "an entry's exchange")?;
        }
    }
    let (mut debits, mut credits) = (Decimal::ZERO, Decimal::ZERO);
    for m in net.into_values() {
        if m > Decimal::ZERO {
            debits = add(debits, m, "an entry's exchange")?;
        } else {
            credits = add(credits, -m, "an entry's exchange")?;
        }
    }
    let exchanged = kernel::exchanged(debits, credits, inflow, outflow)
        .ok_or_else(|| too_large("an entry's exchange"))?;
    Ok(Analysed {
        entry,
        chain,
        depth,
        flip,
        basis,
        parts,
        flows,
        kinds,
        exchanged,
    })
}

/// Reversal links, gathered once (`Ledger::reversal_chain` searches the
/// whole log at each step).
struct Chains {
    reversed_by: HashMap<EntryId, EntryId>,
}

impl Chains {
    fn new(ledger: &Ledger) -> Self {
        let mut reversed_by = HashMap::new();
        for e in ledger.posted_entries() {
            if let Some(o) = e.reversal_of() {
                // Each entry is reversed at most once; the first stands.
                reversed_by.entry(o).or_insert(e.id());
            }
        }
        Chains { reversed_by }
    }

    /// As `Ledger::reversal_chain`: the original it reverses (and so on
    /// up), itself, the entry reversing it (and so on down).
    fn chain(&self, ledger: &Ledger, entry: EntryId) -> Vec<EntryId> {
        let mut top = entry;
        let mut up = 0;
        while let Some(o) = ledger.get_entry(top).and_then(JournalEntry::reversal_of) {
            if ledger.get_entry(o).is_none() || o == entry || up > self.reversed_by.len() {
                break;
            }
            top = o;
            up += 1;
        }
        let mut chain = vec![top];
        let mut cur = top;
        while let Some(&r) = self.reversed_by.get(&cur) {
            if chain.contains(&r) {
                break;
            }
            chain.push(r);
            cur = r;
        }
        chain
    }
}

fn negated(f: &EntryFlows<Decimal>) -> EntryFlows<Decimal> {
    EntryFlows {
        cash: -f.cash,
        flows: f.flows.map(|x| -x),
        noncash: f.noncash.map(|x| -x),
    }
}

/// Rule 6: pair each entry with its reversal when both are in the period,
/// walking each chain from its root, so a reversal of a reversal is shown
/// (review item 1). A pair nets only when its flows, kinds and exchange
/// are exact negatives (review item 8); otherwise both are shown.
fn pairs(kept: &[Analysed<'_>]) -> HashSet<EntryId> {
    let at: HashMap<EntryId, usize> = kept
        .iter()
        .enumerate()
        .map(|(i, a)| (a.entry.id(), i))
        .collect();
    let mut paired = HashSet::new();
    let mut walked = HashSet::new();
    for a in kept {
        if !walked.insert(a.chain[0]) {
            continue;
        }
        let chain = &a.chain;
        let mut i = 0;
        while i + 1 < chain.len() {
            let (Some(&x), Some(&r)) = (at.get(&chain[i]), at.get(&chain[i + 1])) else {
                i += 1;
                continue;
            };
            let (x, r) = (&kept[x], &kept[r]);
            let mirror = r.flows == negated(&x.flows)
                && r.exchanged == x.exchanged
                && (0..3).all(|k| {
                    r.kinds[k].len() == x.kinds[k].len()
                        && r.kinds[k].iter().zip(&x.kinds[k]).all(|(rk, xk)| {
                            rk.bucket == xk.bucket
                                && rk.cash == -xk.cash
                                && rk.on_behalf == -xk.on_behalf
                        })
                });
            if mirror {
                paired.insert(x.entry.id());
                paired.insert(r.entry.id());
                i += 2;
            } else {
                i += 1;
            }
        }
    }
    paired
}

/// A line's or disclosure's label (rule 6): never from the sign.
fn label(ledger: &Ledger, a: &Analysed<'_>) -> String {
    let id = a.entry.id();
    match a.entry.reversal_of() {
        None => format!("{} (#{id})", a.entry.description()),
        Some(orig) if a.depth >= 2 && a.depth.is_multiple_of(2) => {
            format!("Reinstatement of #{} (reverses #{orig})", a.chain[0])
        }
        Some(orig) => {
            let what = ledger
                .get_entry(orig)
                .map_or(String::new(), |o| format!(" — {}", o.description()));
            format!("Reversal of #{orig}{what}")
        }
    }
}

/// Rule 5: the class of one kind of an entry's operating cash, from the
/// direction of its root's cash: interest received or paid by its own
/// lines (whatever else the entry settles); everything else by the
/// structure of the entry's other operating lines.
fn class_of(
    roles: &Roles<'_>,
    a: &Analysed<'_>,
    bucket: &Bucket,
    receipt: bool,
) -> Result<OperatingClass, LedgerError> {
    match (bucket, receipt) {
        (Bucket::InterestReceived, true) => {
            return Ok(OperatingClass::InterestAndDividendsReceived);
        }
        // Interest paid back out, or refunded to the company: neither (b)
        // nor (e), so the other class of its direction.
        (Bucket::InterestReceived, false) => return Ok(OperatingClass::OtherPayments),
        (Bucket::InterestPaid, false) => return Ok(OperatingClass::InterestPaid),
        (Bucket::InterestPaid, true) => return Ok(OperatingClass::OtherReceipts),
        _ => {}
    }
    let mut ops: Vec<&Account> = Vec::new();
    for p in a.parts.iter().filter(|p| {
        p.flow == Flow::Operating
            && matches!(
                p.bucket,
                Bucket::Operating
                    | Bucket::Receivables
                    | Bucket::Income
                    | Bucket::GuaranteedPayments
            )
    }) {
        ops.push(roles.account(p.account)?);
    }
    let receivable = |x: &Account| Some(x.id) == roles.receivables;
    Ok(if receipt {
        let customer = roles
            .ledger
            .entry_parties(a.basis.id())
            .iter()
            .filter_map(|s| roles.ledger.get_party(s.party))
            .any(|p| p.all_roles().contains(&crate::Role::Customer));
        if customer
            || ops
                .iter()
                .any(|x| x.account_type == AccountType::Revenue || receivable(x))
        {
            OperatingClass::FromCustomers
        } else {
            OperatingClass::OtherReceipts
        }
    } else if !ops.is_empty()
        && ops
            .iter()
            .all(|x| x.tax_category.as_deref() == Some("taxes_licenses"))
    {
        // Taxes and fees paid to governments are not suppliers' (ASC
        // 230-10-45-17(c) apart from 45-17(b)); 45-25 names only income
        // taxes, (f), so the rest are (g).
        OperatingClass::OtherPayments
    } else if ops.iter().any(|x| match x.account_type {
        AccountType::Expense | AccountType::Liability => true,
        AccountType::Asset => !receivable(x),
        _ => false,
    }) {
        OperatingClass::ToSuppliersAndEmployees
    } else {
        OperatingClass::OtherPayments
    })
}

/// The codes of the accounts in some parts, sorted, once each.
fn codes<'p>(
    roles: &Roles<'_>,
    parts: impl Iterator<Item = &'p Part>,
) -> Result<Vec<String>, LedgerError> {
    let mut out = BTreeSet::new();
    for p in parts {
        out.insert(roles.account(p.account)?.code.clone());
    }
    Ok(out.into_iter().collect())
}

/// The statement of cash flows for a period, direct method (rules.md
/// B-14).
///
/// ```text
/// GOAL: the statement of cash flows for a period: each entry's cash by
///       section and kind, the noncash investing and financing activities,
///       and net income reconciled to operating cash; footing exactly or
///       not at all.
/// SOURCE: rules.md B-14 (ASC 230-10-10-1 cash receipts and payments;
///         45-4, 45-5 what cash is; 45-7, 45-26 gross; 45-10 the three
///         sections; 45-12 to 45-17 what each holds; 45-24 reconciles
///         beginning and ending cash; 45-25 the
///         operating classes; 45-28 to 45-30 the reconciliation, a
///         separate schedule with the direct method; 50-3 to 50-5 noncash
///         activities disclosed, only the cash portion reported; D-35;
///         D-42 constructive receipt and disbursement, PwC FSP 6.9.3.1,
///         DART Roadmap 7.2, BDO Blueprint 3.4: ASC 230 is silent).
/// INVARIANTS:
///   1. Every non-cash account MUST be in exactly one section: investing
///      (registered fixed assets' cost and accumulated depreciation,
///      investing_accounts), financing (equity, financing_accounts), or
///      operating (everything else); contradictory roles are refused,
///      naming the setting.
///   2. An entry MUST contribute to the body only its cash, applied to the
///      sections moving against it in the order operating, investing,
///      financing, each up to its own movement (CONVENTION: the order),
///      and what a member or lender paid or received for it (rule 11),
///      which nets to zero within the entry; what neither settled is
///      disclosed, never reported as cash.
///   3. The revenue and expense lines of an entry crediting a registered
///      cost or investing account MUST be investing, interest (received
///      or paid) excepted: it is operating in any entry, and its own
///      class; a reversal takes its chain root's sections, classes and
///      kinds, with its own sign.
///   4. Each section's total MUST equal the sum of its lines; the
///      sections MUST sum to the cash accounts' change (beginning to
///      ending balance); net income plus the reconciling items MUST equal
///      operating. Any failure is CashFlowDoesNotFoot and nothing is
///      produced.
///   5. An entry and its reversal both in the period MUST drop out of
///      every line and disclosure, pairing along the chain from its root;
///      a reversal shown alone is labelled "Reversal of #N" ("Reinstatement
///      of #A" a level down), NEVER by the sign.
///   6. NEVER count closing entries.
///   7. The period is [start, end], inclusive.
/// SCENARIOS:
///   1. IF a member contributes property (Dr equipment / Cr capital):
///      - SHOULD show no line, and disclose it as noncash
///      BECAUSE no cash moved (ASC 230-10-50-3).
///   2. IF a member pays a company expense personally (Dr expense / Cr
///      a financing liability, or / Cr the member's capital), or a lender
///      pays for equipment (Dr equipment / Cr the loan):
///      - SHOULD show it as borrowed (or contributed) and spent, on that
///        entry's date: a financing inflow and the operating or investing
///        outflow, `on_behalf` on both lines
///      - NEVER disclose it as noncash
///      BECAUSE the owner chose constructive receipt and disbursement
///      (D-42: ASC 230 is silent; the firm guides present it so); its
///      reimbursement is a financing outflow.
///   3. IF interest is charged into a loan (Dr interest / Cr the loan):
///      - SHOULD reconcile it as noncash, and disclose it
///      BECAUSE the lender's own charge was paid to no one.
///   4. IF equipment is bought on the seller's credit (an operating
///      payable), or sold for a receivable (an IOU):
///      - SHOULD disclose the noncash investing part and warn, with a fix
///      BECAUSE no cash moved (ASC 230-10-50-4), and its later payment or
///      collection would be operating (D-35).
/// ```
pub fn cash_flow(
    ledger: &Ledger,
    params: &CashFlowParams,
) -> Result<CashFlowStatement, LedgerError> {
    if params.end < params.start {
        return Err(LedgerError::InvalidDateRange {
            start: params.start.to_string(),
            end: params.end.to_string(),
        });
    }
    let roles = Roles::checked(ledger, params)?;
    let chains = Chains::new(ledger);
    let splits = bill_splits(&roles, &chains)?;
    let closings = ledger.closing_entry_ids();
    let in_period = |d: NaiveDate| d >= params.start && d <= params.end;

    // Rules 1, 3, 4: every non-closing entry in the period.
    let mut kept: Vec<Analysed<'_>> = Vec::new();
    for entry in ledger.posted_entries() {
        if in_period(entry.date()) && !closings.contains(&entry.id()) {
            kept.push(analyse(&roles, &splits, &chains, entry)?);
        }
    }
    let paired = pairs(&kept);

    // Rule 5: the body, one line per entry, section and kind.
    type Sorted = (KindOrder, NaiveDate, EntryId, CashFlowLine);
    let mut body: [Vec<Sorted>; 3] = Default::default();
    let mut totals = [Decimal::ZERO; 3];
    for a in &kept {
        for (t, f) in totals.iter_mut().zip(a.flows.flows) {
            *t = add(*t, f, "a section's total")?;
        }
        if paired.contains(&a.entry.id()) {
            continue;
        }
        for (k, section) in CashFlowSection::ALL.iter().enumerate() {
            // This entry's lines in the section: kinds that name the same
            // caption (two operating kinds of one class) are one line.
            let mut mine: Vec<Sorted> = Vec::new();
            for KindCash {
                bucket,
                cash: amount,
                on_behalf,
            } in &a.kinds[k]
            {
                let inflow = (*amount > Decimal::ZERO) != a.flip;
                let (order, kind, class) = if *section == CashFlowSection::Operating {
                    let class = class_of(&roles, a, bucket, inflow)?;
                    (
                        (class.order(), 0, String::new()),
                        class.label().to_string(),
                        Some(class),
                    )
                } else {
                    let (order, kind) = roles.kind(bucket, inflow);
                    (order, kind, None)
                };
                let accounts = codes(
                    &roles,
                    a.parts
                        .iter()
                        .filter(|p| p.flow == Flow::SECTIONS[k] && p.bucket == *bucket),
                )?;
                if let Some((_, _, _, same)) =
                    mine.iter_mut().find(|m| m.0 == order && m.3.kind == kind)
                {
                    same.amount = add(same.amount, *amount, "an entry's line")?;
                    same.on_behalf = add(same.on_behalf, *on_behalf, "an entry's line")?;
                    same.accounts.extend(accounts);
                    same.accounts.sort();
                    same.accounts.dedup();
                    continue;
                }
                let line = CashFlowLine {
                    entry: a.entry.id(),
                    date: a.entry.date(),
                    label: label(ledger, a),
                    kind,
                    amount: *amount,
                    on_behalf: *on_behalf,
                    accounts,
                    reverses: a.entry.reversal_of(),
                    class,
                };
                mine.push((order, a.entry.date(), a.entry.id(), line));
            }
            body[k].extend(mine.into_iter().filter(|m| !m.3.amount.is_zero()));
        }
    }
    let mut lines: [Vec<CashFlowLine>; 3] = Default::default();
    for (((out, mut sorted), total), section) in lines
        .iter_mut()
        .zip(body)
        .zip(totals)
        .zip(CashFlowSection::ALL)
    {
        sorted.sort_by(|x, y| (&x.0, x.1, x.2).cmp(&(&y.0, y.1, y.2)));
        *out = sorted.into_iter().map(|(_, _, _, l)| l).collect();
        // Rule 9: each section's total is its lines (the pairs net to
        // zero).
        let mut sum = Decimal::ZERO;
        for l in out.iter() {
            sum = add(sum, l.amount, "a section's lines")?;
        }
        if sum != total {
            return Err(LedgerError::CashFlowDoesNotFoot {
                what: format!("{} lines", section.key()),
                expected: total,
                found: sum,
            });
        }
    }

    // Rule 7: noncash investing and financing activities.
    let mut noncash_disclosures = Vec::new();
    for a in kept.iter().filter(|a| !paired.contains(&a.entry.id())) {
        let r = a.flows.noncash;
        let investing_asset_moved = a
            .parts
            .iter()
            .any(|p| p.flow == Flow::Investing && !p.pl && !p.accum);
        // A purchase on a bill: only what is still unpaid at the end is
        // noncash (what was paid is an investing outflow already).
        let mut exchanged = a.exchanged;
        if a.depth == 0
            && let Some(paid) = splits.paid.get(&a.entry.id())
        {
            for &(date, part) in paid {
                if date <= params.end {
                    exchanged -= part;
                }
            }
            exchanged = exchanged.max(Decimal::ZERO);
        }
        let disclose = !r[2].is_zero()
            || (!r[1].is_zero() && investing_asset_moved)
            || exchanged > Decimal::ZERO;
        if !disclose {
            continue;
        }
        let mut moved = Decimal::ZERO;
        for x in r {
            if x > Decimal::ZERO {
                moved = add(moved, x, "a disclosure")?;
            }
        }
        noncash_disclosures.push(NoncashDisclosure {
            entry: a.entry.id(),
            date: a.entry.date(),
            label: label(ledger, a),
            amount: moved.max(exchanged),
            operating: r[0],
            investing: r[1],
            financing: r[2],
            exchanged,
            reverses: a.entry.reversal_of(),
        });
    }
    noncash_disclosures.sort_by_key(|d| (d.date, d.entry));

    let operating_reconciliation = reconcile(&roles, &kept, &paired, totals[0])?;
    let warnings = warnings(&roles, &kept, &paired)?;

    // Rule 9: the cash accounts' balances, and the footing.
    let (mut cash_beginning, mut cash_ending) = (Decimal::ZERO, Decimal::ZERO);
    for e in ledger.posted_entries() {
        for l in e.lines() {
            if roles.cash.contains(&l.account) {
                let m = signed(l.side, l.amount.amount);
                if e.date() < params.start {
                    cash_beginning = add(cash_beginning, m, "the cash balance")?;
                }
                if e.date() <= params.end {
                    cash_ending = add(cash_ending, m, "the cash balance")?;
                }
            }
        }
    }
    let net_change_in_cash = cash_ending
        .checked_sub(cash_beginning)
        .ok_or_else(|| too_large("the change in cash"))?;
    let sections_sum = add(
        add(totals[0], totals[1], "the sections' sum")?,
        totals[2],
        "the sections' sum",
    )?;
    let reconciles = foot(sections_sum, net_change_in_cash)?;
    let [operating, investing, financing] = lines;
    Ok(CashFlowStatement {
        start: params.start,
        end: params.end,
        operating,
        cash_from_operations: totals[0],
        investing,
        cash_from_investing: totals[1],
        financing,
        cash_from_financing: totals[2],
        net_change_in_cash,
        cash_beginning,
        cash_ending,
        noncash_disclosures,
        operating_reconciliation,
        reconciles,
        warnings,
    })
}

/// Rule 8: net cash from operating = NI + G + X + W, over every kept entry
/// (netted pairs included: they sum to zero), checked against the
/// operating section.
fn reconcile(
    roles: &Roles<'_>,
    kept: &[Analysed<'_>],
    paired: &HashSet<EntryId>,
    operating: Decimal,
) -> Result<OperatingReconciliation, LedgerError> {
    const WHAT: &str = "the operating reconciliation";
    let mut net_income = Decimal::ZERO;
    let mut gains = (Decimal::ZERO, Vec::new());
    // X by key: 0 depreciation, 1 through an account (by code), 2 other.
    let mut noncash: BTreeMap<(u8, String), (Decimal, Vec<EntryId>)> = BTreeMap::new();
    // W by code: the operating balance-sheet accounts' movements.
    let mut balances: BTreeMap<String, (AccountId, Decimal)> = BTreeMap::new();
    for a in kept {
        let shown = !paired.contains(&a.entry.id());
        let mut g = Decimal::ZERO;
        for p in &a.parts {
            if p.pl {
                net_income = add(net_income, -p.signed(), WHAT)?;
                if p.flow == Flow::Investing {
                    g = add(g, p.signed(), WHAT)?;
                }
            } else if p.flow == Flow::Operating {
                let code = roles.account(p.account)?.code.clone();
                let w = balances.entry(code).or_insert((p.account, Decimal::ZERO));
                w.1 = add(w.1, p.signed(), WHAT)?;
            }
        }
        if !g.is_zero() {
            gains.0 = add(gains.0, g, WHAT)?;
            if shown {
                gains.1.push(a.entry.id());
            }
        }
        let x = a.flows.noncash[0];
        if x.is_zero() {
            continue;
        }
        let others: Vec<&Part> = a
            .parts
            .iter()
            .filter(|p| matches!(p.flow, Flow::Investing | Flow::Financing) && !p.pl)
            .collect();
        let through: BTreeSet<AccountId> = others
            .iter()
            .filter(|p| !p.accum)
            .map(|p| p.account)
            .collect();
        let key = if !others.is_empty() && through.is_empty() {
            (0, String::new())
        } else if through.len() == 1 {
            let id = *through.iter().next().expect("one account");
            (1, roles.account(id)?.code.clone())
        } else {
            (2, String::new())
        };
        let slot = noncash.entry(key).or_insert((Decimal::ZERO, Vec::new()));
        slot.0 = add(slot.0, x, WHAT)?;
        if shown {
            slot.1.push(a.entry.id());
        }
    }
    let mut lines = Vec::new();
    let chart = roles.ledger.chart();
    let named = |code: &str| {
        chart
            .find_by_code(code)
            .map_or(code.to_string(), |a| format!("{} {}", a.code, a.name))
    };
    let mut depreciation = None;
    let mut rest = Vec::new();
    for ((key, code), (amount, entries)) in noncash {
        if amount.is_zero() {
            continue;
        }
        let line = match key {
            0 => ReconcilingLine {
                kind: ReconcilingKind::Depreciation,
                label: "Depreciation".to_string(),
                amount,
                account: None,
                entries,
            },
            1 => ReconcilingLine {
                kind: ReconcilingKind::NoncashThrough,
                label: format!("Noncash: through {}", named(&code)),
                amount,
                account: Some(code),
                entries,
            },
            _ => ReconcilingLine {
                kind: ReconcilingKind::OtherNoncash,
                label: "Other noncash items".to_string(),
                amount,
                account: None,
                entries,
            },
        };
        if key == 0 {
            depreciation = Some(line);
        } else {
            rest.push(line);
        }
    }
    lines.extend(depreciation);
    if !gains.0.is_zero() {
        lines.push(ReconcilingLine {
            kind: ReconcilingKind::InvestingGainLoss,
            label: "(Gain) loss and other noncash changes in investing assets".to_string(),
            amount: gains.0,
            account: None,
            entries: gains.1,
        });
    }
    lines.extend(rest);
    for (code, (id, movement)) in balances {
        if movement.is_zero() {
            continue;
        }
        let a = roles.account(id)?;
        let up = match a.account_type {
            AccountType::Liability | AccountType::Equity => movement < Decimal::ZERO,
            _ => movement > Decimal::ZERO,
        };
        lines.push(ReconcilingLine {
            kind: ReconcilingKind::OperatingBalanceChange,
            label: format!(
                "{} {}",
                if up { "Increase in" } else { "Decrease in" },
                named(&code)
            ),
            amount: -movement,
            account: Some(code),
            entries: Vec::new(),
        });
    }
    let mut total = net_income;
    for l in &lines {
        total = add(total, l.amount, WHAT)?;
    }
    if total != operating {
        return Err(LedgerError::CashFlowDoesNotFoot {
            what: "operating reconciliation".to_string(),
            expected: operating,
            found: total,
        });
    }
    Ok(OperatingReconciliation {
        net_income,
        lines,
        net_cash_from_operating: total,
    })
}

/// D-35: entries whose later settlement the books can't classify. Only an
/// original (or a reinstatement): its reversal undoes it.
fn warnings(
    roles: &Roles<'_>,
    kept: &[Analysed<'_>],
    paired: &HashSet<EntryId>,
) -> Result<Vec<CashFlowWarning>, LedgerError> {
    let mut out = Vec::new();
    for a in kept {
        if paired.contains(&a.entry.id()) || a.depth % 2 == 1 {
            continue;
        }
        let id = a.entry.id();
        // The accounts of some of the entry's parts, "code name", once
        // each.
        let named = |keep: &dyn Fn(&Part, &Account) -> bool| -> Result<Vec<String>, LedgerError> {
            let mut v = BTreeSet::new();
            for p in &a.parts {
                let acct = roles.account(p.account)?;
                if keep(p, acct) {
                    v.insert(format!("{} {}", acct.code, acct.name));
                }
            }
            Ok(v.into_iter().collect())
        };
        let investing = |side: Side| {
            named(&|p, _| {
                p.flow == Flow::Investing && roles.investing_asset(p.account) && p.side == side
            })
        };
        let operating = |side: Side, t: AccountType| {
            named(&|p, acct| {
                p.flow == Flow::Operating && !p.pl && p.side == side && acct.account_type == t
            })
        };
        // Only what no cash, investing or financing account settled: an
        // investing part left over, against an operating account.
        let left = a.flows.noncash[1];
        let bought = investing(Side::Debit)?;
        let payables = operating(Side::Credit, AccountType::Liability)?;
        if left > Decimal::ZERO && !bought.is_empty() && !payables.is_empty() {
            let on = payables.join(", ");
            out.push(CashFlowWarning {
                entry: id,
                kind: CashFlowWarningKind::AssetOnOperatingPayable,
                message: format!(
                    "Entry #{id} acquires {} on {on}, an operating payable: the purchase is disclosed as noncash (the seller's own credit, ASC 230-10-50-4), and its payment will show as cash paid to suppliers (operating), following how {on} is classified. If {on} is a lender's credit (a card balance carried, a loan), list it in books.financing_accounts: the purchase then shows as borrowed and spent (D-42) and its payment as a repayment of borrowing.",
                    bought.join(", ")
                ),
            });
        }
        let sold = investing(Side::Credit)?;
        let receivables = operating(Side::Debit, AccountType::Asset)?;
        if left < Decimal::ZERO && !sold.is_empty() && !receivables.is_empty() {
            let on = receivables.join(", ");
            out.push(CashFlowWarning {
                entry: id,
                kind: CashFlowWarningKind::AssetSoldOnOperatingReceivable,
                message: format!(
                    "Entry #{id} disposes of {} for an amount owed on {on}, an operating account: its collection would show as cash from operations, but receipts from sales of property and investments are investing (ASC 230-10-45-12). Fix: reverse it and post the amount owed to an account listed in books.investing_accounts (a note receivable), so its collection is investing.",
                    sold.join(", ")
                ),
            });
        }
    }
    Ok(out)
}
