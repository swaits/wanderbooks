import Bookspec.Basic

/-!
# Entries: acceptance, movements, reversals

Mirrors `kernel::check_lines`, `kernel::movements`, `kernel::reversal`,
and `JournalEntry::validate`.
-/

namespace Bookspec

/-- What the engine accepts (the arithmetic of `JournalEntry::validate`,
i.e. `kernel::check_lines`): two or more lines, every amount positive,
debits equal to credits. (Range, account, and currency checks are not
arithmetic laws and are left out of the model.) -/
def Accepted (ls : List Line) : Prop :=
  2 ≤ ls.length ∧ (∀ l ∈ ls, 0 < l.amount) ∧ debits ls = credits ls

/-- **Every accepted entry nets to zero.** -/
theorem Accepted.net_zero {ls : List Line} (h : Accepted ls) : net ls = 0 := by
  rw [net_eq_debits_sub_credits]; have := h.2.2; omega

/-- Net movement of account `a` over some lines. -/
def movement (a : Nat) : List Line → Int
  | [] => 0
  | l :: ls => (if l.account = a then l.signed else 0) + movement a ls

@[simp] theorem movement_nil (a : Nat) : movement a [] = 0 := rfl
@[simp] theorem movement_cons (a : Nat) (l : Line) (ls : List Line) :
    movement a (l :: ls) = (if l.account = a then l.signed else 0) + movement a ls := rfl

theorem movement_append (a : Nat) (xs ys : List Line) :
    movement a (xs ++ ys) = movement a xs + movement a ys := by
  induction xs with
  | nil => simp
  | cons x xs ih => simp only [List.cons_append, movement_cons, ih]; omega

/-- **Movements partition the net.** Summed over a chart's distinct codes
that cover every line's account, the per-account movements add up to the
net of the lines: grouping by account loses and invents nothing. -/
theorem sum_movements (as : List Nat) (hd : Distinct as) :
    ∀ ls : List Line, (∀ l ∈ ls, l.account ∈ as) →
      sumOver (fun a => movement a ls) as = net ls := by
  intro ls
  induction ls with
  | nil => intro _; simpa using sumOver_zero as
  | cons l ls ih =>
    intro hcov
    simp only [movement_cons, net_cons]
    rw [sumOver_add, ih (fun m hm => hcov m (List.mem_cons_of_mem _ hm)),
      sumOver_indicator l.account l.signed as hd (hcov l (List.mem_cons_self ..))]

/-- So the movements of an accepted entry sum to zero. -/
theorem Accepted.movements_sum_zero {ls : List Line} (h : Accepted ls) (as : List Nat)
    (hd : Distinct as) (hcov : ∀ l ∈ ls, l.account ∈ as) :
    sumOver (fun a => movement a ls) as = 0 := by
  rw [sum_movements as hd ls hcov, h.net_zero]

/-! ## Reversals -/

/-- A line with its side flipped. -/
def Line.reverse (l : Line) : Line := { l with side := l.side.flip }

/-- A reversing entry's lines (`kernel::reversal`). -/
def reversal (ls : List Line) : List Line := ls.map Line.reverse

theorem Line.reverse_signed (l : Line) : l.reverse.signed = -l.signed := by
  simp [Line.reverse, Line.signed, signed_flip]

theorem net_reversal (ls : List Line) : net (reversal ls) = -net ls := by
  induction ls with
  | nil => rfl
  | cons l ls ih =>
    simp only [reversal, List.map_cons] at ih ⊢
    simp only [net_cons, ih, Line.reverse_signed]; omega

theorem movement_reversal (a : Nat) (ls : List Line) :
    movement a (reversal ls) = -movement a ls := by
  induction ls with
  | nil => rfl
  | cons l ls ih =>
    simp only [reversal, List.map_cons] at ih ⊢
    simp only [movement_cons, ih, Line.reverse_signed]
    by_cases h : l.account = a <;> simp [Line.reverse, h] <;> omega

/-- **An entry and its reversal net every account to zero.** -/
theorem movement_with_reversal (a : Nat) (ls : List Line) :
    movement a (ls ++ reversal ls) = 0 := by
  rw [movement_append, movement_reversal]; omega

theorem debits_reversal (ls : List Line) : debits (reversal ls) = credits ls := by
  induction ls with
  | nil => rfl
  | cons l ls ih =>
    simp only [reversal, List.map_cons] at ih ⊢
    obtain ⟨a, s, m⟩ := l
    cases s <;> simp [debits, credits, Line.reverse, Side.flip, ih]

theorem credits_reversal (ls : List Line) : credits (reversal ls) = debits ls := by
  induction ls with
  | nil => rfl
  | cons l ls ih =>
    simp only [reversal, List.map_cons] at ih ⊢
    obtain ⟨a, s, m⟩ := l
    cases s <;> simp [debits, credits, Line.reverse, Side.flip, ih]

/-- **A reversal is accepted exactly when the original is.** -/
theorem accepted_reversal_iff (ls : List Line) : Accepted (reversal ls) ↔ Accepted ls := by
  unfold Accepted
  rw [debits_reversal, credits_reversal]
  have hlen : (reversal ls).length = ls.length := List.length_map ..
  have hpos : (∀ l ∈ reversal ls, 0 < l.amount) ↔ (∀ l ∈ ls, 0 < l.amount) := by
    simp [reversal, Line.reverse]
  rw [hlen, hpos]
  constructor <;> rintro ⟨a, b, c⟩ <;> exact ⟨a, b, c.symm⟩

end Bookspec
