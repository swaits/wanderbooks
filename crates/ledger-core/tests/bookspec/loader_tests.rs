use crate::*;

/// An inline example: two contributions, then `reverses` on the third
/// event.
pub(crate) fn reversing(reverses: usize) -> Example {
    toml::from_str(&format!(
        r#"
id = "loader.reverses"
title = "reverses counts events from 1"
rule = "An event's reverses names the event it reverses, counting from 1."
status = "CONVENTION"
sources = ["bookspec/README.md"]
tags = ["loader"]

[setup]
currency = "USD"
fiscal_year_start = "01-01"
basis = "accrual"
rounding = "half-even"

[[setup.accounts]]
code = "1000"
name = "Cash"
type = "asset"

[[setup.accounts]]
code = "3000"
name = "Capital"
type = "equity"

[[events]]
date = "2026-01-02"
description = "First"
lines = [
  {{ account = "1000", side = "debit", amount = "100.00" }},
  {{ account = "3000", side = "credit", amount = "100.00" }},
]

[[events]]
date = "2026-01-03"
description = "Second"
lines = [
  {{ account = "1000", side = "debit", amount = "250.00" }},
  {{ account = "3000", side = "credit", amount = "250.00" }},
]

[[events]]
date = "2026-01-04"
description = "Reverse the second"
reverses = {reverses}

[[expect]]
query = "none"
[expect.values]
none = "0"
"#
    ))
    .unwrap()
}

/// bookspec/README.md: `reverses = N` is the Nth event (entry #N),
/// counting from 1. `reverses = 2` reverses the second event, 250.00, not
/// the first (as counting from 0 would read it) nor itself.
#[test]
pub(crate) fn reverses_counts_events_from_one() {
    let ex = reversing(2);
    let b = load(&ex);
    let r = b.l().get_entry(3).unwrap();
    assert_eq!(r.reversal_of(), Some(2));
    let amounts: Vec<Decimal> = r.lines().iter().map(|x| x.amount.amount).collect();
    assert_eq!(amounts, [Decimal::new(25000, 2), Decimal::new(25000, 2)]);
    assert_eq!(r.lines()[0].side, Side::Credit);
}

/// `reverses = 0` names no event: refused, saying events count from 1.
#[test]
#[should_panic(expected = "events count from 1")]
pub(crate) fn reverses_zero_is_refused() {
    load(&reversing(0));
}

/// A key the shape doesn't have (a typo for a setting) is refused, never
/// ignored: an example can't silently run with other roles than it says.
#[test]
pub(crate) fn an_unknown_key_is_refused() {
    let text = example_text("cash-flow.reversed-property-contribution.toml")
        .replace("cash_accounts = ", "cash_account = ");
    assert!(toml::from_str::<Example>(&text).is_err());
}

/// The example `file`'s text, changed by `edit`, loaded and its
/// expectations checked as `every_bookspec_example_holds` checks them.
pub(crate) fn run_edited(file: &str, edit: impl Fn(String) -> String) {
    let before = example_text(file).to_string();
    let text = edit(before.clone());
    assert_ne!(text, before, "the edit changed nothing");
    let ex: Example = toml::from_str(&text).unwrap();
    run(&ex);
}

/// A query no crate answers is refused, never skipped: its values would
/// otherwise go unchecked.
#[test]
#[should_panic(expected = "no crate answers query")]
pub(crate) fn an_unknown_query_is_refused() {
    run_edited("as-of-package-stops-at-as-of.toml", |t| {
        t.replacen("query = \"trial_balance\"", "query = \"trial_balances\"", 1)
    });
}

/// A format's query belongs to the formats appendix: an example without
/// `appendix = "formats"` can't ask it.
#[test]
#[should_panic(expected = "appendix \"formats\"")]
pub(crate) fn a_format_query_outside_the_formats_appendix_is_refused() {
    run_edited("iif-quickbooks-journal-file.toml", |t| {
        t.replacen("appendix = \"formats\"\n", "", 1)
    });
}

/// The appendix is "formats" or absent.
#[test]
#[should_panic(expected = "appendix Some(\"format\")")]
pub(crate) fn an_unknown_appendix_is_refused() {
    run_edited("iif-quickbooks-journal-file.toml", |t| {
        t.replacen("appendix = \"formats\"", "appendix = \"format\"", 1)
    });
}

/// An event's payee must be one of the setup's payees.
#[test]
#[should_panic(expected = "not in setup")]
pub(crate) fn a_payee_not_in_the_setup_is_refused() {
    run_edited("1099-worksheet-through-as-of.toml", |t| {
        t.replacen(
            "payee = \"Contractor A\"\nlines",
            "payee = \"Contractor B\"\nlines",
            1,
        )
    });
}

/// A figure query lists everything it covers: a trial balance leaving an
/// account out is refused, so no balance goes unchecked.
#[test]
#[should_panic(expected = "lists exactly what it covers")]
pub(crate) fn a_trial_balance_leaving_an_account_out_is_refused() {
    run_edited("as-of-package-stops-at-as-of.toml", |t| {
        t.replacen(", \"6000\" = \"1200.00\" }", " }", 1)
    });
}

/// A trial balance is as of a day, a profit and loss over a period: the
/// wrong dates are refused, never read some other way.
#[test]
#[should_panic(expected = "is as of a day")]
pub(crate) fn a_trial_balance_over_a_period_is_refused() {
    run_edited("as-of-package-stops-at-as-of.toml", |t| {
        t.replacen(
            "query = \"trial_balance\"\nas_of = \"2026-06-30\"",
            "query = \"trial_balance\"\nfrom = \"2026-01-01\"\nto = \"2026-06-30\"",
            1,
        )
    });
}

/// An `entry` query names every line of the entry: one left out is
/// refused, so the entry is exactly what is expected.
#[test]
#[should_panic(expected = "lists exactly what it covers")]
pub(crate) fn an_entry_leaving_a_line_out_is_refused() {
    run_edited("members-close.true-up-of-a-per-close-split.toml", |t| {
        t.replacen("\"3000.debit\" = \"0.01\"\n", "", 1)
    });
}

/// `after = N` checks the books after event N: the statement to April 30
/// asked before April's close (A 0.03, B 0.01) fails when asked after it
/// (A 0.02, B 0.02).
#[test]
#[should_panic(expected = "members' equity [2026-01-01 .. 2026-04-30]: A.ending")]
pub(crate) fn after_is_the_books_after_that_event() {
    run_edited("members-close.true-up-of-a-per-close-split.toml", |t| {
        t.replacen("after = 7\n", "", 1)
    });
}

/// `after` counts events from 1 up to the last.
#[test]
#[should_panic(expected = "events count from 1")]
pub(crate) fn after_past_the_last_event_is_refused() {
    run_edited("members-close.true-up-of-a-per-close-split.toml", |t| {
        t.replacen("after = 6\n", "after = 9\n", 1)
    });
}

/// A close to the members needs every member's share, and the shares sum
/// to 1.
#[test]
#[should_panic(expected = "the shares sum to 1")]
pub(crate) fn shares_that_dont_sum_to_one_are_refused() {
    run_edited("members-close.true-up-of-a-per-close-split.toml", |t| {
        t.replacen("share = \"0.5\"", "share = \"0.4\"", 1)
    });
}

/// The rounding an example declares is the engine's for what it asks: a
/// members close splits by largest remainder.
#[test]
#[should_panic(expected = "rounds by largest-remainder")]
pub(crate) fn a_members_close_declared_with_another_rounding_is_refused() {
    run_edited("members-close.true-up-of-a-per-close-split.toml", |t| {
        t.replacen(
            "rounding = \"largest-remainder\"",
            "rounding = \"half-even\"",
            1,
        )
    });
}

/// A period's dates are the rule's (BS-period-3): an example saying the
/// fiscal year to date starts on January 1 under a July fiscal year fails.
#[test]
#[should_panic(expected = "period from")]
pub(crate) fn a_period_other_than_the_rules_is_refused() {
    run_edited("default-period-fiscal-year-to-date.toml", |t| {
        t.replacen(
            "values = { from = \"2026-07-01\", to = \"2026-10-20\" }",
            "values = { from = \"2026-01-01\", to = \"2026-10-20\" }",
            1,
        )
    });
}

/// A refusal is checked for its reason: a start after today is not "the
/// end before the start".
#[test]
#[should_panic(expected = "period refused")]
pub(crate) fn a_refusal_for_another_reason_is_refused() {
    run_edited("default-period-fiscal-year-to-date.toml", |t| {
        t.replacen(
            "refused = \"start_after_today\"",
            "refused = \"end_before_start\"",
            1,
        )
    });
}

/// A query that omits a date needs `setup.today`: without it, nothing
/// says what the omitted date means.
#[test]
#[should_panic(expected = "setup.today says what it means")]
pub(crate) fn an_omitted_date_without_today_is_refused() {
    run_edited("default-period-fiscal-year-to-date.toml", |t| {
        t.replacen("today = \"2026-10-20\"\n", "", 1)
    });
}

/// `account_names` lists every account: one left out is refused.
#[test]
#[should_panic(expected = "lists exactly what it covers")]
pub(crate) fn account_names_leaving_an_account_out_is_refused() {
    run_edited("rename-changes-no-amount.toml", |t| {
        t.replacen(", \"6160\" = \"Hosting\" }", " }", 1)
    });
}

/// A change is made after the event it names, once that event's
/// expectations are checked: asked after the rename, the old name fails.
#[test]
#[should_panic(expected = "account 6150's name")]
pub(crate) fn a_change_is_made_after_its_events_expectations() {
    run_edited("rename-changes-no-amount.toml", |t| {
        t.replacen(
            "\"6150\" = \"Software subscriptions\", \"6160\" = \"Hosting\" }",
            "\"6150\" = \"Software\", \"6160\" = \"Hosting\" }",
            1,
        )
    });
}

/// A command is tried under the policy in force when it is asked: without
/// the change to refuse, the account without a category is accepted.
#[test]
#[should_panic(expected = "accepted, expected refused: tax_category_required")]
pub(crate) fn a_command_runs_under_the_policy_in_force() {
    run_edited("tax-category-required.toml", |t| {
        t.replacen(
            "[[changes]]\nafter = 1\nrequire_tax_category = \"refuse\"\n",
            "",
            1,
        )
    });
}

/// A command is accepted or refused, never both, and shows no values.
#[test]
#[should_panic(expected = "is a command")]
pub(crate) fn a_command_with_values_is_refused() {
    run_edited("tax-category-required.toml", |t| {
        t.replacen(
            "category = \"other_deductions\"\naccepted = true\n",
            "category = \"other_deductions\"\naccepted = true\nvalues = { x = \"1\" }\n",
            1,
        )
    });
}

/// An event's parties must be in the setup's directory.
#[test]
#[should_panic(expected = "not in setup")]
pub(crate) fn a_party_not_in_the_setup_is_refused() {
    run_edited("p3-party-signs-and-reversals.toml", |t| {
        t.replacen(
            "parties = [{ party = \"Tobin Harlow\" }]",
            "parties = [{ party = \"Tobin Harlo\" }]",
            1,
        )
    });
}

/// A party's part is checked for its direction as well as its figures:
/// a contribution read as paid fails.
#[test]
#[should_panic(expected = "entry 2 Some(\"Tobin Harlow\"): direction")]
pub(crate) fn a_party_parts_direction_is_checked() {
    run_edited("p3-party-signs-and-reversals.toml", |t| {
        t.replacen(
            "resolved = \"-400.00\", direction = \"received\"",
            "resolved = \"-400.00\", direction = \"paid\"",
            1,
        )
    });
}

/// A reversal takes its original's parties: naming them again is
/// refused, never read as a second tag.
#[test]
#[should_panic(expected = "takes its original's parties")]
pub(crate) fn a_reversal_naming_parties_is_refused() {
    run_edited("p3-party-signs-and-reversals.toml", |t| {
        t.replacen(
            "reverses = 4\n",
            "reverses = 4\nparties = [{ party = \"Tobin Harlow\" }]\n",
            1,
        )
    });
}

/// Party totals name their basis: cash and all differ for property.
#[test]
#[should_panic(expected = "party_totals names its basis")]
pub(crate) fn party_totals_without_a_basis_are_refused() {
    run_edited("p3-party-signs-and-reversals.toml", |t| {
        t.replacen("basis = \"all\"\n", "", 1)
    });
}

/// A `vendor_1099` line lists every figure it covers: one left out is
/// refused.
#[test]
#[should_panic(expected = "lists exactly what it covers")]
pub(crate) fn a_vendor_1099_line_leaving_a_figure_out_is_refused() {
    run_edited("1099-card-payments-are-excluded.toml", |t| {
        t.replacen(", not_reportable = \"0.00\" }", " }", 1)
    });
}

/// A `vendor_1099` line's status is checked: card payments that would
/// reach the threshold leave the payee under it.
#[test]
#[should_panic(expected = "Harbor Design LLC: status")]
pub(crate) fn a_vendor_1099_status_is_checked() {
    run_edited("1099-card-payments-are-excluded.toml", |t| {
        t.replacen("status = \"under_threshold\"", "status = \"reportable\"", 1)
    });
}

/// A payment method an event names must be in the setup.
#[test]
#[should_panic(expected = "payment method \"Harbor debit\" not in setup")]
pub(crate) fn a_payment_method_not_in_the_setup_is_refused() {
    run_edited("1099-card-payments-are-excluded.toml", |t| {
        t.replacen(
            "instrument = \"Harbor debit card\"",
            "instrument = \"Harbor debit\"",
            1,
        )
    });
}

/// A value from a closed list is spelled exactly as bookspec/README.md
/// spells it: the engine's own parse forgives case and spaces, the
/// loader doesn't (a spec has one spelling).
#[test]
#[should_panic(expected = "form \"1099-nec\"")]
pub(crate) fn a_form_in_another_case_is_refused() {
    run_edited("1099-a-dated-status-applies-by-payment-date.toml", |t| {
        t.replacen("form = \"1099-NEC\"", "form = \"1099-nec\"", 1)
    });
}

#[test]
#[should_panic(expected = "kind \"Business_Card\"")]
pub(crate) fn a_payment_method_kind_in_another_case_is_refused() {
    run_edited("1099-card-payments-are-excluded.toml", |t| {
        t.replacen("kind = \"business_card\"", "kind = \"Business_Card\"", 1)
    });
}

#[test]
#[should_panic(expected = "not_reportable needs a reason_kind")]
pub(crate) fn a_reason_kind_in_another_case_is_refused() {
    run_edited("1099-a-dated-status-applies-by-payment-date.toml", |t| {
        t.replacen(
            "reason_kind = \"corporation\"",
            "reason_kind = \"Corporation\"",
            1,
        )
    });
}

/// An event the example says the engine refuses must be refused: one it
/// takes fails the example (March's run reversed on its own day is taken).
#[test]
#[should_panic(expected = "accepted, expected refused: entry_irreversible")]
pub(crate) fn an_event_the_engine_takes_is_not_refused() {
    run_edited(
        "depreciation-reversal-before-its-run-is-refused.toml",
        |t| t.replacen("date = \"2026-02-10\"", "date = \"2026-03-31\"", 1),
    );
}

/// A refused event is refused for the reason it gives, in the examples'
/// words, never another.
#[test]
#[should_panic(expected = "reverses refused")]
pub(crate) fn an_event_refused_for_another_reason_is_refused() {
    run_edited(
        "depreciation-reversal-before-its-run-is-refused.toml",
        |t| {
            t.replacen(
                "refused = \"entry_irreversible\"",
                "refused = \"depreciation_past_disposal\"",
                1,
            )
        },
    );
}

/// A refused event posts nothing, so nothing can name its entry.
#[test]
#[should_panic(expected = "event 6 posted 0 entries")]
pub(crate) fn a_refused_event_has_no_entry_to_reverse() {
    run_edited(
        "depreciation-reversal-before-its-run-is-refused.toml",
        |t| {
            t.replacen(
                "reverses = 5\n\n# Hand check",
                "reverses = 6\n\n# Hand check",
                1,
            )
        },
    );
}

/// A command dates its own entries: a run given a date is refused, never
/// read as the day it runs.
#[test]
#[should_panic(expected = "dates its own entries: no date")]
pub(crate) fn a_run_given_a_date_is_refused() {
    run_edited("depreciation-catch-up-month.toml", |t| {
        t.replacen(
            "[[events]]\nrun_depreciation",
            "[[events]]\ndate = \"2026-01-31\"\nrun_depreciation",
            1,
        )
    });
}

/// `run` asks about a run_depreciation event, nothing else.
#[test]
#[should_panic(expected = "event 2 is no run_depreciation")]
pub(crate) fn a_run_query_naming_another_event_is_refused() {
    run_edited("depreciation-catch-up-month.toml", |t| {
        t.replacen("event = 4", "event = 2", 1)
    });
}

/// A disposal lists every figure it took: one left out is refused.
#[test]
#[should_panic(expected = "lists exactly what it covers")]
pub(crate) fn a_disposal_leaving_a_figure_out_is_refused() {
    run_edited("depreciation-disposal-mid-month.toml", |t| {
        t.replacen("catch_up_depreciation = \"0.00\"\n", "", 1)
    });
}

/// The example's figures are the engine's, checked: a schedule month off
/// by a cent fails it.
#[test]
#[should_panic(expected = "schedule_plan of SUB: 2026-02-28")]
pub(crate) fn a_schedule_month_off_by_a_cent_fails() {
    run_edited("schedule-daily-deferred-revenue.toml", |t| {
        t.replacen(
            "\"2026-02-28\" = \"333.34\"",
            "\"2026-02-28\" = \"333.33\"",
            1,
        )
    });
}

/// An asset is on the register from its acquired date: a row asked for
/// before it is refused, never shown with figures.
#[test]
#[should_panic(expected = "not on the register (acquired later)")]
pub(crate) fn a_register_row_before_the_asset_was_acquired_is_refused() {
    run_edited("assets-acquired-before-in-service.toml", |t| {
        t.replacen("as_of = \"2026-03-31\"", "as_of = \"2026-03-09\"", 1)
    });
}

/// `acquired_by` names an event whose entry put the asset on the books:
/// an event that posts no entry can't.
#[test]
#[should_panic(expected = "event 2 posted 0 entries")]
pub(crate) fn an_asset_acquired_by_an_event_with_no_entry_is_refused() {
    run_edited("assets-a-reversed-run-is-not-posted.toml", |t| {
        t.replacen("acquired_by = 1", "acquired_by = 2", 1)
            .replacen(
                "[[events]]\nrun_depreciation = { asset = \"printer\", period = \"2026-01\" }",
                "[[events]]\nrun_depreciation = { asset = \"printer\", period = \"2025-12\" }\nrefused = \"invalid_depreciation_period\"",
                1,
            )
    });
}

/// A tax line the books can't know is "open", never a figure: an example
/// giving the book figure for it fails.
#[test]
#[should_panic(expected = "k1_tax_basis: Tobin Harlow.net_income")]
pub(crate) fn a_tax_line_the_books_cant_know_is_open() {
    run_edited("partnership-k1-tax-basis-capital.toml", |t| {
        t.replacen(
            "\"Tobin Harlow.net_income\" = \"open\"",
            "\"Tobin Harlow.net_income\" = \"-385.00\"",
            1,
        )
    });
}

/// A member's reconciliation is shown whole: an example leaving out a
/// cause fails, so a figure can't quietly go unchecked.
#[test]
#[should_panic(expected = "Tobin Harlow.property_distributed is missing")]
pub(crate) fn a_members_reconciliation_shows_every_cause() {
    run_edited("partnership-k1-tax-basis-capital.toml", |t| {
        t.replacen(", \"Tobin Harlow.property_distributed\" = \"0.00\"", "", 1)
    });
}

/// A project named twice in `setup.projects` is refused (names are
/// unique, BS-statement-5).
#[test]
#[should_panic(expected = "a project named")]
pub(crate) fn a_project_named_twice_is_refused() {
    run_edited("projects-net-is-its-tagged-lines.toml", |t| {
        t.replacen("name = \"Beta\"", "name = \"alpha\"", 1)
    });
}

/// A line naming a project the setup doesn't have is refused.
#[test]
#[should_panic(expected = "not in setup.projects")]
pub(crate) fn a_line_naming_an_unknown_project_is_refused() {
    run_edited("projects-net-is-its-tagged-lines.toml", |t| {
        t.replacen("project = \"Beta\" }", "project = \"Gamma\" }", 1)
    });
}

/// A project query lists every value it covers.
#[test]
#[should_panic(expected = "lists exactly what it covers")]
pub(crate) fn a_project_query_leaving_a_value_out_is_refused() {
    run_edited("projects-on-spec-costs-are-expenses.toml", |t| {
        t.replacen("unrecovered_cost = \"1460.00\"\n", "", 1)
    });
}

/// An invoice's line naming a project the setup doesn't have is refused.
#[test]
#[should_panic(expected = "project \"Gamma\" not in setup.projects")]
pub(crate) fn a_document_line_naming_an_unknown_project_is_refused() {
    run_edited(
        "projects-invoice-and-bill-lines-count-by-project.toml",
        |t| t.replacen("project = \"Harbor\" }", "project = \"Gamma\" }", 1),
    );
}

/// A document has an amount or lines, exactly one.
#[test]
#[should_panic(expected = "a document has an amount or lines, exactly one")]
pub(crate) fn a_document_with_an_amount_and_lines_is_refused() {
    run_edited(
        "projects-invoice-and-bill-lines-count-by-project.toml",
        |t| {
            t.replacen(
                "invoice = { customer = \"Cedar Client\", lines",
                "invoice = { customer = \"Cedar Client\", amount = \"1.00\", lines",
                1,
            )
        },
    );
}

/// An invoice's projects count: the example fails when a line's project
/// moves (Zeta's figures are no longer the ones worked by hand).
#[test]
#[should_panic(expected = "projects-invoice-and-bill-lines-count-by-project project \"Zeta\"")]
pub(crate) fn an_invoice_lines_project_is_what_the_project_counts() {
    run_edited(
        "projects-invoice-and-bill-lines-count-by-project.toml",
        |t| {
            t.replacen(
                "{ description = \"Revisions\", amount = \"500.00\", project = \"Zeta\" }",
                "{ description = \"Revisions\", amount = \"500.00\", project = \"Harbor\" }",
                1,
            )
        },
    );
}

/// A draft's state the engine does not hold is a failure, not a pass: the
/// example says the entry is still `building` after it posted.
#[test]
#[should_panic(expected = "state")]
pub(crate) fn a_wrong_draft_state_fails() {
    run_edited(
        "entry-a-draft-waits-on-its-evidence-and-posts-once-it-has-it.toml",
        |t| t.replacen("state = \"posted\"", "state = \"building\"", 1),
    );
}

/// An evidence decision the engine does not give is a failure.
#[test]
#[should_panic(expected = "decision")]
pub(crate) fn a_wrong_evidence_decision_fails() {
    run_edited("entry-evidence-is-decided-in-one-place.toml", |t| {
        t.replacen("decision = \"required\"", "decision = \"exempt\"", 1)
    });
}

/// A post the gate refuses is a refusal the example must say: leaving it out
/// is a failure (the engine refused an event the example takes).
#[test]
#[should_panic(expected = "post_draft")]
pub(crate) fn a_post_without_its_evidence_must_be_marked_refused() {
    run_edited(
        "entry-a-draft-waits-on-its-evidence-and-posts-once-it-has-it.toml",
        |t| t.replacen("refused = \"evidence_required\"\n", "", 1),
    );
}

/// A discard of a posted entry is refused: saying it is accepted fails.
#[test]
#[should_panic(expected = "discard_draft")]
pub(crate) fn a_discard_of_a_posted_entry_must_be_marked_refused() {
    run_edited(
        "entry-a-drafts-state-is-a-note-and-a-posted-entry-has-none.toml",
        |t| {
            t.replacen(
                "discard_draft = { entry = 1, reason = \"Posted by mistake\" }\nrefused = \"not_drafted\"",
                "discard_draft = { entry = 1, reason = \"Posted by mistake\" }",
                1,
            )
        },
    );
}

/// A close the books hold back for a draft in its period is refused and
/// changes nothing: saying it is accepted fails.
#[test]
#[should_panic(expected = "draft entries exist")]
pub(crate) fn a_close_with_a_draft_in_its_period_must_be_marked_refused() {
    run_edited(
        "period-a-draft-holds-its-period-open-until-it-is-discarded.toml",
        |t| t.replacen("refused = \"drafts_in_period\"\n", "", 1),
    );
}
