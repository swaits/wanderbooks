# Claims: partner

Wanderbooks' claims for bookspec area **partner**: what each rule requires, where the engine enforces it, and which tests check it. Being reduced to the engine's pick and its evidence as P14 unit S-D lands; the accounting text moves to bookspec's `spec/partner.md`.

Rules: B-13, B-26, B-35, B-36.

---

## B-13 Partners share income by the partnership agreement; capital accounts carry it

**bookspec:** `BS-partner-1`

**Status:** AUTHORITY (shares; capital accounts; a count ends where the
members or shares change: §706(d)(1), Treas. Reg. §1.706-4(a)); CHOICE
(cent rounding, the owner 2026-10-13); CHOICE (counted from the company's
first close, D-10, 2026-10-16); CHOICE (the statement splits unclosed
earnings the same running way, D-38, 2026-10-20)

- 26 U.S.C. §704(a) (`sources/irs-partnerships.md`): "A partner's
  distributive share of income, gain, loss, deduction, or credit shall,
  except as otherwise provided in this chapter, be determined by the
  partnership agreement."
- Treas. Reg. §1.704-1(b)(2)(iv)(b) (`sources/irs-partnerships.md`): each
  partner's capital account "is increased by … (3) allocations to him of
  partnership income and gain" and "is decreased by … (7) allocations of
  partnership loss and deduction", beside the money and property
  contributed and distributed. A capital account is one running balance of
  contributions, allocations and distributions: the closes are the
  allocations.
- Form 1065 Schedule M-2: "Show what caused changes during the tax year in
  the partners' tax-basis capital accounts." Schedule K-1 Item L reports
  each partner's capital account "using the tax-basis method"; that
  analysis "is based on the partnership's books and records" (the Partner's
  Instructions' Basis Limitations section, `sources/irs-partnerships.md`;
  not Item L, which an earlier version of this rule spliced in). The K-1
  worksheet carries the book capital analysis, and beside it the tax-basis
  capital as far as the books know it (B-36).
- **No authority** says how to round a share to the cent. Largest
  remainder, ties to the member listed first: CHOICE (the owner,
  2026-10-13). Closes made before then replay with the last member
  absorbing it.
- **Members closes split cumulatively, counted from the company's first
  close** (P3 defect 17; D-10). A *count* is the members closes with one
  set of (capital account, share) pairs, from the first such close since
  the last members close with another set: closes to retained earnings or
  to one account are skipped (they allocate nothing to the members), and
  neither the members' order nor their distribution accounts matter. Each
  close gives member *i* allocate(C + net)*ᵢ* − given*ᵢ*: their split of
  everything the count has closed (C before it, plus its own net), to the
  base currency's minor unit (cents, whole yen, thousandths of a dinar;
  B-3; for a code ISO 4217 gives none, such as gold, the finest a line
  carries, `kernel::MAX_LINE_SCALE` places, so every amount is still whole
  units), less what the count's earlier closes gave them under whatever
  rule they were made. A tied odd unit goes to the member the count's
  first close listed first, whatever order later closes list the members
  in, so a reorder of the members moves no cent; the statement below
  breaks ties the same way, on a day before a count's first close in the
  order of the count the next members close begins. Therefore each close hands out
  exactly its net; after every close that posts, each member's total from
  the count is one split of the count's total, under one minor unit from
  their exact share; and rounding by closes made under earlier rules is
  trued up by the first close that posts (D-10: from inception; listed in
  the close's `members`). The true-up is under a unit per member for each
  break in an earlier run and each per-close close; a last-absorbs close
  rounded each member but the last to within half a unit, and the last
  absorbed up to half a unit for each of the others (a unit with three
  members, more with four or more).
  - A change of members or shares starts a new count (26 U.S.C.
    §706(d)(1): a year's distributive shares are determined by a method
    "which takes into account the varying interests of the partners in the
    partnership during such taxable year"; Treas. Reg. §1.706-4(a)(3)(vii):
    the partnership "shall treat each segment as though the segment were a
    separate distributive share period"). Income closed under
    the old shares is never split again under the new ones. So the bound
    holds per count: across k changes a member's total can be up to one
    unit per count from the sum of their exact shares (the D-10 lifetime
    bound holds while members and shares are unchanged).
  - A true-up rides only on a close that posts an entry (income to close,
    or a distribution to sweep): a close that posts nothing credits no one,
    and the true-up waits for the next close that posts (close INVARIANT
    5: never post a closing entry when there is nothing to close).
  - The bound's precondition: every amount is a whole number of minor
    units (B-3: every posted line is), and every product |total| × share
    is exact. A product Decimal would round (a share of many decimal places
    times a large total) is refused, never split
    (`periods::allocate_cumulative` INVARIANT 4). Replaying a journal from
    before B-3, a sub-unit fraction can make the bound two units.
  - A year's income per member is the difference of two running totals,
    each under a unit from exact, so under two units from share × the
    year's income; the return is in whole dollars ("Rounding on the
    return", `sources/irs-partnerships.md`).
  - A period closes at the shares in force at the close: a share change
    inside a period isn't split at its date (§706(d) and §1.706-4 would);
    close the period at the change.
  - Journaled `"rounding": "cumulative_since"` with `"since_seq": N`, the
    close the count starts at, under the journal feature
    `members_cumulative` (B-27); replay derives N again and refuses a close
    journaled with another. Closes journaled `"cumulative"` (the run rule of
    2026-10-13 to P3: a back-to-back run with the same members in the same
    order, restarted by any other close, so a tied cent could go to the
    same member at every restart), `"largest_remainder"` (each close on its
    own) and none (the last member absorbs) replay under their own rules,
    never changed (B-8).
- A capital account is one running balance (§1.704-1(b)(2)(iv)(b)). So the
  statement of members' equity, the K-1 worksheet and the viewer split
  undistributed earnings U as of each date the way the count's next close
  would split them: allocate(C(d) + U(d)) − allocate(C(d)), C(d) being what
  the count's closes dated on or before d closed (D-38; P3 review R-1). A
  member's capital on a date never moves when a period ending on or before
  it is later closed; a true-up of rounding by closes made under an earlier
  rule shows only once a close posts it, dated at that close. A
  statement's beginning is as of the day before its period, its ending as
  of its last day: a member's ending on one statement is their beginning on
  the next, to the unit, and a member's period income is the roll-forward's
  remainder (their share within a rounding unit at each end).
- A correction undoes what it corrects (B-9): an entry and its reversal
  both in a statement's span are neither contributions nor
  distributions (`reports::cancelled_in_span`; the K-1 worksheet reads
  the same). The K-1 worksheet's contributed property (Item L) is what
  the member owned and handed over, with its own basis (§723): a prepaid
  service contract credited straight to capital is property (a contract
  handed over), while a prepaid service they paid a vendor for on the
  company's behalf, booked through Due to Shareholders (Dr prepaid / Cr
  2100, then Dr 2100 / Cr capital), is cash (CHOICE, the owner,
  2026-10-15); and it is shared among
  members credited in one entry in proportion to their credits
  (`k1` INVARIANTS 3, 4, 6).

**Enforced:** `CloseTo::Members`, `CloseTo::MembersCumulative`,
`Ledger::members_regime`, `Regime::allocate`,
`periods::allocate_cumulative`, `kernel::apportion_cumulative`,
`periods::allocate`, `kernel::apportion`, `members_equity`,
`Store::close_period_to` (the feature it needs, B-27), the store's
`close_to_from_journal`. **Checked:** Kani `apportion_is_exact`,
`apportion_is_fair`, `apportion_cumulative_is_exact`,
`cumulative_close_never_drifts_a_unit_at_halves`, `_at_tenths`,
`_at_near_thirds`,
`cumulative_closing_entry_balances`; Lean `runPart_telescopes`,
`closePart_total`, `trueUp_total`, `trueUp_running`,
`closePart_eq_trueUp`, `cumulative_drift`, `statement_after_close`,
`statement_unmoved_by_close`, `Apportion.rem_pos_of_rank_lt`,
`apportion_drift`, `allocW_sum`, `allocW_drift`,
`allocW_cumulative_drift`, `allocW_cumulative_total`; properties
`prop_split_*`, `each_members_capital_chains_across_statements`,
`each_member_stays_within_a_cent_of_their_share`,
`members_capital_never_drifts_a_cent_from_their_share`,
`each_members_capital_chains_and_holds_through_closes`,
`period_ends_rise_with_close_order`, `apportion_cumulative_agrees`,
`allocate_cumulative_agrees_with_the_integer_model`;
`the_audit_odd_cent_never_piles_up_across_closes` and the rest of
`ledger-core` `tests/odd_cent.rs`; `a_run_of_closes_never_piles_up_rounding`,
`a_different_close_ends_a_legacy_run`,
`a_different_close_no_longer_restarts_the_count`,
`members_closes_replay_under_the_rule_they_were_journaled_with`,
`ledger-store` `tests/member_closes.rs` and the old-books fixture
`x_member_closes`; `closes_journaled_under_the_old_rule_replay_with_it`;
`ledger-mcp` `close_period_shows_each_members_running_split`,
`old_books_name_each_closes_rule_and_true_up_at_the_next_close`;
`ledger-gui` `a_members_page_shows_the_same_capital_before_and_after_a_close`,
`a_yen_members_page_splits_in_whole_yen`; the bookspec examples
`members-close.*`.

## B-26 Parties: one directory, no full TIN in the books

**bookspec:** `BS-partner-2`

**Status:** AUTHORITY (truncated TINs: `sources/irs-truncated-tin.md`);
CHOICES (the owner, 2026-10-15)

Everyone the company deals with is a party, with roles (vendor,
contractor, customer, employee, member, payout platform, tax agency,
financial institution, landlord). Vendor and customer records link to
their party; entries are tagged with theirs. Names are unique among parties
not archived, ignoring case and surrounding space. A W-9 is recorded as its
facts and the TIN's last four digits (the part a truncated TIN shows,
Treas. Reg. §301.6109-4(a)); no party field may hold a nine-digit number,
and the full TIN lives only in the encrypted vault, outside the journal
(CHOICE). W-9 facts need a 1099 role (contractor, landlord); W-4 facts
need the employee role; payroll lines that name their employee name a
party with that role. A project's text (its name, description, links and
notes: B-37) holds no full TIN either (`settings::line_text`,
`long_text`). A link is a web address, so its rule reads it as one
(`settings::holds_printed_tin_in_link`, CHOICE of this rule, so store
listings can be kept): the printed SSN (3-2-4) and EIN (2-7) forms and a
bare nine digits standing alone are refused, but nine digits joined to a
letter or to `/ = ? & #` are an id in a path or query (an app store
listing's `.../id284882215`) and are kept. A project's name keeps the
plain rule: a nine-digit job or PO number is written by its last four,
as everywhere else in the books.

The full TIN, when the books need it for the 1099 worksheet, lives in the
TIN vault (`vault/` in the book's own folder in the data root, encrypted
under the book's own key, `keys/legacy.key` beside it): `books.set_w9`
`tin` writes it there and
journals only the last four and the vault record's hash; the signed W-9
goes there too, never to the attachment store. Any party may have its
TIN in the vault, whatever its roles: every member's (for their K-1) and
every 1099 payee's belongs there (CHOICE, the owner, 2026-10-15: "In the
vault, that's the WHOLE reason we built it. And it needs to be in the
damn CPA package."). `books.set_tin` stores one for any party; the books
journal its TIN facts (type, last four, the record's hash:
`party.tin_set`), which never disagree with its W-9 facts' last four. No
tool output, error, or log holds a full TIN except `books.vendor_1099`
with `include_tins` (CHOICE, the owner, 2026-10-15: for the CPA, when
asked): a refusal that quotes its input masks any TIN in it
(`parties::redact_full_tins`, checked by `refusals_never_echo_a_tin`),
and a party text field refuses nine digits however they are separated
(`has_full_tin`: up to three spaces, hyphens, dots, slashes or
underscores between neighbors, in any script's digits, a run of spaces
counting once however long it is; and a comma or a parenthesis between
a TIN's 3-2-4 or 2-7 groups (CHOICE, P7, 2026-10-26), so
`123,45,6789` and `(123) 45-6789` read as a TIN while an amount's
comma-grouped thousands do not, `1,234,567.89` being 1-3-3-2; past ASCII
every Unicode space and dash and the fullwidth and small forms count, and
invisible characters such as a zero-width space or a right-to-left mark
don't count toward the three: `sources/unicode-tin-text.md`, Unicode
17.0). Files holding one (`books.get_w9`, `books.vendor_1099`'s
`csv` with `include_tins`, and the CPA package, whose K-1 and 1099
worksheets carry every member's and payee's TIN by default) are never
written to a path: no tool takes one (D-29, P2 slice 6). Each is handed
over as an export the asking token fetches with `wanderbooks download`
(kept 15 minutes), never inline (D-44; since D-46 no tool takes or
gives any file's bytes inline: files come in only by upload and go out
only by download). (Until slice 6 they were
written only outside the books dir: CHOICE, P0 review, 2026-10-16, so
none could be filed back in as a receipt and read out again.) No file
holding a full TIN is filed into the books
(CHOICE, P0 review round 2, 2026-10-16): the server keeps, beside the
vault, the SHA-256 of every file it read or wrote with full TINs (the W-9
`books.set_w9` reads, recorded once the call has passed every check, just
before the vault keeps it, so a refused call marks no file; each
`books.get_w9` copy;
a 1099 worksheet with TINs; a CPA package with TINs and each file in it
that holds one), shared by every session and every later server for the
books, and every way a file enters the books (`books.attach_receipt`,
`books.file_document`, by upload, the only way since D-46) refuses those
exact files,
and refuses any text file (by its type or name, or because it decodes as
UTF-8 or UTF-16, with a byte-order mark or, NUL-interleaved, without one)
holding (a) a TIN the vault holds for a party of the
books, anywhere in the text, as nine digits in a row, as 3-2-4 digits
joined by hyphens, spaces, dots or commas, or as 2-7 digits joined by a
hyphen, a space or a comma (compared on the server against the decrypted
vault, never returned or logged; a vault that can't be opened leaves (b)
alone), or
(b) a number printed as an SSN or EIN is, standing alone: 3-2-4 digits
joined by hyphens or commas (123-45-6789, 123,45,6789) or 2-7 joined by a
hyphen or a comma (12-3456789, 12,3456789), with no digit, and no hyphen
or dot followed by a digit, on either side,
except the company's own EIN as its settings hold it, printed 2-7: it is
the company's, no person's, the settings already hold it in full (B-29),
and every company document goes into the books, its formation papers and
plans printing it (CHOICE, the owner, 2026-10-17); (a) still refuses it
should the vault hold that number for a party. Full-width digits and
Unicode hyphens and spaces count as their ASCII kin (`tin_files.rs`). The text check is narrow on purpose (CHOICE,
the owner, 2026-10-16): bank exports are filed for reconciliation, and
their routing numbers, long references and dates next to amounts are
not TINs; the broad `has_full_tin` reading stays on what the server says
(refusals and results). Because (a) compares against the vault, its
answer would tell an agent whether a text it chose holds a vault TIN;
the tools show each TIN's last four, so an agent filing texts full of
nine-digit numbers ending in those four could find the TIN in about 17
filings. So (a) answers for at most two candidates a filing (CHOICE,
P0 review, 2026-10-16): before the vault is opened, a text holding more
than two distinct TIN-shaped numbers (in any rendering (a) reads; one
number written several ways counts once) that end in the last four of a
TIN the books know (each party's TIN facts and W-9 facts, read from the
books; once the vault is opened, its own records' last fours too) is
refused with words of its own ("more than two numbers shaped like a
TIN"), the same byte for byte whether or not one of them is a vault TIN
and whether or not the vault is open; only a text with no more than two
is compared against the vault. With the last four known, a TIN is one of
100,000 numbers, so finding one this way takes on the order of 50,000
filings, each one refused and seen. What this can't catch: a changed
copy of a binary file (a W-9 PDF re-saved or re-scanned), a TIN in an
image, or a TIN the vault doesn't hold written other than (b);
`plan/served.md` P2 and P7 take the path tools and full TINs out of the
agent's reach. A known limit, open until then: (a) still answers for two
candidates a filing, so an agent that makes those 50,000 refused filings
can find a TIN; P2's access log records every call, refusals included,
and P7's human-only TIN channel takes full TINs, and with them this
check, off the agent's path. A bank export holding three numbers that
happen to end in a known last four is refused the same way (filed as a
PDF instead).
A locked vault or a missing TIN never fails
the package: it says which TINs are missing. `books.purge_tin` deletes a
party's vault records and journals that it did. Wherever the books show a
party's last four (the directory, `books.party`, the 1099 and K-1
worksheets, the purge preview) it is the TIN facts', else the W-9
facts' (`Party::tin_last4`; the two never disagree; exactly four digits
or nothing, however the facts were built), kept after a purge
(P3 defect 11: the directory read W-9 facts only, so a member's read as
missing).

**Enforced:** `crate::parties` (has_full_tin, check_text, W9Facts::check,
W4Facts::check), `Ledger::add_party`, `update_party`, `set_party_role`,
`set_party_w9`, `set_party_w4`, `set_party_tin` (TinFacts::check; TIN and
W-9 facts agree), `set_entry_party`, `validate_payroll`; `ledger-core`
`k1` (the K-1 worksheet); `ledger-vault` (Tin::new, the encrypted
records, records_held, purge_records); `ledger-mcp` `books.set_w9`,
`books.set_tin`, `books.get_w9`, `books.purge_tin`, `books.vendor_1099`,
`books.export_cpa_package` (open_vault opens the book's own key;
vault_tin checks the vault against the books); `ledger-mcp` `tin_files`
(the registry, the text
scan and its probe bound, on every filing tool; `known_tin_last4` gives
the books' last fours).
**Checked:** `ledger-core` `tests/spec_parties.rs`; `ledger-store`
`tests/parties.rs`; `ledger-vault` `tests/vault.rs`; `ledger-cli`
`tests/spec_parties.rs`, `tests/spec_vault.rs` (every response, every
file handed over, the server's log, and every file in the books' folder
outside `vault/` and the files kept for their download scanned for the
TINs in any formatting), `tests/tin_files.rs`
(`a_tin_bearing_file_is_refused_from_every_session_and_after_a_restart`,
`text_with_a_vault_tin_or_a_printed_one_is_refused_and_a_bank_export_is_not`,
`text_probing_for_a_tin_is_refused_alike_by_every_filing_tool`), and the
unit tests in `tin_files.rs`.

## B-35 A party's part of an entry, and a party's totals

**bookspec:** `BS-partner-3`

**Status:** CONVENTION (the sign: B-2's, a line on the party's own
account); CHOICE (the owner, 2026-10-15: `entry_flow`, money paid down
counted once; the owner, 2026-10-20, P3 design settled with each review's
recommendation: a member's entry counts what moved their capital, money
is cash, cards, the members' financing and loans, and a liability credit
never counts as paying that liability's own creditor); AUTHORITY for what
a member's capital counts (Treas. Reg. §1.704-1(b)(2)(iv)(b), (c);
Form 1065 Schedule K-1 instructions, Item L:
`sources/irs-capital-accounts.md`) and for a promise not being payment
(Helvering v. Price, 309 U.S. 409 (1940):
`sources/helvering-v-price-1940.md`). Not ASC 230's cash (see item 8) and
not the books' accounting method (`books.accounting_basis`).

1. **Sign.** A party's part of an entry is signed as a line on the
   party's own account would be (debit-positive, B-2): positive, the
   company paid it or gave it value; negative, the company received from
   it. A bill is negative (owed to the vendor), an invoice positive.
2. **Direction, cash, in kind.** Every part carries its direction (paid,
   received, neither) and what of it moved as money (cash) and as property
   (in kind); what is neither is a promise still to be settled (a bill, an
   invoice, a fee on the party's own card, a distribution still owed) or
   moves nothing with anyone (a close), and counts in no total. A part's
   cash and in kind have its sign and together never exceed it; what of a
   paid part is neither (a distribution partly still owed) is a promise,
   counted when it is paid.
3. **What the shares split** (`shares::target`), in order:
   1. A period's closing entry: its parties' own-account movement (the
      members' equity, when only members are named), else its total
      debits; neither (B-11, B-13: a close allocates income, it moves
      nothing with anyone).
   2. A reversal: its original's target, negated, paid and received
      swapped, from the root original down (B-9: "a mistake is corrected
      by an opposite entry"); never re-read from its own lines, because
      `entry_flow` isn't odd when one liability pays down another (a
      member paying the card from their own funds, reversed, read as a
      second payment: P3 design §1.4).
   3. Every party named has the member role in the directory and the
      entry moves equity other than retained earnings (E): E, a
      contribution received (E < 0) or a distribution paid (E > 0).
      - Of a contribution, in kind is the property in it, at most |E|:
        what asset accounts that are neither cash nor receivables took in
        (the K-1 worksheet's test, k1 INVARIANT 3), so property
        contributed subject to a loan is the member's net property
        (§1.704-1(b)(2)(iv)(b)(2)). The rest is cash, as the K-1 counts
        it: money, or what the member paid for the company (an expense
        booked straight to capital, a debt to them converted,
        §1.704-1(b)(2)(iv)(c)(1)).
      - Of a distribution, cash is the money it paid (case 4's net, at
        most E: §1.704-1(b)(2)(iv)(b)(4)) and in kind the property it
        gave up, at most what is left, as the books carry it (the
        statement's figure; (b)(5) counts its fair market value, which
        stays with the CPA). The rest, credited to a liability that
        isn't money for the entry (distributions payable, or what the
        members advanced once they are named its creditors), is still
        owed: it counts as nothing until it is paid (Helvering v. Price;
        before the P3 review a distribution declared to distributions
        payable counted when declared and again when paid, and a
        distribution of money with depreciated property read the money
        as property).
      Read from journaled facts only (roles, the chart, creditors), so a
      member leaving `[[company.members]]` never changes a posted entry;
      the settings' members matter only to a derived split among several,
      which the server journals as amounts when the entry posts.
   4. Money moved (`parties::entry_flow` net, item 5's accounts less the
      liabilities owed to a party named): the net, paid (+) or received
      (−), all of it cash.
   5. Otherwise its parties' own-account movement (payables, receivables,
      a liability owed to a party named), else the total debits; neither.
   The parts split the target as `shares` INVARIANTS 7-12 say: a
   reversal's parts are exactly its original's negated, and a member's in
   kind is shared as the K-1 worksheet shares property (in proportion,
   leftover cents by largest remainder, ties to the member listed first).
4. **Totals: `basis` cash (the default) or all, always labelled.** Cash
   counts each part's cash; all adds what moved in kind (Item L counts
   money and property alike). Each part counts on its root original's
   side, so a reversal nets what it reverses, and reversing the reversal
   brings it back. A bill or invoice counts on neither basis (a promise is
   not payment: Helvering v. Price); `billed`, `invoiced`, `owed` and
   `receivable` report those. An entry that counts nothing is never
   unresolved (its target is found before its split). For a member, over
   the entries that name only members and move their capital, plus the
   payments of distributions declared to them: basis all's received and
   paid are the statement of members' equity's contributions and
   distributions for the same dates, and basis cash's received is the
   K-1 worksheet's cash contributions, when
   - each such entry names exactly the members whose capital it moves
     (derived from their capital lines, or amounts equal to them) and
     moves every one of them the same way (capital moved from one member
     to another moves nothing with the company: neither, though the
     statement shows a distribution and a contribution);
   - no distribution declared in the span is still unpaid at its end (the
     statement counts it when declared, party totals when paid); and
   - no reversal in the span undoes an entry outside it (the statement
     then counts the reversal by its side; party totals net it on the
     original's side).

   Over any span at all, even one a reversal straddles, received less
   paid still agrees with contributions less distributions, given the
   first two conditions (the property
   `member_totals_are_the_members_equity_statement` checks every span).
5. **Money** (P3 defect 24): the cash accounts (`[books] cash_accounts`;
   every asset account but receivables when none are set), the cards
   (`parties::card_accounts`, the one rule the reconciliation reminders
   and the viewer read too: the liability accounts of the business-card
   payment methods the books record, B-19; while they record none, the
   liability accounts whose names say "card" but not "gift card"; never
   the account of a member's card, a member's cash or check, or a
   payment platform, which pay through the members' financing account or
   their own balance), and `[books] financing_accounts` (what the members
   advanced, loans). Every other liability (payables, the payroll withholdings, an
   accrued expense, deferred revenue, gift cards the company sold) is a
   promise: counted when it is settled, never also when it is booked
   (before P3 an accrual through any liability but payables and payroll
   counted twice). So, before any business card is recorded, a card
   account whose name doesn't say "card" is not money until it is listed
   in `financing_accounts` or a business-card method names it, and with
   `financing_accounts` unset what the members
   advanced is not money either: a purchase through either then counts
   for no one until it is paid off (the handbook tells the agent to list
   them).
6. **A party's own liabilities.** A party may be named the creditor of
   the liabilities the company owes it itself (`creditor_of`: a card's
   issuer, a loan's lender, each member for what the members advanced;
   `party.creditor_set`, a journal feature). For an entry naming it, a
   credit to one never counts as paying it (a fee on its card, interest on
   its loan, equipment on the seller's note: neither, until paid; a
   distribution credited to the members' financing: still owed), and a
   debit to one is what the money paid for (a card payment, a
   reimbursement: paid, as before).
7. **Known limits**, stated in the tools' text: an entry that both repays
   a member and distributes to them counts the distribution only; money a
   member lends under their own name through the members' financing and
   later converts to capital counts both times (received as a loan, then
   as a contribution), as does a distribution declared through it while
   the members aren't named its creditors (paid when declared, and again
   when paid); an entry naming a payee together with the creditor of the
   liability that paid it (a vendor and the member who paid it through
   the members' financing) reads neither for both; an expense a member
   paid that is booked straight to capital and tagged with the vendor
   reads neither (the vendor isn't counted as paid; tagged with the
   member, it is the member's cash contribution). Record them apart, and
   a member's payment for the company through the members' financing,
   tagged with the payee: the house conventions already do.
8. **How party totals relate to the cash flow statement** (B-14, D-42,
   P3 defect 1). The statement reports the company's cash, ASC 230's: a
   member paying a vendor for the company is cash borrowed from the member
   and spent, on that date (a financing inflow and an operating or
   investing outflow); paying the member back is a financing outflow;
   property a member contributes, and a debt to them converted to
   capital, are noncash (disclosed); a distribution declared to a
   distributions payable is a distribution on the statement when it is
   paid (B-14 rule 2), listed under the payable ("Distributions to
   members — 2200 Distributions Payable"), not by member: the statement
   reads accounts, not parties, so the member a payment from the payable
   went to is party totals' (the payment's own parties) and the
   declaration's (the statement of members' equity, at its date). So over
   a span, the statement's distributions (by member and under the payable)
   are the statement of members' equity's less what is declared and still
   unpaid at the span's end, and party totals' paid to the members on the
   entries that move their capital or pay a declared distribution, exactly
   (`the_members_figures_agree_on_every_date`). Party totals report what moved with
   each party: the vendor is paid when the member pays it (the same date
   and amount as the statement's outflow); the member is paid back when
   the statement shows the financing outflow; and a member's capital
   counts when it moves, a conversion as the cash contribution the K-1
   calls it (the money they paid for the company), property in kind; a
   distribution counts when it is paid, as on the statement. Where both
   show a payment they agree on its amount and date. The statement shows
   more than party totals do in two places: the financing inflow from a
   member who pays a vendor is in no party's totals (the entry names the
   vendor; the member's totals show only the repayment), and an expense
   booked straight to capital and tagged with the vendor isn't counted
   as paid to the vendor (item 7). "Cash" in party totals is money
   through the company's cash accounts, cards and financing (and, on a
   member's contribution, the K-1's cash), not the statement's ASC 230
   cash.
9. **How party totals relate to the 1099 worksheet** (B-19; P3 defects
   12, 18, 23; D-40). They answer different questions, and neither is
   read for the other (`books.party` gives both, labelled). Party totals
   report what moved with each party, whoever handled the money; the
   1099 worksheet reports what the company, as the payor, paid a payee
   toward its information return. A member paying a contractor for the
   company through the members' financing account, the entry tagged
   with the contractor: in party totals it is paid to the contractor on
   that date, in money (the members' financing is money, item 5), and
   nothing moves with the member until the company pays them back, which
   is paid to the member. On the 1099 worksheet it is the company's
   payment to the contractor (Treas. Reg. §1.6041-1(e)(1): the member only
   writes the check at the company's direction): it counts toward the
   contractor's 1099 when it was the member's own cash, check or
   transfer, is excluded when it was a card, the member's own included
   (§1.6041-1(a)(1)(iv): Form 1099-K), and is undetermined until its
   payment method is recorded; the reimbursement pays no payee (money
   between the company's own sources). A company card's charge is paid
   in party totals and excluded on the 1099 alike. So the figures never
   contradict: over a calendar year, for a payee every entry of which
   names it alone, in books whose cash accounts are named, whose company
   cards are recorded as business-card payment methods, and that pay only
   out of cash accounts, cards and financing accounts, its party totals'
   paid less received (basis cash) is exactly its 1099 worksheet's
   `paid` + `excluded_card` + `undetermined` + `not_reportable`, less what
   the worksheet lists as money received from it for something else
   (a refund is netted on the worksheet and received in party totals).
   The bookspec example `1099-and-party-totals-for-a-members-payment`
   checks a year of it, and the property
   `party_totals_are_the_1099s_buckets` checks it over random books. A
   bill counts in neither until it is paid, and a payee's dated decision
   changes only the 1099 worksheet.

**Enforced:** `kernel::party_target`, `kernel::reverse_target`,
`shares::{target, split, resolve}`, `parties::{flow, money_accounts,
card_accounts, MoneyAccounts::for_parties, activity_with, Activity::totals,
Basis}`, `Ledger::set_party_creditor`, `Store::set_party_creditor` (the
`party_creditors` journal feature, asked for before its first line);
`ledger-mcp` `books.get_entry` (`resolved`, `direction`, `cash`,
`in_kind`), `books.party` and `books.parties` (`basis`), `books.update_party`
(`creditor_of`); the viewer's party pages and entry page.
**Checked:** `ledger-core` `tests/spec_party_totals.rs` (the audit's
table, the members' equity and K-1 identities over every span, the
reversed card payment, accruals, a gift card, a party's own liabilities,
distributions declared or of depreciated property, the properties), Kani
`party_target_direction_is_its_sign`, `party_target_is_odd`,
`reverse_target_is_an_involution`, bridge `party_target_agrees`; Lean
`reverseTarget_involutive`, `capitalTarget_odd`, `partyTarget_odd`,
`sumOver_movement_reversal`; `ledger-store` `tests/party_creditors.rs`;
`ledger-cli` `tests/spec_party_totals.rs`; `ledger-gui` `tests/parties.rs`.

## B-36 The K-1 worksheet reports tax-basis capital beside book capital

**bookspec:** `BS-partner-4`

**Status:** AUTHORITY (`sources/irs-partnerships.md`: Instructions for
Form 1065 (2025), Item L, "Tax-basis method"; required from tax year
2020: Instructions for Form 1065 (2020), Item L, "you must use the tax
basis method this year"); CHOICE D-41 (the owner, 2026-10-20: "the
K-1 worksheet reports tax-basis capital ... the contributed property at
its §723 carryover basis, beside the book figures")

Schedule K-1 Item L reports each member's capital account on the tax
basis, by the transactional approach: beginning capital (last year's
ending), "the amount of cash plus the adjusted tax basis of all property
contributed", the member's share of income "as figured for tax
purposes", other increases and decreases "for tax purposes", and "the
amount of cash plus the adjusted tax basis of all property distributed".
The K-1 worksheet shows, per member, each Item L line on the book basis
(the members' equity roll-forward, B-13) beside the tax basis, and how
book capital reconciles to tax capital:

- Property contributed enters at its §723 basis (B-25): the member's
  share of the property's book value less their share of its built-in
  gain or loss at contribution, shared within the entry as its book value
  is (B-13). To the cent, these shares (and those of property without a
  basis, of a prepaid at book, and of property distributed) run
  cumulatively, D-38's rule (the owner, 2026-10-22): each member's
  running share is the largest-remainder rounding of their exact running
  share (ties to the member listed first), less what earlier entries
  gave them, so it is never a cent from exact, and tied odd cents
  alternate rather than all going to one member (three equal odd-cent
  properties at 50/50 leave the members a cent apart, never three). How
  the count runs differs from the closes': a close splits by the
  members' shares among all the members, so its count starts afresh when
  members or shares change; here each entry splits by its own amounts
  (each member's property in it, or what each took) among the members
  with a part in it. So there is one count for each set of members with
  a part in an entry, and it never restarts: shares don't enter it, and
  a member outside the set is never moved by it (Item L's capital
  contributed is the basis of the property that member contributed, so
  a member who contributed nothing in an entry takes none of its
  cents). The count takes every posted entry from the books' first in
  date order (ties in posting order), whatever span the worksheet is
  for, and an entry's reversal takes the original's split back out of
  the count, each member exactly their part: an entry's cents are the
  same in every span it is in, and next year's beginning is this year's
  ending to the cent. The book property each member contributed in an
  entry (B-13's share of the entry's property among the members'
  credits) stays split entry by entry, ties to the member listed first:
  each member's property there must stay within their own credit in the
  entry, which a running true-up could break; its odd cents arise only
  in an entry part cash and part property whose credits don't divide
  the property evenly.
  Property contributed with no basis in the books (no
  registered asset linked to the entry holds a contribution basis for it)
  is open. Property contributed to a prepaid account (the balance
  account of a prepaid-expense schedule: a service contract the member
  handed over, B-13) with no fixed asset for it enters at its book
  amount, with a note: a contract's carryover basis is what the books
  recorded for it (CHOICE, P3 review 2026-10-21; the CPA to confirm).
- What the books can't know is never guessed. The share of net income
  "as figured for tax purposes" is the return's, always: the books are
  kept on their own basis, and every book-tax difference (Schedule M-1)
  is the CPA's; so is the ending, and the beginning of any year after
  the first (Item L: "the partner's ending capital account as
  determined for last year"). The line names the differences the books
  do see: the tax depreciation of every fixed asset in service (MACRS,
  and for contributed property the §704(c) method's allocation: the
  CPA's, B-15), and the tax gain on a disposal, with a contributed
  property's built-in gain or loss at contribution, which that gain must
  take account of (IRC §704(c)(1)(A)). Also open: the adjusted tax basis
  of property distributed, and the tax treatment of a change in book
  capital the books can't explain. Each such line is left blank and
  marked open, with what the books know so far and the book amount the
  CPA's figure replaces.
  The book depreciation the tax figure replaces is the period's
  depreciation expense, the P&L's: each asset's runs and what its
  disposal entry posts to its expense account (B-15 rule 8).
- The reconciliation: book ending less tax ending (as far as the books
  know it) = beginning (book less tax) + the built-in gain or loss share
  + property without a basis (at book) − property distributed (at book),
  exactly.

**Enforced:** `ledger_core::tax_capital::tax_basis_capital`;
`ledger-package` `k1_tax` (`k1-tax-basis-capital.csv` in the CPA
package); the odd cents, `ledger_core::tax_capital::RunningSplit` on
`kernel::apportion_cumulative`. **Checked:** `ledger-core`
`tests/tax_capital.rs` (`tied_odd_cents_of_property_basis_run_cumulatively`,
`a_member_outside_an_entry_never_takes_its_cents`,
`an_entrys_odd_cent_is_the_same_in_every_span`,
`the_split_runs_in_date_order`, the law
`a_running_split_never_drifts_a_cent`), the unit tests in
`src/tax_capital.rs`, `bookspec/examples/partnership-k1-tax-basis-odd-cents.toml`;
`ledger-package` `tests/package.rs`
(`the_k1_worksheet_reports_tax_basis_capital`), `tests/p3_seams.rs`
(`depreciation_is_one_figure_everywhere`,
`the_cpa_package_as_of_each_date_is_the_tools_figures`).

