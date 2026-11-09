//! The accounting kernel: the arithmetic every bookkeeping invariant rests
//! on, in one place.
//!
//! Each function here is small, pure, and generic over [`Amount`]. The
//! engine runs it on [`rust_decimal::Decimal`]; the Kani harnesses in `proofs/` run the
//! very same code on machine integers and prove it correct for every input
//! within their bounds; and the Lean model in `verify/lean` proves the
//! laws it implements hold for books of any size. See
//! `docs/verification.md` for which proof covers which law.
//!
//! The laws, in the order they appear (each traced to its bookspec rule
//! and to wanderbooks' claim in `docs/spec/claims/`):
//!
//! * a line's signed amount is `+amount` for a debit, `-amount` for a credit
//!   (BS-entry-2);
//! * an entry is acceptable when it has two or more lines, every amount is
//!   positive and in range, and its debits equal its credits exactly (BS-entry-1,
//!   BS-entry-2, BS-entry-3);
//! * an acceptable entry nets to zero, account by account summed (BS-entry-1);
//! * a reversal flips every side, so it is acceptable exactly when the
//!   original is, and the two together net every account to zero (BS-ledger-5);
//! * an account's balance is its movement read on its normal side, so the
//!   balances satisfy the accounting equation whenever movements net to
//!   zero (BS-ledger-3, BS-statement-2);
//! * a trial balance puts each movement in exactly one column, so its
//!   columns differ by the net of all movements (BS-statement-1);
//! * a closing line zeroes the account it closes (BS-period-1);
//! * a payment to a vendor splits, line by line, into what counts toward
//!   its 1099, what a card or payment network paid, and what is unknown,
//!   and money moved between the company's own sources nets out (BS-1099-1);
//! * an early-payment discount never exceeds its stated percentage of the
//!   total, and a document line's amount is exactly its quantity times its
//!   unit price, neither rounded (BS-arap-3);
//! * an allocation's parts sum to the whole, to the last unit, by largest
//!   remainder (BS-partner-1);
//! * a cumulative allocation hands out exactly the new amount, and leaves
//!   each running total one allocation of everything allocated, so it never
//!   drifts a unit from its exact share (BS-partner-1);
//! * an entry's cash is applied to the sections of the statement of cash
//!   flows moving against it, in a fixed order, each up to its own
//!   movement: the parts sum to the entry's cash, what no cash settled sums
//!   to zero, and a reversal's parts are the original's negated (BS-statement-3);
//! * what a member or lender paid or received for the company, with no
//!   cash account moving, is split among the kinds it settled, netting to
//!   zero, and mirrored by a reversal (BS-statement-3 statement 16);
//! * a party's part of an entry is signed as a line on the party's own
//!   account would be, a reversal's is its original's negated, and what of
//!   it counts as money or as property never exceeds it (BS-partner-3);
//! * a span prorated by days (depreciation, BS-asset-1; daily schedules, BS-prepaid-1)
//!   splits into parts that sum to its total, each but the last its days
//!   times the monthly rate within one unit, the last the remainder;
//! * a catch-up run brings the books to the schedule without ever carrying
//!   more than it says at a month end already posted (BS-asset-1), and a
//!   disposal trues what the books carry up or down to the schedule;
//! * an asset's basis figures follow from its basis rule and facts: the
//!   §723 basis is the contributor's adjusted basis, the depreciable basis
//!   the cost, that basis, or the lesser of it and fair market value, and
//!   the §704(c) built-in gain is fair market value less the §723 basis
//!   (BS-asset-4);
//! * a tax category's deductible amount adds its accounts' exact shares and
//!   rounds once to the cent, half away from zero, never share by share
//!   (BS-ledger-11, BS-entry-3).
//!
//! The kernel is a module directory so each kernel-sweep unit adds its
//! arithmetic in a file of its own (`fx`, `round`, `allocate`, `total`,
//! `schedule`, `ratio`, `payroll`, `control`) and the entries above stay
//! in `entries`. Only this file declares the modules; the units never edit
//! it, and every path still resolves as `crate::kernel::foo`.

mod allocate;
mod control;
mod entries;
mod fx;
mod payroll;
mod ratio;
mod round;
mod schedule;
mod total;

// The unit modules are empty until their sweep unit lands; re-exporting
// them now is what keeps every `crate::kernel::foo` path in place.
#[allow(unused_imports)]
pub use allocate::*;
#[allow(unused_imports)]
pub use control::*;
pub use entries::*;
#[allow(unused_imports)]
pub use fx::*;
#[allow(unused_imports)]
pub use payroll::*;
#[allow(unused_imports)]
pub use ratio::*;
#[allow(unused_imports)]
pub use round::*;
#[allow(unused_imports)]
pub use schedule::*;
#[allow(unused_imports)]
pub use total::*;
