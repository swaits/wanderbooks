# Proof notes for the kernel sweep

`docs/verification.md` is the one place the proofs are described: the law
table, the bounds, the times, and what is and isn't proved. The kernel
sweep (`plan/kernel-sweep-audit.md`) runs several units in parallel, so
editing that file from every branch would collide on every hunk. The rule
instead:

- **Each unit that adds proofs writes `docs/proofs/<unit>.md`**, where
  `<unit>` is the lowercase name of its code files (`fx`, `round`,
  `allocate`, `total`, `schedule`, `ratio`, `payroll`, `control`; `U9`
  display adds no proofs). The note lists, for that unit:
  - every Kani harness it added, with the law each proves;
  - the bounds Kani explores (lines, accounts, amounts, scales), and why
    they are enough;
  - the measured time, alone or in a `just prove` run, with the machine
    and date;
  - every bridge property in `tests/kernel_bridge/<unit>.rs`, and every
    Lean theorem it added;
  - anything the solver could not do, and what carries it instead (Lean
    at any size, a bridge property, an exhaustive unit test).
- **Workers do not edit `docs/verification.md` or `verify/lean/Audit.lean`.**
  The integrator folds the unit notes into `verification.md`'s tables and
  prose, and adds the `#print axioms` lines to `Audit.lean` in one place,
  in one edit per batch, after the batch's merges.
- A unit's note lands in the same commit as its harnesses, so the proof
  and its description are never apart in history.

`just prove` finds the harnesses by scanning `crates/ledger-core/src/proofs/`
(`mod.rs` plus every `<unit>.rs`), not one file; `just prove-list` prints
each harness, its module path, and whether it runs in `prove` or
`prove-slow`.
