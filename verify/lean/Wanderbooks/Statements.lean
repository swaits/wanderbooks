import Wanderbooks.Entry

/-!
# Balances and statements

Mirrors `kernel::normal_balance`, `kernel::columns`,
`Ledger::account_balance`, and `reports::trial_balance`.
-/

namespace Wanderbooks

/-- A chart of accounts: each account's code and type. -/
abbrev Chart := List (Nat × AccountType)

/-- The chart's codes. -/
def codes (c : Chart) : List Nat := c.map Prod.fst

@[simp] theorem codes_nil : codes [] = [] := rfl
@[simp] theorem codes_cons (a : Nat) (t : AccountType) (c : Chart) :
    codes ((a, t) :: c) = a :: codes c := rfl

/-- An account's balance on its normal side (`kernel::normal_balance`). -/
def balance (t : AccountType) (m : Int) : Int := signed t.normal m

/-- **A normal-side line raises a balance by its amount; an opposite-side
line lowers it.** -/
theorem balance_after_line (t : AccountType) (m : Int) (s : Side) (a : Int) :
    balance t (m + signed s a) = balance t m + (if s = t.normal then a else -a) := by
  cases t <;> cases s <;> simp [balance, AccountType.normal, signed] <;> omega

/-- The total balance of every account of type `t`. -/
def typeTotal (t : AccountType) (ls : List Line) : Chart → Int
  | [] => 0
  | (a, t') :: rest => (if t' = t then balance t' (movement a ls) else 0) + typeTotal t ls rest

/-- The accounting equation's two sides differ by the net movement of the
chart's accounts. -/
theorem equation_gap (ls : List Line) (c : Chart) :
    typeTotal .asset ls c + typeTotal .expense ls c
      - typeTotal .liability ls c - typeTotal .equity ls c - typeTotal .revenue ls c
      = sumOver (fun a => movement a ls) (codes c) := by
  induction c with
  | nil => rfl
  | cons x c ih =>
    obtain ⟨a, t⟩ := x
    cases t <;> simp [typeTotal, balance, AccountType.normal, signed] <;> omega

/-- **The accounting equation.** For lines that net to zero, over a chart
with distinct codes covering every line's account:
Assets = Liabilities + Equity + (Revenue - Expenses). -/
theorem accounting_equation (ls : List Line) (c : Chart) (hd : Distinct (codes c))
    (hcov : ∀ l ∈ ls, l.account ∈ codes c) (hnet : net ls = 0) :
    typeTotal .asset ls c
      = typeTotal .liability ls c + typeTotal .equity ls c
        + (typeTotal .revenue ls c - typeTotal .expense ls c) := by
  have := equation_gap ls c
  rw [sum_movements _ hd ls hcov, hnet] at this
  omega

/-! ## Trial balance -/

/-- The debit column for a movement (`kernel::columns`). -/
def colDebit (m : Int) : Int := if 0 < m then m else 0
/-- The credit column for a movement (`kernel::columns`). -/
def colCredit (m : Int) : Int := if 0 < m then 0 else -m

theorem columns_spec (m : Int) :
    0 ≤ colDebit m ∧ 0 ≤ colCredit m ∧ (colDebit m = 0 ∨ colCredit m = 0)
      ∧ colDebit m - colCredit m = m := by
  unfold colDebit colCredit; split <;> omega

/-- The trial balance's debit column total. -/
def tbDebits (ls : List Line) (as : List Nat) : Int := sumOver (fun a => colDebit (movement a ls)) as
/-- The trial balance's credit column total. -/
def tbCredits (ls : List Line) (as : List Nat) : Int :=
  sumOver (fun a => colCredit (movement a ls)) as

theorem tb_gap (ls : List Line) (as : List Nat) :
    tbDebits ls as - tbCredits ls as = sumOver (fun a => movement a ls) as := by
  induction as with
  | nil => rfl
  | cons a as ih =>
    simp only [tbDebits, tbCredits, sumOver_cons] at ih ⊢
    have := (columns_spec (movement a ls)).2.2.2
    omega

/-- **The trial balance balances** for lines that net to zero. -/
theorem trial_balance_balances (ls : List Line) (as : List Nat) (hd : Distinct as)
    (hcov : ∀ l ∈ ls, l.account ∈ as) (hnet : net ls = 0) :
    tbDebits ls as = tbCredits ls as := by
  have := tb_gap ls as
  rw [sum_movements as hd ls hcov, hnet] at this
  omega

/-- And conversely: a balanced trial balance means the lines net to zero.
The trial balance is a complete check of the books' arithmetic. -/
theorem trial_balance_iff (ls : List Line) (as : List Nat) (hd : Distinct as)
    (hcov : ∀ l ∈ ls, l.account ∈ as) :
    tbDebits ls as = tbCredits ls as ↔ net ls = 0 := by
  have := tb_gap ls as
  rw [sum_movements as hd ls hcov] at this
  omega

end Wanderbooks
