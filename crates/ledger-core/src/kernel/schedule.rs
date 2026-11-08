//! U5: schedule plans, the monthly charge a schedule books (kernel sweep).
//!
//! Empty in U0: the unit adds its arithmetic here, in this file alone, so
//! no two sweep units ever edit the same file. The module root re-exports
//! it, keeping every path `crate::kernel::foo`.
