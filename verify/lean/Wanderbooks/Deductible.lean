import Wanderbooks.Prorate

/-!
# A category's deductible amount: the shares added exactly, rounded once,
half away from zero, at any size

Mirrors `crates/ledger-core/src/kernel.rs` `deductible_amount` and
`Amount::checked_div_round_cent` (rules.md B-32, B-21; owner decision D-67).
The kernel adds each account's exact share `book × percent ÷ 100`, then
divides by 100 once and rounds the total to the cent, half away from zero.
On `Decimal` it does this on integer mantissas over powers of ten: a book
is `b × 10^-p`, a percent `q × 10^-s`, so a share is `b·q ÷ 10^(p+s)` of a
percent-point, and the category is `(Σ bᵢ·qᵢ) ÷ (100 · 10^(p+s))` dollars,
found by exact integer division with the remainder deciding the tie, never
a float and never a per-share rounding.

The model is that, with the power of ten left to be any positive `D`:
books and percents are integers (`Share`), `D` the common denominator they
share (`10^(p+s)` above), and the exact category in dollars is `N ÷ (100·D)`
with `N = Σ bᵢ·qᵢ` (`sumProd`). In cents that is `N ÷ D`: the `÷ 100` of the
percent and the `× 100` of dollars-to-cents cancel, so the one rounding is
`roundHA N D`, the integer nearest `N / D`, ties away from zero. It is
built on `Prorate.roundUnit` (the Nat rounding of the proration kernel, with
the cent as the unit), applied to the magnitude and signed after, which is
what the kernel's `a_mag`, `b_mag` and `negative` do.

This proves the law, not the Rust code. Kani proves the code on `i64` for up
to three accounts and bounded books (`deductible_amount_rounds_once`,
`deductible_amount_refuses_overflow`); `tests/kernel_bridge.rs` and the exact
reference property carry it to `Decimal`; here the law holds for any number
of accounts and any sizes: within half a cent of the exact sum (`near`), a
tie to the cent farther from zero (`tie_away_pos`, `tie_away_neg`), never the
opposite sign (`sign`), odd (`neg`), a function of the exact sum alone
(`of_sum`, `of_perm`, `append`: rounded once, on the sum, and so not the
sum of the rounded shares, which `per_account_differs` shows can differ), and independent of how many decimal places
the books and percents carry (`scale_books`, `scale_percents`). Overflow
(the kernel returns None) is outside the model; integers do not overflow.
-/

namespace Wanderbooks.Deductible

open Wanderbooks.Prorate (roundUnit roundUnit_near roundUnit_scale)

/-- One account's share: its book and its deductible percent, as integers
over the category's common denominator. -/
abbrev Share := Int × Int

/-- The exact sum `Σ bookᵢ × percentᵢ` over the common denominator. -/
def sumProd : List Share → Int
  | [] => 0
  | (b, q) :: t => b * q + sumProd t

/-- `n / d` to the nearest whole cent, half away from zero: the magnitude
rounded half up (`Prorate.roundUnit`, unit one), then signed. -/
def roundHA (n : Int) (d : Nat) : Int :=
  if 0 ≤ n then (roundUnit n.natAbs d 1 : Int) else -(roundUnit n.natAbs d 1 : Int)

/-- The category's deductible amount, in cents: the exact sum `N`, over the
common denominator `D`, rounded once. -/
def deductible (D : Nat) (l : List Share) : Int := roundHA (sumProd l) D

/-! ## The rounding -/

theorem roundUnit_zero (d : Nat) (hd : 0 < d) : roundUnit 0 d 1 = 0 := by
  unfold roundUnit
  have : 2 * 0 + d * 1 < 2 * d * 1 := by omega
  rw [Nat.div_eq_of_lt this, Nat.mul_zero]

/-- Natural-number rounding, in integers: `2·d·r` within `d` of `2·m`. -/
private theorem unit_near_int (m d : Nat) (hd : 0 < d) :
    -(d : Int) ≤ 2 * (((d * roundUnit m d 1 : Nat) : Int) - m) ∧
    2 * (((d * roundUnit m d 1 : Nat) : Int) - m) ≤ d := by
  obtain ⟨h1, h2⟩ := roundUnit_near m d 1 hd Nat.one_pos
  have e : 2 * d * roundUnit m d 1 = 2 * (d * roundUnit m d 1) := Nat.mul_assoc 2 d _
  rw [e] at h1 h2
  generalize d * roundUnit m d 1 = P at h1 h2 ⊢
  omega

/-- **Near**: the result is within half a cent of the exact quotient, at any
size: `|2·(d·c − n)| ≤ d`, written without division. (Any other whole cent
is at least as far, so it is a nearest one.) -/
theorem roundHA_near (n : Int) (d : Nat) (hd : 0 < d) :
    -(d : Int) ≤ 2 * ((d : Int) * roundHA n d - n) ∧
    2 * ((d : Int) * roundHA n d - n) ≤ d := by
  have h := unit_near_int n.natAbs d hd
  have e : (d : Int) * (roundUnit n.natAbs d 1 : Nat) = ((d * roundUnit n.natAbs d 1 : Nat) : Int) := by
    push_cast; rfl
  unfold roundHA
  generalize d * roundUnit n.natAbs d 1 = P at h e
  split
  · rename_i hn
    have : (n.natAbs : Int) = n := Int.natAbs_of_nonneg hn
    rw [e]; omega
  · rename_i hn
    have : (n.natAbs : Int) = -n := by omega
    rw [Int.mul_neg, e]; omega

/-- **Odd symmetry**: rounding half away from zero treats the two signs
alike, so negating the amount negates the result. -/
theorem roundHA_neg (n : Int) (d : Nat) (hd : 0 < d) : roundHA (-n) d = -roundHA n d := by
  unfold roundHA
  rw [Int.natAbs_neg]
  by_cases h0 : n = 0
  · subst h0
    simp [roundUnit_zero d hd]
  · by_cases h : 0 ≤ n
    · have h' : ¬ 0 ≤ -n := by omega
      simp only [h, h', ↓reduceIte]
    · have h' : 0 ≤ -n := by omega
      simp only [h, h', ↓reduceIte, Int.neg_neg]

/-- **Sign**: the result is never of the opposite sign to the exact
quotient, and is zero when it is zero. (A small positive amount may round
to zero; none ever rounds across it. An `Int` has no negative zero.) -/
theorem roundHA_sign (n : Int) (d : Nat) (hd : 0 < d) :
    (0 ≤ n → 0 ≤ roundHA n d) ∧ (n ≤ 0 → roundHA n d ≤ 0) ∧ (n = 0 → roundHA n d = 0) := by
  refine ⟨?_, ?_, ?_⟩
  · intro h
    unfold roundHA
    simp [h]
  · intro h
    unfold roundHA
    by_cases h0 : n = 0
    · subst h0; simp [roundUnit_zero d hd]
    · have : ¬ 0 ≤ n := by omega
      simp [this]
  · intro h
    subst h
    unfold roundHA
    simp [roundUnit_zero d hd]

/-- **A tie at exactly a half cent goes away from zero** (positive side):
`n / d = k + 1/2` rounds to `k + 1`, the cent farther from zero. -/
theorem roundHA_tie_pos (n : Int) (d k : Nat) (hd : 0 < d)
    (h : 2 * n = (d : Int) * (2 * k + 1)) : roundHA n d = k + 1 ∧ n < (d : Int) * roundHA n d := by
  have hQ : (d : Int) * (2 * k + 1) = ((d * (2 * k + 1) : Nat) : Int) := by push_cast; rfl
  have hpos : 0 < d * (2 * k + 1) := Nat.mul_pos hd (by omega)
  have hn : 0 ≤ n := by rw [hQ] at h; omega
  have hm : 2 * n.natAbs = d * (2 * k + 1) := by
    have : (n.natAbs : Int) = n := Int.natAbs_of_nonneg hn
    rw [hQ] at h; omega
  have hr : roundUnit n.natAbs d 1 = k + 1 := by
    unfold roundUnit
    have k1 : d * (2 * k + 1) = 2 * (d * k) + d := by
      rw [Nat.mul_add, Nat.mul_left_comm, Nat.mul_one]
    have k2 : 2 * d * 1 * (k + 1) = 2 * (d * k) + 2 * d := by
      rw [Nat.mul_one, Nat.mul_add, Nat.mul_one, Nat.mul_assoc]
    have e : 2 * n.natAbs + d * 1 = 2 * d * 1 * (k + 1) := by
      rw [k2]; omega
    rw [e, Nat.mul_div_cancel_left _ (by omega)]
    omega
  have hc : roundHA n d = k + 1 := by
    unfold roundHA
    simp [hn, hr]
  refine ⟨hc, ?_⟩
  rw [hc, Int.mul_add, Int.mul_one]
  have k1 : (d : Int) * (2 * k + 1) = 2 * ((d : Int) * k) + d := by
    rw [Int.mul_add, Int.mul_left_comm, Int.mul_one]
  omega

/-- **A tie goes away from zero** (negative side): `n / d = −(k + 1/2)`
rounds to `−(k + 1)`. -/
theorem roundHA_tie_neg (n : Int) (d k : Nat) (hd : 0 < d)
    (h : 2 * n = -((d : Int) * (2 * k + 1))) :
    roundHA n d = -((k : Int) + 1) ∧ (d : Int) * roundHA n d < n := by
  have h' : 2 * (-n) = (d : Int) * (2 * k + 1) := by omega
  obtain ⟨h1, h2⟩ := roundHA_tie_pos (-n) d k hd h'
  rw [roundHA_neg n d hd] at h1 h2
  rw [Int.mul_neg] at h2
  constructor <;> omega

/-- Rounding is unchanged when the amount and its denominator are scaled
together by a positive factor: the same quotient at a finer denominator
rounds the same. -/
theorem roundHA_scale (n : Int) (d c : Nat) (hc : 0 < c) :
    roundHA ((c : Int) * n) (c * d) = roundHA n d := by
  unfold roundHA
  have hna : ((c : Int) * n).natAbs = c * n.natAbs := by
    rw [Int.natAbs_mul, Int.natAbs_natCast]
  have hcz : (0 : Int) < c := by omega
  rw [hna, roundUnit_scale _ _ _ _ hc]
  by_cases hn : 0 ≤ n
  · have : 0 ≤ (c : Int) * n := Int.mul_nonneg (by omega) hn
    simp only [hn, this, ↓reduceIte]
  · have : ¬ 0 ≤ (c : Int) * n := by
      have : (c : Int) * n < 0 := Int.mul_neg_of_pos_of_neg hcz (by omega)
      omega
    simp only [hn, this, ↓reduceIte]

/-! ## The category

The headline theorems: the rounding laws above, on the exact sum of a
category's shares. -/

theorem sumProd_append (l l' : List Share) : sumProd (l ++ l') = sumProd l + sumProd l' := by
  induction l with
  | nil => simp [sumProd]
  | cons s t ih => obtain ⟨b, q⟩ := s; simp only [List.cons_append, sumProd, ih]; omega

theorem sumProd_perm {l l' : List Share} (h : l.Perm l') : sumProd l = sumProd l' := by
  induction h with
  | nil => rfl
  | cons s _ ih => obtain ⟨b, q⟩ := s; simp only [sumProd, ih]
  | swap s t l =>
    obtain ⟨b, q⟩ := s; obtain ⟨b', q'⟩ := t
    simp only [sumProd]; omega
  | trans _ _ ih ih' => exact ih.trans ih'

theorem sumProd_neg_books (l : List Share) :
    sumProd (l.map fun s => (-s.1, s.2)) = -sumProd l := by
  induction l with
  | nil => simp [sumProd]
  | cons s t ih =>
    obtain ⟨b, q⟩ := s
    simp only [List.map_cons, sumProd, ih, Int.neg_mul]
    omega

theorem sumProd_scale_books (c : Int) (l : List Share) :
    sumProd (l.map fun s => (c * s.1, s.2)) = c * sumProd l := by
  induction l with
  | nil => simp [sumProd]
  | cons s t ih =>
    obtain ⟨b, q⟩ := s
    simp only [List.map_cons, sumProd, ih, Int.mul_add, Int.mul_assoc]

theorem sumProd_scale_percents (c : Int) (l : List Share) :
    sumProd (l.map fun s => (s.1, c * s.2)) = c * sumProd l := by
  induction l with
  | nil => simp [sumProd]
  | cons s t ih =>
    obtain ⟨b, q⟩ := s
    simp only [List.map_cons, sumProd, ih, Int.mul_add, Int.mul_left_comm b c q]

/-- **Near**: the category's deductible amount `c` (in cents) is within
half a cent of the exact sum `N / D`, for any number of accounts and any
sizes: `|2·(D·c − N)| ≤ D`, written without division. -/
theorem near (D : Nat) (l : List Share) (hD : 0 < D) :
    -(D : Int) ≤ 2 * ((D : Int) * deductible D l - sumProd l) ∧
    2 * ((D : Int) * deductible D l - sumProd l) ≤ D :=
  roundHA_near (sumProd l) D hD

/-- **Ties go away from zero** (positive): an exact sum of `k + 1/2` cents
gives `k + 1`, the cent farther from zero. -/
theorem tie_away_pos (D : Nat) (l : List Share) (k : Nat) (hD : 0 < D)
    (h : 2 * sumProd l = (D : Int) * (2 * k + 1)) :
    deductible D l = k + 1 ∧ sumProd l < (D : Int) * deductible D l :=
  roundHA_tie_pos (sumProd l) D k hD h

/-- **Ties go away from zero** (negative): an exact sum of `−(k + 1/2)`
cents gives `−(k + 1)`. -/
theorem tie_away_neg (D : Nat) (l : List Share) (k : Nat) (hD : 0 < D)
    (h : 2 * sumProd l = -((D : Int) * (2 * k + 1))) :
    deductible D l = -((k : Int) + 1) ∧ (D : Int) * deductible D l < sumProd l :=
  roundHA_tie_neg (sumProd l) D k hD h

/-- **Sign**: never the opposite sign to the exact sum, and zero when the
sum is zero. -/
theorem sign (D : Nat) (l : List Share) (hD : 0 < D) :
    (0 ≤ sumProd l → 0 ≤ deductible D l) ∧ (sumProd l ≤ 0 → deductible D l ≤ 0) ∧
    (sumProd l = 0 → deductible D l = 0) :=
  roundHA_sign (sumProd l) D hD

/-- **Odd symmetry**: negating every book negates the result. -/
theorem neg (D : Nat) (l : List Share) (hD : 0 < D) :
    deductible D (l.map fun s => (-s.1, s.2)) = -deductible D l := by
  unfold deductible
  rw [sumProd_neg_books, roundHA_neg _ _ hD]

/-- **Round once, (a)**: the result depends on the shares only through
their exact sum. -/
theorem of_sum (D : Nat) {l l' : List Share} (h : sumProd l = sumProd l') :
    deductible D l = deductible D l' := by
  unfold deductible; rw [h]

/-- Reordering the accounts does not change the result. -/
theorem of_perm (D : Nat) {l l' : List Share} (h : l.Perm l') :
    deductible D l = deductible D l' :=
  of_sum D (sumProd_perm h)

/-- Two groups of accounts are rounded together, on their exact sum, not
each alone. -/
theorem append (D : Nat) (l l' : List Share) :
    deductible D (l ++ l') = roundHA (sumProd l + sumProd l') D := by
  unfold deductible; rw [sumProd_append]

/-- An account's share split in two (its percent as the sum of two) gives
the same result. -/
theorem split_percent (D : Nat) (b q q' : Int) (t : List Share) :
    deductible D ((b, q + q') :: t) = deductible D ((b, q) :: (b, q') :: t) := by
  apply of_sum
  simp only [sumProd, Int.mul_add]
  omega

/-- **Round once, (b)**: summing per-account roundings can differ from the
category's one rounding. Two accounts of 1.01 (101 cents) at 50% (shares
over `D = 100`, the books' two places): 1.01 once, 1.02 by account. -/
theorem per_account_differs :
    deductible 100 [(101, 50), (101, 50)] = 101 ∧
    deductible 100 [(101, 50)] + deductible 100 [(101, 50)] = 102 ∧
    deductible 100 [(101, 50), (101, 50)] ≠
      deductible 100 [(101, 50)] + deductible 100 [(101, 50)] := by
  decide

/-- The kernel's second scenario: 1.05 at 50% and 1.05 at 30% is 0.84
once (0.525 + 0.315 = 0.84), not 0.85 by account (0.53 + 0.32). -/
theorem per_account_differs_mixed :
    deductible 100 [(105, 50), (105, 30)] = 84 ∧
    deductible 100 [(105, 50)] + deductible 100 [(105, 30)] = 85 := by
  decide

/-- The kernel's third scenario: 333.33 at 50% is 166.665, a tie, so
166.67. -/
theorem tie_example : deductible 100 [(33333, 50)] = 16667 := by
  decide

/-- **Scaling, books**: writing the books at a finer common denominator
(each book times `c`, `D` times `c`) does not change the result: it is
independent of how many decimal places the books carry. -/
theorem scale_books (D c : Nat) (l : List Share) (hc : 0 < c) :
    deductible (c * D) (l.map fun s => ((c : Int) * s.1, s.2)) = deductible D l := by
  unfold deductible
  rw [sumProd_scale_books, roundHA_scale _ _ _ hc]

/-- **Scaling, percents**: likewise for the percents' decimal places. -/
theorem scale_percents (D c : Nat) (l : List Share) (hc : 0 < c) :
    deductible (c * D) (l.map fun s => (s.1, (c : Int) * s.2)) = deductible D l := by
  unfold deductible
  rw [sumProd_scale_percents, roundHA_scale _ _ _ hc]

end Wanderbooks.Deductible
