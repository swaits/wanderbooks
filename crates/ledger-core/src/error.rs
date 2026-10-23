//! Error type for the ledger engine.

use rust_decimal::Decimal;
use thiserror::Error;

/// Every way a ledger command can fail.
///
/// Variants are part of the engine's contract: agents adding features add
/// variants here rather than inventing ad-hoc error reporting.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LedgerError {
    /// A journal entry must have at least two lines (one debit, one credit).
    #[error("journal entry has fewer than two lines")]
    TooFewLines,

    /// Debits and credits must be exactly equal. No epsilon, ever.
    #[error("journal entry does not balance: debits {debits} != credits {credits}")]
    UnbalancedEntry {
        /// Sum of debit lines.
        debits: Decimal,
        /// Sum of credit lines.
        credits: Decimal,
    },

    /// Zero-amount lines are noise; they are rejected, not ignored.
    #[error("journal line has a zero amount")]
    ZeroAmountLine,

    /// Line amounts are positive; the side says which way the money moves.
    #[error("journal line has a negative amount; use the other side instead")]
    NegativeAmountLine,

    /// A line larger than one quadrillion, or with more than ten decimal
    /// places, is a mistake, and one that could overflow exact arithmetic.
    #[error(
        "journal line amount is out of range (at most one quadrillion, with ten decimal places)"
    )]
    AmountOutOfRange,

    /// A date outside the calendar the books keep (`crate::calendar`):
    /// four-digit years, 0001-01-01 through 9999-12-31.
    #[error("date {0} is outside the calendar the books keep (0001-01-01 through 9999-12-31)")]
    DateOutOfRange(chrono::NaiveDate),

    /// An amount other than a journal line's (a document line, a payout, a
    /// payroll figure, a statement balance, a share, a basis) larger than
    /// one quadrillion or with more than ten decimal places: a mistake, and
    /// one that could overflow exact arithmetic.
    #[error("{0} is out of range (at most one quadrillion, with ten decimal places)")]
    ValueOutOfRange(String),

    /// A figure the books derive (a sum, or a share of one) too large for
    /// exact arithmetic: refused, never rounded or wrapped.
    #[error("{0} is too large to compute exactly")]
    TooLargeToCompute(String),

    /// An amount finer than its currency's smallest unit (ISO 4217 minor
    /// units): no one can pay a fraction of a cent (rules.md B-3).
    #[error(
        "amount {amount} {currency} has more than {places} decimal places; round it to the {currency} minor unit first"
    )]
    FinerThanMinorUnit {
        /// The amount as given.
        amount: rust_decimal::Decimal,
        /// Its currency code.
        currency: String,
        /// The decimal places the currency allows.
        places: u32,
    },

    /// The referenced account does not exist in the chart of accounts.
    #[error("unknown account id {0}")]
    UnknownAccount(u64),

    /// Placeholder (header) accounts organize the chart; they take no postings.
    #[error("account '{0}' is a placeholder and cannot take postings")]
    PlaceholderAccount(String),

    /// All lines of an entry (and their accounts) must share one currency.
    #[error("currency mismatch: expected {expected}, found {found}")]
    CurrencyMismatch {
        /// The currency that was expected.
        expected: String,
        /// The currency that was found.
        found: String,
    },

    /// Currency codes are three ASCII letters (ISO 4217 alpha-3).
    #[error("invalid currency code '{0}': must be three ASCII letters")]
    InvalidCurrencyCode(String),

    /// Posted entries are immutable. This is load-bearing: it is what makes
    /// the ledger an audit trail instead of a spreadsheet.
    #[error("entry {0} is already posted and immutable")]
    EntryAlreadyPosted(u64),

    /// Only posted entries can be reversed.
    #[error("entry {0} is not posted yet")]
    EntryNotPosted(u64),

    /// An entry is reversed at most once: a second reversal would undo it
    /// twice. (Reversing the reversal re-instates it.)
    #[error("entry {0} is already reversed by entry {1}")]
    EntryAlreadyReversed(u64, u64),

    /// A period's closing entry is never reversed: that would leave the
    /// period closed with its earnings unswept.
    #[error("entry {0} closes a period and cannot be reversed")]
    ClosingEntryIrreversible(u64),

    /// The engine posts invoices, bills, payroll, and closes to accounts it
    /// finds by code (1200 receivables, 2000 payables, 4000 revenue, ...).
    /// An account with that code exists but is the wrong type, or a
    /// placeholder, so the engine refuses rather than post there or force
    /// the chart into its numbering (owner, 2026-10-13). The bookkeeper
    /// decides with the human how to handle it.
    #[error(
        "account {code} \"{name}\" is {}, but {role} needs account {code} to be a postable {expected:?} account; \
         wanderbooks won't post {role} there. Decide with the books' owner how to handle it: \
         assign {role} to an account of theirs, or renumber that account so {code} is free",
        if *placeholder { format!("a placeholder {found:?} account") } else { format!("a {found:?} account") }
    )]
    ControlAccountMismatch {
        /// The code the engine looks for.
        code: String,
        /// The existing account's name.
        name: String,
        /// What the engine wanted it for, e.g. "accounts receivable".
        role: String,
        /// The type that role needs.
        expected: crate::AccountType,
        /// The existing account's type.
        found: crate::AccountType,
        /// Whether the existing account is a placeholder.
        placeholder: bool,
    },

    /// An asset tag of the wrong form (see `assets::check_asset_tag`).
    #[error("bad asset tag {tag:?}: {why}")]
    InvalidAssetTag {
        /// The tag given.
        tag: String,
        /// Why it was refused.
        why: String,
    },

    /// Asset tags are never shared or reused: another asset holds, or
    /// held, this one.
    #[error("asset tag {tag:?} belongs to asset {asset}; tags are never reused, so choose another")]
    AssetTagTaken {
        /// The tag given.
        tag: String,
        /// The asset that holds or held it.
        asset: u64,
    },

    /// An asset's acquisition entry must be a posted, unreversed entry
    /// that debits the asset's cost account.
    #[error("entry {entry} can't be asset {asset}'s acquisition: {why}")]
    AcquisitionEntryMismatch {
        /// The asset.
        asset: u64,
        /// The entry given.
        entry: u64,
        /// Why not.
        why: String,
    },

    /// An asset's basis record doesn't fit its rule (see
    /// `Ledger::set_asset_basis`).
    #[error("bad tax basis: {0}")]
    InvalidTaxBasis(String),

    /// An asset can't be in service before the company acquired it: its
    /// in-service date is before its acquisition entry's date.
    #[error(
        "asset {asset} can't be in service on {in_service}, before it was acquired on {acquired} (its acquisition entry's date): correct the in-service date, or the entry"
    )]
    InServiceBeforeAcquisition {
        /// The asset.
        asset: u64,
        /// The in-service date given.
        in_service: String,
        /// The acquisition entry's date.
        acquired: String,
    },

    /// A depreciation run that doesn't fit its asset or entry (see
    /// `Ledger::check_depreciation_run`).
    #[error("bad depreciation run for asset {asset}: {why}")]
    InvalidDepreciationRun {
        /// The asset.
        asset: u64,
        /// Why not.
        why: String,
    },

    /// No party has this id.
    #[error("unknown party {0}")]
    UnknownParty(u64),

    /// A party command that breaks a rule of the directory
    /// (`crate::parties`).
    #[error("{0}")]
    InvalidParty(String),

    /// A serial number of the wrong form (see `assets::check_serial`).
    #[error("bad serial number: {0}")]
    InvalidSerial(String),

    /// No control role has that name (see `ledger_core::control_accounts`).
    #[error(
        "no role {0:?}; the roles are receivables, revenue, payables, general_expense, \
         retained_earnings, salary_expense, payroll_tax_expense, federal_withholding, \
         state_withholding, social_security_payable, medicare_payable, other_deductions_payable"
    )]
    UnknownRole(String),

    /// One account per role: an account already filling a role can't take
    /// a second (its balance would mix what the two control).
    #[error("account {code} \"{name}\" already holds {role}; give each role its own account")]
    RoleTaken {
        /// The account's code.
        code: String,
        /// The account's name.
        name: String,
        /// The role it fills, e.g. "accounts receivable".
        role: String,
    },

    /// A balance sheet role (withholdings, retained earnings) can't move
    /// while its account holds a balance: the control account must keep
    /// agreeing with what it controls. Receivables and payables refuse
    /// with [`Self::DocumentRoleHoldsBalance`] instead.
    #[error(
        "{role} is in account {code} \"{name}\", which holds {balance}; move that balance to the \
         new account with a journal entry first, then assign the role"
    )]
    RoleHoldsBalance {
        /// The role, e.g. "accounts receivable".
        role: String,
        /// The code of the account filling it now.
        code: String,
        /// That account's name.
        name: String,
        /// Its balance.
        balance: rust_decimal::Decimal,
    },

    /// A rule set by law (a dollar threshold, a rate) isn't known to
    /// wanderbooks for the year asked; the engine never guesses (see
    /// [`crate::rules`]). The text names the authority, never this repo:
    /// it reaches agents.
    #[error(
        "wanderbooks knows the {rule} only through {known_through}, not for {year}. \
         Find the {year} figure from the official source, {authority}, or ask the books' \
         owner or CPA, and pass it explicitly. Then tell the human the figure and where \
         it came from, so the rule can be updated for everyone."
    )]
    RuleNotKnown {
        /// The rule's plain name.
        rule: String,
        /// The year asked for.
        year: i32,
        /// The last year the rule is known for.
        known_through: i32,
        /// The official source of the rule (`DatedRule::authority`).
        authority: String,
    },

    /// A child account subdivides its parent, so it must be the same type
    /// (owner, 2026-10-13).
    #[error(
        "account {code} is {found:?}, but its parent {parent_code} is {expected:?}; a child account must be the same type as its parent"
    )]
    ParentTypeMismatch {
        /// The new account's code.
        code: String,
        /// The parent's code.
        parent_code: String,
        /// The parent's type, which the child must share.
        expected: crate::AccountType,
        /// The type the child was given.
        found: crate::AccountType,
    },

    /// Periods nest (a month in a quarter in a year) but never partially
    /// overlap (owner, 2026-10-13).
    #[error(
        "period {name} ({start} to {end}) partially overlaps period {other} ({other_start} to {other_end}); periods may nest, but not overlap"
    )]
    PeriodsOverlap {
        /// The new period's name.
        name: String,
        /// Its first day.
        start: chrono::NaiveDate,
        /// Its last day.
        end: chrono::NaiveDate,
        /// The existing period's name.
        other: String,
        /// The existing period's first day.
        other_start: chrono::NaiveDate,
        /// The existing period's last day.
        other_end: chrono::NaiveDate,
    },

    /// A payment can't apply to an invoice or bill dated after it (owner,
    /// 2026-10-13); a real prepayment is a deposit, recorded differently.
    #[error("a payment dated {date} can't apply to a document dated {document_date}")]
    PaymentBeforeDocument {
        /// The payment's date.
        date: chrono::NaiveDate,
        /// The invoice's or bill's date.
        document_date: chrono::NaiveDate,
    },

    /// The referenced journal entry does not exist.
    #[error("entry {0} does not exist")]
    UnknownEntry(u64),

    /// Account codes must be unique within a chart of accounts.
    #[error("account code '{0}' is already in use")]
    DuplicateAccountCode(String),

    /// Account codes must not be empty.
    #[error("account code must not be empty")]
    EmptyAccountCode,

    /// An account's name is empty or only spaces: a chart lists every
    /// account by number and title.
    #[error("account name must not be empty")]
    EmptyAccountName,

    /// A rename to the name the account already has: there is nothing to
    /// journal.
    #[error("account {code} is already named {name:?}: nothing to rename")]
    AccountNameUnchanged {
        /// The account's code.
        code: String,
        /// Its name.
        name: String,
    },

    /// A new name (a new account's, or a rename's) that another account
    /// under the same parent already has, ignoring capitals: QuickBooks
    /// matches accounts by name (docs/spec/sources/intuit-iif.md), so the
    /// CPA's import would merge the two.
    #[error(
        "{sibling} already has the name {name:?} under the same parent (names compare ignoring capitals): QuickBooks matches accounts by name and would merge the two, so choose another name"
    )]
    DuplicateAccountName {
        /// The name refused.
        name: String,
        /// The account that has it, `"CODE Name"`.
        sibling: String,
    },

    /// A new account name holding a run of nine digits: it reads as a full
    /// SSN, EIN or account number, and every report prints it.
    #[error(
        "the account name holds nine digits in a row, which read as a full SSN, EIN or account number, and every report prints it: write only the last four (a full TIN goes only into the vault, books.set_tin)"
    )]
    AccountNameHoldsTin,

    /// A new account name (a new account's, or a rename's) that isn't one
    /// line of text within the bound ([`crate::account::MAX_ACCOUNT_NAME`]
    /// characters): every report, export and the IIF print it on one line.
    #[error("account name {0}")]
    InvalidAccountName(String),

    /// A rename's reason the journal can't keep: blank, too long, holding
    /// a control character or a full TIN.
    #[error("rename reason: {0}")]
    InvalidRenameReason(String),

    /// `policy.require_tax_category = refuse`, and a revenue or expense
    /// account (not a placeholder) would be created, or left, without a
    /// tax category.
    #[error("{}", tax_category_required_text(account, *clearing))]
    TaxCategoryRequired {
        /// The account, `"CODE Name"`.
        account: String,
        /// Whether the command would clear the category it has.
        clearing: bool,
    },

    /// The referenced parent account does not exist.
    #[error("unknown parent account id {0}")]
    UnknownParentAccount(u64),

    /// Any attempt to mutate a posted entry: add lines, delete it, edit it.
    #[error("cannot modify a posted entry; post a reversing entry instead")]
    PostedEntryImmutable,

    /// The referenced fixed asset does not exist.
    #[error("unknown asset id {0}")]
    UnknownAsset(u64),

    /// Asset descriptions must not be empty.
    #[error("asset description must not be empty")]
    EmptyAssetDescription,

    /// Asset cost must be positive.
    #[error("asset cost must be positive, got {0}")]
    InvalidAssetCost(Decimal),

    /// Salvage value must be non-negative and less than cost.
    #[error("salvage value must be between zero and cost, got {0}")]
    InvalidSalvageValue(Decimal),

    /// Useful life must be at least one month.
    #[error("useful life must be at least one month, got {0}")]
    InvalidUsefulLife(u32),

    /// The asset was already disposed.
    #[error("asset {0} is already disposed")]
    AssetAlreadyDisposed(u64),

    /// Asset account links must have the right types: cost and accumulated
    /// depreciation are asset accounts, the depreciation expense account is
    /// an expense account.
    #[error("account '{code}' has type {found}, but the asset {role} account must be {expected}")]
    AssetAccountWrongType {
        /// Account code.
        code: String,
        /// Role the account was given: "cost", "accumulated depreciation", or
        /// "depreciation expense".
        role: &'static str,
        /// The type it must be.
        expected: &'static str,
        /// The type it has.
        found: &'static str,
    },

    /// Disposal proceeds must not be negative.
    #[error("disposal proceeds must not be negative, got {0}")]
    NegativeProceeds(Decimal),

    /// A disposal can't come before the company acquired the asset (its
    /// acquisition entry's date, or its in-service date when none is
    /// linked).
    #[error("disposal date {date} is before the asset was acquired ({acquired})")]
    DisposalBeforeAcquisition {
        /// The disposal date given.
        date: String,
        /// The acquired date.
        acquired: String,
    },

    /// The three asset account links must be three distinct accounts.
    #[error("asset account links must be distinct; account id {0} is linked twice")]
    DuplicateAssetAccount(u64),

    /// in-service date + useful life overflows the date range: a new
    /// asset's last day of depreciation is 9999-12-31 at the latest.
    #[error("in-service date plus useful life overflows the calendar")]
    AssetDateOverflow,

    /// A depreciation run for a month that can't be run: not a month, or
    /// one ending before the asset was placed in service.
    #[error("invalid depreciation period: {0}")]
    InvalidDepreciationPeriod(String),

    /// An entry of a fixed asset's that can't be reversed (its disposal,
    /// or depreciation the disposal took as final), with why.
    #[error("entry {0} can't be reversed: {1}")]
    AssetEntryIrreversible(u64, String),

    /// A disposal refused because a depreciation run of the asset carries
    /// at the end of the disposal's month other than what it carries now
    /// (a run of a later month that stands; a reversal or re-instatement
    /// dated after the month, not undone), with what to do.
    #[error("the disposal is dated before depreciation the books carry: {0}")]
    DepreciationPastDisposal(String),

    /// An amortization schedule or recurring entry is invalid, unknown, or
    /// already posted for that date.
    #[error("invalid schedule: {0}")]
    InvalidSchedule(String),

    /// A payout doesn't reconcile or uses the wrong accounts.
    #[error("invalid payout: {0}")]
    InvalidPayout(String),

    /// A tax category doesn't exist, doesn't fit the account, or has a bad
    /// deductible percent.
    #[error("invalid tax category: {0}")]
    InvalidTaxCategory(String),

    /// Partnership members are misconfigured (shares, capital accounts).
    #[error("invalid members: {0}")]
    InvalidMembers(String),

    /// A report period's end is before its start.
    #[error("invalid date range: end {end} is before start {start}")]
    InvalidDateRange {
        /// Period start, YYYY-MM-DD.
        start: String,
        /// Period end, YYYY-MM-DD.
        end: String,
    },

    /// The statement of cash flows would not foot (rules.md B-14 rule 9):
    /// an engine error, never output. It cannot happen on balanced books
    /// (Lean `cashflow_sections_sum_to_cash_change`); if it does, the
    /// engine is wrong, and nothing is shown rather than a wrong figure.
    #[error(
        "engine error: the cash flow statement does not foot ({what}: {found} against {expected}); nothing was produced, please report it"
    )]
    CashFlowDoesNotFoot {
        /// Which check failed: "sections", "investing lines", "operating
        /// reconciliation", "entry #N", ...
        what: String,
        /// The figure it should equal.
        expected: Decimal,
        /// The figure it came to.
        found: Decimal,
    },

    /// The cash flow statement's account roles (cash, financing, investing)
    /// contradict each other or the chart (rules.md B-14 rule 2). The text
    /// names the setting and says what to change.
    #[error("cash flow account roles: {0}")]
    CashFlowRoles(String),

    /// A report period given only a start, after today: an omitted end is
    /// today (rules.md B-30), so the period would end before it starts.
    #[error(
        "invalid date range: start {start} is after the end, which defaults to today ({today}): pass end"
    )]
    StartAfterToday {
        /// Period start, YYYY-MM-DD.
        start: String,
        /// Today, the end an omitted end means, YYYY-MM-DD.
        today: String,
    },

    /// The referenced accounting period does not exist.
    #[error("unknown period id {0}")]
    UnknownPeriod(u64),

    /// The accounting period is already closed.
    #[error("period {0} is already closed")]
    PeriodAlreadyClosed(u64),

    /// Period end is before period start.
    #[error("invalid period: end {end} is before start {start}")]
    InvalidPeriodRange {
        /// Period start, YYYY-MM-DD.
        start: String,
        /// Period end, YYYY-MM-DD.
        end: String,
    },

    /// A draft entry exists with a date inside the period being closed.
    #[error("cannot close period {0}: draft entries exist with dates in the period")]
    DraftsInPeriod(u64),

    /// A period can't close while a later period is already closed.
    #[error("period {0} can't close: later period {1} is already closed (close in order)")]
    CloseOutOfOrder(u64, u64),

    /// Entries cannot be created or posted in a closed period.
    #[error("period {0} is closed; entries dated {1} are rejected")]
    ClosedPeriod(u64, String),

    /// The referenced customer does not exist.
    #[error("unknown customer id {0}")]
    UnknownCustomer(u64),

    /// The referenced vendor does not exist.
    #[error("unknown vendor id {0}")]
    UnknownVendor(u64),

    /// A 1099 decision, or the 1099 report, was refused: the message says
    /// why and what to do.
    #[error("{0}")]
    Invalid1099(String),

    /// The payment method named does not exist.
    #[error("unknown payment method (instrument) id {0}")]
    UnknownInstrument(u64),

    /// A payment method (instrument) was refused: the message says why.
    #[error("{0}")]
    InvalidInstrument(String),

    /// The referenced invoice does not exist.
    #[error("unknown invoice id {0}")]
    UnknownInvoice(u64),

    /// The referenced bill does not exist.
    #[error("unknown bill id {0}")]
    UnknownBill(u64),

    /// Invoice/bill must have at least one line.
    #[error("invoice/bill must have at least one line")]
    EmptyDocumentLines,

    /// Invoice/bill line amounts must be positive.
    #[error("document line amount must be positive, got {0}")]
    InvalidDocumentLineAmount(Decimal),

    /// Payment amount must be positive.
    #[error("payment amount must be positive, got {0}")]
    InvalidPaymentAmount(Decimal),

    /// Payment exceeds the remaining balance.
    #[error("payment of {0} exceeds remaining balance of {1}")]
    Overpayment(Decimal, Decimal),

    /// A payment's details were refused (its reference): the message says
    /// why.
    #[error("{0}")]
    InvalidPayment(String),

    /// Receivables or payables can't move while their account holds a
    /// balance (rules.md B-23, B-38): that balance is the documents', and
    /// a hand entry moving it is refused by the control guard; it moves
    /// with its documents (plan P4b S7).
    #[error(
        "{role} is in account {code} \"{name}\", which holds {balance} owed on its documents; \
         the role can't move until that is settled (paid or voided) or until moving a balance \
         with its documents is built: a hand entry to the account is refused"
    )]
    DocumentRoleHoldsBalance {
        /// The role, e.g. "accounts receivable".
        role: String,
        /// The code of the account filling it now.
        code: String,
        /// That account's name.
        name: String,
        /// Its balance.
        balance: rust_decimal::Decimal,
    },

    /// Receivables or payables can't move to an account holding lines
    /// that are no document's (rules.md B-38 INVARIANT 1): they would
    /// become hand lines on a control account.
    #[error(
        "account {code} \"{name}\" holds lines no invoice or bill carries (entry {entry} first), \
         so it can't take the {role} role: give the role an account of its own"
    )]
    RoleAccountHasHandLines {
        /// The role, e.g. "accounts receivable".
        role: String,
        /// The account's code.
        code: String,
        /// Its name.
        name: String,
        /// The first posted entry with such a line.
        entry: u64,
    },

    /// A payment's deposit or payer account must be a postable asset or
    /// liability that holds no role: a bank, a card, a member's payable
    /// (plan P4b.3, "Payment accounts").
    #[error(
        "account {code} \"{name}\" can't take a payment: {why}; pay into or from a bank or card account (or a member's payable)"
    )]
    PaymentAccount {
        /// The account's code.
        code: String,
        /// Its name.
        name: String,
        /// Why not, e.g. "it holds the accounts receivable role".
        why: String,
    },

    /// A line on a receivables or payables control account carries no
    /// document (rules.md B-23, B-38): the control account must always
    /// equal its documents, so only the document commands post there.
    #[error(
        "account {account} is a control account (its balance is its invoices or bills): {at} can't be posted there by hand. Use books.create_invoice, books.receive_payment, books.create_bill or books.pay_bill; to void a document, books.reverse_entry of its entry"
    )]
    UntaggedControlLine {
        /// The account's code and name, e.g. `1200 "Accounts Receivable"`.
        account: String,
        /// Which line, e.g. "line 1 of entry 14", "line 0 of the
        /// recurring template".
        at: String,
    },

    /// A reversal or payment would leave a document owing less than nothing
    /// or more than its total at some date (rules.md B-38 INVARIANT 2): the
    /// message says which document, when, and what to reverse first.
    #[error("{0}")]
    DocumentBounds(String),

    /// A reversal of an entry an invoice or bill owns is dated before that
    /// entry (rules.md B-9, B-38): a correction comes after what it
    /// corrects.
    #[error(
        "entry {entry} belongs to {document} and is dated {original}: its reversal can't be dated before it, on {date}"
    )]
    DocumentReversalBeforeOriginal {
        /// The entry reversed.
        entry: u64,
        /// The document, e.g. "invoice 7".
        document: String,
        /// The entry's date.
        original: chrono::NaiveDate,
        /// The reversal's date.
        date: chrono::NaiveDate,
    },

    /// A document can't be voided as asked: the message says why.
    #[error("{0}")]
    InvalidVoid(String),

    /// Invoice/bill due date is before its date.
    #[error("due date {0} is before document date")]
    DueBeforeDate(String),

    /// Line index out of range for cleared-flag update.
    #[error("line index {0} out of range for entry {1}")]
    LineIndexOutOfRange(usize, u64),

    /// Cannot set a cleared flag on a draft entry's line.
    #[error("cannot mark cleared on draft entry {0}; only posted lines can clear")]
    ClearedOnDraft(u64),

    /// FX rate must be positive.
    #[error("fx rate must be positive, got {0}")]
    InvalidFxRate(Decimal),

    /// FX line requires both foreign currency and foreign amount.
    #[error("fx line requires foreign currency, foreign amount, and fx rate")]
    IncompleteFxLine,

    /// Replay recorded an id the allocator was not expecting: the journal
    /// and the engine disagree about event ordering.
    #[error("replay id mismatch: journal says {0}, allocator expects {1}")]
    ReplayIdMismatch(u64, u64),

    /// A payroll run must pay some wages: at least one employee line with
    /// gross pay.
    #[error("payroll run must have at least one employee with gross pay")]
    EmptyPayroll,

    /// Payroll amounts must not be negative.
    #[error("payroll amount must not be negative, got {0}")]
    NegativePayrollAmount(Decimal),

    /// Net pay went negative: withholdings exceed gross for this employee.
    #[error("payroll net pay for '{0}' is negative")]
    NegativeNetPay(String),

    /// No payroll tax payables are outstanding: nothing to remit.
    #[error("no payroll tax payables to remit")]
    NothingToRemit,

    /// The referenced payroll run does not exist.
    #[error("unknown payroll run id {0}")]
    UnknownPayrollRun(u64),

    /// Class (project) names must not be blank.
    #[error("a project (class) name is blank")]
    EmptyClassName,

    /// The referenced class does not exist.
    #[error("unknown class id {0}")]
    UnknownClass(u32),

    /// Another class (project) already has this name, ignoring capitals
    /// (rules.md B-37).
    #[error("a project named {0:?} already exists: names are unique, ignoring capitals")]
    DuplicateClassName(String),

    /// A project's facts the books can't take (rules.md B-37): the reason.
    #[error("{0}")]
    InvalidProject(String),

    /// A setting change the books can't honor (crate::settings): an
    /// unknown key, a value its key refuses, or settings that wouldn't
    /// hold together.
    #[error("{0}")]
    InvalidSetting(String),

    /// A convention, decision, open item or approval the books can't
    /// take (crate::conventions).
    #[error("{0}")]
    InvalidConvention(String),

    /// Approval rules are enforced (`policy.approval_rules = enforce`) and
    /// rules cover this entry that its approvers haven't approved as it
    /// stands.
    #[error(
        "entry {entry} needs approval before it posts: {missing}; record each with books.approve_entry"
    )]
    ApprovalRequired {
        /// The draft.
        entry: u64,
        /// Each rule and its approver, e.g. `over-500 (approver: Pat)`.
        missing: String,
    },

    /// Approval rules are enforced, and the entry an operation would post
    /// at once (an invoice, a bill, a payment, payroll, a disposal) is one
    /// a rule covers: it has no draft to approve.
    #[error(
        "approval rules cover the entry this would post at once ({0}): draft it instead (books.draft_entry), have it approved (books.approve_entry), then post it"
    )]
    ApprovalNeedsDraft(String),
}

/// The refusal [`LedgerError::TaxCategoryRequired`] gives: what to pass,
/// and where the categories are listed.
fn tax_category_required_text(account: &str, clearing: bool) -> String {
    if clearing {
        format!(
            "policy.require_tax_category is refuse: {account} keeps a tax category (give it another with books.set_tax_category; books.tax_categories lists them)"
        )
    } else {
        format!(
            "policy.require_tax_category is refuse: give {account} a tax_category (books.tax_categories lists them)"
        )
    }
}
