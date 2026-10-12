//! PDF financial statements for the CPA handoff package.
//!
//! Simple, correct, readable — not pretty. Uses printpdf (MIT) with built-in
//! Helvetica. Amounts are exact decimals rendered as strings; no floats.

use ledger_core::reports::CashFlowStatement;
use ledger_core::{AccountType, Ledger};
use printpdf::{
    BuiltinFont, Color, Mm, Op, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Point, Pt,
    Rgb, TextItem,
};
use rust_decimal::Decimal;
use thiserror::Error;

/// PDF rendering errors.
#[derive(Debug, Error)]
pub enum PdfError {
    /// printpdf failed to save.
    #[error("pdf save failed: {0}")]
    Save(String),
}

fn money(d: Decimal) -> String {
    // Two decimals, no float anywhere.
    format!("{:.2}", d)
}

fn text_ops(
    title: &str,
    subtitle: &str,
    rows: &[(String, String)],
    totals: &[(String, String)],
) -> Vec<Op> {
    let mut ops = vec![
        Op::StartTextSection,
        Op::SetTextCursor {
            pos: Point::new(Mm(20.0), Mm(280.0)),
        },
        Op::SetFont {
            font: PdfFontHandle::Builtin(BuiltinFont::HelveticaBold),
            size: Pt(18.0),
        },
        Op::SetLineHeight { lh: Pt(22.0) },
        Op::SetFillColor {
            col: Color::Rgb(Rgb {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                icc_profile: None,
            }),
        },
        Op::ShowText {
            items: vec![TextItem::Text(title.to_string())],
        },
        Op::AddLineBreak,
        Op::SetFont {
            font: PdfFontHandle::Builtin(BuiltinFont::Helvetica),
            size: Pt(11.0),
        },
        Op::SetLineHeight { lh: Pt(14.0) },
        Op::ShowText {
            items: vec![TextItem::Text(subtitle.to_string())],
        },
        Op::AddLineBreak,
        Op::AddLineBreak,
    ];

    for (label, amount) in rows {
        // Label left, amount right-ish via padding. Monospace would be
        // nicer; Helvetica with padding is readable enough for a CPA file.
        let line = format!("{:<50} {:>14}", label, amount);
        ops.push(Op::ShowText {
            items: vec![TextItem::Text(line)],
        });
        ops.push(Op::AddLineBreak);
    }

    if !totals.is_empty() {
        ops.push(Op::AddLineBreak);
        ops.push(Op::SetFont {
            font: PdfFontHandle::Builtin(BuiltinFont::HelveticaBold),
            size: Pt(11.0),
        });
        for (label, amount) in totals {
            let line = format!("{:<50} {:>14}", label, amount);
            ops.push(Op::ShowText {
                items: vec![TextItem::Text(line)],
            });
            ops.push(Op::AddLineBreak);
        }
    }

    ops.push(Op::EndTextSection);
    ops
}

fn save_pdf(title: &str, ops: Vec<Op>) -> Result<Vec<u8>, PdfError> {
    let mut doc = PdfDocument::new(title);
    let page = PdfPage::new(Mm(210.0), Mm(297.0), ops);
    let bytes = doc
        .with_pages(vec![page])
        .save(&PdfSaveOptions::default(), &mut Vec::new());
    Ok(bytes)
}

/// Render a profit-and-loss statement PDF: revenue and expense accounts with
/// balances, then totals and net income.
pub fn profit_and_loss_pdf(ledger: &Ledger, start: &str, end: &str) -> Result<Vec<u8>, PdfError> {
    let mut revenue_rows: Vec<(String, String)> = Vec::new();
    let mut expense_rows: Vec<(String, String)> = Vec::new();
    let mut accounts: Vec<_> = ledger.chart().all().collect();
    accounts.sort_by(|a, b| a.code.cmp(&b.code));
    for account in accounts {
        if account.placeholder {
            continue;
        }
        let balance = ledger
            .account_balance(account.id)
            .map_err(|e| PdfError::Save(e.to_string()))?;
        if balance.is_zero() {
            continue;
        }
        let label = format!("{} {}", account.code, account.name);
        match account.account_type {
            AccountType::Revenue => revenue_rows.push((label, money(balance))),
            AccountType::Expense => expense_rows.push((label, money(balance))),
            _ => {}
        }
    }
    let pl =
        ledger_core::reports::profit_and_loss(ledger).map_err(|e| PdfError::Save(e.to_string()))?;

    let mut rows = vec![("REVENUE".to_string(), "".to_string())];
    rows.extend(revenue_rows);
    rows.push(("".to_string(), "".to_string()));
    rows.push(("EXPENSES".to_string(), "".to_string()));
    rows.extend(expense_rows);

    let totals = vec![
        ("Total revenue".to_string(), money(pl.revenue)),
        ("Total expenses".to_string(), money(pl.expenses)),
        ("Net income".to_string(), money(pl.net_income())),
    ];

    let ops = text_ops(
        "Profit and Loss",
        &format!("{} to {}", start, end),
        &rows,
        &totals,
    );
    save_pdf("Profit and Loss", ops)
}

/// Render a balance-sheet PDF: assets, liabilities, equity with balances,
/// then totals proving the accounting equation.
pub fn balance_sheet_pdf(ledger: &Ledger, as_of: &str) -> Result<Vec<u8>, PdfError> {
    let mut asset_rows: Vec<(String, String)> = Vec::new();
    let mut liab_rows: Vec<(String, String)> = Vec::new();
    let mut equity_rows: Vec<(String, String)> = Vec::new();
    let mut accounts: Vec<_> = ledger.chart().all().collect();
    accounts.sort_by(|a, b| a.code.cmp(&b.code));
    for account in accounts {
        if account.placeholder {
            continue;
        }
        let balance = ledger
            .account_balance(account.id)
            .map_err(|e| PdfError::Save(e.to_string()))?;
        if balance.is_zero() {
            continue;
        }
        let label = format!("{} {}", account.code, account.name);
        match account.account_type {
            AccountType::Asset => asset_rows.push((label, money(balance))),
            AccountType::Liability => liab_rows.push((label, money(balance))),
            AccountType::Equity => equity_rows.push((label, money(balance))),
            _ => {}
        }
    }
    let bs =
        ledger_core::reports::balance_sheet(ledger).map_err(|e| PdfError::Save(e.to_string()))?;

    let mut rows = vec![("ASSETS".to_string(), "".to_string())];
    rows.extend(asset_rows);
    rows.push(("".to_string(), "".to_string()));
    rows.push(("LIABILITIES".to_string(), "".to_string()));
    rows.extend(liab_rows);
    rows.push(("".to_string(), "".to_string()));
    rows.push(("EQUITY".to_string(), "".to_string()));
    rows.extend(equity_rows);
    rows.push((
        "Current earnings (in equity)".to_string(),
        money(bs.current_earnings),
    ));

    let totals = vec![
        ("Total assets".to_string(), money(bs.assets)),
        (
            "Total liabilities + equity".to_string(),
            money(bs.liabilities + bs.equity),
        ),
    ];

    let ops = text_ops("Balance Sheet", &format!("As of {}", as_of), &rows, &totals);
    save_pdf("Balance Sheet", ops)
}

/// Render a cash flow statement PDF (indirect method): operating (net
/// income, depreciation add-back, working-capital changes), investing, and
/// financing sections, then the net change reconciled to beginning and
/// ending cash. Takes the computed statement, so the period and cash
/// accounts are whatever the caller chose.
pub fn cash_flow_pdf(cf: &CashFlowStatement) -> Result<Vec<u8>, PdfError> {
    let line = |l: &ledger_core::reports::CashFlowLine| (format!("  {}", l.label), money(l.amount));
    let mut rows = vec![("OPERATING ACTIVITIES".to_string(), "".to_string())];
    rows.push(("  Net income".to_string(), money(cf.net_income)));
    if !cf.depreciation.is_zero() {
        rows.push((
            "  Depreciation (non-cash)".to_string(),
            money(cf.depreciation),
        ));
    }
    rows.extend(cf.working_capital.iter().map(line));
    rows.push((
        "Net cash from operating activities".to_string(),
        money(cf.cash_from_operations),
    ));
    rows.push(("".to_string(), "".to_string()));
    rows.push(("INVESTING ACTIVITIES".to_string(), "".to_string()));
    rows.extend(cf.investing.iter().map(line));
    rows.push((
        "Net cash from investing activities".to_string(),
        money(cf.cash_from_investing),
    ));
    rows.push(("".to_string(), "".to_string()));
    rows.push(("FINANCING ACTIVITIES".to_string(), "".to_string()));
    rows.extend(cf.financing.iter().map(line));
    rows.push((
        "Net cash from financing activities".to_string(),
        money(cf.cash_from_financing),
    ));

    let totals = vec![
        (
            "Net change in cash".to_string(),
            money(cf.net_change_in_cash),
        ),
        (
            "Cash at beginning of period".to_string(),
            money(cf.cash_beginning),
        ),
        ("Cash at end of period".to_string(), money(cf.cash_ending)),
    ];

    let ops = text_ops(
        "Statement of Cash Flows",
        &format!("{} to {}", cf.start, cf.end),
        &rows,
        &totals,
    );
    save_pdf("Statement of Cash Flows", ops)
}
