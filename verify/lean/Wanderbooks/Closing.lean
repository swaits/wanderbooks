import Wanderbooks.Statements

/-!
# Closing a period, and splitting the net

Mirrors `kernel::closing_line`, `kernel::credit_line`, `kernel::plug_last`,
`periods::allocate`, and `Ledger::close_period_to` (income closing, net to
one equity account or split among members).
-/

namespace Wanderbooks

/-- The line that zeroes an account with movement `m`: the opposite side,
same amount; nothing for zero (`kernel::closing_line`). It is also the
line that credits `m` to an account (`kernel::credit_line`). -/
def closingLine (a : Nat) (m : Int) : List Line :=
  if 0 < m then [⟨a, .credit, m⟩] else if m < 0 then [⟨a, .debit, -m⟩] else []

theorem net_closingLine (a : Nat) (m : Int) : net (closingLine a m) = -m := by
  unfold closingLine
  split
  · simp [Line.signed]
  · split <;> simp [Line.signed] <;> omega

theorem movement_closingLine (b a : Nat) (m : Int) :
    movement b (closingLine a m) = if b = a then -m else 0 := by
  by_cases e : b = a
  · subst e
    rcases (by omega : 0 < m ∨ m < 0 ∨ m = 0) with h | h | h
    · simp [closingLine, h, Line.signed]
    · have : ¬ 0 < m := by omega
      simp [closingLine, h, this, Line.signed]
    · simp [closingLine, h]
  · have e' : ¬ a = b := fun x => e x.symm
    unfold closingLine
    split <;> (try split) <;> simp [e']

theorem closingLine_pos (a : Nat) (m : Int) : ∀ l ∈ closingLine a m, 0 < l.amount := by
  unfold closingLine
  split
  · simp; omega
  · split
    · simp; omega
    · simp

theorem closingLine_account (a : Nat) (m : Int) : ∀ l ∈ closingLine a m, l.account = a := by
  unfold closingLine
  split
  · simp
  · split <;> simp

theorem closingLine_eq_nil (a : Nat) (m : Int) : closingLine a m = [] ↔ m = 0 := by
  unfold closingLine
  split
  · simp; omega
  · split
    · simp; omega
    · simp; omega

/-- The closing lines for some accounts: one per account with a nonzero
movement. -/
def closings (as : List Nat) (ls : List Line) : List Line :=
  match as with
  | [] => []
  | a :: as => closingLine a (movement a ls) ++ closings as ls

theorem net_closings (as : List Nat) (ls : List Line) :
    net (closings as ls) = -sumOver (fun a => movement a ls) as := by
  induction as with
  | nil => rfl
  | cons a as ih => simp only [closings, net_append, net_closingLine, ih, sumOver_cons]; omega

theorem movement_closings (b : Nat) (as : List Nat) (ls : List Line) (hd : Distinct as) :
    movement b (closings as ls) = if b ∈ as then -movement b ls else 0 := by
  induction as with
  | nil => simp [closings]
  | cons a as ih =>
    obtain ⟨ha, hd⟩ := hd
    simp only [closings, movement_append, movement_closingLine, ih hd]
    by_cases e : b = a
    · subst e; simp [ha]
    · simp [e]

theorem closings_pos (as : List Nat) (ls : List Line) : ∀ l ∈ closings as ls, 0 < l.amount := by
  induction as with
  | nil => simp [closings]
  | cons a as ih =>
    intro l hl
    simp only [closings, List.mem_append] at hl
    cases hl with
    | inl h => exact closingLine_pos _ _ l h
    | inr h => exact ih l h

theorem closings_account (as : List Nat) (ls : List Line) : ∀ l ∈ closings as ls, l.account ∈ as := by
  induction as with
  | nil => simp [closings]
  | cons a as ih =>
    intro l hl
    simp only [closings, List.mem_append] at hl
    cases hl with
    | inl h => rw [closingLine_account _ _ l h]; simp
    | inr h => exact List.mem_cons_of_mem _ (ih l h)

/-- Nothing to close means every closed account was already at zero. -/
theorem closings_eq_nil (as : List Nat) (ls : List Line) (h : closings as ls = []) :
    ∀ a ∈ as, movement a ls = 0 := by
  induction as with
  | nil => simp
  | cons a as ih =>
    simp only [closings, List.append_eq_nil_iff] at h
    intro b hb
    cases hb with
    | head => exact (closingLine_eq_nil _ _).1 h.1
    | tail _ hb => exact ih h.2 b hb

/-! ## The net to equity -/

/-- The sum of some amounts. -/
def total : List Int → Int
  | [] => 0
  | x :: xs => x + total xs

/-- Lines crediting each target its part (`kernel::credit_line` per
member). -/
def equityLines : List (Nat × Int) → List Line
  | [] => []
  | (t, c) :: ts => closingLine t c ++ equityLines ts

theorem net_equityLines (ts : List (Nat × Int)) :
    net (equityLines ts) = -total (ts.map Prod.snd) := by
  induction ts with
  | nil => rfl
  | cons x ts ih =>
    obtain ⟨t, c⟩ := x
    simp only [equityLines, net_append, net_closingLine, ih, List.map_cons, total]; omega

theorem movement_equityLines_absent (b : Nat) (ts : List (Nat × Int))
    (h : ∀ x ∈ ts, x.1 ≠ b) : movement b (equityLines ts) = 0 := by
  induction ts with
  | nil => rfl
  | cons x ts ih =>
    obtain ⟨t, c⟩ := x
    have ht : b ≠ t := fun e => h (t, c) (by simp) e.symm
    simp only [equityLines, movement_append, movement_closingLine, ht, ite_false,
      ih (fun y hy => h y (List.mem_cons_of_mem _ hy))]
    rfl

theorem equityLines_pos (ts : List (Nat × Int)) : ∀ l ∈ equityLines ts, 0 < l.amount := by
  induction ts with
  | nil => simp [equityLines]
  | cons x ts ih =>
    obtain ⟨t, c⟩ := x
    intro l hl
    simp only [equityLines, List.mem_append] at hl
    cases hl with
    | inl h => exact closingLine_pos _ _ l h
    | inr h => exact ih l h

theorem equityLines_account (ts : List (Nat × Int)) :
    ∀ l ∈ equityLines ts, ∃ x ∈ ts, l.account = x.1 := by
  induction ts with
  | nil => simp [equityLines]
  | cons x ts ih =>
    obtain ⟨t, c⟩ := x
    intro l hl
    simp only [equityLines, List.mem_append] at hl
    cases hl with
    | inl h => exact ⟨(t, c), by simp, closingLine_account _ _ l h⟩
    | inr h =>
      obtain ⟨y, hy, e⟩ := ih l h
      exact ⟨y, List.mem_cons_of_mem _ hy, e⟩

theorem equityLines_eq_nil (ts : List (Nat × Int)) (h : equityLines ts = []) :
    total (ts.map Prod.snd) = 0 := by
  induction ts with
  | nil => rfl
  | cons x ts ih =>
    obtain ⟨t, c⟩ := x
    simp only [equityLines, List.append_eq_nil_iff] at h
    simp only [List.map_cons, total, (closingLine_eq_nil _ _).1 h.1, ih h.2]; rfl

/-- The closing entry: zero the closed accounts, credit the net to the
targets. -/
def closingEntry (as : List Nat) (ts : List (Nat × Int)) (ls : List Line) : List Line :=
  closings as ls ++ equityLines ts

/-- **A closing entry balances** when the targets' parts sum to the net
of the closing lines (the net income), however that net was split. -/
theorem closingEntry_net (as : List Nat) (ts : List (Nat × Int)) (ls : List Line)
    (hsum : total (ts.map Prod.snd) = net (closings as ls)) :
    net (closingEntry as ts ls) = 0 := by
  rw [closingEntry, net_append, net_equityLines, hsum]; omega

/-- **A closing entry zeroes every account it closes.** -/
theorem closingEntry_zeroes (as : List Nat) (ts : List (Nat × Int)) (ls : List Line)
    (hd : Distinct as) (b : Nat) (hb : b ∈ as) (ht : ∀ x ∈ ts, x.1 ≠ b) :
    movement b (ls ++ closingEntry as ts ls) = 0 := by
  rw [movement_append, closingEntry, movement_append, movement_closings b as ls hd,
    movement_equityLines_absent b ts ht]
  simp only [hb, ite_true]; omega

/-- **A non-empty closing entry is acceptable**: two or more lines, every
amount positive, debits equal to credits. -/
theorem closingEntry_accepted (as : List Nat) (ts : List (Nat × Int)) (ls : List Line)
    (hsum : total (ts.map Prod.snd) = net (closings as ls)) (hne : closings as ls ≠ []) :
    Accepted (closingEntry as ts ls) := by
  refine ⟨?_, ?_, ?_⟩
  · rw [closingEntry, List.length_append]
    match hc : closings as ls with
    | [] => exact absurd hc hne
    | [l] =>
      have hl : 0 < l.amount := closings_pos as ls l (by rw [hc]; simp)
      have hnet : net (closings as ls) ≠ 0 := by
        rw [hc]; obtain ⟨a, s, m⟩ := l
        cases s <;> simp [Line.signed] at hl ⊢ <;> omega
      have : equityLines ts ≠ [] := fun h => hnet (hsum ▸ equityLines_eq_nil ts h)
      have : 1 ≤ (equityLines ts).length := by
        cases h : equityLines ts with
        | nil => exact absurd h this
        | cons _ _ => simp
      simp; omega
    | _ :: _ :: _ => simp; omega
  · intro l hl
    simp only [closingEntry, List.mem_append] at hl
    cases hl with
    | inl h => exact closings_pos as ls l h
    | inr h => exact equityLines_pos ts l h
  · have := closingEntry_net as ts ls hsum
    rw [net_eq_debits_sub_credits] at this; omega

/-! ## Net income -/

/-- The chart's revenue and expense codes: what a close zeroes. -/
def nominalCodes : Chart → List Nat
  | [] => []
  | (a, t) :: c => if t = .revenue ∨ t = .expense then a :: nominalCodes c else nominalCodes c

theorem nominalCodes_sub (c : Chart) : ∀ a ∈ nominalCodes c, a ∈ codes c := by
  induction c with
  | nil => simp [nominalCodes]
  | cons x c ih =>
    obtain ⟨a, t⟩ := x
    intro b hb
    simp only [nominalCodes] at hb
    split at hb
    · cases hb with
      | head => simp
      | tail _ h => exact List.mem_cons_of_mem _ (ih b h)
    · exact List.mem_cons_of_mem _ (ih b hb)

theorem nominalCodes_distinct (c : Chart) (hd : Distinct (codes c)) : Distinct (nominalCodes c) := by
  induction c with
  | nil => trivial
  | cons x c ih =>
    obtain ⟨a, t⟩ := x
    obtain ⟨ha, hd⟩ := hd
    simp only [nominalCodes]
    split
    · exact ⟨fun h => ha (nominalCodes_sub c a h), ih hd⟩
    · exact ih hd

/-- **The net a close credits to equity is the period's net income**:
revenue balances minus expense balances. -/
theorem net_closings_is_net_income (c : Chart) (ls : List Line) :
    net (closings (nominalCodes c) ls) = typeTotal .revenue ls c - typeTotal .expense ls c := by
  rw [net_closings]
  induction c with
  | nil => rfl
  | cons x c ih =>
    obtain ⟨a, t⟩ := x
    cases t <;> simp [nominalCodes, typeTotal, balance, AccountType.normal, signed] at ih ⊢ <;> omega

/-! ## Splitting among members -/

/-- Make rounded parts sum to the whole: the last becomes the whole minus
the others (`kernel::plug_last`). -/
def plugLast (whole : Int) : List Int → List Int
  | [] => []
  | [_] => [whole]
  | p :: q :: rest => p :: plugLast (whole - p) (q :: rest)

/-- **An allocation's parts sum to the whole, exactly**, however the
other parts were rounded. -/
theorem plugLast_total (whole : Int) (ps : List Int) (h : ps ≠ []) :
    total (plugLast whole ps) = whole := by
  induction ps generalizing whole with
  | nil => exact absurd rfl h
  | cons p rest ih =>
    match rest, ih with
    | [], _ => show total [whole] = whole; simp [total]
    | q :: rest, ih =>
      simp only [plugLast, total]
      have := ih (whole - p) (by simp)
      omega

theorem plugLast_length (whole : Int) (ps : List Int) : (plugLast whole ps).length = ps.length := by
  induction ps generalizing whole with
  | nil => rfl
  | cons p rest ih =>
    match rest, ih with
    | [], _ => rfl
    | q :: rest, ih => simp only [plugLast, List.length_cons]; rw [ih]; rfl

/-- Only the last part changes. -/
theorem plugLast_keeps (whole : Int) (ps : List Int) :
    (plugLast whole ps).dropLast = ps.dropLast := by
  induction ps generalizing whole with
  | nil => rfl
  | cons p rest ih =>
    match rest, ih with
    | [], _ => rfl
    | q :: rest, ih =>
      have hne : plugLast (whole - p) (q :: rest) ≠ [] := by
        intro h
        have := plugLast_length (whole - p) (q :: rest)
        rw [h] at this; simp at this
      rw [show plugLast whole (p :: q :: rest) = p :: plugLast (whole - p) (q :: rest) from rfl,
        List.dropLast_cons_of_ne_nil hne, ih, List.dropLast_cons_cons]

end Wanderbooks
