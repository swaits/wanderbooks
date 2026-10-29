import Bookspec

/-! Each of the five laws' axioms are printed, and must be only Lean's
standard three (`propext`, `Quot.sound`, `Classical.choice`) or fewer. No
`sorryAx`, nothing assumed. `bookspec/checker/tests/lean.rs` reads this
file's output shape; run it with `lake env lean Audit.lean`. -/

open Bookspec

#print axioms law_debits_equal_credits
#print axioms law_accounting_equation
#print axioms law_trial_balance
#print axioms law_net_income_articulates
#print axioms law_cash_flow_is_change_in_cash
