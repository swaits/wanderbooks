//! Reports: pure functions of a [`Ledger`].
//!
//! Reports never mutate and never guess. They are what the semantic oracle
//! (`docs/oracle.md`) diffs: the same fixture books through two engines must
//! produce byte-identical [`snapshot`] output.

use crate::{AccountType, Class, ClassId, JournalEntry, JournalLine, Ledger, LedgerError};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One line of a trial balance.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrialBalanceLine {
    /// Account code, e.g. `"1000"`.
    pub code: String,
    /// Account name, e.g. `"Cash"`.
    pub name: String,
    /// Debit balance, or zero.
    pub debit: Decimal,
    /// Credit balance, or zero.
    pub credit: Decimal,
}

/// The trial balance: every account with a non-zero balance, debit and credit
/// columns, and column totals. A correct book always has equal totals.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrialBalance {
    /// Lines sorted by account code. Deterministic order matters for goldens.
    pub lines: Vec<TrialBalanceLine>,
    /// Sum of the debit column.
    pub total_debits: Decimal,
    /// Sum of the credit column.
    pub total_credits: Decimal,
}

impl TrialBalance {
    /// The fundamental check: debits equal credits, exactly.
    pub fn balanced(&self) -> bool {
        self.total_debits == self.total_credits
    }
}

/// Build the trial balance from all posted entries.
///
/// ```text
/// GOAL: the trial balance over every posted entry (see trial_balance_as_of).
/// SOURCE: docs/spec/rules.md B-6; R-7.
/// INVARIANTS:
///   1. MUST equal trial_balance_as_of on or after the last entry's date.
///   2. MUST balance (total debits == total credits).
/// ```
pub fn trial_balance(ledger: &Ledger) -> TrialBalance {
    trial_balance_where(ledger, &|_| true)
}

/// The trial balance as of a date: every posted entry dated on or before
/// it (closing entries included, so after a close it is the post-closing
/// trial balance).
///
/// ```text
/// GOAL: every account's balance, in a debit or a credit column, from the
///       posted entries dated on or before `as_of`.
/// SOURCE: docs/spec/rules.md B-6 (Pacioli ch. 36; IRS Pub. 583); R-7, R-9b.
/// INVARIANTS:
///   1. MUST have total debits equal total credits, for any books built
///      through the Ledger commands.
///   2. MUST list each non-placeholder account with a nonzero balance
///      exactly once, in code order; NEVER list a placeholder or a zero
///      balance.
///   3. MUST put each balance in exactly one column, never negative.
///   4. NEVER count a draft, or an entry dated after `as_of`; closing
///      entries dated on or before it count (after a close this is the
///      post-closing trial balance).
/// SCENARIOS:
///   1. IF no entries are posted by `as_of`:
///      - SHOULD be empty with zero totals.
///   2. IF an asset was debited 100 and credited 30:
///      - SHOULD show it with debit 70, credit 0.
///      BECAUSE the trial balance shows each account's net balance on the
///      side it falls.
/// ```
pub fn trial_balance_as_of(ledger: &Ledger, as_of: NaiveDate) -> TrialBalance {
    trial_balance_where(ledger, &|e| e.date() <= as_of)
}

fn trial_balance_where(ledger: &Ledger, include: &dyn Fn(&JournalEntry) -> bool) -> TrialBalance {
    let mut movements: BTreeMap<crate::AccountId, Decimal> = BTreeMap::new();
    for entry in ledger.posted_entries().filter(|e| include(e)) {
        for line in entry.lines() {
            let signed = crate::kernel::signed(line.side, line.amount.amount)
                .expect("Decimal negation is exact");
            *movements.entry(line.account).or_default() += signed;
        }
    }
    let mut accounts: Vec<_> = ledger.chart().all().collect();
    accounts.sort_by(|a, b| a.code.cmp(&b.code));

    let mut lines = Vec::new();
    let mut total_debits = Decimal::ZERO;
    let mut total_credits = Decimal::ZERO;
    for account in accounts {
        if account.placeholder {
            continue;
        }
        let movement = movements.get(&account.id).copied().unwrap_or_default();
        if movement.is_zero() {
            continue;
        }
        let (debit, credit) = crate::kernel::columns(movement).expect("Decimal negation is exact");
        total_debits += debit;
        total_credits += credit;
        lines.push(TrialBalanceLine {
            code: account.code.clone(),
            name: account.name.clone(),
            debit,
            credit,
        });
    }
    TrialBalance {
        lines,
        total_debits,
        total_credits,
    }
}

/// Revenue and expense totals: from [`unclosed_earnings`], what the books
/// hold now (closed periods already moved to equity); for a date range,
/// with a line per account, use [`profit_and_loss_for_period`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfitAndLoss {
    /// Total revenue (credit-normal balances, reported positive).
    pub revenue: Decimal,
    /// Total expenses (debit-normal balances, reported positive).
    pub expenses: Decimal,
}

impl ProfitAndLoss {
    /// Revenue minus expenses. Positive is profit.
    pub fn net_income(&self) -> Decimal {
        self.revenue - self.expenses
    }
}

/// Earnings not yet closed: revenue and expenses over every posted entry,
/// closing entries included, so a closed period's income (already moved
/// to equity) nets out. This is the balance sheet's current-earnings line,
/// not a profit and loss for any period (docs/testing.md #3; OpenStax,
/// Principles of Financial Accounting 5.1).
pub fn unclosed_earnings(ledger: &Ledger) -> Result<ProfitAndLoss, LedgerError> {
    profit_and_loss_filtered(ledger, None)
}

/// Build the profit and loss statement, optionally filtered to one class.
///
/// With `None` every line counts and the result is identical to
/// [`unclosed_earnings`]. With `Some(class)` only lines tagged with that
/// class count. Unlike the account-at-a-time accumulation this works line
/// by line, because classes live on lines: each line contributes its signed
/// amount (positive on the account's normal side) to its account type's
/// total.
pub fn profit_and_loss_filtered(
    ledger: &Ledger,
    class_id: Option<ClassId>,
) -> Result<ProfitAndLoss, LedgerError> {
    profit_and_loss_where(ledger, &|line| match class_id {
        None => true,
        Some(c) => line.class_id == Some(c),
    })
}

/// P&L for one class. `class: None` is the unclassified bucket: lines with
/// no class tag.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClassProfitAndLoss {
    /// The class, or `None` for unclassified lines.
    pub class: Option<Class>,
    /// That bucket's profit and loss.
    pub profit_and_loss: ProfitAndLoss,
}

/// Profit and loss broken down by class: one row per class that has any P&L
/// lines, plus an unclassified row when untagged P&L lines exist. Rows are
/// sorted by class name; the unclassified row comes last.
///
/// The rows partition the ledger: summing every row's revenue and expenses
/// reproduces [`unclosed_earnings`].
///
/// ```text
/// GOAL: a profit and loss per class (department, location, job), plus the
///       unclassified lines.
/// SOURCE: docs/spec/rules.md B-14; R-? Tracking classes.
/// INVARIANTS:
///   1. The per-class P&Ls (with unclassified) MUST sum to the whole P&L,
///      account by account (conservation).
///   2. NEVER count a line in two classes.
/// ```
pub fn profit_and_loss_by_class(ledger: &Ledger) -> Result<Vec<ClassProfitAndLoss>, LedgerError> {
    // Which classes actually have P&L lines, and whether any P&L line is
    // unclassified. Balance-sheet-only classes get no row.
    let mut buckets: BTreeSet<Option<ClassId>> = BTreeSet::new();
    for entry in ledger.posted_entries() {
        for line in entry.lines() {
            let account = ledger
                .chart()
                .get(line.account)
                .ok_or(LedgerError::UnknownAccount(line.account))?;
            if account.placeholder {
                continue;
            }
            if !matches!(
                account.account_type,
                AccountType::Revenue | AccountType::Expense
            ) {
                continue;
            }
            buckets.insert(line.class_id);
        }
    }
    let mut classes: Vec<&Class> = buckets
        .iter()
        .filter_map(|c| c.and_then(|id| ledger.get_class(id)))
        .collect();
    classes.sort_by(|a, b| a.name.cmp(&b.name));
    let mut rows = Vec::with_capacity(classes.len() + 1);
    for class in classes {
        let id = class.id;
        rows.push(ClassProfitAndLoss {
            class: Some(class.clone()),
            profit_and_loss: profit_and_loss_where(ledger, &|line| line.class_id == Some(id))?,
        });
    }
    if buckets.contains(&None) {
        rows.push(ClassProfitAndLoss {
            class: None,
            profit_and_loss: profit_and_loss_where(ledger, &|line| line.class_id.is_none())?,
        });
    }
    Ok(rows)
}

/// P&L over the posted lines accepted by `include`. Each line contributes
/// its signed amount — positive on the account's normal side — to its
/// account type's total.
fn profit_and_loss_where(
    ledger: &Ledger,
    include: &dyn Fn(&JournalLine) -> bool,
) -> Result<ProfitAndLoss, LedgerError> {
    let mut revenue = Decimal::ZERO;
    let mut expenses = Decimal::ZERO;
    for entry in ledger.posted_entries() {
        for line in entry.lines() {
            if !include(line) {
                continue;
            }
            let account = ledger
                .chart()
                .get(line.account)
                .ok_or(LedgerError::UnknownAccount(line.account))?;
            if account.placeholder {
                continue;
            }
            let signed = if line.side == account.account_type.normal_side() {
                line.amount.amount
            } else {
                -line.amount.amount
            };
            match account.account_type {
                AccountType::Revenue => revenue += signed,
                AccountType::Expense => expenses += signed,
                _ => {}
            }
        }
    }
    Ok(ProfitAndLoss { revenue, expenses })
}

/// Balance sheet: assets, liabilities, and equity.
///
/// Because day-one ledgers have no closing entries yet (R-7), current
/// earnings (net income to date) are folded into equity so that the
/// accounting equation holds on a live book: assets = liabilities + equity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BalanceSheet {
    /// Total assets (debit-normal balances, reported positive).
    pub assets: Decimal,
    /// Total liabilities (credit-normal balances, reported positive).
    pub liabilities: Decimal,
    /// Total equity *including* current earnings.
    pub equity: Decimal,
    /// Net income to date, folded into equity above.
    pub current_earnings: Decimal,
}

impl BalanceSheet {
    /// The accounting equation: assets == liabilities + equity, exactly.
    pub fn balanced(&self) -> bool {
        self.assets == self.liabilities + self.equity
    }
}

/// Build the balance sheet from all posted entries.
pub fn balance_sheet(ledger: &Ledger) -> Result<BalanceSheet, LedgerError> {
    let mut assets = Decimal::ZERO;
    let mut liabilities = Decimal::ZERO;
    let mut equity = Decimal::ZERO;
    for account in ledger.chart().all() {
        if account.placeholder {
            continue;
        }
        let balance = ledger.account_balance(account.id)?;
        match account.account_type {
            AccountType::Asset => assets += balance,
            AccountType::Liability => liabilities += balance,
            AccountType::Equity => equity += balance,
            AccountType::Revenue | AccountType::Expense => {}
        }
    }
    let current_earnings = unclosed_earnings(ledger)?.net_income();
    equity += current_earnings;
    Ok(BalanceSheet {
        assets,
        liabilities,
        equity,
        current_earnings,
    })
}

/// One account's line on a financial statement.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatementLine {
    /// Account code, e.g. `"6000"`.
    pub code: String,
    /// Account name.
    pub name: String,
    /// Amount on the account's normal side: positive is a normal balance
    /// (or movement), negative a contra one.
    pub amount: Decimal,
}

/// Profit and loss for a date range, with a line per account.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfitAndLossStatement {
    /// First date, inclusive.
    pub start: NaiveDate,
    /// Last date, inclusive.
    pub end: NaiveDate,
    /// Revenue accounts with movement in the range, by code.
    pub revenue: Vec<StatementLine>,
    /// Expense accounts with movement in the range, by code.
    pub expenses: Vec<StatementLine>,
    /// Sum of the revenue lines.
    pub total_revenue: Decimal,
    /// Sum of the expense lines.
    pub total_expenses: Decimal,
}

impl ProfitAndLossStatement {
    /// Revenue minus expenses. Positive is profit.
    pub fn net_income(&self) -> Decimal {
        self.total_revenue - self.total_expenses
    }
}

/// Balance sheet as of a date, with a line per account.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BalanceSheetStatement {
    /// The as-of date, inclusive.
    pub as_of: NaiveDate,
    /// Asset accounts with a balance, by code.
    pub assets: Vec<StatementLine>,
    /// Liability accounts with a balance, by code.
    pub liabilities: Vec<StatementLine>,
    /// Equity accounts with a balance, by code (current earnings excluded).
    pub equity: Vec<StatementLine>,
    /// Revenue minus expenses through `as_of` not yet closed to retained
    /// earnings. Shown as its own equity line.
    pub current_earnings: Decimal,
    /// Sum of the asset lines.
    pub total_assets: Decimal,
    /// Sum of the liability lines.
    pub total_liabilities: Decimal,
    /// Sum of the equity lines plus current earnings.
    pub total_equity: Decimal,
}

impl BalanceSheetStatement {
    /// The accounting equation: assets == liabilities + equity, exactly.
    pub fn balanced(&self) -> bool {
        self.total_assets == self.total_liabilities + self.total_equity
    }
}

/// Normal-side amount per account over the posted lines accepted by
/// `include(entry, line)`, as statement lines keyed (and so sorted) by
/// code, skipping placeholders and zero amounts.
fn statement_lines(
    ledger: &Ledger,
    include: &dyn Fn(&JournalEntry, &JournalLine) -> bool,
) -> Result<BTreeMap<String, (AccountType, StatementLine)>, LedgerError> {
    let mut movement: BTreeMap<crate::AccountId, Decimal> = BTreeMap::new();
    for entry in ledger.posted_entries() {
        for line in entry.lines() {
            if !include(entry, line) {
                continue;
            }
            let signed = match line.side {
                crate::Side::Debit => line.amount.amount,
                crate::Side::Credit => -line.amount.amount,
            };
            *movement.entry(line.account).or_default() += signed;
        }
    }
    let mut out = BTreeMap::new();
    for (id, m) in movement {
        let account = ledger
            .chart()
            .get(id)
            .ok_or(LedgerError::UnknownAccount(id))?;
        if account.placeholder || m.is_zero() {
            continue;
        }
        let amount = match account.account_type.normal_side() {
            crate::Side::Debit => m,
            crate::Side::Credit => -m,
        };
        out.insert(
            account.code.clone(),
            (
                account.account_type,
                StatementLine {
                    code: account.code.clone(),
                    name: account.name.clone(),
                    amount,
                },
            ),
        );
    }
    Ok(out)
}

/// Profit and loss for `[start, end]`, optionally for one class (`None`
/// counts every line). Closing entries are excluded: they move a period's
/// earnings into retained earnings, so counting them would report a closed
/// period as breaking even.
///
/// ```text
/// GOAL: revenue, expenses, and net income for the entries dated from
///       `start` through `end` (both inclusive), optionally one class's lines.
/// SOURCE: docs/spec/rules.md B-14, B-11; R-9.
/// INVARIANTS:
///   1. MUST count only revenue and expense accounts, a line per account
///      with its normal-side amount, and net income = revenue - expenses.
///   2. NEVER count a closing entry (a close moves earnings; it doesn't
///      earn or spend), a draft, or an entry outside the range.
///   3. MUST partition: for start <= m < end, the period's net income equals
///      [start, m] plus [m+1, end].
///   4. MUST equal the change in current earnings (balance sheet) across
///      the range when no close falls inside it.
///   5. With a class: MUST count only lines tagged with that class.
/// SCENARIOS:
///   1. IF a month was closed: SHOULD still report that month's earnings.
///      BECAUSE closing doesn't make a period break even (R-9).
///   2. IF `start` is after `end`: SHOULD refuse. (DESIGN: no standard governs a reversed range; docs/testing.md #1)
/// ```
pub fn profit_and_loss_for_period(
    ledger: &Ledger,
    start: NaiveDate,
    end: NaiveDate,
    class_id: Option<ClassId>,
) -> Result<ProfitAndLossStatement, LedgerError> {
    Ok(profit_and_loss_counted(ledger, start, end, class_id)?.0)
}

/// [`profit_and_loss_for_period`], and with a class how many revenue and
/// expense lines it counted, (revenue, expense): lines netting to zero
/// leave no statement line but are counted (with no class: (0, 0)).
fn profit_and_loss_counted(
    ledger: &Ledger,
    start: NaiveDate,
    end: NaiveDate,
    class_id: Option<ClassId>,
) -> Result<(ProfitAndLossStatement, (usize, usize)), LedgerError> {
    if end < start {
        return Err(LedgerError::InvalidDateRange {
            start: start.to_string(),
            end: end.to_string(),
        });
    }
    let closings = ledger.closing_entry_ids();
    let counts = std::cell::Cell::new((0usize, 0usize));
    let lines = statement_lines(ledger, &|entry, line| {
        let counted = entry.date() >= start
            && entry.date() <= end
            && !closings.contains(&entry.id())
            && class_id.is_none_or(|c| line.class_id == Some(c));
        if counted && class_id.is_some() {
            let (r, x) = counts.get();
            match ledger.chart().get(line.account).map(|a| a.account_type) {
                Some(AccountType::Revenue) => counts.set((r + 1, x)),
                Some(AccountType::Expense) => counts.set((r, x + 1)),
                _ => {}
            }
        }
        counted
    })?;
    let mut revenue = Vec::new();
    let mut expenses = Vec::new();
    for (typ, line) in lines.into_values() {
        match typ {
            AccountType::Revenue => revenue.push(line),
            AccountType::Expense => expenses.push(line),
            _ => {}
        }
    }
    let total_revenue = revenue.iter().map(|l| l.amount).sum();
    let total_expenses = expenses.iter().map(|l| l.amount).sum();
    Ok((
        ProfitAndLossStatement {
            start,
            end,
            revenue,
            expenses,
            total_revenue,
            total_expenses,
        },
        counts.get(),
    ))
}

/// A project's figures (rules.md B-37 item 3): the period's statement and
/// months, and its figures to date.
#[derive(Clone, Debug)]
pub struct ProjectFigures {
    /// The period's profit and loss, only the lines tagged to it.
    pub period: ProfitAndLossStatement,
    /// The period by calendar month: its last [`PROJECT_MONTHS`] at most.
    pub months: Vec<crate::analysis::IntervalPnl>,
    /// Each month's net to date: every tagged line from the first day
    /// the books keep through that month's last day in the period (the
    /// running net the months move toward or away from break-even; the
    /// last is `to_date`'s net). One per month in `months`.
    pub months_net_to_date: Vec<Decimal>,
    /// How many posted revenue and expense lines tagged to it fall in
    /// the period (closing entries left out): whether anything is tagged
    /// there, even lines that net to zero (a cost and its reversal).
    pub period_lines: usize,
    /// Every tagged line through the period's end.
    pub to_date: ProjectToDate,
}

impl ProjectFigures {
    /// The period's contribution margin: [`contribution_margin_pct`] of
    /// its net and revenue.
    pub fn period_margin_pct(&self) -> Option<Decimal> {
        contribution_margin_pct(self.period.net_income(), self.period.total_revenue)
    }
}

/// A project's figures to date: every posted revenue and expense line
/// tagged to it through `through`, whatever its dates (rules.md B-37).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectToDate {
    /// The last day counted.
    pub through: NaiveDate,
    /// Its tagged revenue.
    pub revenue: Decimal,
    /// Its tagged expenses.
    pub expenses: Decimal,
    /// On spec only: expenses less revenue, never below zero (the "if
    /// repaid" figure, never a receivable). `None` for any other kind.
    pub unrecovered_cost: Option<Decimal>,
    /// Its tagged revenue by account, by code.
    pub revenue_by_account: Vec<StatementLine>,
    /// Its tagged expenses by account, by code: its direct costs by
    /// category (an account credited above its cost, a reimbursement,
    /// is negative).
    pub expenses_by_account: Vec<StatementLine>,
    /// How many posted revenue lines are tagged to it through `through`
    /// (closing entries left out), even ones netting to zero: an account
    /// whose lines cancel out isn't in `revenue_by_account`.
    pub revenue_lines: usize,
    /// How many posted expense lines, likewise (a cost and its reversal,
    /// or a cost reimbursed exactly, are two lines and no account).
    pub expenses_lines: usize,
}

impl ProjectToDate {
    /// Revenue less expenses: the project's contribution before overhead
    /// (rules.md B-37 item 7), since only its own tagged lines count.
    pub fn net(&self) -> Decimal {
        self.revenue - self.expenses
    }

    /// Its contribution margin to date: [`contribution_margin_pct`] of
    /// its net and revenue.
    pub fn margin_pct(&self) -> Option<Decimal> {
        contribution_margin_pct(self.net(), self.revenue)
    }

    /// Whether any revenue or expense line is tagged to it, netting to
    /// zero or not.
    pub fn any_tagged(&self) -> bool {
        self.revenue_lines + self.expenses_lines > 0
    }
}

/// Contribution as a percent of revenue, to one place (half to even), or
/// `None` without revenue to divide by (rules.md B-37 item 7).
///
/// ```text
/// GOAL: one margin the tools and the viewer print alike.
/// SOURCE: docs/spec/rules.md B-37 item 7 (revenue less the costs traced
///         to the project, fixed and variable alike: Garrison's segment
///         margin, as a ratio; not Horngren's variable-cost contribution
///         margin; a presentation choice, not GAAP).
/// INVARIANTS:
///   1. None when revenue <= 0: a margin on no (or negative) revenue
///      means nothing.
///   2. Otherwise contribution / revenue * 100, rounded to one place,
///      half to even, always printed with that place ("100.0").
///   3. NEVER panics: None when the quotient can't be represented.
/// ```
pub fn contribution_margin_pct(contribution: Decimal, revenue: Decimal) -> Option<Decimal> {
    if revenue <= Decimal::ZERO {
        return None;
    }
    let mut x = contribution
        .checked_div(revenue)?
        .checked_mul(Decimal::ONE_HUNDRED)?
        .round_dp_with_strategy(1, rust_decimal::RoundingStrategy::MidpointNearestEven);
    x.rescale(1);
    Some(x)
}

/// The most months [`ProjectFigures::months`] holds: the period's last.
pub const PROJECT_MONTHS: u32 = 240;

/// A project's figures for `[start, end]`, by month, and to date.
///
/// ```text
/// GOAL: what a project earned and cost, from one function the tools, the
///       viewer and bookspec share, so their figures never drift.
/// SOURCE: docs/spec/rules.md B-37 item 3 (job costing), B-11, B-14.
/// INVARIANTS:
///   1. MUST be profit_and_loss_for_period with the project's class: only
///      revenue and expense lines tagged to it, closing entries left out.
///   2. The months partition the period (their nets sum to its net)
///      when it spans at most PROJECT_MONTHS months; a longer one's
///      months are its last PROJECT_MONTHS.
///   3. to_date counts every tagged line through `end`, from the first
///      day the books keep, whatever the project's own dates.
///   4. months_net_to_date[i] is the net to date through months[i].end:
///      the net before the first month plus the months' nets so far; the
///      last equals to_date's net.
/// SCENARIOS:
///   1. IF the class doesn't exist: SHOULD refuse (UnknownClass).
///   2. IF `start` is after `end`: SHOULD refuse.
/// ```
pub fn project_figures(
    ledger: &Ledger,
    id: ClassId,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<ProjectFigures, LedgerError> {
    if ledger.get_class(id).is_none() {
        return Err(LedgerError::UnknownClass(id));
    }
    let (period, (r, x)) = profit_and_loss_counted(ledger, start, end, Some(id))?;
    // The last PROJECT_MONTHS months at most: a span across the calendar
    // is a hundred thousand months, each a statement.
    let months_from = chrono::Datelike::with_day(&end, 1)
        .and_then(|m| m.checked_sub_months(chrono::Months::new(PROJECT_MONTHS - 1)))
        .map_or(start, |m| m.max(start));
    let months = crate::analysis::pnl_by_interval_for(
        ledger,
        months_from,
        end,
        crate::analysis::Interval::Month,
        Some(id),
    )?;
    let to_date = project_to_date(ledger, id, end)?;
    // The net before the first month: to date less the months, which run
    // through `end` too (one scan fewer than a to date through the day
    // before them).
    let mut running = months
        .iter()
        .try_fold(to_date.net(), |t, m| t.checked_sub(m.net()))
        .ok_or(LedgerError::AmountOutOfRange)?;
    let mut months_net_to_date = Vec::with_capacity(months.len());
    for m in &months {
        running = running
            .checked_add(m.net())
            .ok_or(LedgerError::AmountOutOfRange)?;
        months_net_to_date.push(running);
    }
    Ok(ProjectFigures {
        period,
        months,
        months_net_to_date,
        period_lines: r + x,
        to_date,
    })
}

/// A project's figures to date: every posted revenue and expense line
/// tagged to it through `through` (closing entries left out), and for an
/// on-spec project its unrecovered cost (kernel::unrecovered_cost).
pub fn project_to_date(
    ledger: &Ledger,
    id: ClassId,
    through: NaiveDate,
) -> Result<ProjectToDate, LedgerError> {
    let facts = ledger
        .project_facts(id)
        .ok_or(LedgerError::UnknownClass(id))?;
    let start = crate::calendar::FIRST_DAY.min(through);
    let (pl, (revenue_lines, expenses_lines)) =
        profit_and_loss_counted(ledger, start, through, Some(id))?;
    let unrecovered_cost = match facts.kind {
        Some(crate::classes::ProjectKind::OnSpec) => Some(
            crate::kernel::unrecovered_cost(pl.total_revenue, pl.total_expenses)
                .ok_or(LedgerError::AmountOutOfRange)?,
        ),
        _ => None,
    };
    Ok(ProjectToDate {
        through,
        revenue: pl.total_revenue,
        expenses: pl.total_expenses,
        unrecovered_cost,
        revenue_by_account: pl.revenue,
        expenses_by_account: pl.expenses,
        revenue_lines,
        expenses_lines,
    })
}

/// Every entry with a line tagged to the project (drafts included),
/// newest first (by date, then id), with its project amount: its tagged
/// revenue lines less its tagged expense lines, each on its normal side.
/// A tagged balance-sheet line counts zero (rules.md B-37 item 3).
pub fn project_entries(ledger: &Ledger, id: ClassId) -> Vec<(crate::EntryId, Decimal)> {
    let mut out: Vec<(NaiveDate, crate::EntryId, Decimal)> = Vec::new();
    for eid in 1..ledger.next_entry_id() {
        let Some(entry) = ledger.get_entry(eid) else {
            continue;
        };
        let mut tagged = false;
        let mut amount = Decimal::ZERO;
        for line in entry.lines().iter().filter(|l| l.class_id == Some(id)) {
            tagged = true;
            let Some(account) = ledger.chart().get(line.account) else {
                continue;
            };
            let debit = matches!(line.side, crate::Side::Debit);
            match account.account_type {
                AccountType::Revenue => {
                    amount += if debit {
                        -line.amount.amount
                    } else {
                        line.amount.amount
                    }
                }
                AccountType::Expense => {
                    amount -= if debit {
                        line.amount.amount
                    } else {
                        -line.amount.amount
                    }
                }
                _ => {}
            }
        }
        if tagged {
            out.push((entry.date(), eid, amount));
        }
    }
    out.sort_by_key(|a| std::cmp::Reverse((a.0, a.1)));
    out.into_iter().map(|(_, e, a)| (e, a)).collect()
}

/// Balance sheet as of `as_of`: every posted entry dated on or before it.
/// Revenue and expense not yet closed to retained earnings appear as
/// current earnings inside equity, so the accounting equation holds on
/// every date, before and after closes.
///
/// ```text
/// GOAL: assets, liabilities, and equity as of a date, a line per account.
/// SOURCE: docs/spec/rules.md B-7 (CON 8 ¶E62), B-11; R-8.
/// INVARIANTS:
///   1. MUST balance: assets = liabilities + equity + current earnings,
///      exactly.
///   2. MUST count every posted entry dated on or before `as_of`, closing
///      entries included (after a close, earnings sit in retained earnings
///      or members' capital, and current earnings is zero for closed time).
///   3. Current earnings MUST be revenue minus expenses not yet closed.
///   4. NEVER list revenue or expense accounts as lines.
///   5. Lines use each account's normal-side balance (a contra balance is
///      negative).
/// SCENARIOS:
///   1. IF nothing is posted by `as_of`: SHOULD be empty and balanced.
/// ```
pub fn balance_sheet_as_of(
    ledger: &Ledger,
    as_of: NaiveDate,
) -> Result<BalanceSheetStatement, LedgerError> {
    let lines = statement_lines(ledger, &|entry, _| entry.date() <= as_of)?;
    let mut assets = Vec::new();
    let mut liabilities = Vec::new();
    let mut equity = Vec::new();
    let mut current_earnings = Decimal::ZERO;
    for (typ, line) in lines.into_values() {
        match typ {
            AccountType::Asset => assets.push(line),
            AccountType::Liability => liabilities.push(line),
            AccountType::Equity => equity.push(line),
            AccountType::Revenue => current_earnings += line.amount,
            AccountType::Expense => current_earnings -= line.amount,
        }
    }
    let total_assets = assets.iter().map(|l| l.amount).sum();
    let total_liabilities = liabilities.iter().map(|l| l.amount).sum();
    let total_equity = equity.iter().map(|l| l.amount).sum::<Decimal>() + current_earnings;
    Ok(BalanceSheetStatement {
        as_of,
        assets,
        liabilities,
        equity,
        current_earnings,
        total_assets,
        total_liabilities,
        total_equity,
    })
}

// ---------------------------------------------------------------------------
// General ledger (by account)
// ---------------------------------------------------------------------------

/// One posting in the general ledger: a journal line, seen from its account.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LedgerPosting {
    /// Entry date.
    pub date: NaiveDate,
    /// The journal entry: its other lines are the other side.
    pub entry_id: crate::EntryId,
    /// Entry description.
    pub description: String,
    /// The other side: `"CODE Name"` of the one account on the entry's
    /// opposite side, or `"-Split-"` when that side is several accounts
    /// (the QuickBooks convention).
    pub split: String,
    /// Debit amount, or zero.
    pub debit: Decimal,
    /// Credit amount, or zero.
    pub credit: Decimal,
    /// Running balance after this posting, on the account's normal side.
    pub balance: Decimal,
}

/// One account's section of the general ledger.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LedgerAccount {
    /// Account code.
    pub code: String,
    /// Account name.
    pub name: String,
    /// Account type.
    pub account_type: AccountType,
    /// Balance before the period, normal side.
    pub beginning_balance: Decimal,
    /// Postings in the period, by date then entry.
    pub postings: Vec<LedgerPosting>,
    /// Sum of the period's debits.
    pub total_debits: Decimal,
    /// Sum of the period's credits.
    pub total_credits: Decimal,
    /// Balance at period end, normal side. Ties to the trial balance.
    pub ending_balance: Decimal,
}

/// The general ledger for a period: every account with a beginning balance
/// or activity, in chart (code) order.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeneralLedger {
    /// First date, inclusive.
    pub start: NaiveDate,
    /// Last date, inclusive.
    pub end: NaiveDate,
    /// Accounts by code.
    pub accounts: Vec<LedgerAccount>,
}

/// Build the general ledger for `[start, end]`. Every journal line is a
/// posting to its account, with the entry's other side as the split, and a
/// running balance from the account's balance before `start`. Closing
/// entries are postings like any other.
///
/// ```text
/// GOAL: every posting from `start` through `end`, by account in code
///       order, with running balances.
/// SOURCE: docs/spec/rules.md B-6; R-9a (the QuickBooks general ledger).
/// INVARIANTS:
///   1. Each account MUST show its beginning balance (entries before
///      `start`), every posting in the range in date order, and an ending
///      balance equal to beginning plus the postings, on the account's
///      normal side.
///   2. Each account's ending balance MUST equal its trial-balance balance
///      as of `end` (normal side).
///   3. Each posting MUST name its split: the one account on the entry's
///      other side, or "-Split-" when the other side has several.
///   4. NEVER include drafts.
/// ```
pub fn general_ledger(
    ledger: &Ledger,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<GeneralLedger, LedgerError> {
    if end < start {
        return Err(LedgerError::InvalidDateRange {
            start: start.to_string(),
            end: end.to_string(),
        });
    }
    let display = |id: crate::AccountId| -> Result<String, LedgerError> {
        let a = ledger
            .chart()
            .get(id)
            .ok_or(LedgerError::UnknownAccount(id))?;
        Ok(format!("{} {}", a.code, a.name))
    };
    let mut entries: Vec<&JournalEntry> = ledger.posted_entries().collect();
    entries.sort_by_key(|e| (e.date(), e.id()));

    let mut begin: BTreeMap<crate::AccountId, Decimal> = BTreeMap::new();
    let mut postings: BTreeMap<crate::AccountId, Vec<(crate::Side, Decimal, LedgerPosting)>> =
        BTreeMap::new();
    for entry in &entries {
        let lines = entry.lines();
        for (i, line) in lines.iter().enumerate() {
            let signed = match line.side {
                crate::Side::Debit => line.amount.amount,
                crate::Side::Credit => -line.amount.amount,
            };
            if entry.date() < start {
                *begin.entry(line.account).or_default() += signed;
                continue;
            }
            if entry.date() > end {
                continue;
            }
            // The other side is the accounts on the opposite side of the
            // entry: a credit's split is what was debited, and vice versa.
            let others: BTreeSet<crate::AccountId> = lines
                .iter()
                .enumerate()
                .filter(|(j, l)| *j != i && l.side != line.side)
                .map(|(_, l)| l.account)
                .collect();
            let split = match others.len() {
                1 => display(*others.iter().next().expect("one"))?,
                _ => "-Split-".to_string(),
            };
            let (debit, credit) = match line.side {
                crate::Side::Debit => (line.amount.amount, Decimal::ZERO),
                crate::Side::Credit => (Decimal::ZERO, line.amount.amount),
            };
            postings.entry(line.account).or_default().push((
                line.side,
                line.amount.amount,
                LedgerPosting {
                    date: entry.date(),
                    entry_id: entry.id(),
                    description: entry.description().to_string(),
                    split,
                    debit,
                    credit,
                    balance: Decimal::ZERO,
                },
            ));
        }
    }
    let mut accounts: Vec<_> = ledger.chart().all().collect();
    accounts.sort_by(|a, b| a.code.cmp(&b.code));
    let mut out = Vec::new();
    for account in accounts {
        let begin_movement = begin.get(&account.id).copied().unwrap_or_default();
        let lines = postings.remove(&account.id).unwrap_or_default();
        if begin_movement.is_zero() && lines.is_empty() {
            continue;
        }
        let normal = account.account_type.normal_side();
        let to_normal = |m: Decimal| if normal == crate::Side::Debit { m } else { -m };
        let beginning_balance = to_normal(begin_movement) + Decimal::ZERO;
        let mut balance = beginning_balance;
        let (mut total_debits, mut total_credits) = (Decimal::ZERO, Decimal::ZERO);
        let mut posted = Vec::with_capacity(lines.len());
        for (side, amount, mut p) in lines {
            balance += if side == normal { amount } else { -amount };
            total_debits += p.debit;
            total_credits += p.credit;
            p.balance = balance;
            posted.push(p);
        }
        out.push(LedgerAccount {
            code: account.code.clone(),
            name: account.name.clone(),
            account_type: account.account_type,
            beginning_balance: zero_clean(beginning_balance),
            postings: posted,
            total_debits,
            total_credits,
            ending_balance: zero_clean(balance),
        });
    }
    Ok(GeneralLedger {
        start,
        end,
        accounts: out,
    })
}

/// Never let a negative zero reach a report.
fn zero_clean(d: Decimal) -> Decimal {
    if d.is_zero() { Decimal::ZERO } else { d }
}

// ---------------------------------------------------------------------------
// Statement of changes in members' equity (partnership capital accounts)
// ---------------------------------------------------------------------------

/// A member of a partnership (multi-member LLC): their capital account and
/// their share of income.
#[derive(Clone, Debug)]
pub struct MemberCapital {
    /// Member's name.
    pub name: String,
    /// Their capital account (equity): contributions credit it,
    /// distributions debit it.
    pub capital_account: crate::AccountId,
    /// Their distributions account (contra-equity), if distributions are
    /// recorded there rather than directly against capital. Part of their
    /// capital on every statement; zeroed into capital at close.
    pub distribution_account: Option<crate::AccountId>,
    /// Share of income, e.g. 0.5. The shares must sum to exactly 1.
    pub share: Decimal,
}

/// One member's (or the total) capital roll-forward for a period.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MemberEquityColumn {
    /// Member's name, or "Total".
    pub name: String,
    /// Capital at the start of the period, including their share of
    /// earnings not yet distributed.
    pub beginning: Decimal,
    /// Contributions during the period.
    pub contributions: Decimal,
    /// Their share of the period's net income (loss).
    pub net_income: Decimal,
    /// Distributions during the period (a positive number, subtracted).
    pub distributions: Decimal,
    /// Any other change in equity, allocated by share. Zero unless equity
    /// moved through an account that isn't a member's capital account, retained
    /// earnings, or the period's income.
    pub other: Decimal,
    /// Capital at the end of the period.
    pub ending: Decimal,
}

/// The statement of changes in members' equity: each member's capital
/// account roll-forward, plus a total that ties to the balance sheet.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MembersEquityStatement {
    /// First date, inclusive.
    pub start: NaiveDate,
    /// Last date, inclusive.
    pub end: NaiveDate,
    /// One column per member, in the order given.
    pub members: Vec<MemberEquityColumn>,
    /// Sum of the members. `ending` equals total equity on the balance
    /// sheet as of `end`.
    pub total: MemberEquityColumn,
}

/// The posted entries in `[start, end]` that cancel: each reversal dated
/// in the span with the original it reverses, when that is dated in the
/// span too. Paired in journal order, an entry at most once, so in a chain
/// (an entry, its reversal, the reversal's reversal) the first two cancel
/// and the third, which reinstates the original, counts.
pub fn cancelled_in_span(
    ledger: &Ledger,
    start: NaiveDate,
    end: NaiveDate,
) -> BTreeSet<crate::EntryId> {
    let in_span = |d: NaiveDate| d >= start && d <= end;
    let mut out = BTreeSet::new();
    for e in ledger.posted_entries().filter(|e| in_span(e.date())) {
        if let Some(o) = e.reversal_of()
            && ledger
                .get_entry(o)
                .is_some_and(|x| x.status() == crate::EntryStatus::Posted && in_span(x.date()))
            && !out.contains(&o)
            && !out.contains(&e.id())
        {
            out.insert(o);
            out.insert(e.id());
        }
    }
    out
}

/// Build the statement of changes in members' equity for `[start, end]`.
///
/// Each member's capital account supplies contributions (credits) and
/// distributions (debits). Earnings are partnership earnings: net income
/// for the period, and earnings from before it not yet distributed (retained
/// earnings plus unclosed income), are allocated by share. Allocation is to
/// the currency's minor unit, odd units by largest remainder, the same
/// running way the members closes split them, so every row's total is
/// exact and the ending total equals balance-sheet equity.
///
/// ```text
/// GOAL: each member's capital roll-forward for a partnership over a
///       period: beginning, contributions, allocated net income, distributions,
///       ending; with a total.
/// SOURCE: docs/spec/rules.md B-13 (26 U.S.C. §704(a); Treas. Reg.
///         §1.704-1(b)(2)(iv)(b); Form 1065 Schedule M-2; D-38), B-11;
///         R-9c.
/// INVARIANTS:
///   1. Each row MUST roll forward exactly: beginning + contributions +
///      income - distributions + other = ending.
///   2. The total ending MUST equal balance-sheet equity at `end`.
///   3. Undistributed earnings U MUST be split by share, to the base
///      currency's minor unit, by largest remainder (DECISION, owner
///      2026-10-13), as of each date d, the way the next members close
///      would split them: allocate(C + U) − allocate(C), where C is what
///      the members regime's closes dated on or before d closed
///      (Ledger::members_regime; D-38, P3 review R-1). A member's
///      beginning is their capital and share as of the day before
///      `start`, their ending as of `end`. Tied units, "other"'s
///      included, go to the member the count's first close listed first
///      (Regime::allocate; on a day before it, the count the next members
///      close begins), so the order `members` lists them in moves no unit
///      once a close has begun the count.
///   3a. A member's ending on one statement MUST equal their beginning on
///      the next, exactly (a capital account is one running balance of
///      contributions, allocations and distributions: Treas. Reg.
///      §1.704-1(b)(2)(iv)(b)).
///   3b. A member's income is the roll-forward's remainder: their share of
///      the period's income, off by less than two minor units (under one
///      at each end of the cumulative split); the members' incomes MUST
///      sum to the period's net income.
///   3c. A member's capital as of a date MUST NOT change when a period
///      ending on or before it is later closed to the members, except by a
///      true-up of rounding by closes made under an earlier rule, which
///      shows only once a close posts it (D-38; R-1).
///   4. NEVER hide an unexplained change: it goes in "other".
///   5. An entry dated `start` is period activity, not beginning capital.
///   6. An entry and its reversal both dated in the span cancel
///      ([`cancelled_in_span`]): neither is a contribution or a
///      distribution, since the correction undoes the original (B-9: "it
///      is just as if you had written nothing"). A reversal whose original
///      is outside the span counts by side, like any entry.
/// SCENARIOS:
///   1. IF shares don't sum to 1, or are not positive: SHOULD refuse.
///   2. A joint 1,000.00 contribution (500.00 each) reversed the next day:
///      contributions 0.00 and distributions 0.00 for each member, not
///      500.00 of each.
/// ```
pub fn members_equity(
    ledger: &Ledger,
    start: NaiveDate,
    end: NaiveDate,
    members: &[MemberCapital],
) -> Result<MembersEquityStatement, LedgerError> {
    if members.is_empty() {
        return Err(LedgerError::InvalidMembers("no members given".to_string()));
    }
    let shares: Vec<Decimal> = members.iter().map(|m| m.share).collect();
    let total_share: Decimal = shares.iter().sum();
    if total_share != Decimal::ONE || shares.iter().any(|s| *s <= Decimal::ZERO) {
        return Err(LedgerError::InvalidMembers(format!(
            "member shares must be positive and sum to 1, got {total_share}"
        )));
    }
    let mut seen = BTreeSet::new();
    for m in members {
        for id in std::iter::once(m.capital_account).chain(m.distribution_account) {
            let account = ledger
                .chart()
                .get(id)
                .ok_or(LedgerError::UnknownAccount(id))?;
            if account.account_type != AccountType::Equity || !seen.insert(id) {
                return Err(LedgerError::InvalidMembers(format!(
                    "{}: account {} must be a distinct equity account",
                    m.name, account.code
                )));
            }
        }
    }
    let day_before = start.pred_opt().ok_or(LedgerError::InvalidDateRange {
        start: start.to_string(),
        end: end.to_string(),
    })?;
    let pnl = profit_and_loss_for_period(ledger, start, end, None)?;
    let begin_bs = balance_sheet_as_of(ledger, day_before)?;
    let end_bs = balance_sheet_as_of(ledger, end)?;

    // Capital account balances before the period, and their movement in it.
    let capital_before = |id: crate::AccountId| -> Decimal {
        let mut m = Decimal::ZERO;
        for e in ledger.posted_entries().filter(|e| e.date() < start) {
            for l in e.lines().iter().filter(|l| l.account == id) {
                match l.side {
                    crate::Side::Credit => m += l.amount.amount,
                    crate::Side::Debit => m -= l.amount.amount,
                }
            }
        }
        m
    };
    // Contributions and distributions move a capital account; a closing
    // entry allocating earnings into it is income, already counted below.
    let closings = ledger.closing_entry_ids();
    let cancelled = cancelled_in_span(ledger, start, end);
    let flows = |id: crate::AccountId| -> (Decimal, Decimal) {
        let (mut credits, mut debits) = (Decimal::ZERO, Decimal::ZERO);
        for e in ledger.posted_entries().filter(|e| {
            e.date() >= start
                && e.date() <= end
                && !closings.contains(&e.id())
                && !cancelled.contains(&e.id())
        }) {
            for l in e.lines().iter().filter(|l| l.account == id) {
                match l.side {
                    crate::Side::Credit => credits += l.amount.amount,
                    crate::Side::Debit => debits += l.amount.amount,
                }
            }
        }
        (credits, debits)
    };
    // A member's capital is their capital account plus their distributions
    // account (a debit balance, so it reduces capital).
    let capital: Vec<Decimal> = members
        .iter()
        .map(|m| {
            capital_before(m.capital_account)
                + m.distribution_account.map_or(Decimal::ZERO, capital_before)
        })
        .collect();
    // (contributions, distributions): credits to capital are
    // contributions; debits to capital and net debits to the distributions
    // account are distributions.
    let flow: Vec<(Decimal, Decimal)> = members
        .iter()
        .map(|m| {
            let (credits, debits) = flows(m.capital_account);
            let (dc, dd) = m
                .distribution_account
                .map_or((Decimal::ZERO, Decimal::ZERO), flows);
            (credits, debits + dd - dc)
        })
        .collect();

    // Everything in equity that isn't a capital account is partnership
    // earnings not yet distributed. Each member's share of it is allocated
    // cumulatively, as of a date, so a member's capital on a date never
    // depends on which statement shows it: one statement's ending is the
    // next one's beginning, to the minor unit (a capital account is one
    // running balance: Treas. Reg. §1.704-1(b)(2)(iv)(b)).
    let capital_through = |id: crate::AccountId, through: NaiveDate| -> Decimal {
        let mut m = Decimal::ZERO;
        for e in ledger.posted_entries().filter(|e| e.date() <= through) {
            for l in e.lines().iter().filter(|l| l.account == id) {
                match l.side {
                    crate::Side::Credit => m += l.amount.amount,
                    crate::Side::Debit => m -= l.amount.amount,
                }
            }
        }
        m
    };
    let capital_end: Vec<Decimal> = members
        .iter()
        .map(|m| {
            capital_through(m.capital_account, end)
                + m.distribution_account
                    .map_or(Decimal::ZERO, |d| capital_through(d, end))
        })
        .collect();
    // As of a date, a member's share of the earnings not yet in capital is
    // what the next members close would give them of it: their split of
    // everything closed to these members and shares (the regime's closes
    // through that date) plus it, less their split of the closed part
    // alone. Closing a period moves earnings from here into capital
    // without changing the sum, so a member's capital on a date is the
    // same before and after its period closes (D-38); a true-up of
    // rounding by closes made under an earlier rule shows only once a
    // close posts it (P3 review R-1).
    let places = ledger.base_currency().allocation_places();
    let key: Vec<(crate::AccountId, Decimal)> = members
        .iter()
        .map(|m| (m.capital_account, m.share))
        .collect();
    let nothing = vec![Decimal::ZERO; members.len()];
    let unclosed_share = |undistributed: Decimal, regime: &crate::ledger::Regime| {
        let closed = regime.allocate(&key, regime.closed, &nothing, places)?;
        regime.allocate(&key, regime.closed + undistributed, &closed, places)
    };
    let end_regime = ledger.members_regime(&key, Some(end));
    let begin_alloc = unclosed_share(
        begin_bs.total_equity - capital.iter().sum::<Decimal>(),
        &ledger.members_regime(&key, Some(day_before)),
    )?;
    let end_alloc = unclosed_share(
        end_bs.total_equity - capital_end.iter().sum::<Decimal>(),
        &end_regime,
    )?;
    let explained = flow.iter().map(|(c, d)| c - d).sum::<Decimal>() + pnl.net_income();
    // "Other" is split anew on each statement, its tied unit to the member
    // the count as of `end` lists first, as the ending's are: a reorder of
    // the members in the settings moves no cent between their other (and
    // so their income) either.
    let other_alloc = end_regime.allocate(
        &key,
        end_bs.total_equity - begin_bs.total_equity - explained,
        &nothing,
        places,
    )?;

    let mut cols = Vec::with_capacity(members.len());
    for (i, m) in members.iter().enumerate() {
        let beginning = capital[i] + begin_alloc[i];
        let ending = capital_end[i] + end_alloc[i];
        let (contributions, distributions) = flow[i];
        // The member's income is what the roll-forward leaves: their share
        // of the period's income, give or take the rounding cent that keeps
        // the running balance exact. The members' incomes sum to net income.
        let income = ending - beginning - contributions + distributions - other_alloc[i];
        cols.push(MemberEquityColumn {
            name: m.name.clone(),
            beginning: zero_clean(beginning),
            contributions,
            net_income: zero_clean(income),
            distributions,
            other: zero_clean(other_alloc[i]),
            ending: zero_clean(ending),
        });
    }
    let sum = |f: fn(&MemberEquityColumn) -> Decimal| cols.iter().map(f).sum::<Decimal>();
    let total = MemberEquityColumn {
        name: "Total".to_string(),
        beginning: sum(|c| c.beginning),
        contributions: sum(|c| c.contributions),
        net_income: sum(|c| c.net_income),
        distributions: sum(|c| c.distributions),
        other: sum(|c| c.other),
        ending: sum(|c| c.ending),
    };
    debug_assert_eq!(total.ending, end_bs.total_equity);
    Ok(MembersEquityStatement {
        start,
        end,
        members: cols,
        total,
    })
}

/// Render the three core reports as canonical JSON for oracle diffing.
///
/// Amounts are decimal strings (never floats, never locale-formatted), lines
/// are sorted by account code, and object keys are fixed. Two engines fed the
/// same fixture books must produce byte-identical output — that is the whole
/// point (see `docs/oracle.md`).
///
/// ```text
/// GOAL: the trial balance, P&L, and balance sheet as deterministic JSON,
///       the interchange format for cross-checking against other
///       implementations.
/// SOURCE: R-10; docs/oracle.md.
/// INVARIANTS:
///   1. MUST be identical for the same books, whatever order accounts were
///      added in (decimal strings, sorted lines, fixed keys).
///   2. NEVER contain floats.
/// ```
pub fn snapshot(ledger: &Ledger) -> Result<serde_json::Value, LedgerError> {
    let tb = trial_balance(ledger);
    let pl = unclosed_earnings(ledger)?;
    let bs = balance_sheet(ledger)?;
    // By value, not by written scale: a sum taken in hash-map order can
    // write the same zero as "0" or "0.00", and verify compared the
    // strings, so two equal ledgers could differ at random (found
    // 2026-10-15 by a flaky store test).
    let v = |d: Decimal| d.normalize().to_string();
    Ok(serde_json::json!({
        "trial_balance": {
            "lines": tb.lines.iter().map(|l| serde_json::json!({
                "code": l.code,
                "name": l.name,
                "debit": v(l.debit),
                "credit": v(l.credit),
            })).collect::<Vec<_>>(),
            "total_debits": v(tb.total_debits),
            "total_credits": v(tb.total_credits),
        },
        "profit_and_loss": {
            "revenue": v(pl.revenue),
            "expenses": v(pl.expenses),
            "net_income": v(pl.net_income()),
        },
        "balance_sheet": {
            "assets": v(bs.assets),
            "liabilities": v(bs.liabilities),
            "equity": v(bs.equity),
            "current_earnings": v(bs.current_earnings),
            "balanced": bs.balanced(),
        },
    }))
}

// ---------------------------------------------------------------------------
// Cash flow statement (direct method, rules.md B-14): see `crate::cashflow`.
// ---------------------------------------------------------------------------

pub use crate::cashflow::{
    CashFlowFooting, CashFlowKind, CashFlowLine, CashFlowParams, CashFlowSection,
    CashFlowStatement, CashFlowWarning, CashFlowWarningKind, NoncashDisclosure, OperatingClass,
    OperatingReconciliation, ReconcilingKind, ReconcilingLine, cash_flow, foot,
};

/// One asset on the fixed asset register.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRegisterRow {
    /// Asset id.
    pub asset_id: crate::AssetId,
    /// Its tag, if it has one.
    pub tag: Option<String>,
    /// What it is.
    pub description: String,
    /// The manufacturer's serial number, if recorded.
    pub serial: Option<String>,
    /// The day the company acquired it: its acquisition entry's date, or
    /// the in-service date when none is linked
    /// (`Ledger::asset_acquired_date`). On the register from this day.
    pub acquired_date: NaiveDate,
    /// Placed in service: depreciation starts.
    pub in_service_date: NaiveDate,
    /// Capitalized cost.
    pub cost: Decimal,
    /// Salvage value.
    pub salvage: Decimal,
    /// Useful life in months.
    pub useful_life_months: u32,
    /// Accumulated depreciation its schedule says is due through the date
    /// (zero once disposed: it left the books with the asset).
    pub accumulated_scheduled: Decimal,
    /// Accumulated depreciation the books carry for it on the date: its
    /// runs posted, less what reversals took back (`Ledger::
    /// accumulated_posted`; zero once disposed).
    pub accumulated_posted: Decimal,
    /// What the books carry: cost less accumulated_posted (zero once
    /// disposed).
    pub book_value: Decimal,
    /// Disposal date and proceeds, if disposed by the date.
    pub disposed: Option<(NaiveDate, Decimal)>,
    /// How the company got it (the basis rule), if recorded.
    pub basis_rule: Option<crate::BasisRule>,
    /// The contributing member's adjusted basis (a contribution only).
    pub contributor_adjusted_basis: Option<Decimal>,
    /// Fair market value at contribution (a contribution only): recorded,
    /// or the book cost for a record from before P3 (`fmv_recorded`
    /// false).
    pub fmv_at_contribution: Option<Decimal>,
    /// Whether the value at contribution was recorded as a fact.
    pub fmv_recorded: bool,
    /// The §723 basis at contribution (Asset::basis_figures).
    pub tax_basis_723: Option<Decimal>,
    /// The depreciable tax basis at contribution or purchase.
    pub depreciable_basis: Option<Decimal>,
    /// The §704(c) built-in gain (+) or loss (−) at contribution.
    pub built_in_gain_704c: Option<Decimal>,
    /// A built-in loss: it counts only for the contributing member(s),
    /// IRC §704(c)(1)(C).
    pub sec704c_1c: bool,
    /// The §704(c) method chosen for it, if chosen.
    pub sec704c_method: Option<crate::Sec704cMethod>,
    /// Who gave its basis facts, when, and on what.
    pub basis_source: Option<crate::settings::Source>,
}

/// The fixed asset register as of a date, reconciled to the general ledger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRegister {
    /// The date.
    pub as_of: NaiveDate,
    /// Every asset acquired by the date, in registration order; disposed
    /// ones carry their disposal and no cost on the books.
    pub rows: Vec<AssetRegisterRow>,
    /// Total cost of the assets still held.
    pub total_cost: Decimal,
    /// Their accumulated depreciation by schedule.
    pub total_accumulated_scheduled: Decimal,
    /// Their accumulated depreciation the books carry from runs.
    pub total_accumulated_posted: Decimal,
    /// Their book value: total cost less total_accumulated_posted.
    pub total_book_value: Decimal,
    /// The total depreciable tax basis of the assets still held that have
    /// a basis recorded.
    pub total_depreciable_basis: Decimal,
    /// The total §723 basis of the contributed assets still held.
    pub total_tax_basis_723: Decimal,
    /// How many assets still held have no basis recorded.
    pub missing_basis: usize,
    /// The general ledger's balance (debit positive) of the assets' cost
    /// accounts on the date.
    pub ledger_cost: Decimal,
    /// The general ledger's accumulated depreciation (credit positive) in
    /// the assets' accumulated depreciation accounts on the date.
    pub ledger_accumulated: Decimal,
}

impl AssetRegister {
    /// Depreciation the schedules say is due by the date but the books
    /// don't carry yet (run `run_depreciation`); negative if the books
    /// carry more.
    pub fn unposted_depreciation(&self) -> Decimal {
        self.total_accumulated_scheduled - self.ledger_accumulated
    }

    /// Accumulated depreciation in the accounts that no held asset's runs
    /// account for: zero when every depreciation in those accounts came
    /// from a run; otherwise an entry made by hand (or a run reversed
    /// after its asset left the books) is in them.
    pub fn unattributed_accumulated(&self) -> Decimal {
        self.ledger_accumulated - self.total_accumulated_posted
    }

    /// Cost in the ledger's cost accounts that no held asset accounts for
    /// (equipment bought but not registered), or negative if registered
    /// cost isn't in the books.
    pub fn unregistered_cost(&self) -> Decimal {
        self.ledger_cost - self.total_cost
    }
}

/// One asset's register row as of `as_of` (fixed_asset_register
/// INVARIANTS 1, 2 and 5), or `None` before it was acquired: the one
/// derivation the register and books.list_assets share. `posted` is
/// `Ledger::accumulated_posted_all(as_of)`.
pub fn register_row(
    ledger: &Ledger,
    a: &crate::Asset,
    as_of: NaiveDate,
    posted: &std::collections::BTreeMap<crate::AssetId, Decimal>,
) -> Result<Option<AssetRegisterRow>, LedgerError> {
    use crate::AssetStatus;
    let acquired = ledger
        .asset_acquired_date(a.id)
        .unwrap_or(a.in_service_date);
    if acquired > as_of {
        return Ok(None);
    }
    let disposed = match &a.status {
        AssetStatus::Disposed { date, proceeds } if *date <= as_of => {
            Some((*date, proceeds.amount))
        }
        _ => None,
    };
    let (sched, post) = if disposed.is_some() {
        (Decimal::ZERO, Decimal::ZERO)
    } else {
        (
            crate::assets::accumulated_scheduled_through(a, as_of)?,
            posted.get(&a.id).copied().unwrap_or(Decimal::ZERO),
        )
    };
    let figures = a.basis_figures()?;
    let book_value = if disposed.is_some() {
        Decimal::ZERO
    } else {
        a.cost
            .amount
            .checked_sub(post)
            .ok_or_else(|| LedgerError::TooLargeToCompute(format!("asset {}'s book value", a.id)))?
    };
    Ok(Some(AssetRegisterRow {
        asset_id: a.id,
        tag: a.tag.clone(),
        description: a.description.clone(),
        serial: a.serial.clone(),
        acquired_date: acquired,
        in_service_date: a.in_service_date,
        cost: a.cost.amount,
        salvage: a.salvage_value.amount,
        useful_life_months: a.useful_life_months,
        accumulated_scheduled: sched,
        accumulated_posted: post,
        book_value,
        disposed,
        basis_rule: a.basis.as_ref().map(|b| b.rule),
        contributor_adjusted_basis: a.basis.as_ref().and_then(|b| b.contributor_adjusted_basis),
        fmv_at_contribution: a.fmv_at_contribution(),
        fmv_recorded: a.fmv_is_recorded(),
        tax_basis_723: figures.and_then(|f| f.tax_basis_723),
        depreciable_basis: figures.map(|f| f.depreciable_basis),
        built_in_gain_704c: figures.and_then(|f| f.built_in_gain_704c),
        sec704c_1c: figures
            .and_then(|f| f.built_in_gain_704c)
            .is_some_and(|g| g < Decimal::ZERO),
        sec704c_method: a.basis.as_ref().and_then(|b| b.sec704c_method),
        basis_source: a.basis.as_ref().and_then(|b| b.source.clone()),
    }))
}

/// The fixed asset register as of a date.
///
/// ```text
/// GOAL: one list of everything the company owns as a fixed asset: what,
///       which label, cost, depreciation due and posted, book value, tax
///       figures, and whether the books agree.
/// SOURCE: docs/spec/rules.md B-15 (depreciation), B-24 (the register
///         reconciles to the books), B-25 (tax figures beside book);
///         DECISION (owner, 2026-10-15): a fully featured asset register.
/// INVARIANTS:
///   1. Rows: the assets acquired on or before as_of
///      (Ledger::asset_acquired_date), in registration order; an asset
///      disposed on or before as_of shows its disposal, zero accumulated
///      and book value, and counts in no total.
///   2. accumulated_scheduled is the asset's schedule through as_of
///      (assets::accumulated_scheduled_through); accumulated_posted is
///      what the books carry (Ledger::accumulated_posted); book_value =
///      cost − accumulated_posted: the books' figure, never the
///      schedule's.
///   3. Totals are the sums over the held assets, exactly; the
///      depreciable basis total over those with a basis recorded (and
///      missing_basis counts the rest); the §723 total over the
///      contributions.
///   4. ledger_cost and ledger_accumulated are the general ledger's
///      balances on as_of of the distinct cost and accumulated
///      depreciation accounts of the registered assets, so
///      unregistered_cost, unposted_depreciation and
///      unattributed_accumulated are zero exactly when register and books
///      agree.
///   5. Each row's tax fields are the asset's (Asset::basis_figures),
///      never re-derived here.
///   6. NEVER panics: a contributor basis is as journaled, of any size
///      (rules.md B-28), and a total past exact arithmetic, or a schedule
///      that can't be computed, is refused (TooLargeToCompute,
///      DateOutOfRange).
/// ```
pub fn fixed_asset_register(
    ledger: &Ledger,
    as_of: NaiveDate,
) -> Result<AssetRegister, LedgerError> {
    let too_large = |what: &str| LedgerError::TooLargeToCompute(format!("the register's {what}"));
    let posted = ledger.accumulated_posted_all(as_of)?;
    let mut rows = Vec::new();
    let (mut cost, mut scheduled, mut posted_total) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    let (mut depreciable_total, mut total_723, mut missing) =
        (Decimal::ZERO, Decimal::ZERO, 0usize);
    let mut cost_accounts = std::collections::BTreeSet::new();
    let mut accum_accounts = std::collections::BTreeSet::new();
    for a in ledger.registered_assets() {
        let Some(row) = register_row(ledger, a, as_of, &posted)? else {
            continue;
        };
        cost_accounts.insert(a.cost_account);
        accum_accounts.insert(a.accum_depr_account);
        if row.disposed.is_none() {
            cost = cost
                .checked_add(row.cost)
                .ok_or_else(|| too_large("cost"))?;
            scheduled = scheduled
                .checked_add(row.accumulated_scheduled)
                .ok_or_else(|| too_large("accumulated depreciation"))?;
            posted_total = posted_total
                .checked_add(row.accumulated_posted)
                .ok_or_else(|| too_large("posted depreciation"))?;
            match row.depreciable_basis {
                Some(b) => {
                    depreciable_total = depreciable_total
                        .checked_add(b)
                        .ok_or_else(|| too_large("depreciable basis"))?;
                    if let Some(b) = row.tax_basis_723 {
                        total_723 = total_723
                            .checked_add(b)
                            .ok_or_else(|| too_large("§723 basis"))?;
                    }
                }
                None => missing += 1,
            }
        }
        rows.push(row);
    }
    let tb = trial_balance_as_of(ledger, as_of);
    let net = |id: crate::AccountId| -> Decimal {
        let code = match ledger.chart().get(id) {
            Some(a) => &a.code,
            None => return Decimal::ZERO,
        };
        tb.lines
            .iter()
            .find(|l| &l.code == code)
            .map(|l| l.debit - l.credit)
            .unwrap_or(Decimal::ZERO)
    };
    Ok(AssetRegister {
        as_of,
        rows,
        total_cost: cost,
        total_accumulated_scheduled: scheduled,
        total_accumulated_posted: posted_total,
        total_book_value: cost
            .checked_sub(posted_total)
            .ok_or_else(|| too_large("book value"))?,
        total_depreciable_basis: depreciable_total,
        total_tax_basis_723: total_723,
        missing_basis: missing,
        ledger_cost: cost_accounts.iter().map(|&id| net(id)).sum(),
        ledger_accumulated: -accum_accounts.iter().map(|&id| net(id)).sum::<Decimal>(),
    })
}
