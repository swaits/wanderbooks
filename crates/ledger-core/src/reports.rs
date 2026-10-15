//! Reports: pure functions of a [`Ledger`].
//!
//! Reports never mutate and never guess. They are what the semantic oracle
//! (`docs/oracle.md`) diffs: the same fixture books through two engines must
//! produce byte-identical [`snapshot`] output.

use crate::periods::allocate;
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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
    if end < start {
        return Err(LedgerError::InvalidDateRange {
            start: start.to_string(),
            end: end.to_string(),
        });
    }
    let closings = ledger.closing_entry_ids();
    let lines = statement_lines(ledger, &|entry, line| {
        entry.date() >= start
            && entry.date() <= end
            && !closings.contains(&entry.id())
            && class_id.is_none_or(|c| line.class_id == Some(c))
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
    Ok(ProfitAndLossStatement {
        start,
        end,
        revenue,
        expenses,
        total_revenue,
        total_expenses,
    })
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

/// Build the statement of changes in members' equity for `[start, end]`.
///
/// Each member's capital account supplies contributions (credits) and
/// distributions (debits). Earnings are partnership earnings: net income
/// for the period, and earnings from before it not yet distributed (retained
/// earnings plus unclosed income), are allocated by share. Allocation is to
/// the cent, odd cents by largest remainder, so every row's total is exact
/// and the ending total equals balance-sheet equity.
///
/// ```text
/// GOAL: each member's capital roll-forward for a partnership over a
///       period: beginning, contributions, allocated net income, distributions,
///       ending; with a total.
/// SOURCE: docs/spec/rules.md B-13 (26 U.S.C. §704(a); Form 1065 Schedule
///         M-2), B-11; R-9c.
/// INVARIANTS:
///   1. Each row MUST roll forward exactly: beginning + contributions +
///      income - distributions + other = ending.
///   2. The total ending MUST equal balance-sheet equity at `end`.
///   3. Undistributed earnings MUST be split by share, to the cent, by
///      largest remainder (DECISION, owner 2026-10-13), cumulatively as
///      of each date: a member's beginning is their capital and share as
///      of the day before `start`, their ending as of `end`.
///   3a. A member's ending on one statement MUST equal their beginning on
///      the next, exactly (a capital account is one running balance: Form
///      1065 Schedule K-1 instructions, Item L, "beginning capital account
///      and ending capital account ... based on the partnership's books").
///   3b. A member's income is the roll-forward's remainder: their share of
///      the period's income, off by less than two cents (under one cent of
///      rounding at each end of the cumulative split); the members'
///      incomes MUST sum to the period's net income. (Earnings a close has
///      already moved into capital were rounded by that close, B-13.)
///   4. NEVER hide an unexplained change: it goes in "other".
///   5. An entry dated `start` is period activity, not beginning capital.
/// SCENARIOS:
///   1. IF shares don't sum to 1, or are not positive: SHOULD refuse.
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
    let flows = |id: crate::AccountId| -> (Decimal, Decimal) {
        let (mut credits, mut debits) = (Decimal::ZERO, Decimal::ZERO);
        for e in ledger
            .posted_entries()
            .filter(|e| e.date() >= start && e.date() <= end && !closings.contains(&e.id()))
        {
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
    // next one's beginning, to the cent (a capital account is one running
    // balance: Form 1065 Schedule K-1 instructions, Item L).
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
    let begin_alloc = allocate(
        begin_bs.total_equity - capital.iter().sum::<Decimal>(),
        &shares,
    );
    let end_alloc = allocate(
        end_bs.total_equity - capital_end.iter().sum::<Decimal>(),
        &shares,
    );
    let explained = flow.iter().map(|(c, d)| c - d).sum::<Decimal>() + pnl.net_income();
    let other_alloc = allocate(
        end_bs.total_equity - begin_bs.total_equity - explained,
        &shares,
    );

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
// Cash flow statement (indirect method, R-FUTURE-3).
// ---------------------------------------------------------------------------

/// One labeled cash flow line. Positive is an inflow, negative an outflow.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CashFlowLine {
    /// Human label, e.g. `"Increase in Accounts Receivable"` or account code/name.
    pub label: String,
    /// Signed amount: positive = cash in, negative = cash out.
    pub amount: Decimal,
    /// Code of the account the line is about, when it is about one (so a
    /// renderer can relabel, e.g. a member's capital account by name).
    #[serde(default)]
    pub account: Option<String>,
}

/// Parameters for [`cash_flow`].
#[derive(Clone, Debug)]
pub struct CashFlowParams {
    /// Period start, inclusive.
    pub start: chrono::NaiveDate,
    /// Period end, inclusive.
    pub end: chrono::NaiveDate,
    /// Accounts treated as cash. Their net change is the reconciling total.
    pub cash_accounts: Vec<crate::AccountId>,
    /// Liability accounts treated as financing (debt, including what the
    /// owners advanced: ASC 230-10-45-14(b) "other short- or long-term
    /// borrowing"). Default empty: all liabilities are operating working
    /// capital. All equity is financing.
    pub financing_accounts: Vec<crate::AccountId>,
    /// Asset accounts treated as investing: loans the entity makes and
    /// investments it buys (ASC 230-10-20; 230-10-45-13). Registered fixed
    /// assets are investing without being listed. Default empty: other
    /// assets are operating working capital.
    pub investing_accounts: Vec<crate::AccountId>,
}

/// Cash flow statement, indirect method.
///
/// Starts from period net income, adds back non-cash depreciation, adjusts
/// for working-capital changes, then reports investing (fixed-asset
/// purchases at cost, disposal proceeds) and financing (equity changes,
/// designated debt accounts) flows. The three sections always sum to the
/// net change in the designated cash accounts.
///
/// Disposals are identified structurally: a posted entry that credits a
/// registered fixed-asset cost account and debits its linked accumulated
/// depreciation account. The gain/loss in that entry is removed from
/// operating and the proceeds are shown in investing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CashFlowStatement {
    /// Period start, inclusive.
    pub start: chrono::NaiveDate,
    /// Period end, inclusive.
    pub end: chrono::NaiveDate,
    /// Period net income (revenue minus expenses).
    pub net_income: Decimal,
    /// Non-cash depreciation added back to net income.
    pub depreciation: Decimal,
    /// Working-capital adjustments (non-cash balance-sheet changes).
    pub working_capital: Vec<CashFlowLine>,
    /// Net cash from operating activities.
    pub cash_from_operations: Decimal,
    /// Investing flows: asset purchases and disposal proceeds.
    pub investing: Vec<CashFlowLine>,
    /// Net cash from investing activities.
    pub cash_from_investing: Decimal,
    /// Financing flows: equity changes and designated debt.
    pub financing: Vec<CashFlowLine>,
    /// Net cash from financing activities.
    pub cash_from_financing: Decimal,
    /// Net change in cash: the three sections summed.
    pub net_change_in_cash: Decimal,
    /// Cash balance at period start.
    pub cash_beginning: Decimal,
    /// Cash balance at period end.
    pub cash_ending: Decimal,
}

/// Build the indirect-method cash flow statement for a period.
///
/// Pure function of the ledger: net income plus non-cash depreciation plus
/// working-capital changes gives operating cash flow; fixed-asset purchases
/// and disposal proceeds, and the accounts named as investing, give
/// investing; equity and the debt named as financing give financing. The
/// three sections sum to the net change in `cash_accounts`.
///
/// ```text
/// GOAL: the statement of cash flows by the indirect method: net income
///       adjusted to operating cash, plus investing and financing, for a
///       period.
/// SOURCE: docs/spec/rules.md B-14 (ASC 230-10-45-28 indirect method;
///         230-10-45-16 operating inflows; 230-10-45-7/8 gross over net;
///         230-10-45-13/14/15 investing and financing flows; 230-10-20
///         investing activities).
/// INVARIANTS:
///   1. Operating + investing + financing MUST equal the net change in the
///      cash accounts over the period, exactly.
///   2. Operating MUST start from the period's net income (closing entries
///      excluded) and adjust for changes in non-cash working-capital
///      accounts.
///   3. Investing MUST hold fixed-asset purchases and disposal proceeds
///      (a disposal's gain or loss moves out of operating into investing)
///      and the changes in the accounts named as investing.
///   4. Financing MUST hold changes in equity (other than earnings) and in
///      the liabilities named as financing.
///   4a. Investing and financing MUST be gross (ASC 230-10-45-7: gross
///      receipts and payments are "more relevant" than net): per account,
///      cash in and cash out are separate lines (contributions apart from
///      distributions, borrowing apart from repayment), netted only within
///      one entry.
///   5. NEVER count closing entries.
///   6. The period is `[start, end]`, inclusive: an entry dated `start` is
///      period activity, not part of the beginning cash balance.
///   7. Every line's amount is its cash effect: positive is cash in. Its
///      label names the direction ("Increase in" / "Decrease in" for
///      working capital; "Proceeds from" / "Repayment of" for financing
///      debt; "Owner contributions" / "Owner distributions" for equity;
///      "Payments for" / "Collections on" for investing accounts).
/// ```
pub fn cash_flow(
    ledger: &Ledger,
    params: &CashFlowParams,
) -> Result<CashFlowStatement, LedgerError> {
    use std::collections::{HashMap, HashSet};

    if params.end < params.start {
        return Err(LedgerError::InvalidDateRange {
            start: params.start.to_string(),
            end: params.end.to_string(),
        });
    }

    // Fixed-asset account roles from the register.
    let mut cost_accounts: HashSet<crate::AccountId> = HashSet::new();
    let mut accum_accounts: HashSet<crate::AccountId> = HashSet::new();
    let mut depr_expense_accounts: HashSet<crate::AccountId> = HashSet::new();
    // cost_account -> accum_account, for disposal identification.
    let mut cost_to_accum: HashMap<crate::AccountId, crate::AccountId> = HashMap::new();
    for asset in ledger.registered_assets() {
        cost_accounts.insert(asset.cost_account);
        accum_accounts.insert(asset.accum_depr_account);
        depr_expense_accounts.insert(asset.expense_account);
        cost_to_accum.insert(asset.cost_account, asset.accum_depr_account);
    }

    let cash_set: HashSet<crate::AccountId> = params.cash_accounts.iter().copied().collect();
    let financing_set: HashSet<crate::AccountId> =
        params.financing_accounts.iter().copied().collect();
    let investing_set: HashSet<crate::AccountId> =
        params.investing_accounts.iter().copied().collect();

    // Movement helpers: debits minus credits over a date predicate. Closing
    // entries are skipped: a close moves earnings into retained earnings,
    // which is neither net income nor an owner flow.
    let closings = ledger.closing_entry_ids();
    let movement_where = |account: crate::AccountId, pred: &dyn Fn(chrono::NaiveDate) -> bool| {
        let mut total = Decimal::ZERO;
        for entry in ledger.posted_entries() {
            if !pred(entry.date()) || closings.contains(&entry.id()) {
                continue;
            }
            for line in entry.lines() {
                if line.account == account {
                    match line.side {
                        crate::Side::Debit => total += line.amount.amount,
                        crate::Side::Credit => total -= line.amount.amount,
                    }
                }
            }
        }
        total
    };
    let begin_movement = |a: crate::AccountId| movement_where(a, &|d| d < params.start);
    let period_movement =
        |a: crate::AccountId| movement_where(a, &|d| d >= params.start && d <= params.end);

    // Normal-side balance from a movement.
    let normal = |account: crate::AccountId, movement: Decimal| -> Decimal {
        let acct = ledger.chart().get(account).expect("chart account exists");
        match acct.account_type.normal_side() {
            crate::Side::Debit => movement,
            crate::Side::Credit => -movement,
        }
    };

    // --- Net income and depreciation (period P&L) ---
    let mut net_income = Decimal::ZERO;
    let mut depreciation = Decimal::ZERO;
    for account in ledger.chart().all() {
        if account.placeholder {
            continue;
        }
        let pm = period_movement(account.id);
        if pm.is_zero() {
            continue;
        }
        match account.account_type {
            AccountType::Revenue => net_income += -pm, // revenue movement is credits
            AccountType::Expense => {
                net_income -= pm;
                if depr_expense_accounts.contains(&account.id) {
                    depreciation += pm;
                }
            }
            _ => {}
        }
    }

    // --- Classify balance-sheet accounts ---
    let mut working_capital: Vec<CashFlowLine> = Vec::new();
    let mut investing: Vec<CashFlowLine> = Vec::new();
    let mut financing: Vec<CashFlowLine> = Vec::new();

    // Account display helper.
    let display = |account: crate::AccountId| -> String {
        let a = ledger.chart().get(account).expect("chart account exists");
        format!("{} {}", a.code, a.name)
    };

    // By code: the chart iterates in hash order, and a statement's lines
    // must come out in the same order every time.
    let mut accounts: Vec<_> = ledger.chart().all().collect();
    accounts.sort_by(|a, b| a.code.cmp(&b.code));
    for account in accounts {
        if account.placeholder {
            continue;
        }
        let id = account.id;
        if cash_set.contains(&id) || cost_accounts.contains(&id) || accum_accounts.contains(&id) {
            continue; // cash reconciles; fixed assets are investing (gross, below)
        }
        if account.account_type == AccountType::Equity
            || financing_set.contains(&id)
            || investing_set.contains(&id)
        {
            continue; // financing and investing roles are gross, below
        }
        let begin = normal(id, begin_movement(id));
        let end = normal(id, period_movement(id) + begin_movement(id));
        let delta = end - begin;
        if delta.is_zero() {
            continue;
        }
        // Cash effect of the balance change: for both assets and
        // liabilities, it is minus the movement (debits - credits).
        let movement_delta = period_movement(id);
        let cash_effect = -movement_delta;
        let (label, line) = match account.account_type {
            AccountType::Asset => {
                let dir = if delta > Decimal::ZERO {
                    "Increase in"
                } else {
                    "Decrease in"
                };
                (format!("{dir} {}", display(id)), cash_effect)
            }
            AccountType::Liability => {
                let dir = if delta > Decimal::ZERO {
                    "Increase in"
                } else {
                    "Decrease in"
                };
                (format!("{dir} {}", display(id)), cash_effect)
            }
            _ => continue, // revenue/expense handled via net income
        };
        working_capital.push(CashFlowLine {
            label,
            amount: line,
            account: Some(account.code.clone()),
        });
    }

    // --- Financing and investing accounts, gross (ASC 230-10-45-7) ---
    // Per account, cash in and cash out as separate lines; an entry's own
    // debits and credits to the account net first (one transaction, one
    // flow).
    let mut roles: Vec<&crate::Account> = ledger
        .chart()
        .all()
        .filter(|a| {
            !a.placeholder
                && !cash_set.contains(&a.id)
                && !cost_accounts.contains(&a.id)
                && !accum_accounts.contains(&a.id)
                && (a.account_type == AccountType::Equity
                    || financing_set.contains(&a.id)
                    || investing_set.contains(&a.id))
        })
        .collect();
    roles.sort_by(|a, b| a.code.cmp(&b.code));
    for account in roles {
        let (mut cash_in, mut cash_out) = (Decimal::ZERO, Decimal::ZERO);
        for entry in ledger.posted_entries() {
            let d = entry.date();
            if d < params.start || d > params.end || closings.contains(&entry.id()) {
                continue;
            }
            // Cash effect of this entry's change to the account: minus its
            // movement (debits - credits), for assets and claims alike.
            let effect: Decimal = entry
                .lines()
                .iter()
                .filter(|l| l.account == account.id)
                .map(|l| match l.side {
                    crate::Side::Debit => -l.amount.amount,
                    crate::Side::Credit => l.amount.amount,
                })
                .sum();
            if effect > Decimal::ZERO {
                cash_in += effect;
            } else {
                cash_out += effect;
            }
        }
        let name = display(account.id);
        let (in_label, out_label, target) = if investing_set.contains(&account.id) {
            (
                format!("Collections on {name}"),
                format!("Payments for {name}"),
                &mut investing,
            )
        } else if account.account_type == AccountType::Equity {
            (
                format!("Owner contributions — {name}"),
                format!("Owner distributions — {name}"),
                &mut financing,
            )
        } else {
            (
                format!("Proceeds from {name}"),
                format!("Repayment of {name}"),
                &mut financing,
            )
        };
        for (label, amount) in [(in_label, cash_in), (out_label, cash_out)] {
            if !amount.is_zero() {
                target.push(CashFlowLine {
                    label,
                    amount,
                    account: Some(account.code.clone()),
                });
            }
        }
    }

    // --- Investing, gross: purchases and disposal proceeds ---
    // A disposal entry credits a cost account and debits its linked accum.
    let mut disposal_gain_loss = Decimal::ZERO; // +gain / -loss, to remove from operating
    for entry in ledger.posted_entries() {
        let d = entry.date();
        if d < params.start || d > params.end {
            continue;
        }
        // Is this a disposal? Credit to a cost account + debit to its accum.
        let mut disposed: Option<(crate::AccountId, Decimal, Decimal)> = None;
        for line in entry.lines() {
            if line.side == crate::Side::Credit
                && let Some(&accum) = cost_to_accum.get(&line.account)
            {
                // Confirm the linked accum account is debited in this entry.
                let accum_debit: Decimal = entry
                    .lines()
                    .iter()
                    .filter(|l| l.account == accum && l.side == crate::Side::Debit)
                    .map(|l| l.amount.amount)
                    .sum();
                if !accum_debit.is_zero() {
                    disposed = Some((line.account, line.amount.amount, accum_debit));
                    break;
                }
            }
        }
        if let Some((cost_acct, cost_amount, accum_debit)) = disposed {
            let accum_acct = cost_to_accum[&cost_acct];
            let mut proceeds = Decimal::ZERO;
            let mut proceeds_label = String::new();
            let mut gain = Decimal::ZERO;
            let mut loss = Decimal::ZERO;
            for line in entry.lines() {
                if line.account == cost_acct || line.account == accum_acct {
                    continue;
                }
                let acct = ledger
                    .chart()
                    .get(line.account)
                    .expect("chart account exists");
                match (acct.account_type, line.side) {
                    (AccountType::Asset, crate::Side::Debit) => {
                        proceeds += line.amount.amount;
                        proceeds_label = display(line.account);
                    }
                    (AccountType::Revenue, crate::Side::Credit) => gain += line.amount.amount,
                    (AccountType::Expense, crate::Side::Debit) => loss += line.amount.amount,
                    _ => {}
                }
            }
            // Sanity: the entry balances, so proceeds + accum debit = cost + gain - loss.
            debug_assert!((proceeds + accum_debit - cost_amount - gain + loss).is_zero());
            disposal_gain_loss += gain - loss;
            investing.push(CashFlowLine {
                label: format!("Sale of fixed assets — {proceeds_label}"),
                amount: proceeds,
                account: ledger.chart().get(cost_acct).map(|a| a.code.clone()),
            });
            continue;
        }
        // Not a disposal: debits to cost accounts are purchases.
        for line in entry.lines() {
            if cost_accounts.contains(&line.account) && line.side == crate::Side::Debit {
                investing.push(CashFlowLine {
                    label: format!("Purchase of fixed assets — {}", display(line.account)),
                    amount: -line.amount.amount,
                    account: ledger.chart().get(line.account).map(|a| a.code.clone()),
                });
            }
        }
    }

    // Remove disposal gains/losses from operating (they live in investing).
    if !disposal_gain_loss.is_zero() {
        working_capital.push(CashFlowLine {
            label: "Less: gain on sale of fixed assets (in investing)".to_string(),
            amount: -disposal_gain_loss,
            account: None,
        });
    }

    let cash_from_operations: Decimal =
        net_income + depreciation + working_capital.iter().map(|l| l.amount).sum::<Decimal>();
    let cash_from_investing: Decimal = investing.iter().map(|l| l.amount).sum();
    let cash_from_financing: Decimal = financing.iter().map(|l| l.amount).sum();

    // --- Cash reconciliation ---
    let mut cash_beginning = Decimal::ZERO;
    let mut cash_ending = Decimal::ZERO;
    for &id in &params.cash_accounts {
        cash_beginning += normal(id, begin_movement(id));
        cash_ending += normal(id, begin_movement(id) + period_movement(id));
    }
    let net_change_in_cash = cash_ending - cash_beginning;

    Ok(CashFlowStatement {
        start: params.start,
        end: params.end,
        net_income,
        depreciation,
        working_capital,
        cash_from_operations,
        investing,
        cash_from_investing,
        financing,
        cash_from_financing,
        net_change_in_cash,
        cash_beginning,
        cash_ending,
    })
}

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
    /// Placed in service (depreciation starts).
    pub in_service: NaiveDate,
    /// Capitalized cost.
    pub cost: Decimal,
    /// Salvage value.
    pub salvage: Decimal,
    /// Useful life in months.
    pub useful_life_months: u32,
    /// Accumulated depreciation by its schedule through the date (zero once
    /// disposed: it left the books with the asset).
    pub accumulated: Decimal,
    /// Cost less accumulated (zero once disposed).
    pub book_value: Decimal,
    /// Disposal date and proceeds, if disposed by the date.
    pub disposed: Option<(NaiveDate, Decimal)>,
}

/// The fixed asset register as of a date, reconciled to the general ledger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRegister {
    /// The date.
    pub as_of: NaiveDate,
    /// Every asset registered by the date, in registration order; disposed
    /// ones carry their disposal and no cost on the books.
    pub rows: Vec<AssetRegisterRow>,
    /// Total cost of the assets still held.
    pub total_cost: Decimal,
    /// Their accumulated depreciation by schedule.
    pub total_accumulated: Decimal,
    /// Their book value.
    pub total_book_value: Decimal,
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
        self.total_accumulated - self.ledger_accumulated
    }

    /// Cost in the ledger's cost accounts that no held asset accounts for
    /// (equipment bought but not registered), or negative if registered
    /// cost isn't in the books.
    pub fn unregistered_cost(&self) -> Decimal {
        self.ledger_cost - self.total_cost
    }
}

/// The fixed asset register as of a date.
///
/// ```text
/// GOAL: one list of everything the company owns as a fixed asset: what,
///       which label, cost, depreciation, book value, and whether the
///       books agree.
/// SOURCE: docs/spec/rules.md B-15 (depreciation); DECISION (owner,
///         2026-10-15): a fully featured asset register.
/// INVARIANTS:
///   1. Rows: the assets registered on or before as_of, in registration
///      order; an asset disposed on or before as_of shows its disposal,
///      zero accumulated and book value, and counts in no total.
///   2. accumulated is the asset's schedule through as_of
///      (assets::accumulated_depreciation); book_value = cost −
///      accumulated.
///   3. Totals are the sums over the held assets, exactly.
///   4. ledger_cost and ledger_accumulated are the general ledger's
///      balances on as_of of the distinct cost and accumulated
///      depreciation accounts of the registered assets, so
///      unposted_depreciation and unregistered_cost are zero exactly when
///      register and books agree.
/// ```
pub fn fixed_asset_register(ledger: &Ledger, as_of: NaiveDate) -> AssetRegister {
    use crate::AssetStatus;
    let mut rows = Vec::new();
    let (mut cost, mut accumulated) = (Decimal::ZERO, Decimal::ZERO);
    let mut cost_accounts = std::collections::BTreeSet::new();
    let mut accum_accounts = std::collections::BTreeSet::new();
    for a in ledger
        .registered_assets()
        .filter(|a| a.purchase_date <= as_of)
    {
        cost_accounts.insert(a.cost_account);
        accum_accounts.insert(a.accum_depr_account);
        let disposed = match &a.status {
            AssetStatus::Disposed { date, proceeds } if *date <= as_of => {
                Some((*date, proceeds.amount))
            }
            _ => None,
        };
        let acc = if disposed.is_some() {
            Decimal::ZERO
        } else {
            crate::assets::accumulated_depreciation(a, as_of)
        };
        if disposed.is_none() {
            cost += a.cost.amount;
            accumulated += acc;
        }
        rows.push(AssetRegisterRow {
            asset_id: a.id,
            tag: a.tag.clone(),
            description: a.description.clone(),
            serial: a.serial.clone(),
            in_service: a.purchase_date,
            cost: a.cost.amount,
            salvage: a.salvage_value.amount,
            useful_life_months: a.useful_life_months,
            accumulated: acc,
            book_value: if disposed.is_some() {
                Decimal::ZERO
            } else {
                a.cost.amount - acc
            },
            disposed,
        });
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
    AssetRegister {
        as_of,
        rows,
        total_cost: cost,
        total_accumulated: accumulated,
        total_book_value: cost - accumulated,
        ledger_cost: cost_accounts.iter().map(|&id| net(id)).sum(),
        ledger_accumulated: -accum_accounts.iter().map(|&id| net(id)).sum::<Decimal>(),
    }
}
