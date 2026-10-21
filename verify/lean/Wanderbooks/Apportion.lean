import Wanderbooks.Closing

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

/-! ## Bounded drift

With exact shares there is no sub-unit fraction: every amount is a whole
number of minor units (B-3) and every product `|T| × share` is exact
(`periods::allocate_cumulative` INVARIANT 4), so the remainders, scaled to
whole numbers below `m`, sum to whole units of `m`. Then a part that gets
a unit had a positive remainder, and every apportioned part is under one
unit from its exact share (rules.md B-13). -/

/-- Values in `[0, m)` that are zero wherever `p` fails sum to at most
`(m - 1)` per element where `p` holds. -/
theorem sum_le_count_mul (r : Nat → Int) (p : Nat → Bool) (m : Int) : ∀ (l : List Nat),
    (∀ j ∈ l, 0 ≤ r j ∧ r j < m) → (∀ j ∈ l, p j = false → r j = 0) →
    (l.map r).sum ≤ ((l.filter p).length : Int) * (m - 1)
  | [], _, _ => by simp
  | a :: t, hr, hz => by
    have ih := sum_le_count_mul r p m t (fun j hj => hr j (List.mem_cons_of_mem a hj))
      (fun j hj => hz j (List.mem_cons_of_mem a hj))
    have ha := hr a (List.mem_cons_self ..)
    simp only [List.map_cons, List.sum_cons, List.filter_cons]
    cases hp : p a
    · have := hz a (List.mem_cons_self ..) hp
      simp only [Bool.false_eq_true, ↓reduceIte]
      omega
    · simp only [↓reduceIte, List.length_cons]
      have e : (((t.filter p).length + 1 : Nat) : Int) * (m - 1)
          = ((t.filter p).length : Int) * (m - 1) + (m - 1) := by
        rw [Int.natCast_add, Int.add_mul, Int.natCast_one, Int.one_mul]
      rw [e]
      omega

/-- With remainders in `[0, m)` summing to exactly `u · m` (no sub-unit
fraction), every part ranked below `u` (so given a unit) has a positive
remainder. -/
theorem rem_pos_of_rank_lt (r : Nat → Int) (n u : Nat) (m : Int)
    (hr : ∀ i < n, 0 ≤ r i ∧ r i < m)
    (hs : ((List.range n).map r).sum = u * m)
    {i : Nat} (hi : i < n) (hk : rank r n i < u) : 0 < r i := by
  apply Classical.byContradiction
  intro hneg
  have hri : r i = 0 := by have := hr i hi; omega
  -- Every part not ahead of `i` has a remainder no larger than `i`'s: 0.
  have hz : ∀ j ∈ List.range n, ahead r j i = false → r j = 0 := by
    intro j hj hna
    have hj' := List.mem_range.mp hj
    have hrj := hr j hj'
    by_cases e : j = i
    · subst e
      exact hri
    · rcases ahead_total r e with h | h
      · rw [h] at hna
        exact absurd hna (by decide)
      · simp only [ahead, Bool.or_eq_true, Bool.and_eq_true, decide_eq_true_eq] at h
        omega
  have hle := sum_le_count_mul r (fun j => ahead r j i) m (List.range n)
    (fun j hj => hr j (List.mem_range.mp hj)) hz
  have hm : 0 < m := by have := hr i hi; omega
  have hrank : ((List.range n).filter (fun j => ahead r j i)).length = rank r n i := rfl
  rw [hrank, hs] at hle
  have h1 : (rank r n i : Int) * (m - 1) ≤ ((u : Int) - 1) * (m - 1) :=
    Int.mul_le_mul_of_nonneg_right (by omega) (by omega)
  have h2 : ((u : Int) - 1) * (m - 1) = (u : Int) * m - u - m + 1 := by
    simp only [Int.sub_mul, Int.mul_sub, Int.one_mul, Int.mul_one]
    omega
  omega

/-- **Bounded drift of one apportionment** (rules.md B-13): scaled by `m`,
each apportioned part (rounded down, plus its unit if any) is within one
unit of its exact share `parts i + r i / m`, when there is no sub-unit
fraction. -/
theorem apportion_drift (parts r : Nat → Int) (n u : Nat) (m : Int)
    (hr : ∀ i < n, 0 ≤ r i ∧ r i < m)
    (hs : ((List.range n).map r).sum = u * m) {i : Nat} (hi : i < n) :
    -m < (parts i + add r n u 1 0 i) * m - (parts i * m + r i) ∧
    (parts i + add r n u 1 0 i) * m - (parts i * m + r i) < m := by
  have hri := hr i hi
  unfold add
  rw [ite_self, Int.add_zero]
  by_cases hk : rank r n i < u
  · have hpos := rem_pos_of_rank_lt r n u m hr hs hi hk
    simp only [hk, ↓reduceIte]
    rw [Int.add_mul, Int.one_mul]
    omega
  · simp only [hk, ↓reduceIte, Int.add_zero]
    omega

/-- Σ c · f = c · Σ f. -/
theorem sum_map_mul_left (c : Int) (f : Nat → Int) : ∀ (l : List Nat),
    (l.map (fun j => c * f j)).sum = c * (l.map f).sum
  | [] => by simp
  | a :: t => by simp [sum_map_mul_left c f t, Int.mul_add]

/-- Σ -f = -Σ f. -/
theorem sum_map_neg (f : Nat → Int) : ∀ (l : List Nat),
    (l.map (fun j => -f j)).sum = -(l.map f).sum
  | [] => by simp
  | a :: t => by simp [sum_map_neg f t]; omega

/-- Values at most `c` sum to at most `c` per element. -/
theorem sum_le_length_mul (f : Nat → Int) (c : Int) : ∀ (l : List Nat),
    (∀ j ∈ l, f j ≤ c) → (l.map f).sum ≤ (l.length : Int) * c
  | [], _ => by simp
  | a :: t, h => by
    have ih := sum_le_length_mul f c t (fun j hj => h j (List.mem_cons_of_mem a hj))
    have ha := h a (List.mem_cons_self ..)
    simp only [List.map_cons, List.sum_cons, List.length_cons]
    rw [Int.natCast_add, Int.add_mul, Int.natCast_one, Int.one_mul]
    omega

/-- Nonnegative values have a nonnegative sum. -/
theorem sum_nonneg (f : Nat → Int) : ∀ (l : List Nat), (∀ j ∈ l, 0 ≤ f j) → 0 ≤ (l.map f).sum
  | [], _ => by simp
  | a :: t, h => by
    have ih := sum_nonneg f t (fun j hj => h j (List.mem_cons_of_mem a hj))
    have ha := h a (List.mem_cons_self ..)
    simp only [List.map_cons, List.sum_cons]
    omega

/-- Member `i`'s share of `x` rounded down to the unit: `x · w i / W`
(`periods::allocate_cumulative`'s floor; shares `w i / W`). -/
def floorW (w : Nat → Int) (W x : Int) (i : Nat) : Int := x * w i / W

/-- What rounding down took from it, scaled by `W`: `x · w i % W` (it
ranks the members as the Decimal remainder does). -/
def remW (w : Nat → Int) (W x : Int) (i : Nat) : Int := x * w i % W

/-- The whole units the floors fall short of `x`. -/
def shortW (w : Nat → Int) (W : Int) (n : Nat) (x : Int) : Nat :=
  (x - ((List.range n).map (floorW w W x)).sum).toNat

/-- The model of `periods::allocate_cumulative`'s split of a gain (`x ≥
0`) at whole minor units: each floor, plus a unit to each of the members
with the largest remainders (ties to the earlier member). -/
def allocNat (w : Nat → Int) (W : Int) (n : Nat) (x : Int) (i : Nat) : Int :=
  floorW w W x i + add (remW w W x) n (shortW w W n x) 1 0 i

/-- With the sign: a loss splits as the negation of the same gain
(`allocate_cumulative` scenario 3). -/
def allocW (w : Nat → Int) (W : Int) (n : Nat) (x : Int) (i : Nat) : Int :=
  if x < 0 then -allocNat w W n (-x) i else allocNat w W n x i

/-- The floors and remainders of a gain, with shares `w i / W` summing to
one: the remainders sum to whole units, `W · (x − Σ floors)`, each in
`[0, W)`, and the floors fall short by fewer units than there are
members. -/
theorem remW_facts (w : Nat → Int) (W : Int) (n : Nat) (x : Int) (hW : 0 < W)
    (hsum : ((List.range n).map w).sum = W) (hn : 0 < n) :
    (∀ i < n, 0 ≤ remW w W x i ∧ remW w W x i < W) ∧
    ((List.range n).map (remW w W x)).sum = (shortW w W n x : Int) * W ∧
    x - ((List.range n).map (floorW w W x)).sum = (shortW w W n x : Int) ∧
    shortW w W n x ≤ n := by
  have hW0 : W ≠ 0 := by omega
  have bounds : ∀ i < n, 0 ≤ remW w W x i ∧ remW w W x i < W := fun i _ =>
    ⟨Int.emod_nonneg _ hW0, Int.emod_lt_of_pos _ hW⟩
  -- x · w i = W · floor + rem, summed: x · W = W · Σ floors + Σ rems.
  have each : ∀ i, x * w i = W * floorW w W x i + remW w W x i := fun i =>
    (Int.mul_ediv_add_emod (x * w i) W).symm
  have total : x * W = W * ((List.range n).map (floorW w W x)).sum
      + ((List.range n).map (remW w W x)).sum := by
    have h1 : ((List.range n).map (fun j => x * w j)).sum = x * W := by
      rw [sum_map_mul_left, hsum]
    have h2 : ((List.range n).map (fun j => x * w j)).sum
        = ((List.range n).map (fun j => W * floorW w W x j + remW w W x j)).sum := by
      congr 1
      apply List.map_congr_left
      intro i _
      exact each i
    rw [← h1, h2, sum_map_add (fun j => W * floorW w W x j) (remW w W x), sum_map_mul_left]
  let d := x - ((List.range n).map (floorW w W x)).sum
  have hd : ((List.range n).map (remW w W x)).sum = W * d := by
    show _ = W * (x - _)
    rw [Int.mul_sub, Int.mul_comm W x]
    omega
  have rems_nonneg := sum_nonneg (remW w W x) (List.range n)
    (fun j hj => (bounds j (List.mem_range.mp hj)).1)
  have rems_lt := sum_le_length_mul (remW w W x) (W - 1) (List.range n)
    (fun j hj => by have := (bounds j (List.mem_range.mp hj)).2; omega)
  rw [List.length_range] at rems_lt
  have d_nonneg : 0 ≤ d := by
    apply Classical.byContradiction
    intro h
    have : W * d < 0 := Int.mul_neg_of_pos_of_neg hW (by omega)
    omega
  have d_lt : d < n := by
    apply Classical.byContradiction
    intro h
    have h1 : (n : Int) * W ≤ d * W := Int.mul_le_mul_of_nonneg_right (by omega) (by omega)
    have h2 : (n : Int) * (W - 1) = n * W - n := by
      rw [Int.mul_sub, Int.mul_one]
    rw [Int.mul_comm W d] at hd
    omega
  have hshort : (shortW w W n x : Int) = d := Int.toNat_of_nonneg d_nonneg
  refine ⟨bounds, ?_, hshort.symm, ?_⟩
  · rw [hshort, hd, Int.mul_comm]
  · have : ((shortW w W n x : Nat) : Int) < n := by rw [hshort]; exact d_lt
    omega

/-- **The split of a gain sums to it**, exactly. -/
theorem allocNat_sum (w : Nat → Int) (W : Int) (n : Nat) (x : Int) (hW : 0 < W)
    (hsum : ((List.range n).map w).sum = W) (hn : 0 < n) :
    ((List.range n).map (allocNat w W n x)).sum = x := by
  obtain ⟨_, _, hx, hu⟩ := remW_facts w W n x hW hsum hn
  apply apportion_sum (floorW w W x) (remW w W x) n (shortW w W n x) 1 0 x hn hu
  rw [hx]
  omega

/-- **`allocate_cumulative`'s split sums to the amount**, gain or loss. -/
theorem allocW_sum (w : Nat → Int) (W : Int) (n : Nat) (x : Int) (hW : 0 < W)
    (hsum : ((List.range n).map w).sum = W) (hn : 0 < n) :
    ((List.range n).map (allocW w W n x)).sum = x := by
  unfold allocW
  by_cases hx : x < 0
  · simp only [hx, ↓reduceIte]
    rw [sum_map_neg, allocNat_sum w W n (-x) hW hsum hn]
    omega
  · simp only [hx, ↓reduceIte]
    exact allocNat_sum w W n x hW hsum hn

/-- **Bounded drift** (rules.md B-13): each member's part of an amount is
under one minor unit from their exact share, `|allocW x i · W − x · w i| <
W`, gain or loss. -/
theorem allocW_drift (w : Nat → Int) (W : Int) (n : Nat) (x : Int) (hW : 0 < W)
    (hsum : ((List.range n).map w).sum = W) (hn : 0 < n) {i : Nat} (hi : i < n) :
    -W < allocW w W n x i * W - x * w i ∧ allocW w W n x i * W - x * w i < W := by
  have gain : ∀ y, 0 ≤ y →
      -W < allocNat w W n y i * W - y * w i ∧ allocNat w W n y i * W - y * w i < W := by
    intro y _
    obtain ⟨hr, hs, _, _⟩ := remW_facts w W n y hW hsum hn
    have h := apportion_drift (floorW w W y) (remW w W y) n (shortW w W n y) W hr hs hi
    have e : floorW w W y i * W + remW w W y i = y * w i := by
      rw [Int.mul_comm]
      exact Int.mul_ediv_add_emod (y * w i) W
    rw [e] at h
    exact h
  unfold allocW
  by_cases hx : x < 0
  · simp only [hx, ↓reduceIte]
    have h := gain (-x) (by omega)
    simp only [Int.neg_mul] at h ⊢
    omega
  · simp only [hx, ↓reduceIte]
    exact gain x (by omega)

/-- **No member's running total is ever a minor unit from their exact
share**, after a cumulative close, whatever earlier closes gave them
(rules.md B-13; D-10): `Closing.trueUp` with `allocate_cumulative`'s
split (Kani `cumulative_close_never_drifts_a_unit_at_*` check the code on
small amounts). -/
theorem allocW_cumulative_drift (w : Nat → Int) (W : Int) (n : Nat) (hW : 0 < W)
    (hsum : ((List.range n).map w).sum = W) (hn : 0 < n) (closed net : Int)
    (given : Nat → Int) {i : Nat} (hi : i < n) :
    -W < (given i + trueUp (allocW w W n) closed net given i) * W - (closed + net) * w i ∧
    (given i + trueUp (allocW w W n) closed net given i) * W - (closed + net) * w i < W := by
  rw [trueUp_running]
  exact allocW_drift w W n (closed + net) hW hsum hn hi

/-- **A cumulative close hands out exactly its net** with
`allocate_cumulative`'s split, gain or loss, whatever the earlier closes
gave, as long as they gave what they closed. -/
theorem allocW_cumulative_total (w : Nat → Int) (W : Int) (n : Nat) (hW : 0 < W)
    (hsum : ((List.range n).map w).sum = W) (hn : 0 < n) (closed net : Int)
    (given : Nat → Int) (hg : ((List.range n).map given).sum = closed) :
    ((List.range n).map (trueUp (allocW w W n) closed net given)).sum = net :=
  trueUp_total (allocW w W n) (List.range n) (fun x => allocW_sum w W n x hW hsum hn)
    closed net given hg

end Wanderbooks.Apportion
