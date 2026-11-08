use crate::*;

/// Check one `period` expectation: the period a period report asked with
/// these dates covers, by the one rule (rules.md B-30), as `from` and
/// `to`; or the rule's refusal.
pub(crate) fn check_period(ex: &Example, x: &Expect) {
    match (resolved_period(ex, x), &x.refused) {
        (Ok((from, to)), None) => {
            exactly(ex, x, &["from".to_string(), "to".to_string()].into());
            let f = x.figures(&ex.id);
            assert_eq!(from, date(&f["from"], &ex.id), "{}: period from", ex.id);
            assert_eq!(to, date(&f["to"], &ex.id), "{}: period to", ex.id);
        }
        (Err(e), Some(want)) => assert_eq!(refusal(&e), *want, "{}: period refused ({e})", ex.id),
        (Err(e), None) => panic!("{}: period refused ({e})", ex.id),
        (Ok((from, to)), Some(want)) => {
            panic!("{}: period {from} .. {to}, expected refused: {want}", ex.id)
        }
    }
}

/// Check one `day` expectation: the day a report as of a day is made as
/// of (rules.md B-30), as `as_of`.
pub(crate) fn check_day(ex: &Example, x: &Expect) {
    exactly(ex, x, &["as_of".to_string()].into());
    let want = date(&x.figures(&ex.id)["as_of"], &ex.id);
    assert_eq!(day(ex, x), want, "{}: day", ex.id);
}

/// Check one `calendar_year` expectation: the year a calendar-year count
/// (1099s, party totals) asked without one counts (rules.md B-30).
pub(crate) fn check_calendar_year(ex: &Example, x: &Expect) {
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: calendar_year takes no dates",
        ex.id
    );
    exactly(ex, x, &["year".to_string()].into());
    let want: i32 = x.figures(&ex.id)["year"]
        .parse()
        .unwrap_or_else(|e| panic!("{}: calendar_year year: {e}", ex.id));
    assert_eq!(
        defaults(ex, &x.query).calendar_year(),
        want,
        "{}: calendar_year",
        ex.id
    );
}

/// Check one `account_names` expectation: each account's name, by code,
/// as the chart holds it and every report prints it (the trial balance
/// and the profit and loss, over every date: past periods too, rules.md
/// B-31); every account listed.
pub(crate) fn check_account_names(ex: &Example, l: &Ledger, x: &Expect) {
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: account_names takes no dates",
        ex.id
    );
    let keys = l.chart().all().map(|a| a.code.clone()).collect();
    exactly(ex, x, &keys);
    let (first, last) = (
        ledger_core::calendar::FIRST_DAY,
        ledger_core::calendar::LAST_DAY,
    );
    let tb = trial_balance_as_of(l, last);
    let pl = profit_and_loss_for_period(l, first, last, None)
        .unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    for (code, want) in x.figures(&ex.id) {
        let a = l.chart().find_by_code(code).unwrap();
        assert_eq!(&a.name, want, "{}: account {code}'s name", ex.id);
        let printed = tb.lines.iter().map(|t| (&t.code, &t.name)).chain(
            pl.revenue
                .iter()
                .chain(&pl.expenses)
                .map(|s| (&s.code, &s.name)),
        );
        for (c, name) in printed.filter(|(c, _)| *c == code) {
            assert_eq!(name, want, "{}: a report prints {c} as {name:?}", ex.id);
        }
    }
}

/// Check one `uncategorized_accounts` expectation: every account that
/// maps to a line of the return and has no tax category, with activity or
/// not, as `"<code> <name>"`, by code (rules.md B-32).
pub(crate) fn check_uncategorized_accounts(ex: &Example, l: &Ledger, x: &Expect) {
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: uncategorized_accounts takes no dates",
        ex.id
    );
    let Values::Lines(want) = x.values(&ex.id) else {
        panic!("{}: uncategorized_accounts expects a list", ex.id)
    };
    let got: Vec<String> = ledger_core::tax::uncategorized_accounts(l)
        .into_iter()
        .map(|a| format!("{} {}", a.code, a.name))
        .collect();
    assert_eq!(&got, want, "{}: uncategorized_accounts", ex.id);
}
