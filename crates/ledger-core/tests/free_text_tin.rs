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
    check_free_text, check_reference_text, count_printed_tins, holds_printed_tin,
    holds_printed_tin_in_reference, mask_printed_tins, mask_printed_tins_in_reference, tin_refusal,
};
use proptest::prelude::*;

/// Every separator the detector reads between a TIN's groups, the dot
/// aside (a dot joins only the three groups of an SSN, both: `DOTS`).
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
    // a line break reads as a space
    "\n",
    "\r\n",
];

/// The dots: they join an SSN's groups when they join both of them.
const DOTS: &[&str] = &[".", "\u{ff0e}", "\u{2024}"];

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

    /// An SSN written with a dot at both joiners (`000.00.0000`) is found.
    #[test]
    fn a_dotted_ssn_is_found(
        a in digits(3), b in digits(2), c in digits(4),
        s1 in 0..DOTS.len(), s2 in 0..DOTS.len(), ctx in 0..CONTEXTS.len(),
    ) {
        let text = format!("{}{a}{}{b}{}{c}{}", CONTEXTS[ctx].0, DOTS[s1], DOTS[s2], CONTEXTS[ctx].1);
        prop_assert!(holds_printed_tin(&text), "{text:?}");
    }

    /// A dot at one joiner only, or between an EIN's groups, is a decimal or
    /// a version, not a tax id (a coordinate: `47.6062095`).
    #[test]
    fn a_dot_alone_is_not_a_joiner(
        a in digits(3), b in digits(2), c in digits(4), e in digits(7),
        dot in 0..DOTS.len(), sep in 0..SEPARATORS.len(), ctx in 0..CONTEXTS.len(),
    ) {
        let ein = format!("{}{}{}{e}{}", CONTEXTS[ctx].0, &a[..2], DOTS[dot], CONTEXTS[ctx].1);
        prop_assert!(!holds_printed_tin(&ein), "{ein:?}");
        let first = format!("{}{a}{}{b}{}{c}{}", CONTEXTS[ctx].0, DOTS[dot], SEPARATORS[sep], CONTEXTS[ctx].1);
        prop_assert!(!holds_printed_tin(&first), "{first:?}");
        let second = format!("{}{a}{}{b}{}{c}{}", CONTEXTS[ctx].0, SEPARATORS[sep], DOTS[dot], CONTEXTS[ctx].1);
        prop_assert!(!holds_printed_tin(&second), "{second:?}");
    }

    /// A date followed by a seven-digit reference is a date and a
    /// reference, whatever stands between them, and a decimal with seven
    /// digits after its point is a number (`47.6062095`).
    #[test]
    fn a_date_then_a_reference_and_a_decimal_are_no_tax_id(
        y in 1990u32..2100, m in 1u32..13, d in 1u32..29, r in digits(7),
        whole in digits(2), sep in 0..4usize, ctx in 0..CONTEXTS.len(),
    ) {
        let between = [" ", "\t", "\n", "  "][sep];
        let date = format!("{}{y}-{m:02}-{d:02}{between}{r}{}", CONTEXTS[ctx].0, CONTEXTS[ctx].1);
        prop_assert!(!holds_printed_tin(&date), "{date:?}");
        let decimal = format!("{}{whole}.{r}{}", CONTEXTS[ctx].0, CONTEXTS[ctx].1);
        prop_assert!(!holds_printed_tin(&decimal), "{decimal:?}");
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
        // decimals: a coordinate, a version, an amount
        "47.6062095",
        "at 47.6062095, -122.3321",
        "17.2345678",
        "v 12.3456789",
        "$1234567.8901234",
        // a date followed by, or inside, a longer number
        "2026-11-21 0001234",
        "on 2026-11-21 0001234 paid",
        "2026-11-0001234",
        "55-1234567-001",
        "part 5-000-00-0000",
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
        // a line break between the groups reads as a space
        "000\n00\n0000",
        "000\r\n00\r\n0000",
        "ein 00\n0000000",
        "ein 00\r0000000",
        // zero-width and soft-hyphen characters between digits are not there
        "000\u{200b}-00-0000",
        "0\u{200b}0\u{200c}0\u{200d}-0\u{2060}0-00\u{feff}00",
        "00\u{ad}-0000000",
        "00-000\u{ad}0\u{ad}000",
        "0\u{200b}00000000",
        "\u{200b}000000000\u{200b}",
        // the text around a number
        "words 000-00-0000 words",
        "(00-0000000)",
    ] {
        assert!(holds_printed_tin(caught), "{caught:?} must be found");
        let why = check_free_text("the memo", caught).unwrap_err();
        assert!(
            why.starts_with("the memo looks like it holds a tax id"),
            "{why}"
        );
        assert!(why.contains("books.set_tin"), "{why}");
        assert!(why.contains("W-9"), "{why}");
        assert!(
            !why.contains("0000"),
            "a refusal never repeats the number: {why}"
        );
    }
}

/// What the shapes do not take for a tax id: a dot is no joiner but between
/// both groups of an SSN, and a group that is the tail or head of a longer
/// run of digits and dashes belongs to that run.
#[test]
fn a_dot_and_a_longer_run_are_no_tax_id() {
    for fine in [
        "47.6062095",
        "17.2345678",
        "12.3456789",
        "000.00 0000",
        "000 00.0000",
        "000.00-0000",
        "000-00.0000",
        "2026-11-21 0001234",
        "2026-11-21\n0001234",
        "12-3456789-0",
        "1-123-45-6789",
        "0000-00-0000000",
        "123456789.5",
        "0.123456789",
    ] {
        assert!(!holds_printed_tin(fine), "{fine:?} must pass");
        assert!(check_free_text("memo", fine).is_ok(), "{fine:?} must pass");
    }
    // ... while the shapes, with words around them, are still found.
    for caught in [
        "2026-11-21 paid 00-0000000",
        "on 2026-11-21: 000-00-0000",
        "47.6062095 and then 00-0000000",
        "ref 12-3456789.",
        "123456789.",
    ] {
        assert!(holds_printed_tin(caught), "{caught:?} must be found");
    }
    // The line break and the invisible characters are read only outside a
    // replay, as the Unicode forms are (BS-entry-4).
    let broken = "000\n00\n0000";
    assert!(holds_printed_tin(broken));
    ledger_core::parties::replaying(|| {
        assert!(!holds_printed_tin(broken));
        assert!(!holds_printed_tin("0\u{200b}00000000"));
    });
}

/// Letters between digits, and a number split across two fields, are an
/// accident and not a way to hide one: the reading leaves them (WB-partner-15).
#[test]
fn letters_between_digits_are_left_alone() {
    for fine in ["000a00b0000", "00-0000 000", "000-00x0000"] {
        assert!(!holds_printed_tin(fine), "{fine:?}");
    }
}

/// A reference's reading (an asset tag, a serial, an invoice or bill
/// number, a payment reference, an account code, a file name): a bare nine
/// digits is a number there; the printed shapes are still refused.
#[test]
fn a_reference_may_be_nine_digits_but_not_a_printed_tax_id() {
    for fine in [
        "123456789",
        "SN 000000000",
        "tag 000000000.",
        "000000000",
        "INV-2026-11-0042",
        "1234567890",
        "scan 000000000.pdf",
    ] {
        assert!(!holds_printed_tin_in_reference(fine), "{fine:?}");
        assert!(check_reference_text("the tag", fine).is_ok(), "{fine:?}");
        assert_eq!(
            mask_printed_tins_in_reference(fine),
            fine,
            "a bare nine digits stays in a reference"
        );
    }
    for caught in [
        "000-00-0000",
        "00-0000000",
        "SN 000 00 0000",
        "w9 00\u{2013}0000000.pdf",
        "000.00.0000",
        "000\n00\n0000",
        "0\u{200b}0-0000000",
    ] {
        assert!(holds_printed_tin_in_reference(caught), "{caught:?}");
        let why = check_reference_text("the tag", caught).unwrap_err();
        assert!(
            why.starts_with("the tag looks like it holds a tax id") && !why.contains("0000"),
            "{why}"
        );
    }
    // Free text says the same of the same bare run: refused.
    assert!(holds_printed_tin("123456789"));
    // A mask of a reference keeps its bare run and covers its shapes.
    assert_eq!(
        mask_printed_tins_in_reference("SN 123456789 / 000-00-0000"),
        "SN 123456789 / \u{2022}\u{2022}\u{2022}-\u{2022}\u{2022}-0000"
    );
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

// ---------------------------------------------------------------------
// The engine: a caller other than the MCP layer meets the same check.
// ---------------------------------------------------------------------

mod engine {
    use chrono::NaiveDate;
    use ledger_core::arap::{BillFields, DocumentLine, InvoiceFields, PartySnapshot};
    use ledger_core::schedules::{
        Frequency, NewRecurring, NewSchedule, ScheduleConvention, ScheduleKind,
    };
    use ledger_core::{
        AccountId, AccountType, Currency, Ledger, LedgerError, Money, NewAsset, Side,
    };
    use rust_decimal::Decimal;
    use std::str::FromStr;

    const TIN: &str = "000-00-0000";

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, d).unwrap()
    }

    fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
    }

    struct Fx {
        l: Ledger,
        cash: AccountId,
        revenue: AccountId,
        expense: AccountId,
        asset: AccountId,
        accum: AccountId,
        liability: AccountId,
    }

    fn fx() -> Fx {
        let mut l = Ledger::new(Currency::USD);
        let mut add = |code: &str, name: &str, t: AccountType| {
            l.add_account(code, name, t, None, false).unwrap()
        };
        let cash = add("1000", "Cash", AccountType::Asset);
        let asset = add("1500", "Equipment", AccountType::Asset);
        let accum = add("1590", "Accumulated depreciation", AccountType::Asset);
        let liability = add("2500", "Deferred", AccountType::Liability);
        let revenue = add("4000", "Sales", AccountType::Revenue);
        let expense = add("6000", "Supplies", AccountType::Expense);
        Fx {
            l,
            cash,
            revenue,
            expense,
            asset,
            accum,
            liability,
        }
    }

    fn line(desc: &str) -> DocumentLine {
        DocumentLine {
            description: desc.into(),
            amount: Decimal::from(10),
            class_id: None,
            account: None,
            quantity: None,
            unit_price: None,
        }
    }

    fn holds_tin(r: Result<impl std::fmt::Debug, LedgerError>, field: &str) {
        match r {
            Err(LedgerError::FreeTextHoldsTin(why)) => {
                assert!(
                    why.starts_with(&format!("{field} looks like it holds a tax id")),
                    "{why}"
                );
                assert!(
                    why.contains("books.set_tin") && why.contains("W-9"),
                    "{why}"
                );
                assert!(!why.contains("0000"), "never the number: {why}");
            }
            other => panic!("{field}: expected FreeTextHoldsTin, got {other:?}"),
        }
    }

    /// An entry's description, a reversal's, a write-off's and a void's
    /// reason, a period's name, an account's code.
    #[test]
    fn entries_periods_and_codes() {
        let mut f = fx();
        holds_tin(
            f.l.try_create_entry(day(1), &format!("paid {TIN}")),
            "the entry description",
        );
        // Nothing was made: the next entry is still the first.
        let e =
            f.l.try_create_entry(day(1), "inv 2026-11-0042, ref 1234567890")
                .unwrap();
        f.l.add_line(e, f.cash, Side::Debit, usd("5.00")).unwrap();
        f.l.add_line(e, f.revenue, Side::Credit, usd("5.00"))
            .unwrap();
        f.l.post_entry(e).unwrap();
        let before = f.l.clone();
        holds_tin(
            f.l.reverse_entry(e, day(2), &format!("oops {TIN}")),
            "the reversal's description",
        );
        assert_eq!(f.l, before, "a refusal changes nothing");
        holds_tin(
            f.l.create_period(&format!("P {TIN}"), day(1), day(31)),
            "the period name",
        );
        // An account code is a reference: its printed shape is refused, a
        // bare nine digits is a code (WB-partner-15).
        holds_tin(
            f.l.add_account(TIN, "Odd", AccountType::Asset, None, false),
            "the account code",
        );
        let nine =
            f.l.add_account("123456789", "Nine", AccountType::Asset, None, false);
        assert!(nine.is_ok(), "{nine:?}");
        holds_tin(
            f.l.renumber_account(f.cash, "00-0000000"),
            "the account code",
        );
        f.l.renumber_account(f.cash, "000000000").unwrap();
        f.l.create_period("2026-03", day(1), day(31)).unwrap();
    }

    /// A document's memo, references, duplicate reason, lines and the
    /// customer's printed details.
    #[test]
    fn documents() {
        let mut f = fx();
        let c = f.l.add_customer("Acme").unwrap();
        let v = f.l.add_vendor("Beta").unwrap();
        let inv = |l: &mut Ledger, lines: Vec<DocumentLine>, fields: InvoiceFields| {
            l.create_invoice_with(c, day(1), day(31), lines, fields)
        };
        holds_tin(
            inv(
                &mut f.l,
                vec![line(&format!("work {TIN}"))],
                InvoiceFields::default(),
            ),
            "a line's description",
        );
        holds_tin(
            inv(
                &mut f.l,
                vec![line("ok")],
                InvoiceFields {
                    memo: Some(format!("see {TIN}")),
                    ..Default::default()
                },
            ),
            "the memo",
        );
        holds_tin(
            inv(
                &mut f.l,
                vec![line("ok")],
                InvoiceFields {
                    customer_ref: Some("PO 00-0000000".into()),
                    ..Default::default()
                },
            ),
            "the reference",
        );
        let snap = |name: &str| PartySnapshot {
            name: name.into(),
            address: None,
            email: None,
            phone: None,
            contact: None,
            billing_address: None,
            shipping_address: None,
            billing_email: None,
        };
        // The customer's printed details are a party's: any nine digits.
        let strict = inv(
            &mut f.l,
            vec![line("ok")],
            InvoiceFields {
                bill_to: Some(snap("Acme 1234 56789")),
                ..Default::default()
            },
        );
        assert!(
            matches!(strict, Err(LedgerError::InvalidParty(_))),
            "{strict:?}"
        );
        // The company's own details are free text: its shapes only.
        holds_tin(
            inv(
                &mut f.l,
                vec![line("ok")],
                InvoiceFields {
                    seller: Some(snap(&format!("Us {TIN}"))),
                    ..Default::default()
                },
            ),
            "the seller name",
        );
        // Neither a 10-digit PO nor a ZIP+4 in the company's address is a tax id.
        let mut us = snap("Us");
        us.address = Some("1 Main St, Midvale UT 84047-1234".into());
        inv(
            &mut f.l,
            vec![line("ok")],
            InvoiceFields {
                customer_ref: Some("1234567890".into()),
                memo: Some("2026-11-0042".into()),
                seller: Some(us),
                ..Default::default()
            },
        )
        .unwrap();
        let bill = |l: &mut Ledger, fields: BillFields| {
            l.create_bill_with(v, day(1), day(31), vec![line("ok")], fields)
        };
        holds_tin(
            bill(
                &mut f.l,
                BillFields {
                    vendor_ref: Some(TIN.into()),
                    ..Default::default()
                },
            ),
            "the reference",
        );
        // A bare nine digits is a vendor's invoice number, and a customer's
        // purchase order: references.
        bill(
            &mut f.l,
            BillFields {
                vendor_ref: Some("000000000".into()),
                ..Default::default()
            },
        )
        .unwrap();
        inv(
            &mut f.l,
            vec![line("ok")],
            InvoiceFields {
                customer_ref: Some("PO 123456789".into()),
                ..Default::default()
            },
        )
        .unwrap();
        holds_tin(
            bill(
                &mut f.l,
                BillFields {
                    memo: Some(TIN.into()),
                    ..Default::default()
                },
            ),
            "the memo",
        );
        holds_tin(
            bill(
                &mut f.l,
                BillFields {
                    duplicate_reason: Some(format!("again {TIN}")),
                    ..Default::default()
                },
            ),
            "the duplicate reason",
        );
        bill(
            &mut f.l,
            BillFields {
                vendor_ref: Some("INV-2026-11-0042".into()),
                ..Default::default()
            },
        )
        .unwrap();
    }

    /// A payment's reference is the engine's to check, as the tools check
    /// it: a printed SSN or EIN is refused, a bare nine digits (a check or
    /// a confirmation number) is kept, and the entry's description, which
    /// is free text, shows it masked. A discount's reason is free text.
    #[test]
    fn payments_check_their_reference_and_discount_reason() {
        use ledger_core::{Application, DocRef, PaymentDetails, SettleParams};
        let mut f = fx();
        let c = f.l.add_customer("Acme").unwrap();
        let v = f.l.add_vendor("Beta").unwrap();
        let invoice =
            f.l.create_invoice_with(
                c,
                day(1),
                day(31),
                vec![line("work")],
                InvoiceFields::default(),
            )
            .unwrap();
        let bill =
            f.l.create_bill_with(
                v,
                day(1),
                day(31),
                vec![line("parts")],
                BillFields::default(),
            )
            .unwrap();
        let pay = |l: &mut Ledger, reference: &str| {
            l.receive_payment_with(
                invoice,
                day(5),
                Decimal::from(1),
                f.cash,
                &PaymentDetails {
                    instrument: None,
                    reference: Some(reference.into()),
                },
            )
        };
        holds_tin(pay(&mut f.l, TIN), "the payment reference");
        holds_tin(pay(&mut f.l, "check 00-0000000"), "the payment reference");
        let before = f.l.clone();
        holds_tin(
            f.l.pay_bill_with(
                bill,
                day(5),
                Decimal::from(1),
                f.cash,
                &PaymentDetails {
                    instrument: None,
                    reference: Some(format!("wire {TIN}")),
                },
            ),
            "the payment reference",
        );
        assert_eq!(f.l, before, "a refusal changes nothing");
        // A bare nine digits is a reference ...
        let e = pay(&mut f.l, "123456789").unwrap();
        // ... and the entry's description, free text, holds it masked.
        assert_eq!(
            f.l.get_entry(e).unwrap().description(),
            "Payment on invoice 1 (ref \u{2022}\u{2022}\u{2022}\u{2022}\u{2022}6789)"
        );
        // A ten-digit trace number is kept as it is.
        let e = pay(&mut f.l, "1234567890").unwrap();
        assert_eq!(
            f.l.get_entry(e).unwrap().description(),
            "Payment on invoice 1 (ref 1234567890)"
        );
        // The discount reason of a settlement.
        let settle = |l: &mut Ledger, why: &str| {
            l.settle(SettleParams {
                date: day(6),
                description: "Payment".into(),
                cash_account: f.cash,
                party: c,
                applications: vec![Application {
                    doc: DocRef::Invoice(invoice),
                    cash: Decimal::from(1),
                    discount: Decimal::ZERO,
                    discount_reason: Some(why.into()),
                }],
                fee: None,
                instrument: None,
                reference: None,
            })
        };
        holds_tin(
            settle(&mut f.l, &format!("early {TIN}")),
            "the discount reason",
        );
        settle(&mut f.l, "paid within 10 days, ref 1234567890").unwrap();
        // A replay reads none of it.
        ledger_core::parties::replaying(|| {
            let d = PaymentDetails {
                instrument: None,
                reference: Some(TIN.into()),
            };
            d.check().unwrap();
        });
    }

    /// A project's fields are refused by the engine itself, whoever calls
    /// (the tools' own check runs before it, and says it first).
    #[test]
    fn a_project_field_is_refused_by_the_engine() {
        use ledger_core::classes::ProjectFacts;
        let mut f = fx();
        let id = f.l.add_class("Harbor").unwrap();
        let facts = |notes: Option<&str>, description: Option<&str>, links: &[&str]| ProjectFacts {
            notes: notes.map(Into::into),
            description: description.map(Into::into),
            links: links.iter().map(|l| (*l).to_string()).collect(),
            ..Default::default()
        };
        let engine_refusal = |r: Result<(String, ProjectFacts), LedgerError>, field: &str| match r {
            Err(LedgerError::InvalidProject(why)) => {
                assert!(why.contains(field), "{why}");
                assert!(why.contains("full SSN or EIN"), "{why}");
                assert!(!why.contains("0000"), "never the number: {why}");
            }
            other => panic!("{field}: expected the engine's refusal, got {other:?}"),
        };
        engine_refusal(
            f.l.check_set_project(id, "Harbor", &facts(Some(&format!("ssn {TIN}")), None, &[])),
            "notes",
        );
        engine_refusal(
            f.l.check_set_project(id, "Harbor", &facts(None, Some("ein 00-0000000"), &[])),
            "description",
        );
        engine_refusal(
            f.l.check_set_project(
                id,
                "Harbor",
                &facts(None, None, &["https://x.test/000-00-0000"]),
            ),
            "link",
        );
        match f
            .l
            .set_project(id, &format!("Harbor {TIN}"), facts(None, None, &[]))
        {
            Err(LedgerError::InvalidProject(why)) => {
                assert!(why.contains("full SSN or EIN"), "{why}");
            }
            other => panic!("the name: {other:?}"),
        }
        // A reference of ten digits, an invoice number and a store listing's
        // id are no tax id.
        f.l.set_project(
            id,
            "Harbor 2026-11-0042",
            facts(
                Some("ref 1234567890"),
                Some("PO 2026-0042"),
                &["https://apps.example.com/app/id284882215"],
            ),
        )
        .unwrap();
    }

    /// Assets, schedules, recurring entries and payroll names.
    #[test]
    fn assets_schedules_and_payroll() {
        let mut f = fx();
        let new = |d: &str| NewAsset {
            description: d.into(),
            in_service_date: day(1),
            cost: usd("1200.00"),
            salvage_value: usd("0"),
            useful_life_months: 12,
            cost_account: f.asset,
            accum_depr_account: f.accum,
            expense_account: f.expense,
        };
        holds_tin(
            f.l.register_asset(new(&format!("van {TIN}"))),
            "the asset description",
        );
        let a = f.l.register_asset(new("van 1234567890")).unwrap();
        holds_tin(
            f.l.set_asset_serial(a, Some("SN 00-0000000")),
            "the serial number",
        );
        holds_tin(
            f.l.set_asset_description(a, "000000000"),
            "the asset description",
        );
        holds_tin(f.l.set_asset_tag(a, "T-000-00-0000"), "the asset tag");
        f.l.set_asset_serial(a, Some("C02XK1ABCDEF")).unwrap();
        // A tag and a serial number are references: a bare nine digits
        // is one.
        f.l.set_asset_serial(a, Some("000000000")).unwrap();
        f.l.set_asset_tag(a, "123456789").unwrap();
        let sched = |d: &str| NewSchedule {
            description: d.into(),
            kind: ScheduleKind::PrepaidExpense,
            total: usd("1200.00"),
            start: day(1),
            months: 12,
            balance_account: f.asset,
            pl_account: f.expense,
            convention: ScheduleConvention::Daily,
        };
        holds_tin(
            f.l.register_schedule(sched(&format!("rent {TIN}"))),
            "the schedule's description",
        );
        let _ = f.liability;
        let rec = |d: &str| NewRecurring {
            description: d.into(),
            lines: vec![
                (f.expense, Side::Debit, usd("5.00")),
                (f.cash, Side::Credit, usd("5.00")),
            ],
            frequency: Frequency::Monthly,
            start: day(1),
            end: None,
        };
        holds_tin(
            f.l.register_recurring(rec(&format!("fee {TIN}"))),
            "the recurring entry's description",
        );
        f.l.register_recurring(rec("fee 2026-11-0042")).unwrap();
    }

    /// A draft's note, a discard's reason and a no-party reason: free
    /// text now, no longer the strict nine-digit reading.
    #[test]
    fn notes_and_reasons_use_the_shapes() {
        let mut f = fx();
        let e = f.l.try_create_entry(day(1), "x").unwrap();
        let wait = |n: &str| ledger_core::DraftState::AwaitingEvidence { note: n.into() };
        assert!(
            f.l.set_draft_state(e, wait(&format!("waiting {TIN}")))
                .is_err()
        );
        f.l.set_draft_state(e, wait("waiting on ref 1234567890, inv 2026-11-0042"))
            .unwrap();
        assert!(Ledger::check_no_party_reason(&format!("own account {TIN}")).is_err());
        Ledger::check_no_party_reason("own account 1234567890").unwrap();
        assert!(f.l.discard_draft(e, &format!("dup {TIN}")).is_err());
        f.l.discard_draft(e, "entered twice, ref 1234567890")
            .unwrap();
    }

    /// BS-entry-4: a journal line an earlier version took replays, whatever
    /// it holds; the books open on it. (A replay is the only way such a
    /// line is ever applied: every command refuses it.)
    #[test]
    fn a_journaled_line_replays_whatever_it_holds() {
        let mut f = fx();
        let e = f.l.create_entry(day(1), &format!("paid {TIN}"));
        f.l.add_line(e, f.cash, Side::Debit, usd("5.00")).unwrap();
        f.l.add_line(e, f.revenue, Side::Credit, usd("5.00"))
            .unwrap();
        f.l.post_entry(e).unwrap();
        let r =
            f.l.replay_reverse_entry(e, day(2), &format!("undo {TIN}"))
                .unwrap();
        assert_eq!(
            f.l.get_entry(r).unwrap().description(),
            format!("undo {TIN}")
        );
        let ann = ledger_core::Annotation {
            attachment_id: 1,
            fields: ledger_core::AnnotationFields {
                payee: Some(format!("Pat {TIN}")),
                ..Default::default()
            },
            note: Some(format!("see {TIN}")),
        };
        f.l.replay_annotate_attachment(ann.clone()).unwrap();
        // The same annotation as a command is refused.
        holds_tin(f.l.annotate_attachment(ann), "the payee");
    }
}
