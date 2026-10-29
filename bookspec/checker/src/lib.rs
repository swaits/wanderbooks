//! bookspec's checker: the example schema, the rule identifiers, and the
//! [`Engine`] trait an accounting engine implements (`plan/served.md` P13,
//! D-34).
//!
//! bookspec is a specification first: every rule is stated normatively,
//! sourced, and demonstrated by concrete examples (`bookspec/examples/`).
//! This crate is the machine-readable part: [`Example`] is the schema and
//! its parser, [`mod@rules`] the catalogue (`BS-<area>-<n>`), and [`Engine`]
//! the only interface a conformance runner needs. The runner lives in
//! `bookspec-runner`; wanderbooks' adapter is
//! `crates/ledger-core/tests/bookspec.rs`.
//!
//! The crate knows nothing of any engine: no money type, no ledger, no
//! store. That is what lets a second engine (the GnuCash oracle, later) be
//! checked by the same examples.

pub mod derive;
pub mod engine;
pub mod rules;
pub mod schema;

pub use derive::{Derivation, Derived, derive};
pub use engine::{Claims, Engine, Refusal, RuleId, RuleIdError, fallback_refusal};
pub use rules::{Area, Rule, Status, covered_rules, rule, rule_for_id, rule_for_refusal, rules};
pub use schema::*;
