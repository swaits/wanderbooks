//! A demo company, built through the real engine: fifteen months of a
//! small design studio (single-member LLC) with client invoices, template
//! sales paid out by a platform, contractors (one without a W-9), rent on a
//! recurring schedule, a credit card, a laptop on depreciation, prepaid
//! insurance, owner draws, a closed year, PDF receipts, and a few loose
//! ends for the "needs attention" list. Deterministic for a given `today`.

use chrono::{Datelike, Days, Months, NaiveDate};
use ledger_core::{
    AccountType, CloseTo, Currency, Form1099, Frequency, Money, NewAsset, NewRecurring,
    NewSchedule, ScheduleKind, Side, arap::DocumentLine,
};
use ledger_store::{Store, StoreError};
use rust_decimal::Decimal;
use std::path::Path;

/// The demo's `wanderbooks.toml`.
pub const BOOKS_TOML: &str = r#"[company]
name = "Juniper Studio LLC"
entity_type = "LLC, single-member"
tax_classification = "disregarded entity"  # Schedule C
fiscal_year_start = "01-01"
formation_state = "OR"
formed = "2025-06-02"
ein = "00-0000000"
address = "1200 NW Everett St, Portland, OR 97209"
owner = "Alex Rivera"

[books]
accounting_basis = "accrual"
receipt_threshold = "25.00"
close_into = "3000"
close_sweep = ["3100"]
cash_accounts = ["1000", "1010"]
"#;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    /// Uniform in [lo, hi] dollars, to the cent.
    fn amount(&mut self, lo: u64, hi: u64) -> Decimal {
        let cents = lo * 100 + self.next() % ((hi - lo) * 100 + 1);
        Decimal::new(cents as i64, 2)
    }
    fn pick(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn usd(d: Decimal) -> Money {
    Money::new(d, Currency::USD)
}

fn dec(n: i64) -> Decimal {
    Decimal::from(n)
}

fn day(month_start: NaiveDate, d: u32) -> NaiveDate {
    let last = (month_start + Months::new(1) - Days::new(1)).day();
    NaiveDate::from_ymd_opt(month_start.year(), month_start.month(), d.min(last)).expect("valid")
}

/// A one-page PDF receipt with the given lines (Helvetica).
pub fn receipt_pdf(title: &str, lines: &[String]) -> Vec<u8> {
    let esc = |s: &str| {
        s.replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)")
    };
    let mut content = String::new();
    content.push_str("BT /F2 22 Tf 56 760 Td (");
    content.push_str(&esc(title));
    content.push_str(") Tj ET\n");
    content.push_str("0.85 0.85 0.85 RG 56 742 m 556 742 l S\n");
    let mut y = 712;
    for l in lines {
        let (font, size) = if l.starts_with("TOTAL") {
            ("F2", 13)
        } else {
            ("F1", 11)
        };
        content.push_str(&format!(
            "BT /{font} {size} Tf 56 {y} Td ({}) Tj ET\n",
            esc(l)
        ));
        y -= 20;
    }
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for off in offsets {
        pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

fn receipt(vendor: &str, date: NaiveDate, item: &str, amount: Decimal, paid: &str) -> Vec<u8> {
    receipt_pdf(
        vendor,
        &[
            format!("Receipt  {}", date.format("%B %-d, %Y")),
            format!(
                "Order {:08}",
                (date.num_days_from_ce() as u64 * 7919
                    + (amount * dec(100)).try_into().unwrap_or(0u64))
                    % 100_000_000
            ),
            String::new(),
            item.to_string(),
            format!("Amount: ${amount}"),
            String::new(),
            format!("TOTAL  ${amount}"),
            format!("Paid with {paid}"),
            String::new(),
            "Thank you for your business.".into(),
        ],
    )
}

/// Build the demo books in `dir` (which must not already hold books): the
/// journal and blobs under `dir/books-data`, the settings in
/// `dir/wanderbooks.toml`.
pub fn build(dir: &Path, today: NaiveDate) -> Result<(), StoreError> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("wanderbooks.toml"), BOOKS_TOML)?;
    let mut s = Store::create(&dir.join("books-data"), Currency::USD)?;
    let mut rng = Rng(0x5eed_1234);
    let start =
        NaiveDate::from_ymd_opt(today.year(), today.month(), 1).expect("valid") - Months::new(15);
    let this_month = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).expect("valid");

    let acct =
        |s: &mut Store, code: &str, name: &str, t, tax: Option<&str>| -> Result<u64, StoreError> {
            let id = s.add_account(code, name, t, None, false)?;
            if let Some(c) = tax {
                s.set_account_tax(id, Some(c), None)?;
            }
            Ok(id)
        };
    use AccountType::*;
    let checking = acct(&mut s, "1000", "Operating Checking", Asset, None)?;
    let savings = acct(&mut s, "1010", "Savings", Asset, None)?;
    let prepaid = acct(&mut s, "1300", "Prepaid Insurance", Asset, None)?;
    let equipment = acct(&mut s, "1500", "Computer Equipment", Asset, None)?;
    let accum = acct(&mut s, "1510", "Accumulated Depreciation", Asset, None)?;
    let card = acct(&mut s, "2100", "Business Credit Card", Liability, None)?;
    let capital = acct(&mut s, "3000", "Owner's Capital", Equity, None)?;
    let draws = acct(&mut s, "3100", "Owner's Draws", Equity, None)?;
    let _services = acct(
        &mut s,
        "4000",
        "Design Services",
        Revenue,
        Some("gross_receipts"),
    )?;
    let templates = acct(
        &mut s,
        "4100",
        "Template Sales",
        Revenue,
        Some("gross_receipts"),
    )?;
    let fees = acct(
        &mut s,
        "6000",
        "Platform Fees",
        Expense,
        Some("commissions_fees"),
    )?;
    let contract = acct(
        &mut s,
        "6050",
        "Contract Labor",
        Expense,
        Some("contract_labor"),
    )?;
    let software = acct(
        &mut s,
        "6100",
        "Software & Subscriptions",
        Expense,
        Some("office_expense"),
    )?;
    let rent = acct(
        &mut s,
        "6200",
        "Studio Rent",
        Expense,
        Some("rent_property"),
    )?;
    let insurance = acct(&mut s, "6300", "Insurance", Expense, Some("insurance"))?;
    let advertising = acct(&mut s, "6400", "Advertising", Expense, Some("advertising"))?;
    let meals = acct(&mut s, "6500", "Meals", Expense, Some("meals"))?;
    let travel = acct(&mut s, "6600", "Travel", Expense, Some("travel"))?;
    let supplies = acct(&mut s, "6700", "Office Supplies", Expense, Some("supplies"))?;
    let depreciation = acct(
        &mut s,
        "6800",
        "Depreciation",
        Expense,
        Some("depreciation"),
    )?;
    let bank_fees = acct(&mut s, "6900", "Bank Fees", Expense, None)?;
    let client_class = s.add_class("Client work")?;
    let template_class = s.add_class("Templates")?;

    let sam = s.add_vendor("Sam Okafor")?;
    s.set_vendor_1099(sam, Some(Form1099::Nec), true)?;
    let riley = s.add_vendor("Riley Chen")?;
    s.set_vendor_1099(riley, Some(Form1099::Nec), false)?;
    let landlord = s.add_vendor("Everett Lofts LLC")?;
    s.set_vendor_1099(landlord, Some(Form1099::Misc), true)?;
    let customers: Vec<u64> = [
        "Fern & Field Coffee",
        "Bramble Outdoor Co.",
        "Northwind Clinics",
        "Tidepool Records",
        "Larkspur Bakery",
    ]
    .iter()
    .map(|n| s.add_customer(n))
    .collect::<Result<_, _>>()?;

    // The party directory: who they are, beyond the vendor and customer
    // records (roles, W-9 facts with only the TIN's last four, sites).
    {
        use ledger_core::{PartyField, Role, W9Facts};
        let party_of = |s: &Store, v: u64| s.ledger().party_of_vendor(v).expect("linked");
        let sam_p = party_of(&s, sam);
        s.set_party_role(sam_p, Role::Contractor, true)?;
        s.set_party_w9(
            sam_p,
            Some(W9Facts {
                received: start + Days::new(20),
                entity_type: "Individual/sole proprietor".into(),
                exempt_payee_code: None,
                fatca_code: None,
                tin_last4: "4821".into(),
                kept_at: "wanderbooks vault".into(),
                vault_record: None,
            }),
        )?;
        s.update_party(
            sam_p,
            PartyField::Notes(Some(
                "Illustrator for client projects; invoices after each delivery.".into(),
            )),
        )?;
        let riley_p = party_of(&s, riley);
        s.set_party_role(riley_p, Role::Contractor, true)?;
        s.update_party(riley_p, PartyField::Website(Some("rileychen.dev".into())))?;
        let lofts = party_of(&s, landlord);
        s.set_party_role(lofts, Role::Landlord, true)?;
        s.set_party_w9(
            lofts,
            Some(W9Facts {
                received: start + Days::new(3),
                entity_type: "LLC taxed as a partnership".into(),
                exempt_payee_code: None,
                fatca_code: None,
                tin_last4: "3307".into(),
                kept_at: "CPA portal".into(),
                vault_record: None,
            }),
        )?;
        s.update_party(
            lofts,
            PartyField::Address(Some("1200 NW Everett St, Portland, OR 97209".into())),
        )?;
        s.update_party(lofts, PartyField::DisplayName(Some("Everett Lofts".into())))?;
        if let Some(fern) = s.ledger().party_of_customer(customers[0]) {
            s.update_party(
                fern,
                PartyField::Website(Some("fernandfield.coffee".into())),
            )?;
            s.update_party(
                fern,
                PartyField::Address(Some("88 SE Division St, Portland, OR 97202".into())),
            )?;
        }
        let irs = s.add_party("Internal Revenue Service", &[Role::TaxAgency])?;
        s.update_party(irs, PartyField::DisplayName(Some("IRS".into())))?;
        s.update_party(irs, PartyField::Website(Some("irs.gov".into())))?;
        let odr = s.add_party("Oregon Department of Revenue", &[Role::TaxAgency])?;
        s.update_party(odr, PartyField::Website(Some("oregon.gov/dor".into())))?;
        let old = s.add_party("Etsy, Inc.", &[Role::PayoutPlatform])?;
        s.update_party(
            old,
            PartyField::Notes(Some("Sold templates here before moving to Gumroad.".into())),
        )?;
        s.update_party(old, PartyField::Archived(true))?;
    }
    let gumroad = s.add_party("Gumroad, Inc.", &[ledger_core::Role::PayoutPlatform])?;
    s.update_party(
        gumroad,
        ledger_core::PartyField::DisplayName(Some("Gumroad".into())),
    )?;
    s.update_party(
        gumroad,
        ledger_core::PartyField::Website(Some("gumroad.com".into())),
    )?;
    let chase = s.add_party(
        "JPMorgan Chase Bank, N.A.",
        &[ledger_core::Role::FinancialInstitution],
    )?;
    s.update_party(
        chase,
        ledger_core::PartyField::DisplayName(Some("Chase".into())),
    )?;
    let alex = s.add_party("Alex Rivera", &[ledger_core::Role::Member])?;

    // Formation documents.
    s.file_document(
        "formation",
        "Articles of Organization.pdf",
        "application/pdf",
        &receipt_pdf(
            "Articles of Organization",
            &[
                "Juniper Studio LLC".into(),
                "Oregon Secretary of State".into(),
                format!("Filed {}", start.format("%B %-d, %Y")),
            ],
        ),
        Some("Filed with the state"),
    )?;
    s.file_document(
        "tax",
        "IRS EIN confirmation.pdf",
        "application/pdf",
        &receipt_pdf(
            "EIN Assignment",
            &[
                "Juniper Studio LLC".into(),
                "Employer Identification Number assigned".into(),
            ],
        ),
        Some("CP 575"),
    )?;

    // Helper: post a two-line entry with an optional receipt and vendor.
    #[allow(clippy::too_many_arguments)]
    fn post(
        s: &mut Store,
        date: NaiveDate,
        desc: &str,
        dr: u64,
        cr: u64,
        amount: Decimal,
        receipt: Option<(&str, Vec<u8>)>,
        vendor: Option<u64>,
        class: Option<u32>,
    ) -> Result<u64, StoreError> {
        let e = s.create_entry(date, desc)?;
        s.add_line(e, dr, Side::Debit, usd(amount))?;
        s.add_line(e, cr, Side::Credit, usd(amount))?;
        if let Some(c) = class {
            s.set_line_class(e, 0, Some(c))?;
            s.set_line_class(e, 1, Some(c))?;
        }
        if let Some((name, bytes)) = receipt {
            s.attach_receipt(Some(e), name, "application/pdf", &bytes, None)?;
        }
        s.post_entry(e)?;
        if vendor.is_some() {
            s.set_entry_vendor(e, vendor)?;
        }
        Ok(e)
    }

    post(
        &mut s,
        day(start, 2),
        "Owner contribution — startup capital",
        checking,
        capital,
        dec(15000),
        Some((
            "bank-transfer.pdf",
            receipt(
                "Transfer confirmation",
                day(start, 2),
                "Transfer from Alex Rivera personal checking",
                dec(15000),
                "ACH",
            ),
        )),
        None,
        None,
    )?;

    // Rent: a recurring entry, posted every month with its invoice; this
    // month's is left as a draft.
    s.register_recurring(NewRecurring {
        description: "Studio rent — Everett Lofts".into(),
        lines: vec![
            (rent, Side::Debit, usd(dec(1800))),
            (checking, Side::Credit, usd(dec(1800))),
        ],
        frequency: Frequency::Monthly,
        start: day(start, 1),
        end: None,
    })?;

    // Laptop and its depreciation.
    let laptop_date = day(start + Months::new(1), 9);
    let laptop = dec(3199);
    post(
        &mut s,
        laptop_date,
        "MacBook Pro 16\" — studio workstation",
        equipment,
        card,
        laptop,
        Some((
            "apple-store.pdf",
            receipt(
                "Apple Store",
                laptop_date,
                "MacBook Pro 16-inch, M-series",
                laptop,
                "Visa ending 4242",
            ),
        )),
        None,
        None,
    )?;
    let asset = s.register_asset(NewAsset {
        description: "MacBook Pro 16\"".into(),
        purchase_date: laptop_date,
        cost: usd(laptop),
        salvage_value: usd(dec(400)),
        useful_life_months: 36,
        cost_account: equipment,
        accum_depr_account: accum,
        expense_account: depreciation,
    })?;
    // The label on the laptop, and its serial number.
    s.set_asset_tag(asset, "JS-0001")?;
    s.set_asset_serial(asset, Some("C02FK3XJQ6L4"))?;
    // Bought by the company, so its federal tax basis is its cost.
    s.set_asset_tax_basis(
        asset,
        ledger_core::TaxBasis {
            rule: ledger_core::TaxBasisRule::Cost,
            contributor_basis: None,
        },
    )?;

    // Prepaid insurance, spread over a year.
    let ins_date = day(start + Months::new(2), 15);
    post(
        &mut s,
        ins_date,
        "General liability insurance — annual premium",
        prepaid,
        checking,
        dec(1440),
        Some((
            "hiscox-policy.pdf",
            receipt(
                "Hiscox",
                ins_date,
                "General liability, 12 months",
                dec(1440),
                "ACH",
            ),
        )),
        None,
        None,
    )?;
    s.register_schedule(NewSchedule {
        description: "General liability insurance".into(),
        kind: ScheduleKind::PrepaidExpense,
        total: usd(dec(1440)),
        start: ins_date,
        months: 12,
        balance_account: prepaid,
        pl_account: insurance,
    })?;

    let mut open_invoices: Vec<(u64, NaiveDate, Decimal)> = Vec::new();
    let mut card_balance = Decimal::ZERO;
    let mut m = start;
    let mut month_index = 0;
    while m <= this_month {
        let current = m == this_month;
        let in_range = |d: NaiveDate| d <= today;
        // Software on the card.
        for (vendor, item, lo, hi, d) in [
            ("Figma", "Figma Professional — monthly", 45, 45, 3),
            ("Adobe", "Creative Cloud All Apps", 59, 60, 7),
            ("Google Workspace", "Business Starter — 2 users", 28, 28, 11),
        ] {
            let date = day(m, d);
            if !in_range(date) {
                continue;
            }
            let amt = rng.amount(lo, hi);
            post(
                &mut s,
                date,
                &format!("{vendor} — {item}"),
                software,
                card,
                amt,
                Some((
                    &format!("{}-{}.pdf", vendor.to_lowercase().replace(' ', "-"), date),
                    receipt(vendor, date, item, amt, "Visa ending 4242"),
                )),
                None,
                None,
            )?;
            card_balance += amt;
        }
        // Advertising some months.
        if month_index % 3 == 1 {
            let date = day(m, 14);
            if in_range(date) {
                let amt = rng.amount(150, 420);
                post(
                    &mut s,
                    date,
                    "Instagram ads — portfolio campaign",
                    advertising,
                    card,
                    amt,
                    Some((
                        "meta-ads.pdf",
                        receipt("Meta Ads", date, "Sponsored posts", amt, "Visa ending 4242"),
                    )),
                    None,
                    None,
                )?;
                card_balance += amt;
            }
        }
        // Meals and supplies.
        let date = day(m, 17);
        if in_range(date) {
            let amt = rng.amount(28, 96);
            let missing = current || month_index == 13;
            post(
                &mut s,
                date,
                "Client lunch — project kickoff",
                meals,
                card,
                amt,
                (!missing).then(|| {
                    (
                        "lunch.pdf",
                        receipt(
                            "Tasty n Alder",
                            date,
                            "Lunch for 2",
                            amt,
                            "Visa ending 4242",
                        ),
                    )
                }),
                None,
                None,
            )?;
            card_balance += amt;
        }
        if month_index % 4 == 2 {
            let date = day(m, 21);
            if in_range(date) {
                let amt = rng.amount(40, 180);
                post(
                    &mut s,
                    date,
                    "Paper, ink, and sketchbooks",
                    supplies,
                    card,
                    amt,
                    Some((
                        "blick.pdf",
                        receipt(
                            "Blick Art Materials",
                            date,
                            "Supplies",
                            amt,
                            "Visa ending 4242",
                        ),
                    )),
                    None,
                    None,
                )?;
                card_balance += amt;
            }
        }
        if month_index == 7 {
            let date = day(m, 9);
            if in_range(date) {
                let amt = dec(1186) + rng.amount(0, 90);
                post(
                    &mut s,
                    date,
                    "Flights and hotel — client workshop, Austin",
                    travel,
                    card,
                    amt,
                    Some((
                        "alaska-air.pdf",
                        receipt(
                            "Alaska Airlines",
                            date,
                            "PDX-AUS round trip + 2 nights",
                            amt,
                            "Visa ending 4242",
                        ),
                    )),
                    None,
                    None,
                )?;
                card_balance += amt;
            }
        }
        // Pay the card in full on the 25th.
        let date = day(m, 25);
        if in_range(date) && card_balance > Decimal::ZERO {
            let paid = post(
                &mut s,
                date,
                "Credit card payment",
                card,
                checking,
                card_balance,
                Some((
                    "card-statement.pdf",
                    receipt(
                        "Chase Ink",
                        date,
                        "Statement balance payment",
                        card_balance,
                        "ACH from Operating Checking",
                    ),
                )),
                None,
                None,
            )?;
            s.set_entry_party(paid, Some(chase))?;
            card_balance = Decimal::ZERO;
        }
        // Client invoices: two or three a month, net 30.
        for k in 0..(2 + rng.pick(2)) {
            let date = day(m, 5 + 9 * k as u32);
            if !in_range(date) {
                continue;
            }
            let cust = customers[rng.pick(customers.len() as u64) as usize];
            let amt = (rng.amount(1800, 7800) / dec(50)).round() * dec(50);
            let inv = s.create_invoice(
                cust,
                date,
                date + Days::new(30),
                vec![DocumentLine {
                    description: [
                        "Brand identity",
                        "Website design",
                        "Packaging",
                        "Illustration set",
                        "Design retainer",
                    ][rng.pick(5) as usize]
                        .into(),
                    amount: amt,
                }],
            )?;
            let entry = s.ledger().get_invoice(inv).map(|i| i.entry_id);
            if let Some(e) = entry {
                let _ = e;
            }
            open_invoices.push((inv, date, amt));
        }
        // Collect invoices 20-50 days out; leave the latest ones open and one overdue.
        let mut still_open = Vec::new();
        for (inv, date, amt) in open_invoices.drain(..) {
            let paid_on = (date + Days::new(20 + rng.pick(30))).max(day(m, 2));
            let overdue_one = month_index == 12 && inv % 2 == 0;
            if paid_on <= today && paid_on < m + Months::new(1) && !overdue_one {
                s.receive_payment(inv, paid_on, amt, checking)?;
            } else {
                still_open.push((inv, date, amt));
            }
        }
        open_invoices = still_open;
        // Template sales, paid out by the platform on the 1st for last month.
        if month_index > 1 {
            let date = day(m, 1);
            if in_range(date) {
                let gross = rng.amount(450, 1600);
                let fee = (gross * Decimal::new(10, 2)).round_dp(2) + Decimal::new(30, 2);
                let net = gross - fee;
                let e = s.create_entry(date, "Gumroad payout — template sales")?;
                s.add_line(e, checking, Side::Debit, usd(net))?;
                s.add_line(e, fees, Side::Debit, usd(fee))?;
                s.add_line(e, templates, Side::Credit, usd(gross))?;
                for i in 0..3 {
                    s.set_line_class(e, i, Some(template_class))?;
                }
                s.attach_receipt(
                    Some(e),
                    "gumroad-payout.pdf",
                    "application/pdf",
                    &receipt_pdf(
                        "Gumroad payout",
                        &[
                            format!("Payout {}", date.format("%B %-d, %Y")),
                            format!("Gross sales: ${gross}"),
                            format!("Fees: ${fee}"),
                            format!("TOTAL  ${net}"),
                            "Deposited to checking ending 0187".into(),
                        ],
                    ),
                    None,
                )?;
                s.post_entry(e)?;
                s.set_entry_party(e, Some(gumroad))?;
            }
        }
        // Contractors.
        if month_index % 2 == 0 && month_index > 0 {
            let date = day(m, 28);
            if in_range(date) {
                let amt = (rng.amount(900, 2200) / dec(25)).round() * dec(25);
                post(
                    &mut s,
                    date,
                    "Sam Okafor — illustration (invoice)",
                    contract,
                    checking,
                    amt,
                    Some((
                        "sam-okafor-invoice.pdf",
                        receipt(
                            "Sam Okafor Illustration",
                            date,
                            "Illustration for client projects",
                            amt,
                            "ACH",
                        ),
                    )),
                    Some(sam),
                    Some(client_class),
                )?;
            }
        }
        if month_index == 11 || month_index == 13 {
            let date = day(m, 19);
            if in_range(date) {
                post(
                    &mut s,
                    date,
                    "Riley Chen — front-end development",
                    contract,
                    checking,
                    dec(1400),
                    Some((
                        "riley-chen.pdf",
                        receipt(
                            "Riley Chen",
                            date,
                            "Website build, milestone",
                            dec(1400),
                            "ACH",
                        ),
                    )),
                    Some(riley),
                    Some(client_class),
                )?;
            }
        }
        // Owner draws and savings.
        if month_index >= 3 {
            let date = day(m, 28);
            if in_range(date) {
                let draw = post(
                    &mut s,
                    date,
                    "Owner draw",
                    draws,
                    checking,
                    dec(3500),
                    None,
                    None,
                    None,
                )?;
                s.set_entry_party(draw, Some(alex))?;
            }
        }
        if month_index % 3 == 2 {
            let date = day(m, 26);
            if in_range(date) {
                post(
                    &mut s,
                    date,
                    "Transfer to savings",
                    savings,
                    checking,
                    dec(4000),
                    None,
                    None,
                    None,
                )?;
            }
        }
        let date = day(m, 30);
        if in_range(date) && month_index % 5 == 4 {
            post(
                &mut s,
                date,
                "Wire fee",
                bank_fees,
                checking,
                dec(30),
                None,
                None,
                None,
            )?;
        }
        // Month-end: rent occurrences, depreciation, amortization.
        let month_end = m + Months::new(1) - Days::new(1);
        let through = if current { today } else { month_end };
        for (_, date, entry) in s.run_recurring(through)? {
            if current {
                continue; // this month's rent waits as a draft
            }
            s.attach_receipt(
                Some(entry),
                &format!("rent-{date}.pdf"),
                "application/pdf",
                &receipt(
                    "Everett Lofts LLC",
                    date,
                    "Studio 4B rent",
                    dec(1800),
                    "ACH",
                ),
                None,
            )?;
            s.post_entry(entry)?;
            s.set_entry_vendor(entry, Some(landlord))?;
        }
        if !current {
            if m >= laptop_date.with_day(1).expect("valid") {
                s.run_depreciation(asset, &m.format("%Y-%m").to_string())?;
            }
            s.run_schedules(month_end)?;
        }
        // Close each finished calendar year into capital, draws swept in.
        if m.month() == 12 && !current {
            let ys = NaiveDate::from_ymd_opt(m.year(), 1, 1).expect("valid");
            let p = s.create_period(&m.year().to_string(), ys.max(start), month_end)?;
            s.close_period_to(
                p,
                &CloseTo::Account {
                    into: capital,
                    sweep: vec![draws],
                },
            )?;
        }
        m = m + Months::new(1);
        month_index += 1;
    }
    let _ = (bank_fees, client_class);
    Ok(())
}
