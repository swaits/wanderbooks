//! The CPA handoff package, and the reports, worksheets, and text helpers
//! the server's tools and the viewer both make: built from the books
//! read-only, through [`PackageBooks`] (the server's
//! [`ledger_store::Store`], the viewer's [`ledger_store::Snapshot`]).
//!
//! ```text
//! GOAL: the server (books.export_cpa_package, books.vendor_1099) and the
//!       viewer make the same package and the same 1099 worksheet, byte
//!       for byte, from one piece of code, and neither can change the
//!       books making them.
//! SOURCE: the books.export_cpa_package and books.vendor_1099 contracts
//!         (ledger-mcp server.rs; docs/mcp-interface.md), moved here
//!         unchanged so the viewer, which reads the books only through a
//!         Snapshot and doesn't depend on ledger-mcp, builds them too.
//! INVARIANTS:
//!   1. Read-only: nothing here writes anywhere (the books, the vault, a
//!      file). The builders return bytes; recording a file that holds
//!      full TINs (ledger-mcp tin_files.rs) and handing it over are the
//!      caller's.
//!   2. Full TINs come only from the TIN source the caller gives
//!      ([`TinSource`]); without one, a worksheet carries last fours only.
//!   3. The same books, settings, and request give the same bytes,
//!      whichever [`PackageBooks`] they are read through.
//! ```

mod books;
mod iif;
mod package;
mod reports;
mod text;
mod tins;
mod vendor_1099;

pub use books::PackageBooks;
pub use iif::export_iif;
pub use package::{Package, PackageRequest, build_package, package_file_name};
pub use reports::{
    Settings, as_of_date, bs_statement, cash_flow_for, cash_flow_statement, member_capitals,
    pnl_statement, report_members, report_meta, statement_range, year_earnings,
};
pub use text::{csv_escape, money, parse_date, share_text};
pub use tins::{TinSource, tin_kind, tin_type_key, vault_tin};
pub use vendor_1099::{
    VENDOR_1099_CSV_HEADER, Vendor1099Worksheet, Vendor1099WorksheetLine, form_1099_label,
    payment_accounts, vendor_1099_csv, vendor_1099_csv_name, vendor_1099_report,
    vendor_1099_worksheet,
};
