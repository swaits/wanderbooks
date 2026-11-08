use crate::*;

use crate::queries::commands::COMMANDS;
use crate::queries::formats::FORMAT_QUERIES;

/// Check one expectation against the books as they stand: a query the
/// core answers, value by value; a format query's shape, its values left
/// to the test [`FORMAT_QUERIES`] names (returned); a query no crate
/// answers is refused.
pub(crate) fn check(b: &Books<'_>, x: &Expect) -> Option<&'static str> {
    let (ex, l) = (b.ex, b.l());
    let q = x.query.as_str();
    let only = |given: bool, key: &str, queries: &[&str]| {
        assert!(
            !given || queries.contains(&q),
            "{}: {q}: {key} is for {} only",
            ex.id,
            queries.join(" and ")
        );
    };
    only(x.entry.is_some(), "entry", &["entry", "party_part"]);
    only(x.event.is_some(), "event", &["run"]);
    only(
        x.asset.is_some(),
        "asset",
        &[
            "disposal",
            "depreciation_schedule",
            "register_row",
            "asset_basis",
        ],
    );
    only(x.schedule.is_some(), "schedule", &["schedule_plan"]);
    only(
        x.party.is_some(),
        "party",
        &["party_part", "party_totals", "party_tin_last4"],
    );
    only(x.basis.is_some(), "basis", &["party_totals"]);
    only(x.payee.is_some(), "payee", &["vendor_1099"]);
    only(
        x.code.is_some(),
        "code",
        &["rename_account", "set_tax_category"],
    );
    only(x.name.is_some(), "name", &["rename_account"]);
    only(x.category.is_some(), "category", &["set_tax_category"]);
    only(x.account.is_some(), "account", &["add_account"]);
    only(x.accepted.is_some(), "accepted", &COMMANDS);
    only(
        x.refused.is_some(),
        "refused",
        &[
            "rename_account",
            "add_account",
            "set_tax_category",
            "period",
        ],
    );
    if COMMANDS.contains(&q) {
        // A command is accepted or refused, and shows no values.
        assert!(
            x.values.is_none() && (x.accepted == Some(true)) != x.refused.is_some(),
            "{}: {q} is a command: accepted = true, or refused, and no values",
            ex.id
        );
        check_command(ex, l, x);
        return None;
    }
    // Anything else shows its values; a period may be refused instead.
    assert!(
        x.refused.is_some() != x.values.as_ref().is_some_and(|v| !v.is_empty()),
        "{}: {q} expects values (a period: values or refused)",
        ex.id
    );
    match q {
        "period" => check_period(ex, x),
        "day" => check_day(ex, x),
        "calendar_year" => check_calendar_year(ex, x),
        "account_names" => check_account_names(ex, l, x),
        "uncategorized_accounts" => check_uncategorized_accounts(ex, l, x),
        "cash_flow" => check_cash_flow(b, x),
        "trial_balance" => check_trial_balance(ex, l, x),
        "profit_and_loss" => check_profit_and_loss(ex, l, x),
        "vendor_1099_paid" => check_vendor_1099_paid(b, x),
        "vendor_1099" => check_vendor_1099(b, x),
        "entry" => check_entry(b, x),
        "run" => check_run(b, x),
        "disposal" => check_disposal(b, x),
        "depreciation_schedule" => check_depreciation_schedule(b, x),
        "schedule_plan" => check_schedule_plan(b, x),
        "register_row" => check_register_row(b, x),
        "register_totals" => check_register_totals(b, x),
        "asset_basis" => check_asset_basis(b, x),
        "k1_tax_basis" => check_k1_tax_basis(b, x),
        "k1_reconciliation" => check_k1_reconciliation(b, x),
        "balances" => check_balances(ex, l, x),
        "members_equity" => check_members_equity(ex, l, &b.members, x),
        "k1" => check_k1(ex, l, &b.members, x),
        "party_part" => check_party_part(b, x),
        "party_totals" => check_party_totals(b, x),
        "party_tin_last4" => check_party_tin_last4(b, x),
        "project" => check_project(b, x),
        q => {
            let &(_, by) = FORMAT_QUERIES
                .iter()
                .find(|(f, _)| *f == q)
                .unwrap_or_else(|| panic!("{}: no crate answers query {q:?}", ex.id));
            check_format_shape(ex, x);
            return Some(by);
        }
    }
    None
}
