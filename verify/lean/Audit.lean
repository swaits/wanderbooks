import Wanderbooks

/-! `just prove` runs this file: each headline theorem's axioms are printed,
and must be only Lean's standard three (`propext`, `Quot.sound`,
`Classical.choice`) or fewer. No `sorryAx`, nothing assumed. -/

open Wanderbooks

#print axioms Accepted.net_zero
#print axioms sum_movements
#print axioms movement_with_reversal
#print axioms accepted_reversal_iff
#print axioms accounting_equation
#print axioms balance_after_line
#print axioms trial_balance_iff
#print axioms closingEntry_net
#print axioms closingEntry_zeroes
#print axioms closingEntry_accepted
#print axioms net_closings_is_net_income
#print axioms plugLast_total
#print axioms plugLast_keeps
#print axioms runPart_telescopes
#print axioms closePart_total
#print axioms trueUp_total
#print axioms trueUp_running
#print axioms closePart_eq_trueUp
#print axioms cumulative_drift
#print axioms statement_after_close
#print axioms statement_unmoved_by_close
#print axioms Reachable.inv
#print axioms Reachable.books_net_zero
#print axioms Reachable.trial_balance
#print axioms Reachable.trial_balance_through
#print axioms Reachable.accounting_equation
#print axioms Reachable.accounting_equation_through
#print axioms Steps.extends
#print axioms Steps.frozen
#print axioms close_zeroes
#print axioms close_credits_net_income
#print axioms statements_add_up
#print axioms Apportion.ranks_perm
#print axioms Apportion.apportion_sum
#print axioms Apportion.apportion_sum_of_shortfall
#print axioms Apportion.rem_pos_of_rank_lt
#print axioms Apportion.apportion_drift
#print axioms Apportion.allocW_sum
#print axioms Apportion.allocW_drift
#print axioms Apportion.allocW_cumulative_drift
#print axioms Apportion.allocW_cumulative_total
#print axioms Step.record_changes_no_figure
#print axioms Reachable.records
