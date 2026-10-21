//! The 1099 worksheet: what `books.vendor_1099` answers and hands over as
//! CSV, what the CPA package carries, and what the viewer saves and shows,
//! with each payee's TIN when asked.
//!
//! ```text
//! GOAL: the CPA gets one 1099 worksheet, the same from the server, the
//!       package and the viewer: per vendor what counts toward its 1099,
//!       what a card or payment network paid, what is unknown, its 1099
//!       decision with the reason and who made it, who lacks a W-9, and
//!       each payee's TIN.
//! SOURCE: ledger_core::form1099 (the one report: payments out of the cash
//!         and financing accounts, classified by payment method; an entry's
//!         shares resolved with the members; dated decisions); IRS
//!         thresholds (rules.md B-19: never guessed); RFC 4180 (CSV).
//! INVARIANTS:
//!   1. Payments come from the cash and financing accounts of the
//!      `Settings` given (codes the chart lacks are skipped): the books'
//!      settings, or the CPA package's and `books.vendor_1099`'s request
//!      accounts (`Settings::with_accounts`), an entry's shares resolved
//!      with the company's members; with no cash accounts named,
//!      refused.
//!   2. A year whose threshold isn't known is refused, never guessed.
//!   3. Full TINs only from the TIN source given (crate INVARIANT 2): with
//!      one, every line carries its TIN or a note why not; without one,
//!      none does.
//!   4. The CSV has a header and one row per line, in the worksheet's
//!      order; the TIN columns only when asked.
//!   5. The notes are quiet: the untagged entries are one count and a
//!      tool hint, each list of payments one line.
//! ```

use chrono::NaiveDate;
use ledger_core::{Decision1099, EntryId, Ledger, PartyId, Payment1099, Status1099, VendorId};
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

/// The 1099 report for `year`, the one every tool, the worksheet, the
/// CPA package and the viewer use: payments out of [`payment_accounts`],
/// an entry's shares resolved with the settings' members. Given
/// `through`, only the payments from January 1 through it (the CPA
/// package as of a day, rules.md B-33).
pub fn vendor_1099_report(
    ledger: &Ledger,
    settings: &Settings<'_>,
    year: i32,
    threshold: Option<Decimal>,
    through: Option<NaiveDate>,
) -> Result<ledger_core::Vendor1099Report, ledger_core::LedgerError> {
    ledger_core::vendor_1099_report(
        ledger,
        year,
        threshold,
        &payment_accounts(ledger, settings),
        &settings.company.party_members(ledger),
        through,
    )
}

/// A 1099 form's label ("1099-NEC", "1099-MISC"), if the vendor files one.
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
    /// "1099-NEC", "1099-MISC", or none: the form its year-end decision
    /// files.
    pub form_1099: Option<String>,
    /// Its decision at the end of the year: "nec", "misc",
    /// "not_reportable" or "undetermined".
    pub decision_1099: String,
    /// The decision itself, with its reason when not reportable.
    pub decision: Decision1099,
    /// Who decided it and when (none: never decided, or decided before
    /// decisions were kept).
    pub decided: Option<ledger_core::settings::Source>,
    /// The date it took effect (none: from the beginning).
    pub effective: Option<NaiveDate>,
    /// Every decision in force during the year, in date order.
    pub decisions: Vec<ledger_core::Dated1099>,
    /// W-9 on file.
    pub tin_on_file: bool,
    /// What counts toward its 1099.
    pub paid: Decimal,
    /// Paid by card or payment network: reported on Form 1099-K.
    pub excluded_card: Decimal,
    /// Paid with no payment method recorded where it matters.
    pub undetermined: Decimal,
    /// Paid while it was decided not reportable.
    pub not_reportable: Decimal,
    /// A decision is needed for some of the year's payments.
    pub decision_needed: bool,
    /// Where it stands.
    pub status: Status1099,
    /// File a 1099 (status reportable).
    pub reportable: bool,
    /// The vendor's party.
    pub party_id: Option<PartyId>,
    /// The TIN's last four ([`ledger_core::Party::tin_last4`]: its TIN
    /// facts, else its W-9 facts).
    pub tin_last4: Option<String>,
    /// The full TIN, formatted (123-45-6789, 12-3456789): only with a TIN
    /// source, and only when it has one.
    pub tin: Option<String>,
    /// The TIN's type (ssn, ein, itin): only with a TIN source.
    pub tin_type: Option<String>,
    /// With a TIN source, why `tin` is missing.
    pub tin_note: Option<String>,
}

impl Vendor1099WorksheetLine {
    /// Whether a 1099 may be due: reportable, or undetermined while decided
    /// for one some time in the year (P3 review R16: its W-9 and TIN
    /// matter as much).
    pub fn may_file(&self) -> bool {
        self.reportable
            || (self.status == Status1099::Undetermined
                && self.decisions.iter().any(|d| d.decision.form().is_some()))
    }
}

/// The 1099 worksheet for a calendar year.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vendor1099Worksheet {
    /// Calendar year.
    pub year: i32,
    /// The threshold applied.
    pub threshold: Decimal,
    /// The last day counted, when it stops before December 31.
    pub through: Option<NaiveDate>,
    /// One line per vendor decided for a 1099 or paid in the year, by name.
    pub lines: Vec<Vendor1099WorksheetLine>,
    /// Vendors a 1099 is or may be due for with no W-9 on file (" (may be
    /// reportable)" after those undetermined).
    pub missing_w9: Vec<String>,
    /// Entries naming several parties whose shares don't resolve (each
    /// counted in full toward its vendor).
    pub unresolved: Vec<EntryId>,
    /// Payments by card or payment network, excluded.
    pub excluded_card_payments: Vec<Payment1099>,
    /// Payments whose method must be recorded before they can be
    /// classified.
    pub undetermined_payments: Vec<Payment1099>,
    /// Money received from a vendor for something else, not netted.
    pub other_receipts: Vec<Payment1099>,
    /// Posted entries that paid money out with no vendor tag.
    pub untagged_payments: Vec<EntryId>,
    /// What to check: what counts, exemptions, the threshold's indexing,
    /// untagged entries, card payments, unknown methods, decisions needed,
    /// unresolved shares.
    pub notes: Vec<String>,
    /// Whether the lines carry TINs from a TIN source.
    pub tins_included: bool,
}

/// "entry 4" or "entries 4, 9".
fn entries_text(ids: &[EntryId]) -> String {
    let list = ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if ids.len() == 1 {
        format!("entry {list}")
    } else {
        format!("entries {list}")
    }
}

/// "1 payment" or "3 payments", with their total.
fn payments_text(list: &[Payment1099]) -> String {
    let total: Decimal = list.iter().map(|p| p.amount).sum();
    let n = list.len();
    format!(
        "{n} payment{} totaling {}",
        if n == 1 { "" } else { "s" },
        money(total)
    )
}

/// The worksheet's notes (INVARIANT 5).
fn notes_for(ledger: &Ledger, r: &ledger_core::Vendor1099Report) -> Vec<String> {
    let year = r.year;
    let name = |id: VendorId| {
        ledger
            .get_vendor(id)
            .map_or_else(|| format!("vendor {id}"), |v| v.name.clone())
    };
    let mut notes = vec![
        "Only entries tagged with a vendor count (books.set_entry_vendor; books.pay_bill tags automatically).".to_string(),
        "Each vendor's 1099 decision says why when none is due (a corporation, the United States or a state: Treas. Reg. §1.6041-3); payments to a corporation for legal or medical services stay reportable, and royalties report at $10. Confirm with the CPA.".to_string(),
    ];
    if let Some(t) = r.through {
        notes.push(format!(
            "Payments from {year}-01-01 through {t} only: later payments in {year} aren't counted."
        ));
    }
    if year > 2026 {
        notes.push(format!(
            "The $2,000 threshold is indexed for inflation after 2026; confirm the {year} amount and pass threshold to override."
        ));
    }
    if !r.excluded_card_payments.is_empty() {
        notes.push(format!(
            "{} by card or payment network are excluded: the processor reports them on Form 1099-K, whoever's card it was (Treas. Reg. §1.6041-1(a)(1)(iv)).",
            payments_text(&r.excluded_card_payments)
        ));
    }
    if !r.undetermined_payments.is_empty() {
        let ids: Vec<EntryId> = r.undetermined_payments.iter().map(|p| p.entry_id).collect();
        notes.push(format!(
            "{} through a financing account (or a member's or a payment platform's) have no payment method recorded ({}), so whether they count is undetermined: a card or the platform's goods-and-services payment is excluded, cash, a check or a friends-and-family transfer counts. Record how each was paid with books.set_entry_instrument (books.instruments lists the methods).",
            payments_text(&r.undetermined_payments),
            entries_text(&ids)
        ));
    }
    let undecided: Vec<&str> = r
        .vendors
        .iter()
        .filter(|v| v.decision_needed)
        .map(|v| v.name.as_str())
        .collect();
    if !undecided.is_empty() {
        notes.push(format!(
            "No 1099 decision for payments that could count: {}. Record one with books.set_vendor_1099.",
            undecided.join(", ")
        ));
    }
    for v in &r.vendors {
        let mut forms: Vec<&'static str> = v
            .decisions
            .iter()
            .filter_map(|d| d.decision.form().map(|f| f.label()))
            .collect();
        forms.dedup();
        if forms.len() > 1 {
            notes.push(format!(
                "{}'s 1099 form changed during {year} ({}): each payment goes on the form for what it paid for. Confirm with the CPA.",
                v.name,
                forms.join(", then ")
            ));
        }
    }
    for p in &r.other_receipts {
        // Money in is negative; its reversal gives it back.
        let what = if p.amount < Decimal::ZERO {
            format!("received {} from", money(-p.amount))
        } else {
            format!("reversed {} received from", money(p.amount))
        };
        notes.push(format!(
            "Entry {} {what} {} for something else (a sale, its payment of an invoice, a disposal, a loan): not netted against its payments.",
            p.entry_id,
            name(p.vendor_id)
        ));
    }
    let untagged = &r.untagged_payments;
    if !untagged.is_empty() {
        let n = untagged.len();
        let which = if n <= 10 {
            format!(" ({})", entries_text(untagged))
        } else {
            String::new()
        };
        notes.push(format!(
            "{n} posted entr{} in {year} paid money out with no vendor tag{which}; tag any that paid a contractor or landlord with books.set_entry_vendor.",
            if n == 1 { "y" } else { "ies" }
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
    notes
}

/// The 1099 worksheet for `year` (threshold: the IRS figure to use when
/// the year's isn't known, else none), entries dated after `through`
/// left out, each line with its full TIN from `tins` when given
/// (INVARIANT 3).
pub fn vendor_1099_worksheet(
    ledger: &Ledger,
    settings: &Settings<'_>,
    year: i32,
    threshold: Option<Decimal>,
    through: Option<NaiveDate>,
    tins: Option<TinSource<'_>>,
) -> Result<Vendor1099Worksheet, String> {
    // A year past what wanderbooks knows is refused with guidance
    // (research or ask, pass `threshold`, then update the rule).
    let r = vendor_1099_report(ledger, settings, year, threshold, through)
        .map_err(|e| e.to_string())?;
    let notes = notes_for(ledger, &r);
    let mut lines = Vec::new();
    for v in &r.vendors {
        let party_id = ledger.party_of_vendor(v.vendor_id);
        let tin_last4 = party_id
            .and_then(|id| ledger.get_party(id))
            .and_then(|p| p.tin_last4())
            .map(str::to_string);
        let mut line = Vendor1099WorksheetLine {
            vendor_id: v.vendor_id,
            name: v.name.clone(),
            form_1099: form_1099_label(v.form()),
            decision_1099: v.decision.key().to_string(),
            decision: v.decision.clone(),
            decided: v.decided.clone(),
            effective: v.effective,
            decisions: v.decisions.clone(),
            tin_on_file: v.tin_on_file,
            paid: v.paid,
            excluded_card: v.excluded_card,
            undetermined: v.undetermined,
            not_reportable: v.not_reportable,
            decision_needed: v.decision_needed,
            status: v.status,
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
    let missing_w9 = lines
        .iter()
        .filter(|l| l.may_file() && !l.tin_on_file)
        .map(|l| {
            if l.reportable {
                l.name.clone()
            } else {
                format!("{} (may be reportable)", l.name)
            }
        })
        .collect();
    Ok(Vendor1099Worksheet {
        year: r.year,
        threshold: r.threshold,
        through: r.through,
        lines,
        missing_w9,
        unresolved: r.unresolved,
        excluded_card_payments: r.excluded_card_payments,
        undetermined_payments: r.undetermined_payments,
        other_receipts: r.other_receipts,
        untagged_payments: r.untagged_payments,
        notes,
        tins_included: tins.is_some(),
    })
}

/// A dated decision for the CSV: its key, with its kind when not
/// reportable, and its date when it has one ("nec",
/// "not_reportable:corporation from 2026-12-30").
fn decision_cell(d: &ledger_core::Dated1099) -> String {
    let mut s = d.decision.key().to_string();
    if let Decision1099::NotReportable { kind, .. } = &d.decision {
        s.push(':');
        s.push_str(kind.key());
    }
    if let Some(e) = d.effective {
        s.push_str(&format!(" from {e}"));
    }
    s
}

/// The worksheet's CSV header (INVARIANT 4).
pub const VENDOR_1099_CSV_HEADER: &str = "vendor_id,name,decision_1099,form_1099,reason_kind,reason,decided_by,decided_on,effective,decisions_in_year,paid,excluded_card,undetermined,not_reportable,status,reportable,w9_on_file,tin_last4";

/// The worksheet as CSV (INVARIANT 4): per vendor its id and name, its
/// year-end 1099 decision (key, form, reason kind and words, who decided,
/// when, the date it took effect) and every decision in force during the
/// year, what counts, what was paid by card or network, what is
/// undetermined, what was paid while not reportable, its status, whether
/// it is reportable, its W-9 flag and last four; `with_tins` adds the TIN,
/// its type, and the note why it's missing.
pub fn vendor_1099_csv(worksheet: &Vendor1099Worksheet, with_tins: bool) -> String {
    let mut csv = String::from(VENDOR_1099_CSV_HEADER);
    if with_tins {
        csv.push_str(",tin,tin_type,tin_note");
    }
    csv.push('\n');
    for l in &worksheet.lines {
        let (kind, reason) = match &l.decision {
            Decision1099::NotReportable { kind, reason } => (kind.key(), reason.as_str()),
            _ => ("", ""),
        };
        let mut cells = vec![
            l.vendor_id.to_string(),
            csv_escape(&l.name),
            l.decision_1099.clone(),
            l.form_1099.clone().unwrap_or_default(),
            kind.to_string(),
            csv_escape(reason),
            csv_escape(l.decided.as_ref().map_or("", |s| s.decided_by.as_str())),
            l.decided
                .as_ref()
                .map(|s| s.decided_on.to_string())
                .unwrap_or_default(),
            l.effective.map(|e| e.to_string()).unwrap_or_default(),
            csv_escape(
                &l.decisions
                    .iter()
                    .map(decision_cell)
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
            money(l.paid),
            money(l.excluded_card),
            money(l.undetermined),
            money(l.not_reportable),
            l.status.key().to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::{AccountType, Currency, Money, Side};

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn date(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    /// The untagged note (P3 review R7), by hand. Bank 1000 (cash), Due to
    /// Members 2100 (financing), card 2050 (a business card's), capital
    /// 3000, labor 6000, prepaid 1300, equipment 1500. Untagged, in 2026:
    ///   1. a contribution, Dr 1000 / Cr 3000 500: money in, not counted;
    ///   2. a member reimbursed, Dr 2100 / Cr 1000 80: nets to zero;
    ///   3. a property contribution, Dr 1500 / Cr 3000: no payment line;
    ///   4. the card bill, Dr 2050 / Cr 1000 300: nets to zero;
    ///   5. a distribution, Dr 3000 / Cr 1000 200: paid out, but for no
    ///      expense or asset: not counted;
    ///   6. a fee a member paid, Dr 6000 / Cr 2100 25: counted;
    ///   7. a prepayment, Dr 1300 / Cr 1000 120: counted;
    ///   8. a payment, Dr 6000 / Cr 1000 40, and 9 its reversal: both in
    ///      the year, so neither counts.
    ///
    /// Two entries: 6 and 7.
    #[test]
    fn the_untagged_note_counts_only_money_out() {
        let mut l = Ledger::new(Currency::USD);
        let mut acct = |code: &str, t| l.add_account(code, code, t, None, false).unwrap();
        let bank = acct("1000", AccountType::Asset);
        let prepaid = acct("1300", AccountType::Asset);
        let equipment = acct("1500", AccountType::Asset);
        let card = acct("2050", AccountType::Liability);
        let due = acct("2100", AccountType::Liability);
        let capital = acct("3000", AccountType::Equity);
        let labor = acct("6000", AccountType::Expense);
        let post = |l: &mut Ledger, dr, cr, amt: &str| {
            let e = l.create_entry(date("2026-05-01"), "x");
            let m = Money::new(dec(amt), Currency::USD);
            l.add_line(e, dr, Side::Debit, m).unwrap();
            l.add_line(e, cr, Side::Credit, m).unwrap();
            l.post_entry(e).unwrap();
            e
        };
        post(&mut l, bank, capital, "500");
        post(&mut l, due, bank, "80");
        post(&mut l, equipment, capital, "900");
        post(&mut l, card, bank, "300");
        post(&mut l, capital, bank, "200");
        let fee = post(&mut l, labor, due, "25");
        let pre = post(&mut l, prepaid, bank, "120");
        let wrong = post(&mut l, labor, bank, "40");
        l.reverse_entry(wrong, date("2026-05-02"), "oops").unwrap();
        l.add_instrument(
            ledger_core::InstrumentKind::BusinessCard,
            "Card",
            None,
            None,
            card,
        )
        .unwrap();
        let r = ledger_core::vendor_1099_report(
            &l,
            2026,
            None,
            &ledger_core::PaymentAccounts {
                cash: vec![bank],
                financing: vec![due],
            },
            &[],
            None,
        )
        .unwrap();
        assert_eq!(r.untagged_payments, vec![fee, pre]);
        let notes = notes_for(&l, &r);
        assert!(
            notes.contains(&format!(
                "2 posted entries in 2026 paid money out with no vendor tag (entries {fee}, {pre}); tag any that paid a contractor or landlord with books.set_entry_vendor."
            )),
            "{notes:?}"
        );
    }

    /// The note on money received from a vendor for something else says
    /// what each entry did: the sale received the money, its reversal
    /// gave it back (never "received -500.00").
    #[test]
    fn the_other_receipts_note_says_what_each_entry_did() {
        let mut l = Ledger::new(Currency::USD);
        let bank = l
            .add_account("1000", "Bank", AccountType::Asset, None, false)
            .unwrap();
        let sales = l
            .add_account("4000", "Sales", AccountType::Revenue, None, false)
            .unwrap();
        let dana = l.add_vendor("Dana").unwrap();
        let e = l.create_entry(date("2026-04-01"), "laptop sold to Dana");
        let m = Money::new(dec("500.00"), Currency::USD);
        l.add_line(e, bank, Side::Debit, m).unwrap();
        l.add_line(e, sales, Side::Credit, m).unwrap();
        l.post_entry(e).unwrap();
        l.set_entry_vendor(e, Some(dana)).unwrap();
        let undo = l.reverse_entry(e, date("2026-04-02"), "not Dana").unwrap();
        let r = ledger_core::vendor_1099_report(
            &l,
            2026,
            None,
            &ledger_core::PaymentAccounts {
                cash: vec![bank],
                financing: vec![],
            },
            &[],
            None,
        )
        .unwrap();
        let notes = notes_for(&l, &r);
        for want in [
            format!(
                "Entry {e} received 500.00 from Dana for something else (a sale, its payment of an invoice, a disposal, a loan): not netted against its payments."
            ),
            format!(
                "Entry {undo} reversed 500.00 received from Dana for something else (a sale, its payment of an invoice, a disposal, a loan): not netted against its payments."
            ),
        ] {
            assert!(notes.contains(&want), "{want} not in {notes:?}");
        }
    }
}
