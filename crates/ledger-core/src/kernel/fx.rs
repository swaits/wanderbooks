//! U1: foreign-currency conversion, the booking of a foreign amount at the transaction-date rate (kernel sweep).
//!
//! Empty in U0: the unit adds its arithmetic here, in this file alone, so
//! no two sweep units ever edit the same file. The module root re-exports
//! it, keeping every path `crate::kernel::foo`.
