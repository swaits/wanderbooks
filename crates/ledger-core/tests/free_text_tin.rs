//! Free text never holds a tax id (BS-partner-2): the one detector free
//! text meets (`ledger_core::settings::holds_printed_tin`), the masking a
//! bank's description gets, and the engine commands that take free text.
//!
//! Oracle: the owner's decision of 2026-11-21 (docs/spec/claims/partner.md,
//! the CHOICE on BS-partner-2): free text refuses what looks like a tax id,
//! that is nine digits standing alone, an SSN's 3-2-4 and an EIN's 2-7, with
//! any dash, dot or space between the groups; it lets an invoice number, a
//! phone number, an order number, a reference of ten digits or more, an
//! amount and a date through. The fake ids are `000-00-0000` and
//! `00-0000000`; no real one appears here.

use ledger_core::settings::{
    check_free_text, count_printed_tins, holds_printed_tin, mask_printed_tins, tin_refusal,
};
use proptest::prelude::*;

/// Every separator the detector reads between a TIN's groups.
const SEPARATORS: &[&str] = &[
    "-",
    "\u{2010}",
    "\u{2011}",
    "\u{2012}",
    "\u{2013}",
    "\u{2014}",
    "\u{2015}",
    "\u{2212}",
    "\u{fe58}",
    "\u{fe63}",
    "\u{ff0d}",
    ".",
    "\u{ff0e}",
    " ",
    "  ",
    "\t",
    "\u{a0}",
    "\u{2002}",
    "\u{2003}",
    "\u{2009}",
    "\u{202f}",
    "\u{3000}",
    " - ",
    " \u{2013} ",
    "\u{a0}\u{2014}\u{a0}",
];

/// Text around a number that has no digit at its edge.
const CONTEXTS: &[(&str, &str)] = &[
    ("", ""),
    ("paid ", ""),
    ("", " paid"),
    ("SSN ", "."),
    ("(", ")"),
    ("ref: ", ", thanks"),
    ("a\u{e9}", "z"),
];

fn digits(n: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(0u8..10, n)
        .prop_map(|d| d.into_iter().map(|x| char::from(b'0' + x)).collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Every 3-2-4 grouping with any listed separator is found.
    #[test]
    fn every_ssn_grouping_is_found(
        a in digits(3), b in digits(2), c in digits(4),
        s1 in 0..SEPARATORS.len(), s2 in 0..SEPARATORS.len(), ctx in 0..CONTEXTS.len(),
    ) {
        let text = format!("{}{a}{}{b}{}{c}{}", CONTEXTS[ctx].0, SEPARATORS[s1], SEPARATORS[s2], CONTEXTS[ctx].1);
        prop_assert!(holds_printed_tin(&text), "{text:?}");
    }

    /// Every 2-7 grouping with any listed separator is found.
    #[test]
    fn every_ein_grouping_is_found(
        a in digits(2), b in digits(7), s in 0..SEPARATORS.len(), ctx in 0..CONTEXTS.len(),
    ) {
        let text = format!("{}{a}{}{b}{}", CONTEXTS[ctx].0, SEPARATORS[s], CONTEXTS[ctx].1);
        prop_assert!(holds_printed_tin(&text), "{text:?}");
    }

    /// Nine digits standing alone are found.
    #[test]
    fn nine_digits_alone_are_found(a in digits(9), ctx in 0..CONTEXTS.len()) {
        let text = format!("{}{a}{}", CONTEXTS[ctx].0, CONTEXTS[ctx].1);
        prop_assert!(holds_printed_tin(&text), "{text:?}");
    }

    /// Ten digits or more are a reference, never a tax id: one digit more
    /// before, after or inside a shape takes it out of the reading.
    #[test]
    fn ten_or_more_digits_are_a_reference(a in digits(10), extra in 0usize..6, ctx in 0..CONTEXTS.len()) {
        let tail: String = "7".repeat(extra);
        let text = format!("{}{a}{tail}{}", CONTEXTS[ctx].0, CONTEXTS[ctx].1);
        prop_assert!(!holds_printed_tin(&text), "{text:?}");
    }

    /// Masking leaves nothing for the detector to find, keeps the last four
    /// digits of each number and its length, and changes nothing else.
    #[test]
    fn masking_keeps_the_last_four_and_the_rest(
        a in digits(3), b in digits(2), c in digits(4), s1 in 0..SEPARATORS.len(),
        s2 in 0..SEPARATORS.len(), ctx in 0..CONTEXTS.len(),
    ) {
        let text = format!("{}{a}{}{b}{}{c}{}", CONTEXTS[ctx].0, SEPARATORS[s1], SEPARATORS[s2], CONTEXTS[ctx].1);
        let masked = mask_printed_tins(&text).into_owned();
        prop_assert!(!holds_printed_tin(&masked), "{masked:?}");
        prop_assert_eq!(masked.chars().count(), text.chars().count());
        prop_assert_eq!(count_printed_tins(&text), 1);
        let expect = format!("{}\u{2022}\u{2022}\u{2022}{}\u{2022}\u{2022}{}{c}{}", CONTEXTS[ctx].0, SEPARATORS[s1], SEPARATORS[s2], CONTEXTS[ctx].1);
        prop_assert_eq!(masked, expect);
    }
}

/// What must go through: the allow-list of the owner's table.
#[test]
fn the_allow_list_passes() {
    for fine in [
        // invoice numbers
        "INV-2026-11-0042",
        "2026-11-0042",
        "invoice 2026-0042",
        "INV 20261121",
        // phone numbers
        "801-555-0142",
        "(801) 555-0142",
        "+1 801 555 0142",
        "+44 20 7946 0958",
        // order numbers
        "ORD-48213-7",
        "order 4821-3377-12",
        "PO 12-345",
        // references of ten digits or more
        "1234567890",
        "REF 202611210042",
        "000000000000",
        "12345678901234567890",
        // amounts
        "$1,234,567.89",
        "1,234,567",
        "$12,345,678.00",
        "USD 1 234 567,89",
        // dates
        "2026-11-21",
        "11/21/2026",
        "21.11.2026",
        "2026-11-21T10:30:00Z",
        // plain words
        "Office rent, November",
        "",
    ] {
        assert!(!holds_printed_tin(fine), "{fine:?} must pass");
        assert!(check_free_text("memo", fine).is_ok(), "{fine:?} must pass");
    }
}

/// What must be found, written out: the shapes with the fake ids.
#[test]
fn the_shapes_are_found() {
    for caught in [
        "000-00-0000",
        "00-0000000",
        "000000000",
        "tin 000 00 0000",
        "ein 00 0000000",
        "000\u{2013}00\u{2013}0000",
        "000\u{a0}00\u{a0}0000",
        "\u{ff10}\u{ff10}\u{ff10}-\u{ff10}\u{ff10}-\u{ff10}\u{ff10}\u{ff10}\u{ff10}",
        "\u{660}\u{660}\u{660}-\u{660}\u{660}-\u{660}\u{660}\u{660}\u{660}",
        "pay 00-0000000 now",
        "SSN: 000.00.0000.",
    ] {
        assert!(holds_printed_tin(caught), "{caught:?} must be found");
        let why = check_free_text("the memo", caught).unwrap_err();
        assert!(why.starts_with("the memo looks like it holds a tax id"), "{why}");
        assert!(why.contains("books.set_tin"), "{why}");
        assert!(why.contains("W-9"), "{why}");
        assert!(!why.contains("0000"), "a refusal never repeats the number: {why}");
    }
}

/// Two numbers in one text are two finds, each masked to its last four.
#[test]
fn masking_masks_each_number() {
    let text = "ACH 000-00-0000 and ref 00-0000000, then 1234567890";
    assert_eq!(count_printed_tins(text), 2);
    assert_eq!(
        mask_printed_tins(text),
        "ACH \u{2022}\u{2022}\u{2022}-\u{2022}\u{2022}-0000 and ref \u{2022}\u{2022}-\u{2022}\u{2022}\u{2022}0000, then 1234567890"
    );
    // Nothing to mask: the text comes back as it was.
    assert!(matches!(
        mask_printed_tins("Whole Foods #1234"),
        std::borrow::Cow::Borrowed(_)
    ));
}

#[test]
fn the_refusal_names_the_field_and_never_a_number() {
    let why = tin_refusal("books.add_class name");
    assert!(why.starts_with("books.add_class name looks like it holds a tax id"));
    assert!(why.contains("books.set_tin") && why.contains("books.set_w9"));
}
