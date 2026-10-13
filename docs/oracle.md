# The semantic oracle: how we prove the books are right

storytold's clones are verified with a *pixel oracle*: re-render a PSD and
diff it against Photoshop's own embedded merged composite, passing at
≤ 2/255 per channel. We steal the shape of that idea and change the domain:
our oracle is *semantic*, not visual.

## The core loop

1. **Fixture books.** A canonical set of journal entries exercising a feature
   (the January 2026 fixture company in `tests/invariants.rs` is the first).
2. **More than one engine.** The same fixtures go through our engine and
   through independent implementations: **GnuCash** (open source, open file
   formats), the old Google Sheet, and later QBO/Xero exports. These are
   *cross-checks*, not authorities (see "Who is right" below).
3. **Exact diff.** `reports::snapshot()` renders trial balance, P&L, and
   balance sheet as canonical JSON. The outputs must be **identical** —
   decimal strings, no floats, no locale, no epsilon. Where storytold allows
   2/255 of perceptual slop, we allow zero: accounting has no "close enough".

## Who is right

No other program is the spec. GnuCash, the Sheet, QuickBooks: each is
*an* oracle, and each can be wrong. The order of authority:

1. **Accounting principles, stated as proofs.** The laws in
   `docs/verification.md`, proved in Lean and model-checked by Kani. They
   do not depend on any other implementation's behavior.
2. **Golden files recomputed by hand** from those principles (Layer 1).
3. **External oracles** (GnuCash, the Sheet, QBO/Xero exports). A
   disagreement with one is a *finding*: work out from principles which
   side is wrong and write the reasoning down. Never change our output just
   to match theirs. If theirs is wrong, add a test that pins our correct
   behavior, and note their bug in `docs/parity.md`.

Nothing in this repo may rest on an external oracle's behavior alone.

## The oracle layers (today → future)

**Layer 1 — Golden files (live now).** `tests/golden/*.json` hold
hand-verified expected reports for canonical fixtures. The golden test fails
on any behavior change, intentional or not. Intentional changes require
recomputing the goldens *by hand from accounting principles* and documenting
the recomputation in the commit. This is the seed of everything below.

**Layer 2 — Property tests (live now, growing).**
`crates/ledger-core/tests/properties.rs` uses `proptest` (with shrinking)
to assert the accounting laws over hundreds of random inputs: trial balance
always balances, the accounting equation holds, reversals restore zero,
posted entries are immutable and JSON-round-trip clean, and depreciation
schedules are exact to the penny. This layer caught a real bug on its second
case: partial first/final months in different-length calendar months don't
complement, so the lifetime total drifted by cents (fixed with the
final-month true-up in `depreciation_schedule`).

**Layer 3 — Mutation testing (live now).** `cargo mutants -p ledger-core`
(228 mutants as of 2026-10-10: 159 caught, 28 unviable, 2 timeouts from
induced infinite loops, 39 missed). Every *missed* mutant is triaged:
real gaps get new tests (this pass added parent-account validation,
wrong-typed asset links, zero-proceeds disposal, two-asset ids, 1-month
life, `Money` arithmetic, `is_balance_sheet`, `movements()`), equivalent
mutants are documented as such (e.g. `days_held > 0` vs `>=`: the zero
case is unreachable), and untestable-by-construction mutants are noted
(e.g. `TrialBalance::balanced -> true`: the engine cannot produce an
unbalanced ledger to falsify it).

**Layer 4 — Fuzzing (live now).** `cargo fuzz` targets in `fuzz/`
(nightly, libfuzzer): `fuzz_journal_line` (journal parsing never panics;
canonical form is a fixed point), `fuzz_entry_validate` (a validated entry
always balances exactly), `fuzz_op_sequence` (arbitrary op sequences via
`arbitrary`; the trial balance always balances). 11.5M execs, zero crashes
as of 2026-10-10. Corpora persist in `fuzz/corpus/` for regression.

**Layer 4½ — Formal proofs (live now).** The accounting arithmetic lives
in one generic kernel (`kernel.rs`). Kani model-checks it exhaustively
within bounds, Lean 4 proves the laws for books of any size (trial
balance, accounting equation, reversals, posted-is-forever, frozen closed
periods, closing to net income), and a property test bridges the two.
`just prove`; see `docs/verification.md`. Its first pass found three
engine bugs that Layers 1–4 had all missed.

**Layer 5 — Cross-implementation diff harness (live: GnuCash).**
`just oracle` builds GnuCash 5.10 in a pinned container
(`oracle/gnucash/`) and runs `tests/oracle_gnucash.rs`: 200 random books
built through wanderbooks (entries, foreign lines, reversals, closes),
every account's balance at month ends and random dates, and each month's
P&L, compared exactly (15,800 comparisons). Its first run found one real
difference, adjudicated from principles: wanderbooks kept sub-cent
amounts on foreign lines; it now books in cents (rules.md B-3). The
original plan follows. A
`ledger-oracle` crate will run fixture books through our engine and through
other implementations (GnuCash headless first), then diff `snapshot()`
output automatically: `cargo xtask oracle`. This is the direct analog of
storytold's `cargo xtask corpus` / `oracle_diff`, with one difference: a
diff does not mean we are wrong. Each one is adjudicated from principles
(see "Who is right"). Note also: GnuCash is *a* behavioral cross-check, not
a file-format target
(the owner: "for file format import/export I could not give a single shit
about GnuCash XML — QBO is all that matters"). The harness feeds fixtures to
GnuCash via whatever it imports reliably (QIF/OFX) or its Python bindings;
the exact mechanism is decided at implementation time. The proofs and
Layers 1–4 are the foundation and are not optional. The harness adds
evidence. It never overrides them.

## What the oracle does NOT cover (yet)

- **Workflows**, only semantics: that an invoice *posts the right lines* is
  oracle-covered; that the invoice *screen* is usable is a UI concern for the
  egui phase, where storytold's screenshot-in-the-loop comes back.
- **Tax law.** The engine computes what you tell it; it does not know the
  tax code. Tax-adjacent logic gets its own fixtures and its own goldens
  when it arrives, sourced from IRS publications — never from memory.

## Clean-room boundary

GnuCash is GPL. We read its documentation, its file formats, and its
*behavior*. We never copy its code, and no GPL code enters this repo
(MIT OR Apache-2.0). If an agent is ever unsure whether something is
"learning from behavior" vs "copying implementation", it stops and asks.
