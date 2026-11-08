//! U3: proportional shares, splitting a whole by weights without drift (kernel sweep).
//!
//! Empty in U0: the unit adds its arithmetic here, in this file alone, so
//! no two sweep units ever edit the same file. The module root re-exports
//! it, keeping every path `crate::kernel::foo`.
