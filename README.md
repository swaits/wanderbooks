# wanderbooks

Canonical double-entry bookkeeping, rebuilt from scratch — the storytold way:
an engine verified against oracles, with automated parity tracking, on its
way to becoming a self-hosted bookkeeping app that never charges rent.

## Status

Phase 1: the semantics engine. `ledger-core` implements the accounting laws
in Rust — exact decimal money, five account types, balanced journal entries,
immutable posted log with reversing entries, trial balance, P&L, and balance
sheet — with 14 invariant tests and golden-file report snapshots.

See [`plan/STATUS.md`](plan/STATUS.md) for where things stand and
[`plan/roadmap.md`](plan/roadmap.md) for where they're going.

## Quickstart

```bash
cargo test
```

That's the whole test suite: validation rejections, immutability, the
accounting equation on a fixture company, a 300-entry randomized invariant
test, and the golden oracle test.

## Layout

```
crates/ledger-core/   # the engine: money, accounts, journal, ledger, reports
docs/                 # requirements, architecture, oracle design, parity, glossary
plan/                 # STATUS.md, roadmap.md
AGENTS.md             # operating manual for agents working here
```

## The idea, briefly

storytold/ArtCraft showed that agents can clone full applications with the
right machinery: screenshot-in-the-loop verification, render oracles, and
automated parity floors. `wanderbooks` adapts that pipeline to bookkeeping,
where the oracle is semantic instead of visual — GnuCash (open source, open
formats) is both the spec and the reference implementation we diff against,
and two engines fed the same books must produce byte-identical reports.
Details in [`docs/oracle.md`](docs/oracle.md).

## License

MIT OR Apache-2.0. See `LICENSE-MIT` and `LICENSE-APACHE`.
