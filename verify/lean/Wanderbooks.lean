import Bookspec

/-!
# The accounting laws

The laws live once, in bookspec's Lean (`bookspec/lean/Bookspec/`, namespace
`Bookspec`), and this project requires that package (D-69). This module is
the local target `lake build` builds: importing it builds and checks every
law, and `just prove` prints their axioms with bookspec's own
`lean/Audit.lean` (from `.lake/packages/bookspec`; no copy here).

A law that changes is edited in bookspec, and this project moves its pin
(`lakefile.toml`, `lake-manifest.json`) to bookspec's new rev — the same rev
`crates/ledger-core/Cargo.toml` pins the Rust checker to. Nothing here
restates the model.
-/
