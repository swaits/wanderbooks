//! U2: one rounding primitive, the single place money is rounded to a step (kernel sweep).
//!
//! Empty in U0: the unit adds its arithmetic here, in this file alone, so
//! no two sweep units ever edit the same file. The module root re-exports
//! it, keeping every path `crate::kernel::foo`.
