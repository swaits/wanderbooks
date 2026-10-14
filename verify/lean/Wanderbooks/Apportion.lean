/-!
# Apportioning: the parts sum to the whole, for any number of parts

Mirrors `crates/ledger-core/src/kernel.rs` `apportion`: each part is
first rounded down; the shortfall `s = whole - Σ parts` (not negative) is
`u` whole units plus a fraction `f` (`s = u * unit + f`, `u` below the
number of parts, else the function refuses). A part's *rank* is how many
parts are ahead of it: a strictly larger remainder, or an equal one at an
earlier index. A part of rank below `u` gets one unit; the part of rank 0
also gets the fraction.

Kani proves INVARIANT 1 for up to three parts. Here it holds for every
number of parts: the ranks are exactly `0, …, n-1` in some order (a strict
total order ranks a finite set without gaps or ties), so exactly `u` parts
get a unit and exactly one gets the fraction.
-/

namespace Wanderbooks.Apportion

/-- Part `j` is ahead of part `i` (the closure in `kernel::apportion`). -/
def ahead (r : Nat → Int) (j i : Nat) : Bool :=
  decide (r j > r i) || (decide (r j = r i) && decide (j < i))

/-- A part's rank among `n` parts: the parts ahead of it. -/
def rank (r : Nat → Int) (n i : Nat) : Nat :=
  ((List.range n).filter (fun j => ahead r j i)).length

/-- What part `i` gets added: a unit if its rank is below `u`, and the
fraction too if its rank is 0. -/
def add (r : Nat → Int) (n u : Nat) (unit f : Int) (i : Nat) : Int :=
  (if rank r n i < u then unit else 0) + (if rank r n i = 0 then f else 0)

theorem ahead_irrefl (r : Nat → Int) (i : Nat) : ahead r i i = false := by
  simp [ahead]

theorem ahead_trans (r : Nat → Int) {a b c : Nat}
    (h1 : ahead r a b = true) (h2 : ahead r b c = true) : ahead r a c = true := by
  simp only [ahead, Bool.or_eq_true, Bool.and_eq_true, decide_eq_true_eq] at *
  omega

theorem ahead_total (r : Nat → Int) {a b : Nat} (h : a ≠ b) :
    ahead r a b = true ∨ ahead r b a = true := by
  simp only [ahead, Bool.or_eq_true, Bool.and_eq_true, decide_eq_true_eq]
  omega

theorem ahead_asymm (r : Nat → Int) {a b : Nat} (h : ahead r a b = true) :
    ahead r b a = false := by
  simp only [ahead, Bool.or_eq_true, Bool.and_eq_true, decide_eq_true_eq] at h
  simp only [ahead, Bool.or_eq_false_iff, Bool.and_eq_false_imp, decide_eq_true_eq,
    decide_eq_false_iff_not]
  omega

/-- A filter by a weaker predicate is no longer. -/
theorem filter_length_le {p q : Nat → Bool} : ∀ (l : List Nat),
    (∀ x ∈ l, p x = true → q x = true) → (l.filter p).length ≤ (l.filter q).length
  | [], _ => by simp
  | a :: t, sub => by
    have ih := filter_length_le t (fun x hx => sub x (List.mem_cons_of_mem a hx))
    have ha := sub a (List.mem_cons_self ..)
    cases hpa : p a <;> cases hqa : q a <;> simp_all <;> omega

/-- ... and strictly shorter when some element passes only the weaker one. -/
theorem filter_length_lt {p q : Nat → Bool} : ∀ (l : List Nat),
    (∀ x ∈ l, p x = true → q x = true) → (∃ x ∈ l, q x = true ∧ p x = false) →
    (l.filter p).length < (l.filter q).length
  | [], _, w => by simp at w
  | a :: t, sub, w => by
    have sub' : ∀ x ∈ t, p x = true → q x = true :=
      fun x hx => sub x (List.mem_cons_of_mem a hx)
    have le := filter_length_le t sub'
    have ha := sub a (List.mem_cons_self ..)
    obtain ⟨x, hx, hq, hp⟩ := w
    rcases List.mem_cons.mp hx with rfl | hx'
    · simp [hq, hp]
      omega
    · have := filter_length_lt t sub' ⟨x, hx', hq, hp⟩
      cases hpa : p a <;> cases hqa : q a <;> simp_all <;> omega

/-- Being ahead means a strictly smaller rank. -/
theorem rank_lt_of_ahead (r : Nat → Int) {n a b : Nat} (ha : a < n)
    (h : ahead r a b = true) : rank r n a < rank r n b := by
  unfold rank
  apply filter_length_lt
  · intro x _ hx
    exact ahead_trans r hx h
  · exact ⟨a, List.mem_range.mpr ha, h, ahead_irrefl r a⟩

/-- Distinct parts have distinct ranks. -/
theorem rank_inj (r : Nat → Int) {n a b : Nat} (ha : a < n) (hb : b < n)
    (h : rank r n a = rank r n b) : a = b := by
  apply Classical.byContradiction
  intro hne
  rcases ahead_total r hne with h1 | h1
  · have := rank_lt_of_ahead r ha h1; omega
  · have := rank_lt_of_ahead r hb h1; omega

/-- Every rank is below the number of parts (a part isn't ahead of itself). -/
theorem rank_lt (r : Nat → Int) {n i : Nat} (hi : i < n) : rank r n i < n := by
  unfold rank
  have := (List.length_filter_lt_length_iff_exists (p := fun j => ahead r j i)
    (l := List.range n)).mpr ⟨i, List.mem_range.mpr hi, by simp [ahead_irrefl]⟩
  simpa using this

/-- A duplicate-free list inside another is no longer. -/
theorem nodup_length_le : ∀ (l m : List Nat), l.Nodup → (∀ x ∈ l, x ∈ m) →
    l.length ≤ m.length
  | [], _, _, _ => by simp
  | a :: t, m, hn, hs => by
    rw [List.nodup_cons] at hn
    have ham : a ∈ m := hs a (by simp)
    have ih := nodup_length_le t (m.erase a) hn.2 (fun x hx => by
      have hxa : x ≠ a := fun e => hn.1 (e ▸ hx)
      exact (List.mem_erase_of_ne hxa).mpr (hs x (by simp [hx])))
    rw [List.length_erase_of_mem ham] at ih
    have : 0 < m.length := List.length_pos_of_mem ham
    simp
    omega

/-- Mapping a duplicate-free list by a function injective on it keeps it
duplicate-free. -/
theorem nodup_map_of_inj {f : Nat → Nat} : ∀ (l : List Nat), l.Nodup →
    (∀ a ∈ l, ∀ b ∈ l, f a = f b → a = b) → (l.map f).Nodup
  | [], _, _ => by simp
  | a :: t, hn, hi => by
    rw [List.nodup_cons] at hn
    rw [List.map_cons, List.nodup_cons]
    refine ⟨?_, nodup_map_of_inj t hn.2 (fun x hx y hy => hi x (by simp [hx]) y (by simp [hy]))⟩
    intro hm
    obtain ⟨b, hb, hfb⟩ := List.mem_map.mp hm
    have := hi a (by simp) b (by simp [hb]) hfb.symm
    exact hn.1 (this ▸ hb)

/-- The ranks of `n` parts are `0, …, n-1` in some order. -/
theorem ranks_perm (r : Nat → Int) (n : Nat) :
    ((List.range n).map (rank r n)).Perm (List.range n) := by
  let L := (List.range n).map (rank r n)
  have nd : L.Nodup := nodup_map_of_inj _ List.nodup_range (fun a ha b hb h =>
    rank_inj r (List.mem_range.mp ha) (List.mem_range.mp hb) h)
  have sub : ∀ x ∈ L, x < n := by
    intro x hx
    obtain ⟨i, hi, rfl⟩ := List.mem_map.mp hx
    exact rank_lt r (List.mem_range.mp hi)
  have len : L.length = n := by simp [L]
  -- Pigeonhole: n distinct values below n are all of them.
  have all : ∀ v < n, v ∈ L := by
    intro v hv
    apply Classical.byContradiction
    intro hnot
    have := nodup_length_le L ((List.range n).erase v) nd (fun x hx => by
      have hxv : x ≠ v := fun e => hnot (e ▸ hx)
      exact (List.mem_erase_of_ne hxv).mpr (List.mem_range.mpr (sub x hx)))
    rw [List.length_erase_of_mem (List.mem_range.mpr hv), List.length_range] at this
    omega
  rw [List.perm_iff_count]
  intro a
  rw [List.Nodup.count nd, List.Nodup.count List.nodup_range]
  by_cases ha : a < n
  · simp [all a ha, List.mem_range.mpr ha]
  · have h1 : a ∉ L := fun h => ha (sub a h)
    simp [h1, List.mem_range, ha]

/-- Sums over permuted lists agree. -/
theorem perm_sum {l₁ l₂ : List Int} (h : l₁.Perm l₂) : l₁.sum = l₂.sum := by
  induction h with
  | nil => rfl
  | cons x _ ih => simp [ih]
  | swap x y l => simp; omega
  | trans _ _ ih1 ih2 => exact ih1.trans ih2

/-- What the ranks `0, …, n-1` get, together: `u` units and the fraction. -/
theorem sum_by_rank (u : Nat) (unit f : Int) : ∀ n, 0 < n → u ≤ n →
    ((List.range n).map (fun v => (if v < u then unit else 0) + (if v = 0 then f else 0))).sum
      = u * unit + f
  | 0, h, _ => absurd h (by decide)
  | 1, _, hu => by
    rcases Nat.le_one_iff_eq_zero_or_eq_one.mp hu with rfl | rfl <;> simp
  | n + 2, _, hu => by
    rw [List.range_succ, List.map_append, List.sum_append]
    by_cases h : u ≤ n + 1
    · rw [sum_by_rank u unit f (n + 1) (by omega) h]
      have : ¬ (n + 1 < u) := by omega
      simp [this]
    · have hu' : u = n + 2 := by omega
      subst hu'
      have ih := sum_by_rank (n + 1) unit f (n + 1) (by omega) (Nat.le_refl _)
      have eq : ((List.range (n + 1)).map
          (fun v => (if v < n + 2 then unit else 0) + (if v = 0 then f else 0))) =
          ((List.range (n + 1)).map
          (fun v => (if v < n + 1 then unit else 0) + (if v = 0 then f else 0))) := by
        apply List.map_congr_left
        intro v hv
        have := List.mem_range.mp hv
        simp [show v < n + 2 by omega, show v < n + 1 by omega]
      rw [eq, ih]
      have e : ((n + 2 : Nat) : Int) = ((n + 1 : Nat) : Int) + 1 := by omega
      simp only [List.map_cons, List.map_nil, List.sum_cons, List.sum_nil]
      rw [e, Int.add_mul, Int.one_mul]
      simp
      omega

theorem sum_map_add (f g : Nat → Int) : ∀ (l : List Nat),
    (l.map (fun i => f i + g i)).sum = (l.map f).sum + (l.map g).sum
  | [] => by simp
  | a :: t => by simp [sum_map_add f g t]; omega

/-- **INVARIANT 1, for any number of parts.** When the shortfall
`whole - Σ parts` is `u` units plus `f` (with `u` at most the number of
parts, as `apportion` requires before it changes anything), the parts
after apportioning sum to `whole` exactly. -/
theorem apportion_sum (parts r : Nat → Int) (n u : Nat) (unit f whole : Int)
    (hn : 0 < n) (hu : u ≤ n)
    (hs : whole - ((List.range n).map parts).sum = u * unit + f) :
    ((List.range n).map (fun i => parts i + add r n u unit f i)).sum = whole := by
  rw [sum_map_add]
  let g := fun v => (if v < u then unit else 0) + (if v = 0 then f else 0)
  have h1 : (List.range n).map (add r n u unit f) = ((List.range n).map (rank r n)).map g := by
    simp [List.map_map, add, g, Function.comp_def]
  have h2 := perm_sum ((ranks_perm r n).map g)
  have h3 := sum_by_rank u unit f n hn hu
  show ((List.range n).map parts).sum + ((List.range n).map (add r n u unit f)).sum = whole
  rw [h1, h2, h3]
  omega

/-- The same, from the shortfall as `apportion` computes it: `u` whole
units by repeated subtraction (`s / unit`) and the rest (`s % unit`). -/
theorem apportion_sum_of_shortfall (parts r : Nat → Int) (n : Nat) (unit whole : Int)
    (hn : 0 < n) (hunit : 0 < unit)
    (hs : 0 ≤ whole - ((List.range n).map parts).sum)
    (hu : ((whole - ((List.range n).map parts).sum) / unit).toNat ≤ n) :
    let s := whole - ((List.range n).map parts).sum
    ((List.range n).map (fun i =>
      parts i + add r n (s / unit).toNat unit (s % unit) i)).sum = whole := by
  intro s
  apply apportion_sum parts r n _ unit _ whole hn hu
  have hq : 0 ≤ s / unit := Int.ediv_nonneg hs (Int.le_of_lt hunit)
  rw [Int.toNat_of_nonneg hq]
  have := Int.mul_ediv_add_emod s unit
  rw [Int.mul_comm] at this
  exact this.symm

end Wanderbooks.Apportion
