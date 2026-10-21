//! The financial statements as the server's report tools and the package
//! make them: from the books' settings, a date range, and account codes.
//!
//! ```text
//! GOAL: a statement in the package is the one the report tool shows for
//!       the same range, with the same refusals.
//! SOURCE: ledger-mcp server.rs (books.profit_and_loss, books.balance_sheet,
//!         books.cash_flow, books.export_cpa_package); ASC 230-10-20
//!         (financing and investing activities).
//! INVARIANTS:
//!   1. A range's omitted dates are the one default (rules.md B-30,
//!      `ledger_core::calendar::PeriodDefaults`): an omitted end is
//!      `today`, an omitted start the first day of the fiscal year that
//!      holds the end; a start after the end, given or derived, is
//!      refused, never swapped. An omitted as_of is `today`.
//!   2. The cash flow statement needs at least one cash account, and
//!      every code it is given must be in the chart; account lists a call
//!      leaves empty default to the settings' roles. It foots, or it is
//!      refused (an engine error, never a figure: rules.md B-14 rule 9);
//!      roles that contradict each other or the chart are refused by the
//!      engine, naming the setting, so the viewer and the server refuse
//!      alike. Members are named on their contributions and
//!      distributions (D-35).
//!   3. The year's earnings split net income year to date from the part
//!      not yet closed into equity (closes are cumulative).
//! ```

use chrono::NaiveDate;
use ledger_core::settings::{Company, Member};
use ledger_core::{AccountId, Ledger, PeriodStatus};
use ledger_import::ReportMeta;
use rust_decimal::Decimal;

use crate::text::parse_date;

/// The books' settings the package and its reports read: the company, and
/// the account roles (`books.cash_accounts`, `books.financing_accounts`,
/// `books.investing_accounts`) as chart codes.
#[derive(Clone, Copy, Debug)]
pub struct Settings<'a> {
    /// The company: name, basis, fiscal year, members, tax form.
    pub company: &'a Company,
    /// The cash accounts' codes.
    pub cash_accounts: &'a [String],
    /// The financing accounts' codes.
    pub financing_accounts: &'a [String],
    /// The investing accounts' codes.
    pub investing_accounts: &'a [String],
}

impl<'a> Settings<'a> {
    /// These settings with the cash and financing accounts a request
    /// names: each list the request's when it gives one, else these
    /// settings'. The one rule the CPA package (one list of each per
    /// package, for every file in it: crate::package INVARIANT 7) and
    /// `books.vendor_1099` read, so the package's 1099 worksheet is the
    /// tool's for the same accounts.
    pub fn with_accounts(
        &self,
        cash_accounts: &'a [String],
        financing_accounts: &'a [String],
    ) -> Settings<'a> {
        let or = |given: &'a [String], kept: &'a [String]| {
            if given.is_empty() { kept } else { given }
        };
        Settings {
            cash_accounts: or(cash_accounts, self.cash_accounts),
            financing_accounts: or(financing_accounts, self.financing_accounts),
            ..*self
        }
    }
}

/// Resolve a statement's date range (INVARIANT 1): the dates as given,
/// each omitted one the books' default for `company` on `today`
/// (rules.md B-30).
pub fn statement_range(
    company: &Company,
    start: Option<&str>,
    end: Option<&str>,
    today: NaiveDate,
) -> Result<(NaiveDate, NaiveDate), String> {
    let start = start.map(parse_date).transpose()?;
    let end = end.map(parse_date).transpose()?;
    ledger_core::calendar::period_defaults(company, today)
        .statement(start, end)
        .map_err(|e| e.to_string())
}

/// Resolve a point-in-time report's day (INVARIANT 1): as given, else
/// `today`.
pub fn as_of_date(
    company: &Company,
    as_of: Option<&str>,
    today: NaiveDate,
) -> Result<NaiveDate, String> {
    let given = as_of.map(parse_date).transpose()?;
    Ok(ledger_core::calendar::period_defaults(company, today).as_of(given))
}

/// Profit and loss for a range (see [`statement_range`]), optionally one
/// class.
pub fn pnl_statement(
    ledger: &Ledger,
    company: &Company,
    start: Option<&str>,
    end: Option<&str>,
    class_id: Option<ledger_core::ClassId>,
    today: NaiveDate,
) -> Result<ledger_core::ProfitAndLossStatement, String> {
    let (start, end) = statement_range(company, start, end, today)?;
    ledger_core::profit_and_loss_for_period(ledger, start, end, class_id).map_err(|e| e.to_string())
}

/// Balance sheet as of a day (see [`as_of_date`]).
pub fn bs_statement(
    ledger: &Ledger,
    company: &Company,
    as_of: Option<&str>,
    today: NaiveDate,
) -> Result<ledger_core::BalanceSheetStatement, String> {
    let as_of = as_of_date(company, as_of, today)?;
    ledger_core::balance_sheet_as_of(ledger, as_of).map_err(|e| e.to_string())
}

/// Build the cash flow statement from account codes, as the exports need it:
/// the range as [`statement_range`] resolves it, and at least one cash
/// account (the statement reconciles to them). A member whose capital
/// account the chart lacks is left unnamed (equity is then named by
/// account), never a refusal of the statement.
#[allow(clippy::too_many_arguments)]
pub fn cash_flow_statement(
    ledger: &Ledger,
    company: &Company,
    start: Option<&str>,
    end: Option<&str>,
    cash_codes: &[String],
    financing_codes: &[String],
    investing_codes: &[String],
    members: &[Member],
    today: NaiveDate,
) -> Result<ledger_core::reports::CashFlowStatement, String> {
    let resolve = |codes: &[String], what: &str| -> Result<Vec<AccountId>, String> {
        codes
            .iter()
            .map(|code| {
                ledger
                    .chart()
                    .find_by_code(code)
                    .map(|a| a.id)
                    .ok_or_else(|| format!("unknown {what} account code: {code}"))
            })
            .collect()
    };
    let cash_accounts = resolve(cash_codes, "cash")?;
    if cash_accounts.is_empty() {
        return Err(
            "cash_accounts is required for the cash flow statement (pass it, or set the books' cash accounts setting: books.settings shows it)"
                .to_string(),
        );
    }
    let financing_accounts = resolve(financing_codes, "financing")?;
    let investing_accounts = resolve(investing_codes, "investing")?;
    let (start, end) = statement_range(company, start, end, today)?;
    let chart = ledger.chart();
    let members = members
        .iter()
        .filter_map(|m| {
            Some(ledger_core::MemberCapital {
                name: m.name.clone(),
                capital_account: chart.find_by_code(&m.capital_account)?.id,
                distribution_account: m
                    .distribution_account
                    .as_deref()
                    .and_then(|c| chart.find_by_code(c))
                    .map(|a| a.id),
                share: m.share,
            })
        })
        .collect();
    ledger_core::cash_flow(
        ledger,
        &ledger_core::CashFlowParams {
            start,
            end,
            cash_accounts,
            financing_accounts,
            investing_accounts,
            members,
        },
    )
    .map_err(|e| e.to_string())
}

/// The cash flow statement from account codes, each list defaulting to
/// the settings' roles when the call passes none.
#[allow(clippy::too_many_arguments)]
pub fn cash_flow_for(
    ledger: &Ledger,
    settings: &Settings<'_>,
    start: Option<&str>,
    end: Option<&str>,
    cash: &[String],
    financing: &[String],
    investing: &[String],
    today: NaiveDate,
) -> Result<ledger_core::reports::CashFlowStatement, String> {
    let or = |given: &'_ [String], default: &'_ [String]| -> Vec<String> {
        if given.is_empty() {
            default.to_vec()
        } else {
            given.to_vec()
        }
    };
    cash_flow_statement(
        ledger,
        settings.company,
        start,
        end,
        &or(cash, settings.cash_accounts),
        &or(financing, settings.financing_accounts),
        &or(investing, settings.investing_accounts),
        &settings.company.members,
        today,
    )
}

/// Header and provenance for a rendered report: the company's name and
/// basis, the day it was prepared, and the journal state it reflects.
pub fn report_meta(company: &Company, prepared_on: NaiveDate, journal_seq: u64) -> ReportMeta {
    ReportMeta {
        company: company.name.clone(),
        basis: company.basis.clone(),
        prepared_on,
        journal_seq,
    }
}

/// Members as reports name them (empty unless a multi-member LLC).
pub fn report_members(company: &Company) -> Vec<ledger_import::Member> {
    company
        .members
        .iter()
        .map(|m| ledger_import::Member {
            name: m.name.clone(),
            capital_code: m.capital_account.clone(),
            distribution_code: m.distribution_account.clone(),
        })
        .collect()
}

/// Members with their capital (and distribution) accounts resolved in the
/// chart, for the statement of members' equity and the K-1 worksheet; a
/// code the chart lacks is refused, naming the member.
pub fn member_capitals(
    ledger: &Ledger,
    members: &[Member],
) -> Result<Vec<ledger_core::MemberCapital>, String> {
    members
        .iter()
        .map(|m| {
            let account = ledger
                .chart()
                .find_by_code(&m.capital_account)
                .ok_or_else(|| {
                    format!(
                        "member {}: unknown capital account {}",
                        m.name, m.capital_account
                    )
                })?;
            let distribution_account = match &m.distribution_account {
                Some(code) => Some(
                    ledger
                        .chart()
                        .find_by_code(code)
                        .ok_or_else(|| {
                            format!("member {}: unknown distribution account {code}", m.name)
                        })?
                        .id,
                ),
                None => None,
            };
            Ok(ledger_core::MemberCapital {
                name: m.name.clone(),
                capital_account: account.id,
                distribution_account,
                share: m.share,
            })
        })
        .collect()
}

/// The fiscal year's earnings through `as_of`, for the balance sheet:
/// net income (loss) year to date (closing entries excluded), and the
/// part of it not yet closed into equity: the income after the last
/// close (closes are cumulative), from no earlier than the year's start.
pub fn year_earnings(
    ledger: &Ledger,
    company: &Company,
    as_of: NaiveDate,
) -> Result<ledger_import::YearEarnings, String> {
    let fy_start = ledger_core::calendar::fiscal_year_start_on(company.fiscal_year_start, as_of);
    let net_income_ytd = ledger_core::profit_and_loss_for_period(ledger, fy_start, as_of, None)
        .map_err(|e| e.to_string())?
        .net_income();
    // A close takes everything unclosed through its end, earlier years
    // included, so this year's unclosed part is the income from the day
    // after the last close (never before the year's start) through
    // as_of. (Counting the closing entries dated this year instead
    // charged this year with closing last year's income: found by
    // year_earnings_split_closed_from_unclosed, 2026-10-14.)
    let last_close = ledger
        .list_periods()
        .iter()
        .filter(|p| p.status == PeriodStatus::Closed && p.end <= as_of)
        .map(|p| p.end)
        .max();
    let from = match last_close.and_then(|e| e.succ_opt()) {
        Some(after) if after > fy_start => after,
        _ => fy_start,
    };
    let unclosed = if from <= as_of {
        ledger_core::profit_and_loss_for_period(ledger, from, as_of, None)
            .map_err(|e| e.to_string())?
            .net_income()
    } else {
        Decimal::ZERO
    };
    Ok(ledger_import::YearEarnings {
        net_income_ytd,
        unclosed,
    })
}
