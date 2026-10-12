//! 1099 tracking: what was paid to each vendor in a calendar year.
//!
//! A vendor is marked with the information return it gets (1099-NEC for
//! nonemployee compensation, 1099-MISC for rents and the like) and whether
//! its W-9 is on file. Entries are tagged with the vendor they paid
//! (`pay_bill` tags automatically). The amount paid is the cash that left
//! asset accounts on the vendor's tagged entries: credits minus debits to
//! asset accounts. A card payment (Dr expense, Cr credit card) moves no
//! asset, so it is excluded, as it should be: card payments are reported by
//! the card processor on 1099-K, not by the payer.
//!
//! Thresholds are set by law and change; [`default_threshold`] encodes
//! them and the report takes an override. Confirm with the CPA.

use crate::{AccountType, Ledger, VendorId};
use chrono::Datelike;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// The information return a vendor gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Form1099 {
    /// 1099-NEC: nonemployee compensation (contractors, attorneys' fees).
    Nec,
    /// 1099-MISC: rents, royalties, prizes, other income.
    Misc,
}

impl Form1099 {
    /// Parse "nec", "1099-NEC", "misc", "1099-MISC".
    pub fn parse(s: &str) -> Option<Form1099> {
        let s = s.trim().to_lowercase();
        match s.strip_prefix("1099-").unwrap_or(&s) {
            "nec" => Some(Form1099::Nec),
            "misc" => Some(Form1099::Misc),
            _ => None,
        }
    }

    /// "1099-NEC" or "1099-MISC".
    pub fn label(self) -> &'static str {
        match self {
            Form1099::Nec => "1099-NEC",
            Form1099::Misc => "1099-MISC",
        }
    }
}

/// The reporting threshold for payments made in `year`: $600 through 2025;
/// $2,000 from 2026 (One Big Beautiful Bill Act), indexed for inflation
/// after 2026, so later years return $2,000 until updated. Royalties
/// (1099-MISC) have a $10 threshold this doesn't model. Pass an explicit
/// threshold to override.
pub fn default_threshold(year: i32) -> Decimal {
    if year <= 2025 {
        Decimal::new(600, 0)
    } else {
        Decimal::new(2000, 0)
    }
}

/// One vendor's year.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vendor1099Line {
    /// The vendor.
    pub vendor_id: VendorId,
    /// Name.
    pub name: String,
    /// The return it gets, if marked.
    pub form: Option<Form1099>,
    /// W-9 on file.
    pub tin_on_file: bool,
    /// Cash paid in the year (net of refunds and reversals).
    pub paid: Decimal,
    /// Marked for a 1099 and paid at least the threshold.
    pub reportable: bool,
}

/// The year's 1099 report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vendor1099Report {
    /// Calendar year.
    pub year: i32,
    /// The threshold applied.
    pub threshold: Decimal,
    /// Every vendor marked for a 1099 or paid in the year, by name.
    pub vendors: Vec<Vendor1099Line>,
}

/// Cash paid per vendor in calendar `year`, from posted entries tagged
/// with the vendor.
pub fn vendor_1099_report(
    ledger: &Ledger,
    year: i32,
    threshold: Option<Decimal>,
) -> Vendor1099Report {
    let threshold = threshold.unwrap_or_else(|| default_threshold(year));
    let mut paid: std::collections::HashMap<VendorId, Decimal> = Default::default();
    for entry in ledger.posted_entries() {
        if entry.date().year() != year {
            continue;
        }
        let Some(vendor) = ledger.entry_vendor(entry.id()) else {
            continue;
        };
        let cash: Decimal = entry
            .lines()
            .iter()
            .filter(|l| {
                ledger
                    .chart()
                    .get(l.account)
                    .is_some_and(|a| a.account_type == AccountType::Asset)
            })
            .map(|l| match l.side {
                crate::Side::Credit => l.amount.amount,
                crate::Side::Debit => -l.amount.amount,
            })
            .sum();
        *paid.entry(vendor).or_default() += cash;
    }
    let mut vendors: Vec<Vendor1099Line> = ledger
        .list_vendors()
        .into_iter()
        .filter(|v| v.form_1099.is_some() || paid.get(&v.id).is_some_and(|p| !p.is_zero()))
        .map(|v| {
            let amount = paid.get(&v.id).copied().unwrap_or_default();
            Vendor1099Line {
                vendor_id: v.id,
                name: v.name.clone(),
                form: v.form_1099,
                tin_on_file: v.tin_on_file,
                paid: amount,
                reportable: v.form_1099.is_some() && amount >= threshold,
            }
        })
        .collect();
    vendors.sort_by(|a, b| a.name.cmp(&b.name).then(a.vendor_id.cmp(&b.vendor_id)));
    Vendor1099Report {
        year,
        threshold,
        vendors,
    }
}
