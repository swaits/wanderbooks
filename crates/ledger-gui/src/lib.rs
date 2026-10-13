//! `ledger-gui`: a read-only, live view of wanderbooks books for the people
//! who own them and the accountant who checks them. See `plan/gui.md`.

#![allow(missing_docs)]

pub mod app;
mod attention;
pub mod books;
mod charts;
pub mod data;
pub mod demo;
mod fmt;
pub mod nav;
pub mod pages;
mod palette;
mod receipts;
mod theme;
mod widgets;

pub use app::App;
pub use data::Source;
