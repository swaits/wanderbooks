# Roadmap

Where wanderbooks has been, where it is, and where it is going. The
detailed plan for the remaining work is `plan/served.md` (section 8.0:
status, order, hand-off); `plan/STATUS.md` says where things stand this
week. Revised 2026-11-10.

## Done

### Phase 1 — Semantics core (2026-10-10 to 2026-10-13)

The engine: exact money, five account types, balanced entries, the
immutable posted log with reversals, trial balance, P&L, balance sheet,
the cash flow statement, period close, fixed assets, schedules, tax
categories, 1099 tracking, parties, A/R and A/P, payroll (accounting
only), projects; golden files, property tests, mutation testing, fuzzing;
the formal methods bar (Kani on the kernel, Lean on the laws,
`just prove`).

### Phase 2 — AI-first interface (2026-10-10 to 2026-10-15)

MCP is the primary interface, not a cloned GUI (the owner, 2026-10-10).
`ledger-store` (hash-chained journal, SQLite projection, blob store),
`ledger-mcp` (now 122 tools on `main`, 2026-10-28), `ledger-import`
(QuickBooks CSV in; IIF, XLSX, PDF out), `ledger-package` (the CPA
package), the read-only viewer `ledger-gui` (first version 2026-10-12;
`plan/gui.md`). Capability parity 12 of 12 against the GSheet+GDrive
workflow; the Sheet was frozen as of 2026-10-11 and the books cut over
on 2026-10-20 (`docs/capability-parity.md`, `docs/parallel-run.md`, both
historical now).

### Phase 3 — Oracle harness (partly done)

Done: the GnuCash differential (`just oracle`: 200 random books, 15,800
exact comparisons, adjudicated from principles; `docs/oracle.md` Layer
5), the mutation, fuzz and proof layers, and the parity catalog with its
floor enforced (36 `done` rows; `parity_floor.rs` in `just check`, and
CI's `scripts/parity-floor.sh` against the base commit). Open: fixture
books diffed as `snapshot()` output automatically.

### "Served" P0 to P3, Projects, P4b S1 to S4.3 (2026-10-16 to 2026-10-27)

The books became a server that owns its data, reached only through the
MCP from anywhere, every setting in the journal, one executable, backups
that restore, the format gate, the live books moved and re-verified
(`plan/served.md` 8.1, `plan/STATUS.md`). Engine correctness (the 24
audit defects), projects, receivables and payables first class
(documents read their figures from tagged standing lines), the invoice
PDF, terms, numbers and line accounts in the core and the journal.

## Landed 2026-10-29

The wave paused on 2026-10-27, merged in order on `main`: P4b S4.4 (the
create tools preview and confirm; the party contact block, D-55), S5
(sent tracking), S6 (paying on terms: multi-document payments,
discounts, the processor fee, bills due, aging exports), S7 (bad debts),
P6 (the evidence pipeline: extraction, dedup, annotations). In flight:
nothing. Next: S4.5 (the PDF, viewer and handbook of `documents_v2`),
P11a step 0 beside it, and after P4 the owner's one upgrade of the live
books (D-57).

## Since (2026-10-29 to 2026-11-10)

The build units of `plan/served.md` 8.2 have landed; `plan/STATUS.md`
has each, by date. Then P14 (`plan/p14-bookspec.md`): bookspec became
the definitive accounting specification, its rules cited by id, its Lean
the one copy of the laws, and the kernel sweep moved every piece of
money arithmetic behind proofs. The 2026-11-09 evaluation
(`plan/2026-11-09_TRIAGE.md`) then closed what the gates did not see.

What is left is the triage's order of work (the Lean steps the command
inventory found missing, the allocation preludes, bookspec's examples
and second implementation) and the owner's own: the overnight P12 run,
the trials on a Mac, on Windows and on a clean Linux, the first release,
signing, the re-audit (P10), and Q-6 before P9b.

## The plan as it stood on 2026-10-28

Kept for the record; both tracks below have landed except where the
paragraph above says otherwise. The order was `plan/served.md` 8.0.2.
Two tracks ran beside each other:

1. **The installed app: the destination (D-56, owner 2026-10-28).**
   P11a: the platform groundwork first (the macOS and Windows keyring
   stores, the Windows data root and pipes, a macOS and Windows CI
   build; today the agent path works on Linux only), then the
   cargo-packager trial, `wanderbooks update` and `rollback`, the Linux
   packages (AUR, apt, dnf, AppImage), signing, the `.dmg` and the
   per-user Windows installer, updates staged and applied at restart
   (D-19, D-22, D-24 to D-26). Then P11b: the agent dock (Claude Code or
   Codex in a terminal along the bottom of the viewer) and books homes
   (D-27). "One app, one executable, separate processes."
2. **The agent surface and the re-audit.** P4 (draft states, one
   evidence gate, the preview digest, the digest tool), P5 (the result
   envelope, prompts, `books.start` v2, journal history, agent evals),
   P8 (audit and attestations), P10 (the owner's full re-audit of the
   first company's books through the MCP alone). P13 (bookspec, the
   formal accounting specification: steps 4 to 6, the spec text, the
   spec's own checker, the GnuCash adapter) runs beside. P12 (the whole
   max-testing pass again: mutation, fuzzing, full property runs, proofs)
   comes last, over everything.

## And then beyond

Self-host for the owner is built (P9a: a system unit, the move with the
head check, a static bearer over a tunnel). What follows the installed
app, in order of the plan's decisions: claude.ai and ChatGPT reaching a
self-hosted server through an external identity provider (P9b, after
Q-6); then the paid, multi-tenant hosted service `plan/served.md` section
11 describes and does not build (per-tenant processes, client-side vault
unwrap before any paid launch, a browser viewer, email-in, invitations).
The design is kept compatible with it at every step.

## Explicitly not on the roadmap

Bank feeds, payment processing, payroll tax tables and filing, tax
filing. Those are integrations and regulated domains, not cloning
targets. The engine gets clean seams for them; someone else's API
provides them. Budgets were declined as bloat (R-FUTURE-6).
