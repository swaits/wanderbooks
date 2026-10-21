//! The CPA handoff package: every statement, ledger, and worksheet the CPA
//! needs for the return, with the evidence behind the year's entries, in
//! one zip.
//!
//! ```text
//! GOAL: everything the CPA needs for the return in one zip, the K-1s'
//!       and 1099s' TINs included when asked, built the same by the
//!       server and the viewer.
//! SOURCE: docs/mcp-interface.md; B-26; B-33 (as of one day); B-34 (the
//!         IIF); DECISION (owner, 2026-10-15): "it needs to be in the damn
//!         CPA package."
//! INVARIANTS:
//!   1. The statements (profit and loss, balance sheet, cash flow, members'
//!      equity with members, tax summary, trial balance) each as PDF and
//!      XLSX, for the fiscal year containing as_of, through as_of; the
//!      fixed asset register when an asset was bought by as_of; the
//!      general ledger and journal (XLSX); the journal through as_of as
//!      IIF; the 1099 worksheet for as_of's calendar year through as_of
//!      (or why it can't be made); with members, the K-1 worksheet; a
//!      README first.
//!   2. With members, k1-members.csv has one row per member: name, party,
//!      share, capital account, TIN and type (or last four without a TIN
//!      source), and the K-1 worksheet's capital figures (ledger_core::k1)
//!      for the fiscal year to date; the members' total row with more than
//!      one. The 1099 worksheet adds each payee's party, last four, TIN,
//!      and type with a TIN source.
//!   3. A TIN the source can't give never fails the package: the row says
//!      why, and so do the README and tin_gaps (members, and 1099 payees
//!      that are reportable).
//!   4. With include_receipts: every current file behind the year's
//!      posted entries, filed on them or linked to them, once per entry,
//!      each with a row in receipts/index.csv; and the company's
//!      documents, the files behind no entry (loose receipts aside).
//!      Withdrawn files never.
//!   5. Read-only (crate INVARIANT 1): the package is returned, its files
//!      and the zip, and written nowhere.
//!   6. As of one day (B-33): no figure, entry, receipt or worksheet row
//!      reflects anything dated after as_of; the chart and documents/ are
//!      as they stand when the package is made.
//!   7. One list of cash accounts and one of financing accounts per
//!      package: the request's, else the settings'. The cash flow, the K-1
//!      worksheet, the IIF's BANK accounts and the 1099 worksheet's
//!      payments all read the same.
//!   8. Every account name, number or place and every memo
//!      general-journal.iif writes differently from the books (it holds
//!      plain ASCII, within QuickBooks Desktop's limits and its rule that
//!      a subaccount has its parent's type: B-34) is in `warnings` and the
//!      README, the memos grouped by what is written differently; the
//!      README also names the entries with a line on accounts receivable
//!      or payable, which the file gives no customer or vendor.
//! ```

use chrono::NaiveDate;
use ledger_core::{AccountId, Side, reports};
use rust_decimal::Decimal;

use crate::books::PackageBooks;
use crate::iif::export_iif;
use crate::reports::{
    Settings, bs_statement, cash_flow_for, member_capitals, pnl_statement, report_members,
    report_meta, year_earnings,
};
use crate::text::{csv_escape, money, share_text};
use crate::tins::{TinSource, tin_type_key};
use crate::vendor_1099::vendor_1099_report;

/// What the package is built for.
#[derive(Clone, Copy)]
pub struct PackageRequest<'a> {
    /// The day it is as of: the balance sheet's date, the end of every
    /// period report (the fiscal year to date).
    pub as_of: NaiveDate,
    /// The day it is prepared, for the reports' headers.
    pub prepared_on: NaiveDate,
    /// Cash account codes the cash flow statement reconciles to (empty:
    /// the settings' cash accounts).
    pub cash_accounts: &'a [String],
    /// Financing account codes (empty: the settings' financing accounts).
    pub financing_accounts: &'a [String],
    /// Investing account codes (empty: the settings' investing accounts).
    pub investing_accounts: &'a [String],
    /// Whether the receipts and the company's documents go in
    /// (INVARIANT 4).
    pub include_receipts: bool,
    /// Where the worksheets' full TINs come from: none, last fours only.
    pub tins: Option<TinSource<'a>>,
}

/// A built package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    /// The zip's name: `cpa-package-<as_of>.zip`.
    pub file_name: String,
    /// Every file in the zip, in its order: (name in the zip, bytes).
    pub files: Vec<(String, Vec<u8>)>,
    /// The zip.
    pub zip: Vec<u8>,
    /// With a TIN source: each member or reportable 1099 payee whose TIN
    /// the package lacks, and why (INVARIANT 3).
    pub tin_gaps: Vec<String>,
    /// The receipts in receipts/ (one per entry they are behind).
    pub receipt_count: usize,
    /// What general-journal.iif writes differently from the books to fit
    /// QuickBooks Desktop (INVARIANT 8), as `ledger_import::Iif::warnings`
    /// says it: each account name and number a line, each thing written
    /// differently in the memos a line with the entries it is in.
    pub warnings: Vec<String>,
    /// How many account names, numbers and memos general-journal.iif
    /// writes differently (each one the warnings cover).
    pub iif_changes: usize,
}

/// The package's file name for `as_of`.
pub fn package_file_name(as_of: NaiveDate) -> String {
    format!("cpa-package-{as_of}.zip")
}

/// Build the CPA package from `books` (INVARIANTS 1-5).
pub fn build_package<B: PackageBooks + ?Sized>(
    books: &B,
    settings: &Settings<'_>,
    req: &PackageRequest<'_>,
) -> Result<Package, String> {
    use ledger_import::{XlsxReport, report, report_to_xlsx, statement_pdf};
    let company = settings.company;
    let (as_of, today) = (req.as_of, req.prepared_on);
    let meta = report_meta(company, req.prepared_on, books.journal_seq());
    let ledger = books.ledger();
    let tins = req.tins;
    let mut tin_gaps: Vec<String> = Vec::new();

    // The fiscal year to date: the tax year holding as_of (IRS Pub. 538),
    // the fiscal-year fact itself, not "what omitted dates mean" (rules.md
    // B-30), so the package's year never follows a default.
    let fy = ledger_core::calendar::fiscal_year_start_on(company.fiscal_year_start, as_of);
    let (fy_start, as_of_s) = (fy.to_string(), as_of.to_string());
    let xlsx = |r: XlsxReport<'_>| report_to_xlsx(&r, &meta).map_err(|e| e.to_string());
    let pdf = |st: &ledger_import::Statement| statement_pdf(st, &meta).map_err(|e| e.to_string());
    let members = report_members(company);
    // One list of each per package (INVARIANT 7).
    let cash_codes = if req.cash_accounts.is_empty() {
        settings.cash_accounts
    } else {
        req.cash_accounts
    };
    let financing_codes = if req.financing_accounts.is_empty() {
        settings.financing_accounts
    } else {
        req.financing_accounts
    };

    // The financial statements, each as PDF and XLSX.
    let mut statements: Vec<(&str, ledger_import::Statement)> = vec![
        (
            "profit-and-loss",
            report::profit_and_loss(&pnl_statement(
                ledger,
                company,
                Some(&fy_start),
                Some(&as_of_s),
                None,
                today,
            )?),
        ),
        (
            "balance-sheet",
            report::balance_sheet(
                &bs_statement(ledger, company, Some(&as_of_s), today)?,
                year_earnings(ledger, company, as_of)?,
                &members,
            ),
        ),
        (
            "cash-flow",
            // The package's one list of each (INVARIANT 7); investing
            // accounts are the cash flow's alone (the request's, else the
            // settings').
            report::cash_flow(&cash_flow_for(
                ledger,
                settings,
                Some(&fy_start),
                Some(&as_of_s),
                cash_codes,
                financing_codes,
                req.investing_accounts,
                today,
            )?),
        ),
    ];
    let mut k1_csv: Option<(String, Vec<u8>)> = None;
    if !company.members.is_empty() {
        let capitals = member_capitals(ledger, &company.members)?;
        let me =
            ledger_core::members_equity(ledger, fy, as_of, &capitals).map_err(|e| e.to_string())?;
        statements.push(("members-equity", report::members_equity(&me)));
        // The K-1 worksheet: each member's capital account for the
        // year (Schedule K-1 Item L) and their TIN (INVARIANT 2).
        let cash: Vec<AccountId> = cash_codes
            .iter()
            .filter_map(|c| ledger.chart().find_by_code(c).map(|a| a.id))
            .collect();
        k1_csv = Some(
            match ledger_core::k1::k1_worksheet(ledger, fy, as_of, &capitals, &cash)
                .map_err(|e| e.to_string())
                .and_then(|rows| {
                    // A row per member, in their order, or none: a row is
                    // never paired with another member.
                    if rows.len() == company.members.len() {
                        Ok(rows)
                    } else {
                        Err(format!(
                            "the K-1 worksheet has {} rows for {} members",
                            rows.len(),
                            company.members.len()
                        ))
                    }
                }) {
                Ok(rows) => {
                    let mut w = String::from("member,party_id,share,capital_account,");
                    w.push_str(if tins.is_some() {
                        "tin,tin_type,tin_note,"
                    } else {
                        "tin_last4,tin_type,"
                    });
                    w.push_str("beginning_capital,contributions_cash,contributions_property,net_income,distributions,other,ending_capital\n");
                    for (row, m) in rows.iter().zip(&company.members) {
                        let party = ledger_core::k1::member_party(ledger, &m.name);
                        let facts = party
                            .and_then(|id| ledger.get_party(id))
                            .and_then(|p| p.tin.clone());
                        let mut cells = vec![
                            csv_escape(&m.name),
                            party.map(|id| id.to_string()).unwrap_or_default(),
                            share_text(m.share),
                            m.capital_account.clone(),
                        ];
                        match tins {
                            Some(tins) => {
                                let got = match party {
                                    None => Err(format!(
                                        "TIN not in the vault: no party named {:?} with the member role (books.add_party with role member, or books.set_party_roles, then books.set_tin)",
                                        m.name
                                    )),
                                    Some(id) => tins(id),
                                };
                                match got {
                                    Ok(t) => {
                                        cells.push(t.formatted());
                                        cells.push(tin_type_key(t.kind).into());
                                        cells.push(String::new());
                                    }
                                    Err(why) => {
                                        tin_gaps.push(format!("member {}: {why}", m.name));
                                        cells.push(String::new());
                                        cells.push(
                                            facts
                                                .map(|f| f.tin_type.key().to_string())
                                                .unwrap_or_default(),
                                        );
                                        cells.push(csv_escape(&why));
                                    }
                                }
                            }
                            None => {
                                cells.push(
                                    facts.as_ref().map(|f| f.last4.clone()).unwrap_or_default(),
                                );
                                cells.push(
                                    facts
                                        .map(|f| f.tin_type.key().to_string())
                                        .unwrap_or_default(),
                                );
                            }
                        }
                        for v in [
                            row.beginning,
                            row.contributions_cash,
                            row.contributions_property,
                            row.net_income,
                            row.distributions,
                            row.other,
                            row.ending,
                        ] {
                            cells.push(money(v));
                        }
                        w.push_str(&cells.join(","));
                        w.push('\n');
                    }
                    // The members together: ties to the 1065's Schedule L
                    // capital and the statement of members' equity. Not
                    // a K-1 (each member gets their own).
                    if rows.len() > 1 {
                        let sum = |f: fn(&ledger_core::k1::K1Row) -> Decimal| {
                            rows.iter().map(f).sum::<Decimal>()
                        };
                        let share: Decimal = company.members.iter().map(|m| m.share).sum();
                        let mut cells = vec![
                            csv_escape("Total, all members (not a K-1)"),
                            String::new(),
                            share_text(share),
                            String::new(),
                            String::new(),
                            String::new(),
                        ];
                        if tins.is_some() {
                            cells.push(String::new());
                        }
                        for v in [
                            sum(|r| r.beginning),
                            sum(|r| r.contributions_cash),
                            sum(|r| r.contributions_property),
                            sum(|r| r.net_income),
                            sum(|r| r.distributions),
                            sum(|r| r.other),
                            sum(|r| r.ending),
                        ] {
                            cells.push(money(v));
                        }
                        w.push_str(&cells.join(","));
                        w.push('\n');
                    }
                    ("k1-members.csv".into(), w.into_bytes())
                }
                Err(e) => (
                    "k1-members-NOT-INCLUDED.txt".into(),
                    format!("{e}\n").into_bytes(),
                ),
            },
        );
    }
    statements.push((
        "tax-summary",
        report::tax_summary(
            &ledger_core::tax_summary(ledger, fy, as_of, company.tax_form)
                .map_err(|e| e.to_string())?,
        ),
    ));
    statements.push((
        "trial-balance",
        report::trial_balance(&reports::trial_balance_as_of(ledger, as_of), as_of),
    ));
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, st) in &statements {
        files.push((format!("{name}.pdf"), pdf(st)?));
        files.push((format!("{name}.xlsx"), xlsx(XlsxReport::Statement(st))?));
    }
    // The fixed asset register, when an asset was bought by as_of (one
    // bought later is not in it, so neither is an empty register).
    if ledger.registered_assets().any(|a| a.purchase_date <= as_of) {
        let r = ledger_core::fixed_asset_register(ledger, as_of).map_err(|e| e.to_string())?;
        files.push((
            "fixed-asset-register.pdf".into(),
            ledger_import::asset_register_pdf(&r, &meta).map_err(|e| e.to_string())?,
        ));
        files.push((
            "fixed-asset-register.xlsx".into(),
            ledger_import::asset_register_xlsx(&r, &meta).map_err(|e| e.to_string())?,
        ));
    }
    // The books behind them: by account, and entry by entry.
    let gl = ledger_core::general_ledger(ledger, fy, as_of).map_err(|e| e.to_string())?;
    files.push((
        "general-ledger.xlsx".into(),
        xlsx(XlsxReport::GeneralLedger(&gl))?,
    ));
    files.push((
        "general-journal.xlsx".into(),
        xlsx(XlsxReport::GeneralJournal(ledger, fy, as_of))?,
    ));
    // 1099 worksheet for the calendar year of as_of, through as_of
    // (INVARIANT 6): the payments from January 1, out of the package's
    // cash and financing accounts (INVARIANT 7).
    let year = chrono::Datelike::year(&as_of);
    let mut worksheet_1099 = false;
    let payments = Settings {
        cash_accounts: cash_codes,
        financing_accounts: financing_codes,
        ..*settings
    };
    match vendor_1099_report(ledger, &payments, year, Some(as_of), None) {
        Ok(r1099) => {
            let mut w = String::from("vendor,form,w9_on_file,paid,reportable");
            if tins.is_some() {
                w.push_str(",party_id,tin_last4,tin,tin_type,tin_note");
            }
            w.push('\n');
            for v in &r1099.vendors {
                w.push_str(&format!(
                    "\"{}\",{},{},{},{}",
                    v.name.replace('"', "\"\""),
                    v.form.map(|f| f.label()).unwrap_or(""),
                    v.tin_on_file,
                    money(v.paid),
                    v.reportable
                ));
                // The payee's full TIN from the source (INVARIANT 2).
                if let Some(tins) = tins {
                    let party = ledger.party_of_vendor(v.vendor_id);
                    let last4 = party.and_then(|id| ledger.get_party(id)).and_then(|p| {
                        p.w9.as_ref()
                            .map(|f| f.tin_last4.clone())
                            .or_else(|| p.tin.as_ref().map(|t| t.last4.clone()))
                    });
                    let got = match party {
                        None => Err("TIN not in the vault: no party for this vendor".into()),
                        Some(id) => tins(id),
                    };
                    let (tin, kind, note) = match got {
                        Ok(t) => (
                            t.formatted(),
                            tin_type_key(t.kind).to_string(),
                            String::new(),
                        ),
                        Err(why) => {
                            if v.reportable {
                                tin_gaps.push(format!("1099 payee {}: {why}", v.name));
                            }
                            (String::new(), String::new(), why)
                        }
                    };
                    w.push_str(&format!(
                        ",{},{},{tin},{kind},{}",
                        party.map(|id| id.to_string()).unwrap_or_default(),
                        last4.unwrap_or_default(),
                        csv_escape(&note)
                    ));
                }
                w.push('\n');
            }
            files.push((format!("1099-worksheet-{year}.csv"), w.into_bytes()));
            worksheet_1099 = true;
        }
        // The threshold isn't known for this year: never guess it
        // (rules.md B-19); say so in the package instead.
        Err(e) => files.push((
            format!("1099-worksheet-{year}-NOT-INCLUDED.txt"),
            format!("{e}\n").into_bytes(),
        )),
    }
    // The evidence: every current file behind the year's posted
    // entries, filed on them or linked to them (a file linked to an
    // entry counts as its receipt), one copy per entry with a row in
    // the index; and the company's documents, the files behind no
    // entry (INVARIANT 4).
    let mut receipt_count = 0;
    let mut document_count = 0;
    // Each file's name in the zip is its own: a second `r.pdf` is
    // `r (2).pdf` (compared ignoring case, so it unpacks the same on every
    // system), and the index names the file as it is stored.
    let mut used: std::collections::HashSet<String> =
        files.iter().map(|(n, _)| n.to_lowercase()).collect();
    let mut unique = |name: String| -> String {
        // The extension is the last segment's own: a folder with a dot
        // ("a.b/notes") keeps its name, and a name that is all extension
        // (".env") has none.
        let cut = name.rfind('/').map_or(0, |i| i + 1);
        let (stem, ext) = match name[cut..].rfind('.') {
            Some(i) if i > 0 => (name[..cut + i].to_string(), name[cut + i..].to_string()),
            _ => (name.clone(), String::new()),
        };
        let mut n = 1;
        let mut candidate = name;
        while !used.insert(candidate.to_lowercase()) {
            n += 1;
            candidate = format!("{stem} ({n}){ext}");
        }
        candidate
    };
    if req.include_receipts {
        let clean = |s: &str| -> String {
            let c: String = s
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || ".-_ ".contains(c) {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            // A name of dots alone ("", ".", "..") would name no file, or
            // climb out of its folder when the zip is unpacked.
            if c.chars().all(|c| c == '.') {
                "_".repeat(c.len().max(1))
            } else {
                c
            }
        };
        let mut index = String::from("entry_id,date,description,amount,file,sha256\n");
        for a in books.all_attachments().map_err(|e| e.to_string())? {
            if a.withdrawn_at.is_some() {
                continue;
            }
            let mut behind: Vec<u64> = a.entry_id.into_iter().collect();
            for sub in books.links_of(a.id).map_err(|e| e.to_string())? {
                if let ledger_store::Subject::Entry(e) = sub
                    && !behind.contains(&e)
                {
                    behind.push(e);
                }
            }
            behind.sort_unstable();
            let bytes = books.attachment_bytes(a.id).map_err(|e| e.to_string())?;
            if behind.is_empty() {
                if a.category != "receipts" {
                    files.push((
                        unique(format!(
                            "documents/{}/{}",
                            clean(&a.category),
                            clean(&a.file_name)
                        )),
                        bytes,
                    ));
                    document_count += 1;
                }
                continue;
            }
            for e in behind.iter().filter_map(|&e| ledger.get_entry(e)) {
                if e.status() != ledger_core::EntryStatus::Posted
                    || e.date() < fy
                    || e.date() > as_of
                {
                    continue;
                }
                let name = unique(format!(
                    "receipts/{}-entry-{}-{}",
                    e.date(),
                    e.id(),
                    clean(&a.file_name)
                ));
                let amount: Decimal = e
                    .lines()
                    .iter()
                    .filter(|l| l.side == Side::Debit)
                    .map(|l| l.amount.amount)
                    .sum();
                index.push_str(&format!(
                    "{},{},\"{}\",{},{},{}\n",
                    e.id(),
                    e.date(),
                    e.description().replace('"', "\"\""),
                    money(amount),
                    name,
                    a.blob_sha
                ));
                files.push((name, bytes.clone()));
                receipt_count += 1;
            }
        }
        files.push(("receipts/index.csv".into(), index.into_bytes()));
    }
    let k1_included = k1_csv
        .as_ref()
        .is_some_and(|(name, _)| name == "k1-members.csv");
    if let Some(k1) = k1_csv {
        files.push(k1);
    }
    let seq = books.journal_seq();
    // The journal through as_of for QuickBooks, closes left out, and what
    // it writes differently (INVARIANT 8).
    let iif = export_iif(ledger, cash_codes, as_of)?;
    let (warnings, iif_changes) = (iif.warnings(), iif.changes.len());
    let receivable_payable: Vec<String> = iif
        .receivable_payable
        .iter()
        .map(ToString::to_string)
        .collect();
    files.push(("general-journal.iif".into(), iif.text.into_bytes()));
    let year_end = NaiveDate::from_ymd_opt(year, 12, 31) == Some(as_of);
    let include_tins = tins.is_some();
    let readme = format!(
        "{name} — books package for the CPA\n\nFiscal year to date: {fy_start} through {as_of_s}. {basis} basis.\nKept in wanderbooks: double-entry books from an append-only, hash-chained journal ({seq} events, verified).\n\nStatements (PDF and XLSX): profit and loss, balance sheet, cash flow, {members}tax summary, trial balance.\nLedgers (XLSX): general ledger by account; general journal entry by entry.\n{iif}{worksheet}{k1}{receipts}{tins}\nThe tax summary maps accounts to return categories; it's a worksheet, not a filing. Questions go to the owners, who can ask their bookkeeping agent to trace any number to its entries and receipts.\n",
        name = company.name,
        basis = company.basis,
        members = if company.members.is_empty() {
            ""
        } else {
            "members' equity, "
        },
        iif = {
            let mut t = format!(
                "general-journal.iif: every posted entry from the first through {as_of_s}, for import into QuickBooks Desktop. Plain ASCII text (its import turns anything else into \"?\"), CR LF line endings; DOCNUM is each entry's id in these books. Period-closing entries are left out: QuickBooks keeps none and works out retained earnings itself, so an imported close would zero its period's profit; trial-balance includes them{}.\n",
                if company.members.is_empty() {
                    ""
                } else {
                    ", and the members' equity statement and the K-1 worksheet show each member's share"
                }
            );
            if !receivable_payable.is_empty() {
                t.push_str(&format!(
                    "It names no customer or vendor on its lines to accounts receivable or payable, in {} {}: QuickBooks Desktop asks for one on a journal entry to A/R or A/P (and takes only one A/R or A/P account a transaction); add it there if QuickBooks asks.\n",
                    if receivable_payable.len() == 1 { "entry" } else { "entries" },
                    receivable_payable.join(", ")
                ));
            }
            if !warnings.is_empty() {
                t.push_str(&format!(
                    "Written differently in general-journal.iif to fit QuickBooks Desktop (plain ASCII; account names at most {} characters a level, no two alike; a subaccount of another QuickBooks type than its parent's at the top level; account numbers at most {} characters, no two alike; memos at most {} characters); every other file keeps the books' text:\n",
                    ledger_import::NAME_MAX,
                    ledger_import::NUMBER_MAX,
                    ledger_import::MEMO_MAX
                ));
                for w in &warnings {
                    t.push_str(&format!("- {w}\n"));
                }
            }
            t
        },
        worksheet = if worksheet_1099 {
            format!(
                "1099-worksheet-{year}.csv: cash paid per vendor marked for a 1099{}, against the year's threshold{}.\n",
                if year_end {
                    String::new()
                } else {
                    format!(" from January 1 through {as_of_s}")
                },
                if include_tins {
                    ", with each payee's TIN"
                } else {
                    ""
                }
            )
        } else {
            format!(
                "1099-worksheet-{year}-NOT-INCLUDED.txt: why the year's 1099 worksheet isn't here.\n"
            )
        },
        k1 = if company.members.is_empty() {
            String::new()
        } else if !k1_included {
            "k1-members-NOT-INCLUDED.txt: why the members' K-1 worksheet isn't here.\n".to_string()
        } else {
            let mut k = format!(
                "k1-members.csv: each member's capital account for the year (Schedule K-1 Item L: beginning, contributions in cash and in property, share of income, distributions, ending), with their {}{}.\n",
                if include_tins {
                    "taxpayer ID"
                } else {
                    "taxpayer ID's last four"
                },
                if company.members.len() > 1 {
                    "; the last row totals the members (not a K-1) to tie to Schedule L and the statement of members' equity"
                } else {
                    ""
                }
            );
            let members = &company.members;
            for (i, m) in members.iter().enumerate() {
                if let Some(j) = &m.files_jointly_with
                    && members[..i].iter().all(|o| o.name != *j)
                {
                    k.push_str(&format!(
                        "{} and {j} file a joint individual return: each receives their own Schedule K-1; both are reported on the one Form 1040 (Schedule E, Part II).\n",
                        m.name
                    ));
                }
            }
            k
        },
        tins = if include_tins {
            let mut t = String::from(
                "\nThis package holds full taxpayer identification numbers: keep it out of email and shared folders, and delete copies once filed.\n",
            );
            if !tin_gaps.is_empty() {
                t.push_str("TINs not included:\n");
                for g in &tin_gaps {
                    t.push_str(&format!("- {g}\n"));
                }
            }
            t
        } else {
            String::new()
        },
        receipts = if req.include_receipts {
            format!(
                "receipts/: the {receipt_count} receipts behind the period's entries; index.csv ties each to its entry, date, amount, and SHA-256.\n{}",
                if document_count > 0 {
                    "documents/: the company's formation, legal, and tax documents.\n"
                } else {
                    ""
                }
            )
        } else {
            String::new()
        },
    );
    files.insert(0, ("README.txt".into(), readme.into_bytes()));

    // Zip it.
    let mut zip_buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_buf));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, bytes) in &files {
            zip.start_file(name.as_str(), opts)
                .map_err(|e| e.to_string())?;
            use std::io::Write;
            zip.write_all(bytes).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
    }
    Ok(Package {
        file_name: package_file_name(as_of),
        files,
        zip: zip_buf,
        tin_gaps,
        receipt_count,
        warnings,
        iif_changes,
    })
}
