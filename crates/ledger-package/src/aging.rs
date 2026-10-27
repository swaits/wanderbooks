//! The receivables and payables agings as files (plan P4b S6, 8.3.10;
//! rules.md B-18, B-23): one row per open document, and a tie-out line
//! against the control account. The server's `books.export` and the CPA
//! package build the same CSV from this, and its XLSX/PDF from
//! [`aging_statement`], so a CPA's two copies can't differ.

use ledger_core::{DocRef, Ledger};
use rust_decimal::Decimal;

use crate::text::{csv_escape, money};

/// The CSV's header: one row per open document (8.3.10).
pub const AGING_CSV_HEADER: &str =
    "number,party,date,due_date,terms,total,paid,discounted,outstanding,bucket,days_overdue";

/// One aging's rows and tie-out as CSV: the header, one row per open
/// invoice (`invoices` true) or bill, then `TOTAL,,,,,,,,<Σ
/// outstanding>,GL <account> <balance>,difference <difference>`.
pub fn aging_csv(
    ledger: &Ledger,
    invoices: bool,
    as_of: chrono::NaiveDate,
) -> Result<String, String> {
    let docs = ledger
        .aging_documents(as_of, invoices)
        .map_err(|e| e.to_string())?;
    let mut out = String::from(AGING_CSV_HEADER);
    out.push('\n');
    let mut total = Decimal::ZERO;
    for d in &docs {
        total += d.outstanding;
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_escape(&number(ledger, d.doc)),
            csv_escape(&d.party),
            d.date,
            d.due_date,
            csv_escape(d.terms.as_deref().unwrap_or("")),
            money(d.total),
            money(d.paid),
            money(d.discounted),
            money(d.outstanding),
            d.bucket,
            d.days_overdue,
        ));
    }
    let (code, balance, difference) = tie_out(ledger, invoices, as_of)?;
    let account = if code.is_empty() {
        "GL —".to_string()
    } else {
        format!("GL {code}")
    };
    out.push_str(&format!(
        "TOTAL,,,,,,,,{total},{account} {balance},difference {difference}\n",
        total = money(total),
        balance = money(balance),
        difference = money(difference),
    ));
    Ok(out)
}

/// The same aging as a laid-out statement for XLSX and PDF: one line per
/// party with its six bucket amounts, the totals, and the tie-out.
pub fn aging_statement(
    ledger: &Ledger,
    invoices: bool,
    as_of: chrono::NaiveDate,
) -> Result<ledger_import::Statement, String> {
    use ledger_import::report::{Row, RowKind, Statement};
    let party_rows = if invoices {
        ledger.ar_aging(as_of)
    } else {
        ledger.ap_aging(as_of)
    }
    .map_err(|e| e.to_string())?;
    let (code, balance, difference) = tie_out(ledger, invoices, as_of)?;
    let columns: Vec<String> = ["Current", "1-30", "31-60", "61-90", "90+", "Total"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let mut out_rows: Vec<Row> = vec![Row {
        kind: RowKind::Section,
        label: if invoices {
            "Receivables".into()
        } else {
            "Payables".into()
        },
        amounts: vec![],
        currency: false,
        rule_above: false,
    }];
    let (mut current, mut d30, mut d60, mut d90, mut d90p) = (
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
    );
    for r in &party_rows {
        let total = r.total().map_err(|e| e.to_string())?;
        current += r.current;
        d30 += r.days_30;
        d60 += r.days_60;
        d90 += r.days_90;
        d90p += r.days_90_plus;
        out_rows.push(Row {
            kind: RowKind::Line,
            label: r.name.clone(),
            amounts: vec![
                Some(r.current),
                Some(r.days_30),
                Some(r.days_60),
                Some(r.days_90),
                Some(r.days_90_plus),
                Some(total),
            ],
            currency: false,
            rule_above: false,
        });
    }
    let total = current + d30 + d60 + d90 + d90p;
    out_rows.push(Row {
        kind: RowKind::Total,
        label: "Total".into(),
        amounts: vec![
            Some(current),
            Some(d30),
            Some(d60),
            Some(d90),
            Some(d90p),
            Some(total),
        ],
        currency: true,
        rule_above: true,
    });
    out_rows.push(Row {
        kind: RowKind::Spacer,
        label: String::new(),
        amounts: vec![],
        currency: false,
        rule_above: false,
    });
    out_rows.push(Row {
        kind: RowKind::Section,
        label: "Tie-out".into(),
        amounts: vec![],
        currency: false,
        rule_above: false,
    });
    out_rows.push(Row {
        kind: RowKind::Line,
        label: if code.is_empty() {
            "GL balance".to_string()
        } else {
            format!("GL {code} balance")
        },
        amounts: vec![None, None, None, None, None, Some(balance)],
        currency: false,
        rule_above: false,
    });
    out_rows.push(Row {
        kind: RowKind::Total,
        label: "Difference".into(),
        amounts: vec![None, None, None, None, None, Some(difference)],
        currency: true,
        rule_above: true,
    });
    Ok(Statement {
        // Excel sheet names take no slash; the statement's title is the
        // one both XLSX and PDF draw.
        title: if invoices {
            "AR Aging".into()
        } else {
            "AP Aging".into()
        },
        period: format!("As of {}", as_of.format("%B %-d, %Y")),
        columns,
        rows: out_rows,
        percent_from: None,
    })
}

/// The document's number for the aging's first column: an invoice's issued
/// number, a bill's vendor number, else the document's own name.
fn number(ledger: &Ledger, doc: DocRef) -> String {
    match doc {
        DocRef::Invoice(id) => ledger
            .get_invoice(id)
            .and_then(|i| i.number.clone())
            .unwrap_or_else(|| format!("Invoice {id}")),
        DocRef::Bill(id) => ledger
            .get_bill(id)
            .and_then(|b| b.vendor_ref.clone())
            .unwrap_or_else(|| format!("Bill {id}")),
    }
}

/// The control account's code, balance and difference as of the day
/// ([`Ledger::control_check`]): the aging's tie-out.
fn tie_out(
    ledger: &Ledger,
    invoices: bool,
    as_of: chrono::NaiveDate,
) -> Result<(String, Decimal, Decimal), String> {
    let role = if invoices { "receivables" } else { "payables" };
    let checks = ledger.control_check(as_of).map_err(|e| e.to_string())?;
    let Some(check) = checks.iter().find(|c| c.role == role) else {
        return Ok((String::new(), Decimal::ZERO, Decimal::ZERO));
    };
    let code = ledger
        .chart()
        .get(check.account)
        .map(|a| a.code.clone())
        .unwrap_or_default();
    Ok((code, check.balance, check.difference))
}
