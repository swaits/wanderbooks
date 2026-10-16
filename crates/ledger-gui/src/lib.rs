//! `ledger-gui`: a read-only, live view of wanderbooks books for the people
//! who own them and the accountant who checks them. See `plan/gui.md`.

#![allow(missing_docs)]

pub mod app;
pub mod attention;
pub mod books;
pub mod charts;
pub mod data;
pub mod demo;
pub mod export;
pub mod fmt;
pub mod nav;
pub mod pages;
pub mod palette;
mod receipts;
#[cfg(test)]
mod test_support;
mod theme;
mod widgets;

pub use app::App;
pub use data::Source;
