//! The 1099 worksheet: what `books.vendor_1099` answers and hands over as
//! CSV, and what the viewer saves, with each payee's TIN when asked.
//!
//! ```text
//! GOAL: the CPA gets one 1099 worksheet, the same from the server and
//!       the viewer: cash paid per vendor, who is reportable, who lacks a
//!       W-9, and each payee's TIN.
//! SOURCE: ledger_core::form1099 (payments out of the cash and financing
//!         accounts; an entry's shares resolved with the members); IRS
//!         thresholds (rules.md B-19: never guessed); RFC 4180 (CSV).
//! INVARIANTS:
//!   1. Payments come from the settings' cash and financing accounts
//!      (codes the chart lacks are skipped), an entry's shares resolved
//!      with the company's members.
//!   2. A year whose threshold isn't known is refused, never guessed.
//!   3. Full TINs only from the TIN source given (crate INVARIANT 2): with
//!      one, every line carries its TIN or a note why not; without one,
//!      none does.
//!   4. The CSV has a header and one row per line, in the worksheet's
//!      order; the TIN columns only when asked.
//! ```

use ledger_core::{EntryId, Ledger, PartyId, VendorId};
use rust_decimal::Decimal;

use crate::reports::Settings;
use crate::text::{csv_escape, money};
use crate::tins::{TinSource, tin_type_key};

/// Where payments to vendors come from, for the 1099 worksheet: the
/// settings' cash and financing accounts in the chart (codes the chart
/// lacks are skipped; `books.settings` reports them).
pub fn payment_accounts(ledger: &Ledger, settings: &Settings<'_>) -> ledger_core::PaymentAccounts {
    let ids = |codes: &[String]| {
        codes
            .iter()
            .filter_map(|c| ledger.chart().find_by_code(c).map(|a| a.id))
            .collect()
    };
    ledger_core::PaymentAccounts {
        cash: ids(settings.cash_accounts),
        financing: ids(settings.financing_accounts),
    }
}

/// The 1099 report for `year`, the one every tool, the worksheet, and the
/// CPA package use: payments out of [`payment_accounts`], an entry's
/// shares resolved with the settings' members. Given `through`, only the
/// payments from January 1 through it (the CPA package as of a day,
/// rules.md B-33).
pub fn vendor_1099_report(
    ledger: &Ledger,
    settings: &Settings<'_>,
    year: i32,
    through: Option<chrono::NaiveDate>,
    threshold: Option<Decimal>,
) -> Result<ledger_core::Vendor1099Report, ledger_core::LedgerError> {
    ledger_core::vendor_1099_report_with_members(
        ledger,
        year,
        through,
        threshold,
        &payment_accounts(ledger, settings),
        &settings.company.party_members(ledger),
    )
}

/// A 1099 form's label ("1099-NEC", "1099-MISC"), if the vendor is marked.
pub fn form_1099_label(form: Option<ledger_core::Form1099>) -> Option<String> {
    form.map(|f| f.label().to_string())
}

/// One vendor's line of the 1099 worksheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vendor1099WorksheetLine {
    /// The vendor.
    pub vendor_id: VendorId,
    /// Its name.
    pub name: String,
    /// "1099-NEC", "1099-MISC", or none.
    pub form_1099: Option<String>,
    /// W-9 on file.
    pub tin_on_file: bool,
    /// Cash paid in the year.
    pub paid: Decimal,
    /// Marked for a 1099 and paid at least the threshold.
    pub reportable: bool,
    /// The vendor's party.
    pub party_id: Option<PartyId>,
    /// The TIN's last four, from the party's W-9 facts, else its TIN
    /// facts.
    pub tin_last4: Option<String>,
    /// The full TIN, formatted (123-45-6789, 12-3456789): only with a TIN
    /// source, and only when it has one.
    pub tin: Option<String>,
    /// The TIN's type (ssn, ein, itin): only with a TIN source.
    pub tin_type: Option<String>,
    /// With a TIN source, why `tin` is missing.
    pub tin_note: Option<String>,
}

/// The 1099 worksheet for a calendar year.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vendor1099Worksheet {
    /// Calendar year.
    pub year: i32,
    /// The threshold applied.
    pub threshold: Decimal,
    /// One line per vendor marked for a 1099 or paid in the year, by name.
    pub lines: Vec<Vendor1099WorksheetLine>,
    /// Reportable vendors with no W-9 on file.
    pub missing_w9: Vec<String>,
    /// Entries naming several parties whose shares don't resolve (each
    /// counted in full toward its vendor).
    pub unresolved: Vec<EntryId>,
    /// What to check: what counts, exemptions, the threshold's indexing,
    /// untagged entries, unresolved shares.
    pub notes: Vec<String>,
    /// Whether the lines carry TINs from a TIN source.
    pub tins_included: bool,
}

/// The 1099 worksheet for `year` (threshold: the IRS figure to use when
/// the year's isn't known, else none), each line with its full TIN from
/// `tins` when given (INVARIANT 3).
pub fn vendor_1099_worksheet(
    ledger: &Ledger,
    settings: &Settings<'_>,
    year: i32,
    threshold: Option<Decimal>,
    tins: Option<TinSource<'_>>,
) -> Result<Vendor1099Worksheet, String> {
    // A year past what wanderbooks knows is refused with guidance
    // (research or ask, pass `threshold`, then update the rule).
    let r =
        vendor_1099_report(ledger, settings, year, None, threshold).map_err(|e| e.to_string())?;
    let missing_w9 = r
        .vendors
        .iter()
        .filter(|v| v.reportable && !v.tin_on_file)
        .map(|v| v.name.clone())
        .collect();
    let untagged = ledger
        .posted_entries()
        .filter(|e| {
            chrono::Datelike::year(&e.date()) == year && ledger.entry_vendor(e.id()).is_none()
        })
        .count();
    let mut notes = vec![
        "Only entries tagged with a vendor count (books.set_entry_vendor; books.pay_bill tags automatically).".to_string(),
        "Corporations are generally exempt, except attorneys' fees and medical payments; royalties report at $10. Confirm with the CPA.".to_string(),
    ];
    if year > 2026 {
        notes.push(format!(
            "The $2,000 threshold is indexed for inflation after 2026; confirm the {year} amount and pass threshold to override."
        ));
    }
    if untagged > 0 {
        notes.push(format!(
            "{untagged} posted entries in {year} have no vendor tag; review any that paid contractors or rent."
        ));
    }
    if !r.unresolved.is_empty() {
        notes.push(format!(
            "Entries {} name several parties whose shares don't resolve, so each counts in full toward its vendor: fix the shares with books.set_entry_party.",
            r.unresolved
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let mut lines = Vec::new();
    for v in r.vendors {
        let party_id = ledger.party_of_vendor(v.vendor_id);
        let tin_last4 = party_id
            .and_then(|id| ledger.get_party(id))
            .and_then(|p| p.tin_last4())
            .map(str::to_string);
        let mut line = Vendor1099WorksheetLine {
            vendor_id: v.vendor_id,
            name: v.name,
            form_1099: form_1099_label(v.form),
            tin_on_file: v.tin_on_file,
            paid: v.paid,
            reportable: v.reportable,
            party_id,
            tin_last4,
            tin: None,
            tin_type: None,
            tin_note: None,
        };
        if let Some(tins) = tins {
            match party_id.map(tins) {
                None => line.tin_note = Some("no party for this vendor".into()),
                Some(Err(why)) => line.tin_note = Some(why),
                Some(Ok(t)) => {
                    line.tin = Some(t.formatted());
                    line.tin_type = Some(tin_type_key(t.kind).to_string());
                }
            }
        }
        lines.push(line);
    }
    Ok(Vendor1099Worksheet {
        year: r.year,
        threshold: r.threshold,
        lines,
        missing_w9,
        unresolved: r.unresolved,
        notes,
        tins_included: tins.is_some(),
    })
}

/// The worksheet as CSV (INVARIANT 4): vendor id, name, form, paid,
/// reportable, W-9 on file, and last four; `with_tins` adds the TIN, its
/// type, and the note why it's missing.
pub fn vendor_1099_csv(worksheet: &Vendor1099Worksheet, with_tins: bool) -> String {
    let mut csv = String::from("vendor_id,name,form_1099,paid,reportable,w9_on_file,tin_last4");
    if with_tins {
        csv.push_str(",tin,tin_type,tin_note");
    }
    csv.push('\n');
    for l in &worksheet.lines {
        let mut cells = vec![
            l.vendor_id.to_string(),
            csv_escape(&l.name),
            l.form_1099.clone().unwrap_or_default(),
            money(l.paid),
            l.reportable.to_string(),
            l.tin_on_file.to_string(),
            l.tin_last4.clone().unwrap_or_default(),
        ];
        if with_tins {
            cells.push(l.tin.clone().unwrap_or_default());
            cells.push(l.tin_type.clone().unwrap_or_default());
            cells.push(csv_escape(l.tin_note.as_deref().unwrap_or_default()));
        }
        csv.push_str(&cells.join(","));
        csv.push('\n');
    }
    csv
}

/// The worksheet CSV's file name: `1099-worksheet-<year>.csv`, or
/// `1099-worksheet-<year>-with-tins.csv`.
pub fn vendor_1099_csv_name(year: i32, with_tins: bool) -> String {
    format!(
        "1099-worksheet-{}{}.csv",
        year,
        if with_tins { "-with-tins" } else { "" }
    )
}
