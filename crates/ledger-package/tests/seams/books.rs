//! The P3 seam review's books (shared by `ledger-package`'s
//! `tests/p3_seams.rs`, `ledger-gui`'s `tests/p3_seams.rs` and
//! `ledger-mcp`'s `tests/p3_seams.rs`, which include this file): one year
//! of fictional books for a two-member LLC, Ana Example and Ben Example,
//! 50/50, built through the real [`Store`], with the settings the server
//! and the viewer read written beside them (`wanderbooks.toml`). Every
//! name and amount is made up.
//!
//! What happens, by month (2026):
//! - Jan: the members contribute 7,500.00 each in cash.
//! - Feb: Ana contributes her personal laptop (value 1,800.00, her basis
//!   2,400.00: a personal-use conversion, D-41); Delta Studio LLC, a
//!   1099-NEC contractor, is paid 1,500.00 by check.
//! - Mar: Delta 700.00 on the business card; a year of insurance,
//!   1,095.00, prepaid from March 15 on a daily schedule; supplies
//!   300.25 on the card; Ana pays a 120.00 subscription on her own card
//!   (through Due to Members).
//! - Apr: an 8,000.00 bank loan; a 40.00 refund from the supplies
//!   vendor; the card paid, 1,000.25; a camera bought, 3,600.00, in
//!   service April 15; Ana reimbursed.
//! - May: Ben pays Delta 900.00 with his own check (Due to Members).
//! - Jun: a loan payment; Ben reimbursed; May's camera run reversed on
//!   June 3 (caught up by July's run); a 2,000.00 distribution declared
//!   to Distributions Payable on June 30.
//! - Jul: the distribution paid; a loan payment.
//! - Aug 1: Delta elects corporate taxation (not reportable from then).
//! - Sep: Delta paid 1,200.00 by check.
//! - Nov: November's camera run made, then the camera sold on November 20
//!   for 2,900.00 (the disposal credits back the run's last ten days).
//! - Dec: a 1,000.00 distribution paid directly.
//! - Every month: rent 450.00, revenue with odd cents, depreciation, the
//!   insurance schedule, and a close to the members.

#![allow(dead_code)]

use chrono::NaiveDate;
use ledger_core::settings::{Company, Member, Source};
use ledger_core::{
    AccountId, AccountType, AssetBasis, BasisRule, CloseTo, Currency, Dated1099, Decision1099,
    Form1099, InstrumentKind, MemberClose, Money, NewAsset, NewSchedule, PartyId, PartyShare, Role,
    ScheduleConvention, ScheduleKind, Sec704cMethod, Side,
};
use ledger_store::Store;
use rust_decimal::Decimal;
use std::str::FromStr;

pub fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

pub fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

pub fn usd(s: &str) -> Money {
    Money::new(dec(s), Currency::USD)
}

fn src(on: &str) -> Source {
    Source {
        decided_by: "Ana Example".into(),
        decided_on: d(on),
        citation: None,
        note: None,
    }
}

pub const CASH: &str = "1000";
pub const FINANCING: [&str; 3] = ["2050", "2100", "2300"];

/// The scenario's books and the ids the checks need.
pub struct Books {
    pub store: Store,
    pub company: Company,
    pub cash: Vec<String>,
    pub financing: Vec<String>,
    pub ana: PartyId,
    pub ben: PartyId,
    pub delta: u64,
    pub delta_party: PartyId,
    pub paper_party: PartyId,
    pub laptop: u64,
    pub camera: u64,
}

pub fn id(s: &Store, code: &str) -> AccountId {
    s.ledger().chart().find_by_code(code).unwrap().id
}

fn post(s: &mut Store, date: &str, desc: &str, lines: &[(&str, Side, &str)]) -> u64 {
    let e = s.create_entry(d(date), desc).unwrap();
    for (code, side, a) in lines {
        let acct = id(s, code);
        s.add_line(e, acct, *side, usd(a)).unwrap();
    }
    e
}

fn done(s: &mut Store, e: u64) -> u64 {
    s.post_entry(e).unwrap();
    e
}

fn with_party(s: &mut Store, e: u64, p: PartyId) -> u64 {
    s.set_entry_party(e, Some(p)).unwrap();
    done(s, e)
}

fn with_vendor(s: &mut Store, e: u64, v: u64) -> u64 {
    s.set_entry_vendor(e, Some(v)).unwrap();
    done(s, e)
}

fn both(s: &mut Store, e: u64, ana: PartyId, ben: PartyId, each: &str) -> u64 {
    let share = ledger_core::shares::Share::Amount(dec(each));
    s.set_entry_parties(
        e,
        &[
            PartyShare { party: ana, share },
            PartyShare { party: ben, share },
        ],
    )
    .unwrap();
    done(s, e)
}

/// The settings the server and the viewer read for these books (beside
/// `books-data`, as an older books dir keeps them): [`company`] and the
/// account roles, the receipt threshold high so no entry needs a receipt.
pub const SETTINGS_TOML: &str = "[company]
name = \"Example Trails LLC\"
basis = \"Cash\"
fiscal_year_start = \"01-01\"

[[company.members]]
name = \"Ana Example\"
share = \"50%\"
capital_account = \"3000\"
distribution_account = \"3010\"
files_jointly_with = \"Ben Example\"

[[company.members]]
name = \"Ben Example\"
share = \"50%\"
capital_account = \"3001\"
distribution_account = \"3011\"
files_jointly_with = \"Ana Example\"

[books]
receipt_threshold = \"1000000.00\"
cash_accounts = [\"1000\"]
financing_accounts = [\"2050\", \"2100\", \"2300\"]
";

/// The company as [`SETTINGS_TOML`] gives it.
pub fn company() -> Company {
    let member = |name: &str, capital: &str, draws: &str, other: &str| Member {
        name: name.into(),
        share: dec("0.5"),
        capital_account: capital.into(),
        distribution_account: Some(draws.into()),
        files_jointly_with: Some(other.into()),
    };
    Company {
        name: "Example Trails LLC".into(),
        basis: "Cash".into(),
        members: vec![
            member("Ana Example", "3000", "3010", "Ben Example"),
            member("Ben Example", "3001", "3011", "Ana Example"),
        ],
        ..Company::default()
    }
}

pub const MONTHS: [&str; 12] = [
    "2026-01", "2026-02", "2026-03", "2026-04", "2026-05", "2026-06", "2026-07", "2026-08",
    "2026-09", "2026-10", "2026-11", "2026-12",
];

pub fn month_end(m: &str) -> NaiveDate {
    let first = d(&format!("{m}-01"));
    let next = first.checked_add_months(chrono::Months::new(1)).unwrap();
    next.pred_opt().unwrap()
}

pub fn books(dir: &std::path::Path) -> Books {
    let mut s = Store::create(&dir.join("books-data"), Currency::USD).unwrap();
    for (code, name, ty) in [
        ("1000", "Checking", AccountType::Asset),
        ("1300", "Prepaid Insurance", AccountType::Asset),
        ("1500", "Camera", AccountType::Asset),
        (
            "1510",
            "Accumulated Depreciation - Camera",
            AccountType::Asset,
        ),
        ("1520", "Laptop", AccountType::Asset),
        (
            "1530",
            "Accumulated Depreciation - Laptop",
            AccountType::Asset,
        ),
        ("2050", "Business Card", AccountType::Liability),
        ("2100", "Due to Members", AccountType::Liability),
        ("2200", "Distributions Payable", AccountType::Liability),
        ("2300", "Bank Loan", AccountType::Liability),
        ("3000", "Capital - Ana", AccountType::Equity),
        ("3001", "Capital - Ben", AccountType::Equity),
        ("3010", "Distributions - Ana", AccountType::Equity),
        ("3011", "Distributions - Ben", AccountType::Equity),
        ("4000", "Guiding Revenue", AccountType::Revenue),
        ("6000", "Rent", AccountType::Expense),
        ("6150", "Software", AccountType::Expense),
        ("6200", "Supplies", AccountType::Expense),
        ("6300", "Insurance", AccountType::Expense),
        ("6400", "Contract Labor", AccountType::Expense),
        ("6500", "Depreciation", AccountType::Expense),
        ("6900", "Interest Expense", AccountType::Expense),
        ("7000", "Gain or Loss on Disposal", AccountType::Revenue),
    ] {
        s.add_account(code, name, ty, None, false).unwrap();
    }
    let ana = s.add_party("Ana Example", &[Role::Member]).unwrap();
    let ben = s.add_party("Ben Example", &[Role::Member]).unwrap();
    for m in [ana, ben] {
        s.set_party_creditor(m, id(&s, "2100"), true).unwrap();
    }
    let client = s.add_customer("Summit Client Co").unwrap();
    let client = s.ledger().party_of_customer(client).unwrap();
    let lender = s
        .add_party("Example Savings Bank", &[Role::FinancialInstitution])
        .unwrap();
    s.set_party_creditor(lender, id(&s, "2300"), true).unwrap();
    let delta = s.add_vendor("Delta Studio LLC").unwrap();
    let delta_party = s.ledger().party_of_vendor(delta).unwrap();
    // Delta files a 1099-NEC, then elects corporate taxation effective
    // August 1 (Form 8832): not reportable from that day (D-40).
    s.set_vendor_1099(
        delta,
        &Dated1099::from_beginning(Decision1099::Files(Form1099::Nec), Some(src("2026-01-02"))),
        Some(true),
    )
    .unwrap();
    s.set_vendor_1099(
        delta,
        &Dated1099 {
            effective: Some(d("2026-08-01")),
            decision: Decision1099::not_reportable(
                ledger_core::ExemptReason::Corporation,
                "elected corporate taxation, Form 8832 effective 2026-08-01",
            )
            .unwrap(),
            source: Some(src("2026-08-03")),
        },
        Some(true),
    )
    .unwrap();
    let paper = s.add_vendor("Paper Goods Co").unwrap();
    let paper_party = s.ledger().party_of_vendor(paper).unwrap();
    let cloud = s.add_vendor("Cloud Tools Inc").unwrap();
    let lens = s.add_vendor("Lens Shop Inc").unwrap();
    let insurer = s.add_vendor("Safe Insurance Inc").unwrap();
    let landlord = s.add_vendor("Trailhead Rentals LLC").unwrap();

    let card = s
        .add_instrument(
            InstrumentKind::BusinessCard,
            "Business card",
            Some("4242"),
            None,
            id(&s, "2050"),
        )
        .unwrap();
    let ana_card = s
        .add_instrument(
            InstrumentKind::MemberCard,
            "Ana's personal card",
            Some("1111"),
            Some(ana),
            id(&s, "2100"),
        )
        .unwrap();
    let ben_check = s
        .add_instrument(
            InstrumentKind::MemberCashOrCheck,
            "Ben's personal checks",
            None,
            Some(ben),
            id(&s, "2100"),
        )
        .unwrap();

    let members: Vec<MemberClose> = company()
        .members
        .iter()
        .map(|m| MemberClose {
            capital: id(&s, &m.capital_account),
            share: m.share,
            distributions: m.distribution_account.as_deref().map(|c| id(&s, c)),
        })
        .collect();

    let mut laptop = 0;
    let mut camera = 0;
    // Odd-cent revenue so the closes split odd cents.
    let revenue = [
        "2100.01", "1850.37", "2433.33", "2950.05", "3100.11", "3333.33", "2875.99", "2600.01",
        "2999.97", "3050.03", "2200.21", "1999.99",
    ];
    for (i, m) in MONTHS.iter().enumerate() {
        let end = month_end(m);
        let day = |n: u32| format!("{m}-{n:02}");
        match i {
            0 => {
                let e = post(
                    &mut s,
                    &day(5),
                    "Members' opening contributions",
                    &[
                        ("1000", Side::Debit, "15000.00"),
                        ("3000", Side::Credit, "7500.00"),
                        ("3001", Side::Credit, "7500.00"),
                    ],
                );
                both(&mut s, e, ana, ben, "7500.00");
            }
            1 => {
                // Ana contributes the laptop she used personally (D-41).
                let e = post(
                    &mut s,
                    &day(1),
                    "Ana contributes her laptop",
                    &[
                        ("1520", Side::Debit, "1800.00"),
                        ("3000", Side::Credit, "1800.00"),
                    ],
                );
                let e = with_party(&mut s, e, ana);
                laptop = s
                    .register_asset(NewAsset {
                        description: "Laptop".into(),
                        in_service_date: d("2026-02-01"),
                        cost: usd("1800.00"),
                        salvage_value: usd("0.00"),
                        useful_life_months: 36,
                        cost_account: id(&s, "1520"),
                        accum_depr_account: id(&s, "1530"),
                        expense_account: id(&s, "6500"),
                    })
                    .unwrap();
                s.set_asset_acquisition(laptop, e).unwrap();
                s.set_asset_basis(
                    laptop,
                    AssetBasis {
                        rule: BasisRule::PersonalUseConversion,
                        contributor_adjusted_basis: Some(dec("2400.00")),
                        fmv_at_contribution: Some(dec("1800.00")),
                        sec704c_method: Some(Sec704cMethod::Traditional),
                        source: Some(src("2026-02-01")),
                    },
                )
                .unwrap();
                // Delta paid by check from the bank: no method recorded,
                // presumed a check (D-40).
                let e = post(
                    &mut s,
                    &day(10),
                    "Delta Studio: trail maps",
                    &[
                        ("6400", Side::Debit, "1500.00"),
                        ("1000", Side::Credit, "1500.00"),
                    ],
                );
                with_vendor(&mut s, e, delta);
            }
            2 => {
                let e = post(
                    &mut s,
                    &day(12),
                    "Delta Studio: logo work, on the card",
                    &[
                        ("6400", Side::Debit, "700.00"),
                        ("2050", Side::Credit, "700.00"),
                    ],
                );
                s.set_entry_instrument(e, Some(card)).unwrap();
                with_vendor(&mut s, e, delta);
                let e = post(
                    &mut s,
                    &day(15),
                    "Liability insurance, a year from March 15",
                    &[
                        ("1300", Side::Debit, "1095.00"),
                        ("1000", Side::Credit, "1095.00"),
                    ],
                );
                with_vendor(&mut s, e, insurer);
                s.register_schedule(NewSchedule {
                    description: "Liability insurance".into(),
                    kind: ScheduleKind::PrepaidExpense,
                    total: usd("1095.00"),
                    start: d("2026-03-15"),
                    months: 12,
                    balance_account: id(&s, "1300"),
                    pl_account: id(&s, "6300"),
                    convention: ScheduleConvention::Daily,
                })
                .unwrap();
                let e = post(
                    &mut s,
                    &day(20),
                    "Paper Goods: maps paper, on the card",
                    &[
                        ("6200", Side::Debit, "300.25"),
                        ("2050", Side::Credit, "300.25"),
                    ],
                );
                s.set_entry_instrument(e, Some(card)).unwrap();
                with_vendor(&mut s, e, paper);
                // Ana pays a software subscription on her own card.
                let e = post(
                    &mut s,
                    &day(22),
                    "Cloud Tools: route planner, Ana's card",
                    &[
                        ("6150", Side::Debit, "120.00"),
                        ("2100", Side::Credit, "120.00"),
                    ],
                );
                s.set_entry_instrument(e, Some(ana_card)).unwrap();
                with_vendor(&mut s, e, cloud);
            }
            3 => {
                let e = post(
                    &mut s,
                    &day(1),
                    "Equipment loan",
                    &[
                        ("1000", Side::Debit, "8000.00"),
                        ("2300", Side::Credit, "8000.00"),
                    ],
                );
                with_party(&mut s, e, lender);
                let e = post(
                    &mut s,
                    &day(5),
                    "Paper Goods refund: damaged paper",
                    &[
                        ("1000", Side::Debit, "40.00"),
                        ("6200", Side::Credit, "40.00"),
                    ],
                );
                with_vendor(&mut s, e, paper);
                let e = post(
                    &mut s,
                    &day(8),
                    "Card payment",
                    &[
                        ("2050", Side::Debit, "1000.25"),
                        ("1000", Side::Credit, "1000.25"),
                    ],
                );
                s.set_entry_no_party(e, "paying the company's own card")
                    .unwrap();
                done(&mut s, e);
                let e = post(
                    &mut s,
                    &day(10),
                    "Lens Shop: trail camera",
                    &[
                        ("1500", Side::Debit, "3600.00"),
                        ("1000", Side::Credit, "3600.00"),
                    ],
                );
                let e = with_vendor(&mut s, e, lens);
                camera = s
                    .register_asset(NewAsset {
                        description: "Trail camera".into(),
                        in_service_date: d("2026-04-15"),
                        cost: usd("3600.00"),
                        salvage_value: usd("0.00"),
                        useful_life_months: 48,
                        cost_account: id(&s, "1500"),
                        accum_depr_account: id(&s, "1510"),
                        expense_account: id(&s, "6500"),
                    })
                    .unwrap();
                s.set_asset_acquisition(camera, e).unwrap();
                s.set_asset_basis(
                    camera,
                    AssetBasis {
                        rule: BasisRule::Purchase,
                        contributor_adjusted_basis: None,
                        fmv_at_contribution: None,
                        sec704c_method: None,
                        source: Some(src("2026-04-10")),
                    },
                )
                .unwrap();
                let e = post(
                    &mut s,
                    &day(25),
                    "Ana reimbursed for the route planner",
                    &[
                        ("2100", Side::Debit, "120.00"),
                        ("1000", Side::Credit, "120.00"),
                    ],
                );
                with_party(&mut s, e, ana);
            }
            4 => {
                // Ben pays Delta with his own check, at the company's
                // direction: the company is the payor (B-19).
                let e = post(
                    &mut s,
                    &day(20),
                    "Delta Studio: guidebook layout, Ben's check",
                    &[
                        ("6400", Side::Debit, "900.00"),
                        ("2100", Side::Credit, "900.00"),
                    ],
                );
                s.set_entry_instrument(e, Some(ben_check)).unwrap();
                with_vendor(&mut s, e, delta);
            }
            5 => {
                let e = post(
                    &mut s,
                    &day(1),
                    "Loan payment",
                    &[
                        ("2300", Side::Debit, "300.00"),
                        ("6900", Side::Debit, "40.00"),
                        ("1000", Side::Credit, "340.00"),
                    ],
                );
                with_party(&mut s, e, lender);
                let e = post(
                    &mut s,
                    &day(15),
                    "Ben reimbursed for the guidebook layout",
                    &[
                        ("2100", Side::Debit, "900.00"),
                        ("1000", Side::Credit, "900.00"),
                    ],
                );
                with_party(&mut s, e, ben);
                let e = post(
                    &mut s,
                    &day(30),
                    "Distribution declared",
                    &[
                        ("3010", Side::Debit, "1000.00"),
                        ("3011", Side::Debit, "1000.00"),
                        ("2200", Side::Credit, "2000.00"),
                    ],
                );
                both(&mut s, e, ana, ben, "1000.00");
            }
            6 => {
                let e = post(
                    &mut s,
                    &day(10),
                    "Distribution paid",
                    &[
                        ("2200", Side::Debit, "2000.00"),
                        ("1000", Side::Credit, "2000.00"),
                    ],
                );
                both(&mut s, e, ana, ben, "1000.00");
                let e = post(
                    &mut s,
                    &day(1),
                    "Loan payment",
                    &[
                        ("2300", Side::Debit, "302.00"),
                        ("6900", Side::Debit, "38.00"),
                        ("1000", Side::Credit, "340.00"),
                    ],
                );
                with_party(&mut s, e, lender);
            }
            8 => {
                // After Delta's election: not reportable (D-40).
                let e = post(
                    &mut s,
                    &day(10),
                    "Delta Studio: signage",
                    &[
                        ("6400", Side::Debit, "1200.00"),
                        ("1000", Side::Credit, "1200.00"),
                    ],
                );
                with_vendor(&mut s, e, delta);
            }
            10 => {
                // November's run is made before the camera's sale is
                // recorded: the disposal takes back the days after it.
                s.run_depreciation(camera, m).unwrap();
                let e = s
                    .dispose_asset(
                        camera,
                        d("2026-11-20"),
                        usd("2900.00"),
                        id(&s, "1000"),
                        id(&s, "7000"),
                    )
                    .unwrap();
                let buyer = lens_buyer(&mut s);
                s.set_entry_party(e, Some(buyer)).unwrap();
            }
            11 => {
                let e = post(
                    &mut s,
                    &day(15),
                    "Year-end distribution",
                    &[
                        ("3010", Side::Debit, "500.00"),
                        ("3011", Side::Debit, "500.00"),
                        ("1000", Side::Credit, "1000.00"),
                    ],
                );
                both(&mut s, e, ana, ben, "500.00");
            }
            _ => {}
        }
        let e = post(
            &mut s,
            &day(3),
            "Trail office rent",
            &[
                ("6000", Side::Debit, "450.00"),
                ("1000", Side::Credit, "450.00"),
            ],
        );
        with_vendor(&mut s, e, landlord);
        let e = post(
            &mut s,
            &end.to_string(),
            "Guided hikes",
            &[
                ("1000", Side::Debit, revenue[i]),
                ("4000", Side::Credit, revenue[i]),
            ],
        );
        with_party(&mut s, e, client);
        // Month end: depreciation, the schedules, the close.
        for asset in [laptop, camera] {
            if asset == 0 {
                continue;
            }
            let a = s.ledger().get_asset(asset).unwrap().clone();
            if matches!(a.status, ledger_core::AssetStatus::Disposed { .. }) {
                continue;
            }
            if s.ledger().live_depreciation_run(asset, m).is_none() {
                s.run_depreciation(asset, m).unwrap();
            }
        }
        if i == 5 {
            let may = s
                .ledger()
                .live_depreciation_run(camera, "2026-05")
                .unwrap()
                .entry;
            s.reverse_entry(
                may,
                d("2026-06-03"),
                "May's camera run reversed: wrong asset",
            )
            .unwrap();
            // June's run (made above) came first; reversing May leaves
            // May's depreciation for the next run to catch up.
        }
        s.run_schedules(end).unwrap();
        let p = s.create_period(m, d(&format!("{m}-01")), end).unwrap();
        s.close_period_to(p, &CloseTo::Members(members.clone()))
            .unwrap();
    }
    std::fs::write(dir.join("wanderbooks.toml"), SETTINGS_TOML).unwrap();
    Books {
        store: s,
        company: company(),
        cash: vec![CASH.to_string()],
        financing: FINANCING.iter().map(|c| c.to_string()).collect(),
        ana,
        ben,
        delta,
        delta_party,
        paper_party,
        laptop,
        camera,
    }
}

fn lens_buyer(s: &mut Store) -> PartyId {
    let c = s.add_customer("Ridge Outfitters Inc").unwrap();
    s.ledger().party_of_customer(c).unwrap()
}
