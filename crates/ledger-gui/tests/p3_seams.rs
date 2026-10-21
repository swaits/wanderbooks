//! P3's seams in the viewer: on the seam review's books (a year of a
//! two-member LLC, `ledger-package`'s `tests/seams/books.rs`), the
//! viewer's figures, read through its own [`Books`] from the books' folder
//! and settings file as it opens them, are the engine's (the server's
//! tools make theirs with the same calls and settings): each member's
//! capital statement, party money, the cash flow statement, the fixed
//! asset register and the 1099 worksheet, on several dates.

#[path = "../../ledger-package/tests/seams/books.rs"]
mod books;

use books::{books, d};
use chrono::NaiveDate;
use ledger_gui::books::Books as Viewer;
use ledger_gui::{Source, data};

const START: &str = "2026-01-01";

fn viewer(dir: &std::path::Path, today: NaiveDate) -> Viewer {
    data::load_once(&Source {
        data_dir: dir.join("books-data"),
        today: Some(today),
    })
    .unwrap()
}

#[test]
fn the_viewers_figures_are_the_engines_on_every_date() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let l = b.store.ledger();
    let cash: Vec<_> = b
        .cash
        .iter()
        .map(|c| l.chart().find_by_code(c).unwrap().id)
        .collect();
    let financing: Vec<_> = b
        .financing
        .iter()
        .map(|c| l.chart().find_by_code(c).unwrap().id)
        .collect();
    let settings = ledger_package::Settings {
        company: &b.company,
        cash_accounts: &b.cash,
        financing_accounts: &b.financing,
        investing_accounts: &[],
    };
    let capitals = ledger_package::member_capitals(l, &b.company.members).unwrap();
    for end in ["2026-06-30", "2026-09-15", "2026-12-31"].map(d) {
        let v = viewer(dir.path(), end);
        let span = (d(START), end);
        // The settings the viewer read are the books'.
        assert_eq!(v.money_cash_accounts, cash);
        assert_eq!(v.financing_accounts, financing);
        // Each member's capital statement is the K-1 worksheet's row.
        let k1 = ledger_core::k1::k1_worksheet(l, span.0, span.1, &capitals, &cash).unwrap();
        for (i, party) in [b.ana, b.ben].into_iter().enumerate() {
            let c = v.capital_statement(party, span).unwrap();
            let k = &k1[i];
            assert_eq!(
                (
                    c.opening,
                    c.contributions_cash,
                    c.contributions_property,
                    c.income_share,
                    c.distributions_taken,
                    c.other,
                    c.closing
                ),
                (
                    k.beginning,
                    k.contributions_cash,
                    k.contributions_property,
                    k.net_income,
                    k.distributions,
                    k.other,
                    k.ending
                ),
                "{end}"
            );
        }
        // Party money is the engine's for every party.
        let money = ledger_core::parties::money_accounts(l, &cash, &financing);
        let members = b.company.party_members(l);
        for p in l.parties() {
            assert_eq!(
                v.activity(p.id, span),
                ledger_core::parties::activity_with(l, p.id, span, &money, &members),
                "{} {end}",
                p.name
            );
        }
        // The cash flow statement the viewer lays out is the tools'.
        let tools = ledger_package::cash_flow_for(
            l,
            &settings,
            Some(START),
            Some(&end.to_string()),
            &[],
            &[],
            &[],
            end,
        )
        .unwrap();
        let viewers = ledger_package::cash_flow_for(
            &v.ledger,
            &ledger_package::Settings {
                company: &v.settings.company,
                cash_accounts: &v.settings.cash_accounts,
                financing_accounts: &v.settings.financing_accounts,
                investing_accounts: &v.settings.investing_accounts,
            },
            Some(START),
            Some(&end.to_string()),
            &[],
            &[],
            &[],
            v.today,
        )
        .unwrap();
        assert_eq!(viewers, tools, "{end}");
        // The register, and the 1099 worksheet as of the day.
        assert_eq!(
            v.asset_register(end).unwrap(),
            ledger_core::fixed_asset_register(l, end).unwrap()
        );
        let year = ledger_package::vendor_1099_report(l, &settings, 2026, None, None).unwrap();
        assert_eq!(v.vendor_1099(2026).unwrap(), year, "{end}");
    }
}
