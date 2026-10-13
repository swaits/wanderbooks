/-!
# Lines, sides, and nets

The vocabulary of double entry. Money is `Int`: an exact count of the
smallest unit the books use. Any finite set of exact decimals (what the
engine stores) scales to integers without loss, and every law here is a
law of addition, so `Int` is a faithful model.

Mirrors `crates/ledger-core/src/kernel.rs` (`flip`, `signed`, `net`,
`totals`) and `account.rs` (`Side`, `AccountType`, `normal_side`).
-/

namespace Wanderbooks

/-- Debit or credit. -/
inductive Side where
  | debit
  | credit
  deriving DecidableEq, Repr

/-- The other side (`kernel::flip`). -/
def Side.flip : Side → Side
  | .debit => .credit
  | .credit => .debit

@[simp] theorem Side.flip_flip (s : Side) : s.flip.flip = s := by
  cases s <;> rfl

theorem Side.flip_ne (s : Side) : s.flip ≠ s := by
  cases s <;> simp [Side.flip]

/-- The five account types. -/
inductive AccountType where
  | asset
  | liability
  | equity
  | revenue
  | expense
  deriving DecidableEq, Repr

/-- The side that increases an account (`AccountType::normal_side`). -/
def AccountType.normal : AccountType → Side
  | .asset | .expense => .debit
  | .liability | .equity | .revenue => .credit

/-- A line's signed amount: `+a` for a debit, `-a` for a credit
(`kernel::signed`). -/
def signed : Side → Int → Int
  | .debit, a => a
  | .credit, a => -a

@[simp] theorem signed_debit (a : Int) : signed .debit a = a := rfl
@[simp] theorem signed_credit (a : Int) : signed .credit a = -a := rfl

theorem signed_flip (s : Side) (a : Int) : signed s.flip a = -signed s a := by
  cases s <;> simp [Side.flip]

/-- One journal line. -/
structure Line where
  account : Nat
  side : Side
  amount : Int
  deriving DecidableEq, Repr

/-- A line's signed amount. -/
def Line.signed (l : Line) : Int := Wanderbooks.signed l.side l.amount

/-- The net of some lines: debits minus credits (`kernel::net`). -/
def net : List Line → Int
  | [] => 0
  | l :: ls => l.signed + net ls

/-- Total debits. -/
def debits : List Line → Int
  | [] => 0
  | l :: ls => (if l.side = .debit then l.amount else 0) + debits ls

/-- Total credits. -/
def credits : List Line → Int
  | [] => 0
  | l :: ls => (if l.side = .credit then l.amount else 0) + credits ls

@[simp] theorem net_nil : net [] = 0 := rfl
@[simp] theorem net_cons (l : Line) (ls : List Line) : net (l :: ls) = l.signed + net ls := rfl

theorem net_append (xs ys : List Line) : net (xs ++ ys) = net xs + net ys := by
  induction xs with
  | nil => simp
  | cons x xs ih => simp only [List.cons_append, net_cons, ih]; omega

/-- The net is debits minus credits. -/
theorem net_eq_debits_sub_credits (ls : List Line) : net ls = debits ls - credits ls := by
  induction ls with
  | nil => rfl
  | cons l ls ih =>
    obtain ⟨a, s, m⟩ := l
    cases s <;> simp [net, debits, credits, Line.signed, signed, ih] <;> omega

/-- A sum over a list of account codes. -/
def sumOver (f : Nat → Int) : List Nat → Int
  | [] => 0
  | a :: as => f a + sumOver f as

@[simp] theorem sumOver_nil (f : Nat → Int) : sumOver f [] = 0 := rfl
@[simp] theorem sumOver_cons (f : Nat → Int) (a : Nat) (as : List Nat) :
    sumOver f (a :: as) = f a + sumOver f as := rfl

theorem sumOver_add (f g : Nat → Int) (as : List Nat) :
    sumOver (fun a => f a + g a) as = sumOver f as + sumOver g as := by
  induction as with
  | nil => rfl
  | cons a as ih => simp only [sumOver_cons, ih]; omega

theorem sumOver_neg (f : Nat → Int) (as : List Nat) :
    sumOver (fun a => -f a) as = -sumOver f as := by
  induction as with
  | nil => rfl
  | cons a as ih => simp only [sumOver_cons, ih]; omega

theorem sumOver_zero (as : List Nat) : sumOver (fun _ => 0) as = 0 := by
  induction as with
  | nil => rfl
  | cons a as ih => simp [ih]

theorem sumOver_congr (f g : Nat → Int) (as : List Nat) (h : ∀ a ∈ as, f a = g a) :
    sumOver f as = sumOver g as := by
  induction as with
  | nil => rfl
  | cons a as ih =>
    simp only [sumOver_cons]
    rw [h a (by simp), ih (fun b hb => h b (by simp [hb]))]

/-- No code repeats (a chart's codes are unique). -/
def Distinct : List Nat → Prop
  | [] => True
  | a :: as => a ∉ as ∧ Distinct as

/-- Summing an indicator over codes that do not include `k` gives zero. -/
theorem sumOver_indicator_absent (k : Nat) (x : Int) (as : List Nat) (h : k ∉ as) :
    sumOver (fun a => if k = a then x else 0) as = 0 := by
  induction as with
  | nil => rfl
  | cons a as ih =>
    have hka : k ≠ a := fun e => h (e ▸ List.mem_cons_self ..)
    have hk : k ∉ as := fun hm => h (List.mem_cons_of_mem _ hm)
    simp [hka, ih hk]

/-- Summing an indicator over distinct codes that include `k` picks out
exactly one term. -/
theorem sumOver_indicator (k : Nat) (x : Int) (as : List Nat) (hd : Distinct as)
    (hk : k ∈ as) : sumOver (fun a => if k = a then x else 0) as = x := by
  induction as with
  | nil => simp at hk
  | cons a as ih =>
    obtain ⟨ha, hd⟩ := hd
    by_cases e : k = a
    · subst e
      simp [sumOver_indicator_absent k x as ha]
    · have : k ∈ as := by
        cases hk with
        | head => exact absurd rfl e
        | tail _ h => exact h
      simp [e, ih hd this]

end Wanderbooks
