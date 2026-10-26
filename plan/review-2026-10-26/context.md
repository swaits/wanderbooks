# Review context (written 2026-10-26 by the reviewing session; facts, not opinions)

## Where the repository stands
- Branch `main`, head `c1f9001c` ("feat: the invoice PDF, with stamps and each payment (P4b S3)"). Working tree clean.
- `plan/served.md` is the plan under review (2058 lines). Its phases are P0..P13, with P4b inserted after P4.
- Built and live on the owner's company books: P0, P1, P2, P3, Projects, D-44 (stdio server removed), D-46 (files only by upload/download handles), P4b slices S1, S2, S3. See `plan/STATUS.md` lines 1-357 (current status) for what each one did.
- NOT built: P4 (drafts, reversal as draft, evidence gate, digest), P4b S4..S7, P5, P6, P7, P8, P9, P10, P11, P12, P13.
- The order the plan lists P4 before P4b, but P4b was started first (D-45, urgent defect). P4b.16 says "slice S1 next" although S1-S3 have landed.
- `bookspec/` already exists in this repo as a seed (56 example TOML files + README), checked by `cargo test`. P13 says bookspec gets its own repository later.
- Mutation testing is paused until P12 (D-33). Each phase lists what P12 must mutate in `docs/testing.md`.
- Crates (lines of Rust): ledger-core 74.6k, ledger-gui 43.9k, ledger-store 42.3k, ledger-cli 34.6k, ledger-mcp 28.3k, ledger-import 11.5k, ledger-host 8.7k, ledger-package 6.7k, ledger-config 3.4k, ledger-vault 0.7k. Lean proofs in `verify/lean/`. Kani harnesses in `crates/ledger-core/src/proofs.rs`.
- 118 MCP tools, documented in `docs/mcp-interface.md`; a test fails if a tool is undocumented.

## What the plan will be used for next (the purpose the review judges against)
The owner will hand the remaining work (P4b S4 through P13) to a team of ENTRY-LEVEL engineers working in parallel git worktrees, coordinated by an engineering manager who merges their branches. The engineers are expected to follow instructions literally and will not infer missing design. The plan must therefore:
1. say for each remaining unit of work exactly what to build, where (crate, module, file), what tests prove it, and what "done" means;
2. say which units can run in parallel and which must wait for another (dependencies, shared files, format features that need the owner's go-ahead);
3. not rely on knowledge that lives only in the previous engineer's head or in private files outside the repo;
4. be consistent with what has already been built (no scope text that was superseded by D-32, D-44, D-46, D-31 and still reads as if it were to be built).

## Repo rules the plan must respect (from AGENTS.md)
Money is exact Decimal; every entry balances; posted is forever; reports are pure functions of the ledger; clean room; money arithmetic in kernel.rs with Kani proofs; laws as Lean theorems; `just check` and `just prove` green before every commit; `docs/parity.md`, `docs/spec/rules.md`, `docs/mcp-interface.md`, `docs/testing.md`, `docs/verification.md` updated in the same commit; conventional commits.
