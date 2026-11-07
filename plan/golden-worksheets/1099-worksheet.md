# Golden worksheet: the 1099 vendor worksheet

Golden: `crates/ledger-core/tests/golden/form1099_worksheet.json`
Test: `golden_1099_worksheet` in `crates/ledger-core/tests/form1099.rs`
Fixture: `books()` in that test file (the existing hand-verified fixture:
bill payments, a card charge, a reversal, a year boundary), **plus** one
step the golden test itself takes: it records `2100 Credit Card` as a
business-card payment method ("Harbor Visa") before building the reports
(section "With the card recorded"). `books()` is unchanged; other tests
rely on its unrecorded card.
Rules: `docs/spec/rules.md` B-19 (thresholds, payment methods, decisions,
status), and D-40 as B-19 records it.

Every number below was worked from the fixture's entries before the report
was serialized. The report's own buckets are `paid` (counts),
`excluded_card` (a card or payment network paid: the processor's 1099-K),
`undetermined` (a financing liability with no method recorded), and
`not_reportable` (paid while the vendor was decided not reportable).

## The accounts and the payments

Chart: `1000 Bank` (asset, the cash account), `2100 Credit Card`
(liability; in `books()` **no** business-card payment method records it and
it is not a financing account; the golden test records it, see "With the
card recorded"), `6000 Contract Labor` and `6600 Rent` (expenses).
`create_bill` adds its control accounts (general expenses, payables); they
are not cash and no payment comes out of them.

| # | Date | Entry | Vendor | Dr | Cr | Amount |
|---|---|---|---|---|---|---|
| 1 | 2026-03-01 | Bill 1 (due 2026-03-31) | Dana | general expenses | payables | 1,500 |
| 2 | 2026-03-20 | pay Bill 1 from the bank | Dana | payables | 1000 Bank | 1,500 |
| 3 | 2026-06-01 | payment | Dana | 6000 Contract Labor | 1000 Bank | 700 |
| 4 | 2025-12-30 | payment | Dana | 6000 Contract Labor | 1000 Bank | 300 |
| 5 | 2026-01-01 | payment | Ace Rentals | 6600 Rent | 1000 Bank | 1,200 |
| 6 | 2026-02-01 | payment | Ace Rentals | 6600 Rent | 1000 Bank | 1,200 |
| 7 | 2026-02-02 | reversal of #6 (duplicate) | Ace Rentals | 1000 Bank | 6600 Rent | 1,200 |
| 8 | 2026-04-01 | payment | Sam | 6000 Contract Labor | 2100 Credit Card | 5,000 |
| 9 | 2026-05-01 | payment | Bigco | 6000 Contract Labor | 1000 Bank | 9,000 |
| 10 | 2026-07-01 | payment, no vendor tag | — | 6000 Contract Labor | 1000 Bank | 50 |

Decisions, each from the beginning (`effective: null`), decided by Riley
Chen on 2026-01-05: Dana 1099-NEC with a W-9, Ace Rentals 1099-MISC
without one, Sam 1099-NEC with a W-9. Bigco is never decided.

## Buckets, by entry

A line pays only out of a source B-19 names. `1000 Bank` is a cash
account, so a bank credit counts (presumed check/ACH/transfer, D-40).
`2100 Credit Card` is not a cash account and not a financing account.
(In `books()` alone, with no method recording it, it would be "anything
else" in B-19's table and the 5,000 at #8 would be in no bucket; that
unrecorded-card analysis is kept as a note under "With the card recorded"
and is why the golden records the card: the unrecorded case exercises no
`excluded_card` figure at all.) With the card recorded (below), #8 is in
the **card** bucket. A bill is not a payment until it is paid
(B-19), so #1 counts nothing and #2 counts 1,500. A reversal nets on its
original's side, so #7 cancels #6.

Dana:
- 2026: #2 1,500 (bank, counts) + #3 700 (bank, counts) = **2,200**.
  #4 is dated 2025-12-30, outside the 2026 window.
- 2025: #4 **300**.

Ace Rentals:
- 2026: #5 1,200 counts; #6 (+1,200) and #7 (−1,200) net **0**.
  Total **1,200**.
- 2025: no payments → 0.

Sam:
- 2026: #8 5,000 out of 2100, a recorded company card's account →
  `excluded_card` **5,000**, `paid` 0 (see "With the card recorded").
- 2025: no payments → 0.

Bigco:
- 2026: #9 9,000 out of the bank, counts, and Bigco is undecided for the
  whole year → it lands in the undecided paid side (`paid`), with
  `decision_needed` true. **9,000**.
- 2025: no payments, no decision → not on the worksheet.

## Status, per B-19

T is the year's threshold: 600 for 2025, 2,000 for 2026 (26 U.S.C.
§6041(a), §6041A(a)(2), Pub. L. 119-21 §70433; payments after 2025-12-31).

- Dana 2026: decided for a 1099, nothing undecided, lo(F) = 2,200 ≥ 2,000
  → **reportable**. A W-9 is on file.
- Ace Rentals 2026: decided for a 1099, lo(F) = hi(F) = 1,200 < 2,000
  → **under_threshold**.
- Sam 2026: decided for a 1099, F = 0, U = 0 (the 5,000 is in the card
  bucket, outside both), no payment while not reportable
  → **under_threshold** (a card charge is a property of the payment, never
  the vendor's reason, B-19).
- Bigco 2026: undecided money; hi(F) + max(hi(U), 0) = 0 + 9,000 ≥ 2,000
  → **undetermined**, `decision_needed` true, `reportable` false. Absent a
  decision it is never reportable (defect 18).
- 2025: Dana 300 < 600 → **under_threshold**. Ace Rentals and Sam are on
  the 2025 worksheet with 0 because each has a decision for a 1099 in
  force during 2025 (a decision from the beginning; B-19's list of "every
  vendor with a decision for a 1099 in force during the year"), both
  **under_threshold**. Bigco has neither a decision nor a figure in 2025,
  so it is absent.

## Lists

- 2026 `untagged_payments`: [10], the 50.00 contract-labor payment with no
  vendor tag (B-19's "quieter note").
- 2026 `excluded_card_payments`: [#8] — one `Payment1099`: entry 8, date
  2026-04-01, vendor Sam (3), amount 5,000, `instrument` null (the entry
  is untagged; a company card's account is excluded with or without a tag,
  B-19's table, last-but-one row; the existing test
  `card_account_payments_are_listed_not_dropped` reports `None` the same
  way).
- 2026 `undetermined_payments`: empty — no payment came through a
  financing liability with its method unrecorded.
- `other_receipts`, `unresolved`: empty.
- 2025 has no card, undetermined, other-receipt, unresolved or untagged
  entries.

## With the card recorded

The golden test calls `add_instrument(BusinessCard, "Harbor Visa",
Some("0042"), None, <2100>)` before the reports. Worked from B-19's table
for #8 (Dr 6000 5,000 / Cr 2100 5,000, vendor Sam, no method tag), line
by line, first match wins:

- Cr 2100 (−5,000 of credit): the entry has no method, so rows 1 and 2
  (the *entry's* method) do not match; 2100 is not a cash account (row 3)
  and not a financing account nor a member/platform method's account
  (row 4); it **is** a business_card method's account (row 5) → **card
  (excluded)**, 5,000.
- Dr 6000: an expense → "anything else", no payment line.
- Net of the entry's buckets: card 5,000, nothing else; no signs differ,
  so nothing goes to undetermined.

Effect on the reports (and nothing else moves):

| Report | Field | Unrecorded | Recorded |
|---|---|---|---|
| 2026 | Sam `excluded_card` | 0 | **5,000** |
| 2026 | `excluded_card_payments` | [] | **[#8 5,000, instrument null]** |
| 2026 | Sam `paid`, `undetermined`, `not_reportable` | 0, 0, 0 | 0, 0, 0 |
| 2026 | Sam `status`, `reportable`, `decision_needed` | under_threshold, false, false | unchanged: F = 0, U = 0 |
| 2025 | everything | — | unchanged (#8 is dated 2026-04-01) |

Why nothing else moves: every other payment is out of `1000 Bank`, a cash
account, whose rule (row 3) does not depend on the card's method; #10 is
untagged and from the bank, so `untagged_payments` stays [10]; the card
method adds no vendor line and no list entry of its own; the instrument's
id appears in the report only through an entry tagged with it, and none
is.

The unrecorded case (the earlier version of this golden) is kept as a note:
with no method recording 2100, #8 is "anything else": `paid` 0,
`excluded_card` 0, the list empty, Sam under_threshold. That is correct per
B-19 ("Cards and party totals") but it exercises no card figure, so the
parity row's "card exclusion" claim was not covered. The unrecorded card's
charge counting for no one is still pinned by
`cash_paid_per_vendor_and_who_is_reportable` (Sam `paid` 0, and the
exclusion buckets 0 on the unrecorded `books()`) and by the first half of
`card_account_payments_are_listed_not_dropped`.

## Fields fixed by the fixture

Names sort by name then vendor id: Ace Rentals (2), Bigco (4), Dana (1),
Sam (3) in 2026; Ace (2), Dana (1), Sam (3) in 2025. Each `decisions` list
is the one decision from the beginning, with its source (Riley Chen,
2026-01-05). `tin_on_file`: Dana true, Sam true, Ace false, Bigco false.
`through` is null (the whole calendar year).
