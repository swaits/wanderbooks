# Golden worksheet — draft states, discard, and the one evidence gate

Rules: `docs/spec/rules.md` B-43 (a draft waits on its evidence; one that
will never post can be discarded), D-47 (a reversal needs no receipt of its
own: its evidence is its original's, D-12), B-12 (a period with an open
draft cannot close), B-9 (corrections are reversing entries).
Golden: `crates/ledger-store/tests/golden/draft_states_and_evidence_gate.json`.
Test: `golden_draft_states_and_evidence_gate` in
`crates/ledger-store/tests/golden_drafts.rs`.

Parity rows this serves: "Draft states and discard" and "One evidence gate
(`evidence::requirement`)".

## Fixture

A new fixture, built like `books_for_drafts` in `store.rs` (cash, equity,
one expense account, a seed posted from a receipt) and extended. It is a
`Store`, not a bare `Ledger`, because the gate's second half (does the entry
have a receipt filed on it) lives in the store. Default settings: receipts
are required strictly above 25.00 (`policy.evidence.threshold`, B-43 and the
owner's ">$25", 2026-10-12).

Accounts: 1000 Cash (Asset), 3000 Owner's Equity (Equity), 6100 Software
(Expense). Period `2026-01` = 2026-01-01..2026-01-31 (period id 1).

| # | Date | Description | Lines | Becomes |
|---|------|-------------|-------|---------|
| 1 | 01-02 | Seed | Dr 1000 500.00; Cr 3000 500.00 | receipt filed, posted |
| 2 | 01-10 | Figma annual plan | Dr 6100 40.00; Cr 1000 40.00 | state `awaiting_evidence` "invoice due from Figma"; refused at the gate; later receipt filed, posted |
| 3 | 01-11 | Duplicate hosting invoice | Dr 6100 60.00; Cr 1000 60.00 | `building`; refused at the gate; **discarded** "entered twice" |
| 4 | 01-15 | Office supplies | Dr 6100 12.50; Cr 1000 12.50 | `building`, no receipt, posted (under threshold) |
| 5 | 01-20 | Hosting | Dr 6100 100.00; Cr 1000 100.00 | `building`, receipt filed while still a draft, posted |
| 6 | 01-25 | Reverse hosting (billed in error) | reversal of 5: Dr 1000 100.00; Cr 6100 100.00 | posted at once, **no receipt of its own** |
| 7 | 01-31 | Close period 2026-01 (generated) | see below | the close, accepted once 3 is gone |

Order of commands (ids are allocated at creation, so this is the id order):
create 1, receipt on 1, post 1; create 2, 3, 4, 5; state of 2; receipt on 5;
list the drafts; try to post 2 and 3 (refused); post 4, 5; reverse 5 (id 6);
receipt on 2, post 2; create the period; try to close (refused: draft 3
open); try to discard the posted entry 1 (refused); trial balance; discard
3; trial balance; close (id 7); trial balance.

## Hand computation

### What the gate requires of each entry (`evidence::requirement`, B-43 INVARIANT 2)

The decision is a pure function of the entry and the settings: it never asks
whether a receipt exists. Required when debits are strictly greater than
25.00 and the entry has no origin exemption; Exempt otherwise.

- #1 debits 500.00 > 25.00, hand entry: **Required**.
- #2 debits 40.00 > 25.00: **Required**.
- #3 debits 60.00 > 25.00: **Required**.
- #4 debits 12.50 <= 25.00 (not strictly greater): **Exempt** by the policy.
  Not part of a split group: no posted entry shares its date and description.
- #5 debits 100.00 > 25.00: **Required**.
- #6 debits 100.00 > 25.00 would be Required, but it is a reversal:
  **Exempt** (D-47, D-12).
- #7 debits 52.50 > 25.00 would be Required, but it is a period's closing
  entry: **Exempt** (derived from the books, B-43).

Whether a receipt is *on file* is a separate count, pinned beside it:
#1 1, #2 0 then 1 (filed before posting), #3 0, #4 0, #5 1, #6 0, #7 0.
The gate therefore refuses #2 and #3 when tried (Required, no receipt) and
lets #4 (Exempt), #5 (Required, has one), and later #2 (receipt filed) post.
A refusal journals nothing: the journal head is unchanged.

### Draft states (B-43 INVARIANT 3)

A state is a note: it moves no figure. Only #2 had a state set; #3, #4, #5
stay at the default `building`. The journal holds exactly two draft lines:
`entry.draft_state_set` for #2 (state `awaiting_evidence`, the note) and
`entry.draft_discarded` for #3 (reason "entered twice").

### Trial balance as of 2026-01-31, before the discard (entries 1, 2, 4, 5, 6 posted)

- 1000 Cash: debit 500.00 (#1); credits 40.00 (#2) + 12.50 (#4) + 100.00
  (#5) = 152.50; debit 100.00 (#6). Net debit 500.00 - 152.50 + 100.00 =
  **447.50**.
- 3000 Owner's Equity: credit **500.00**.
- 6100 Software: debits 40.00 + 12.50 + 100.00 = 152.50; credit 100.00 (#6).
  Net debit **52.50**.
- Debits 447.50 + 52.50 = **500.00**; credits **500.00**. Balanced. The
  accounting equation: assets 447.50 = equity 500.00 - loss 52.50.

Draft #3 (60.00) is not in any balance: drafts are not in the books.

### Close with a draft open (B-12)

The period holds draft #3, so the close is refused (`DraftsInPeriod`, period
1). Drafts #2, #4, #5 have posted, so #3 is the only one holding it open.

### Discard (B-43 INVARIANT 4)

#3 is discarded with a reason. Nothing posted moves: the trial balance is
identical to the one before (same numbers, as a boolean in the golden too);
no id is spent (next entry id is 7 both before and after: ids 1-6 were
allocated, 3 stays a hole); the draft list goes from [3] to []. A posted
entry cannot be discarded (#1 refused).

### Close, accepted (B-12, B-43 INVARIANT 4's last sentence)

Net income for January = revenue 0 - expense 52.50 = loss 52.50. The
closing entry sweeps 6100 to zero and posts the loss to Retained Earnings
(3999, created by the close): **Dr 3999 52.50; Cr 6100 52.50**, entry id 7.
It is Exempt (origin), no receipt.

### Trial balance as of 2026-01-31, after the close

- 1000 Cash debit **447.50** (unchanged).
- 3000 Owner's Equity credit **500.00**.
- 3999 Retained Earnings debit **52.50** (a loss).
- 6100 Software: 52.50 - 52.50 = 0, omitted (a trial balance shows non-zero
  balances only).
- Debits 447.50 + 52.50 = **500.00**; credits **500.00**.

## What the golden pins

`drafts_open` (the four drafts before the gate is tried), `gate_refusals`
(#2 and #3 refused, journal head unchanged), `posted_entries` (every entry
that posted with its requirement and receipt count, including the reversal
with no receipt of its own; the closing entry is pinned under
`close_after_discard`), `close_refused_with_draft`,
`discard_of_posted_refused`, `before_discard` / `after_discard` (drafts, next
id, trial balance, equal), `draft_journal_lines`, `close_after_discard`, and
`after_close`.

The requirement's `reason` / `source` wording is pinned as the code words it
(B-43 INVARIANT 2 fixes the decision and its sources, not the sentence);
a reworded message shows up as a golden diff to be re-read.
