import Bookspec.Ledger

/-!
# bookspec's five laws

The five laws of the first edition (`bookspec/spec/00-model.md`, "The five
laws"), each stated once, by name, as a theorem about *every* reachable
state of the books, ported from `verify/lean` (P13 step 4). Money is `Int`,
an exact count of the smallest unit; any finite set of exact decimals
scales to integers without loss, and every law is a law of addition.

1. Debits equal credits (BS-entry-1).
2. Assets = Liabilities + Equity + (Revenue - Expenses) (BS-statement-2).
3. The trial balance balances (BS-statement-1).
4. Net income articulates to equity: a close credits equity with exactly
   revenue less expenses (BS-period-1).
5. The cash-flow sections sum to the change in cash (BS-statement-3).
-/

namespace Bookspec

/-- Law 1. In every reachable state the posted lines' debits equal their
credits (`net` is debits less credits). -/
theorem law_debits_equal_credits {L : Ledger} (r : Reachable L) :
    net (allLines L.posted) = 0 :=
  r.books_net_zero

/-- Law 2. The accounting equation, in every reachable state. -/
theorem law_accounting_equation {L : Ledger} (r : Reachable L) :
    let ls := allLines L.posted
    typeTotal .asset ls L.chart
      = typeTotal .liability ls L.chart + typeTotal .equity ls L.chart
        + (typeTotal .revenue ls L.chart - typeTotal .expense ls L.chart) :=
  r.accounting_equation

/-- Law 3. The trial balance balances, in every reachable state. -/
theorem law_trial_balance {L : Ledger} (r : Reachable L) :
    tbDebits (allLines L.posted) (codes L.chart)
      = tbCredits (allLines L.posted) (codes L.chart) :=
  r.trial_balance

/-- Law 4. A close credits equity with exactly the period's net income. -/
theorem law_net_income_articulates (L : Ledger) (p : Period) (ts : List (Nat × Int))
    (hsum : total (ts.map Prod.snd)
      = net (closings (nominalCodes L.chart) (allLines (through p.stop L.posted)))) :
    let ls := allLines (through p.stop L.posted)
    total (ts.map Prod.snd) = typeTotal .revenue ls L.chart - typeTotal .expense ls L.chart :=
  close_credits_net_income L p ts hsum

/-- Law 5. The three sections of the statement of cash flows sum to the
change in the cash accounts. -/
theorem law_cash_flow_is_change_in_cash {L : Ledger} (r : Reachable L) (b c : Nat)
    (hbc : b ≤ c) (F : List Line → Line → Flow) (C : List Line → List Int)
    (hC : ∀ e, sum3 (C e) = 0) (cash : List Nat) (hd : Distinct cash)
    (hF : ∀ e l, F e l = .cash ↔ l.account ∈ cash) :
    let es := (between b c L.posted).map Entry.lines
    sectionTotal F C 0 es + sectionTotal F C 1 es + sectionTotal F C 2 es
      = sumOver (fun a => movement a (allLines (through c L.posted))) cash
        - sumOver (fun a => movement a (allLines (through b L.posted))) cash :=
  r.cashflow_is_change_in_cash b c hbc F C hC cash hd hF

end Bookspec
