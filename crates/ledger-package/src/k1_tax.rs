//! The K-1 worksheet on the tax basis: `k1-tax-basis-capital.csv`, each
//! member's Item L capital account on the tax basis beside the book
//! figures of `k1-members.csv`, with the reconciliation between them.
//!
//! ```text
//! GOAL: hand the CPA Item L on the tax basis as far as the books know it,
//!       line by line beside the book figures, each figure the books can't
//!       know left blank and named, never guessed.
//! SOURCE: ledger_core::tax_capital (INVARIANTS 1-5); Instructions for Form
//!         1065 (2025), Item L, "Tax-basis method" (rules.md B-36);
//!         DECISION D-41 (owner, 2026-10-20).
//! INVARIANTS:
//!   1. Per member, in order: the seven Item L lines (beginning; capital
//!      contributed, cash and property; current year net income (loss);
//!      other increase (decrease); withdrawals and distributions; ending),
//!      each with its book figure, its tax-basis figure (blank while open),
//!      book less tax (blank while open), its status ("tax basis" or
//!      "open"), and a note: for an open line, what the books know so far
//!      and every figure the CPA supplies, with the book amount it
//!      replaces.
//!   2. Then the reconciliation of book to tax capital, by cause, ending
//!      with "book ending less tax ending" (tax_capital INVARIANT 5): when
//!      the ending is open, it reconciles to what the books know so far.
//!   3. Every amount is exact (crate::text::money); nothing is rounded.
//! ```

use crate::text::{csv_escape, money};
use ledger_core::Ledger;
use ledger_core::tax_capital::{Open, TaxCapital, TaxLine};
use rust_decimal::Decimal;

/// The file's name in the package.
pub const FILE_NAME: &str = "k1-tax-basis-capital.csv";

/// What the CPA supplies for an open item, with the book amount it
/// replaces.
pub fn open_text(ledger: &Ledger, o: &Open) -> String {
    let asset = |id: u64| {
        ledger
            .get_asset(id)
            .and_then(|a| a.tag.clone())
            .unwrap_or_else(|| format!("asset {id}"))
    };
    match o {
        Open::PriorYears => {
            "last year's ending capital on the tax basis (Item L of last year's return)".into()
        }
        Open::PropertyBasis { entry, book } => format!(
            "the adjusted basis of property contributed in entry {entry} with no tax basis in the books ({} at book; record it on its asset with books.set_asset_tax_basis)",
            money(*book)
        ),
        Open::LinkedValueExceedsEntry {
            entry,
            linked,
            property,
        } => format!(
            "the basis of the property contributed in entry {entry}: the assets linked to it are valued at {}, more than the {} of property it contributed (correct the link or the values)",
            money(*linked),
            money(*property)
        ),
        Open::PropertyDistribution { entry, book } => format!(
            "the company's adjusted tax basis in property distributed in entry {entry} ({} at book)",
            money(*book)
        ),
        Open::TaxDepreciation {
            book,
            contributed_book,
        } => format!(
            "tax depreciation of the fixed assets in service (MACRS, IRC §168), in place of the book depreciation posted ({} in the span, {} of it on contributed property, whose tax depreciation each property's §704(c) method allocates, Treas. Reg. §1.704-3)",
            money(*book),
            money(*contributed_book)
        ),
        Open::Disposal {
            asset: id,
            proceeds,
        } => format!(
            "the tax gain or loss on {}'s disposal (proceeds {}), from its adjusted tax basis",
            asset(*id),
            money(*proceeds)
        ),
        Open::OtherChange { book } => format!(
            "the tax treatment of a {} change in book capital the books can't explain",
            money(*book)
        ),
    }
}

/// The worksheet (INVARIANTS 1-3). `names` are the members' names, in the
/// rows' order.
pub fn tax_basis_csv(ledger: &Ledger, names: &[String], tc: &TaxCapital) -> String {
    let mut w = String::from("member,item,book,tax_basis,book_less_tax,status,note\n");
    let mut push = |name: &str, item: &str, cells: [String; 5]| {
        w.push_str(&csv_escape(name));
        w.push(',');
        w.push_str(&csv_escape(item));
        for c in cells {
            w.push(',');
            w.push_str(&csv_escape(&c));
        }
        w.push('\n');
    };
    for (row, name) in tc.rows.iter().zip(names) {
        let line = |book: Decimal, tax: &TaxLine, note: &str| -> [String; 5] {
            match tax.tax() {
                Some(t) => [
                    money(book),
                    money(t),
                    money(book - t),
                    "tax basis".into(),
                    note.into(),
                ],
                None => {
                    let needs: Vec<String> =
                        tax.open.iter().map(|o| open_text(ledger, o)).collect();
                    [
                        money(book),
                        String::new(),
                        String::new(),
                        "open: the CPA's".into(),
                        format!(
                            "known so far {}; needs {}",
                            money(tax.known),
                            needs.join("; ")
                        ),
                    ]
                }
            }
        };
        let b = &row.book;
        let cash = TaxLine {
            known: row.contributions_cash,
            open: Vec::new(),
        };
        push(
            name,
            "Beginning capital",
            line(b.beginning, &row.beginning, ""),
        );
        push(
            name,
            "Capital contributed: cash",
            line(b.contributions_cash, &cash, ""),
        );
        push(
            name,
            "Capital contributed: property",
            line(
                b.contributions_property,
                &row.contributions_property,
                "at the property's adjusted basis (IRC §723); book: its fair market value",
            ),
        );
        push(
            name,
            "Current year net income (loss)",
            line(
                b.net_income,
                &row.net_income,
                "the book figure: the books know of no difference from it (any other, Schedule M-1, is the CPA's)",
            ),
        );
        push(
            name,
            "Other increase (decrease)",
            line(b.other, &row.other, ""),
        );
        push(
            name,
            "Withdrawals and distributions",
            line(
                b.distributions,
                &row.distributions,
                "cash, plus the adjusted tax basis of property distributed",
            ),
        );
        push(name, "Ending capital", line(b.ending, &row.ending, ""));
        let r = &row.reconciliation;
        let recon = |amount: Option<Decimal>, note: &str| -> [String; 5] {
            [
                String::new(),
                String::new(),
                amount.map(money).unwrap_or_default(),
                "reconciliation".into(),
                match amount {
                    Some(_) => note.into(),
                    None => "too large to compute exactly".into(),
                },
            ]
        };
        push(
            name,
            "Reconciliation: beginning capital, book less tax",
            recon(Some(r.beginning), ""),
        );
        push(
            name,
            "Reconciliation: built-in gain (loss) of property contributed",
            recon(
                Some(r.built_in_gain_704c),
                "fair market value less §723 basis at contribution, IRC §704(c) (Treas. Reg. §1.704-3(a)(3)(ii))",
            ),
        );
        push(
            name,
            "Reconciliation: property contributed with no tax basis in the books, at book",
            recon(
                Some(r.property_without_basis),
                "its adjusted basis is the CPA's",
            ),
        );
        push(
            name,
            "Reconciliation: less property distributed, at book",
            recon(
                Some(-r.property_distributed),
                "the company's adjusted tax basis in it is the CPA's",
            ),
        );
        push(
            name,
            "Reconciliation: book ending less tax ending",
            recon(
                r.total(),
                if row.ending.open.is_empty() {
                    ""
                } else {
                    "to the tax ending the books know so far, before the CPA's figures above"
                },
            ),
        );
    }
    w
}
