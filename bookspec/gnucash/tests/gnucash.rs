//! GnuCash, run through bookspec's examples (`just oracle` builds the
//! container). Needs Docker, so the test is ignored by default:
//! `cargo test -p bookspec-gnucash -- --ignored --nocapture`.

use std::path::{Path, PathBuf};

use bookspec_checker::{Example, rules};
use bookspec_gnucash::{GnuCash, claimed};

fn examples() -> Vec<Example> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|f| Example::from_toml(&std::fs::read_to_string(f).unwrap()).unwrap())
        .collect()
}

/// The differences GnuCash shows, each settled from the sources
/// (`bookspec/README.md`, "GnuCash, the second engine"). A new difference,
/// or one that goes away, fails this test until it is adjudicated again.
const ADJUDICATED: [(&str, &str); 3] = [
    (
        "entry-an-unbalanced-entry-is-refused",
        "event 2: accepted, expected refused: unbalanced",
    ),
    (
        "entry-a-line-amount-is-positive",
        "event 1: accepted, expected refused: negative_amount",
    ),
    (
        "ledger-a-fraction-of-a-cent-is-refused",
        "event 1: accepted, expected refused: finer_than_minor_unit",
    ),
];

fn run_claimed() -> bookspec_runner::Report {
    let claimed = claimed();
    let mut wanted = std::collections::BTreeSet::new();
    for r in rules().iter().filter(|r| claimed.contains(&r.id)) {
        wanted.extend(r.examples.iter().copied());
    }
    let examples: Vec<Example> = examples()
        .into_iter()
        .filter(|e| wanted.contains(e.id.as_str()))
        .collect();
    bookspec_runner::run(&examples, rules(), |_| GnuCash::new())
}

#[test]
#[ignore = "needs Docker and the GnuCash image: `just oracle`"]
fn gnucash_differs_only_where_it_was_adjudicated() {
    let report = run_claimed();
    let found: Vec<(&str, &str)> = report
        .examples
        .iter()
        .filter_map(|e| e.failure.as_deref().map(|f| (e.id.as_str(), f)))
        .collect();
    let mut want: Vec<(&str, &str)> = ADJUDICATED.to_vec();
    want.sort();
    let mut got = found.clone();
    got.sort();
    assert_eq!(got, want, "GnuCash's differences changed: adjudicate again");
    // The rules that every example passes are passed.
    for rule in [
        "BS-entry-3",
        "BS-ledger-3",
        "BS-statement-1",
        "BS-statement-2",
    ] {
        let id = bookspec_checker::RuleId::parse(rule).unwrap();
        assert_eq!(report.rules[&id], bookspec_runner::Outcome::Pass, "{rule}");
    }
}

/// The evidence the adjudications rest on, from GnuCash's own answers:
/// what it did with the entry it accepted.
#[test]
#[ignore = "needs Docker and the GnuCash image: `just oracle`"]
fn what_gnucash_did_with_the_entries_the_specification_refuses() {
    use bookspec_checker::Engine;
    use std::str::FromStr;
    let by_id = |id: &str| examples().into_iter().find(|e| e.id == id).unwrap();
    let net = |id: &str, event: usize| -> rust_decimal::Decimal {
        let ex = by_id(id);
        let mut g = GnuCash::new();
        g.load(&ex.setup).unwrap();
        for e in ex.events.iter().take(event) {
            g.apply(e).expect("GnuCash accepts it");
        }
        let q = ex
            .expect
            .iter()
            .find(|x| x.query == "trial_balance")
            .unwrap()
            .query();
        let answer = g.query(&q).unwrap();
        answer
            .as_object()
            .unwrap()
            .values()
            .map(|v| rust_decimal::Decimal::from_str(v.as_str().unwrap()).unwrap())
            .sum()
    };
    // Debits 100.00 against credits 90.00: the accounts GnuCash lists no
    // longer net to zero; ten dollars sit where GnuCash parked them, in an
    // Imbalance account of its own making.
    assert_eq!(
        net("entry-an-unbalanced-entry-is-refused", 2).to_string(),
        "10.00"
    );
    // A credit of minus 100.00 is booked as a debit of 100.00: both lines
    // are debits, and 200.00 sits in the Imbalance account.
    assert_eq!(
        net("entry-a-line-amount-is-positive", 1).to_string(),
        "200.00"
    );
    // 10.005 on each side is rounded to the cent, silently: GnuCash's USD
    // balance reads 10.01 (the choice rules.md B-3 records the owner took
    // the other way, after the first cross-check found it).
    let ex = by_id("ledger-a-fraction-of-a-cent-is-refused");
    let mut g = GnuCash::new();
    g.load(&ex.setup).unwrap();
    g.apply(&ex.events[0]).unwrap();
    let q = ex
        .expect
        .iter()
        .find(|x| x.query == "trial_balance")
        .unwrap()
        .query();
    assert_eq!(g.query(&q).unwrap()["1000"], "10.01");
}
