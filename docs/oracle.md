# The semantic oracle: how we prove the books are right

storytold's clones are verified with a *pixel oracle*: re-render a PSD and
diff it against Photoshop's own embedded merged composite, passing at
≤ 2/255 per channel. We steal the shape of that idea and change the domain:
our oracle is *semantic*, not visual.

## The core loop

1. **Fixture books.** A canonical set of journal entries exercising a feature
   (the January 2026 fixture company in `tests/invariants.rs` is the first).
2. **Two engines.** The same fixtures go through our engine and through
   **GnuCash** — the open-source, double-entry reference implementation with
   open file formats. GnuCash is both the spec and the oracle.
3. **Exact diff.** `reports::snapshot()` renders trial balance, P&L, and
   balance sheet as canonical JSON. The outputs must be **identical** —
   decimal strings, no floats, no locale, no epsilon. Where storytold allows
   2/255 of perceptual slop, we allow zero: accounting has no "close enough".

## The three oracle layers (today → future)

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

**Layer 5 — GnuCash diff harness (roadmap phase 3).** A `ledger-oracle` crate
will run fixture books through our engine and through GnuCash headless, then
diff `snapshot()` output automatically — `cargo xtask oracle`. This is the
direct analog of storytold's `cargo xtask corpus` / `oracle_diff`. Note the
distinction: GnuCash is the *behavioral* oracle, not a file-format target
(the owner: "for file format import/export I could not give a single shit
about GnuCash XML — QBO is all that matters"). The harness feeds fixtures to
GnuCash via whatever it imports reliably (QIF/OFX) or its Python bindings;
the exact mechanism is decided at implementation time. Until the harness
exists, Layers 1 and 2 are the oracle, and they are not optional.

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
