/-!
# Proration by days: the parts sum to the total, and none is negative,
for any number of parts

Mirrors `crates/ledger-core/src/kernel.rs` `prorated_through` and
`prorate_by_days` (depreciation, rules.md B-15; daily schedules, B-16).
The kernel computes, through any day of a span, the elapsed share of the
span as a fraction (the months held before, each as days held over days in
the month, plus the days of this one; in the last month, what the others
leave spread over its days). Here that share is `w / V`: `V` the whole
span over any common denominator, `w` the elapsed part of it, which the
walk only ever raises. The charge through it is `cum`: the total times the
share, rounded half up to a multiple of `unit`, capped at the total, and
the total itself once the whole span has elapsed (the kernel's last day,
exact even for a total that isn't a whole number of units).

Kani proves the kernel's laws for up to three parts at bounded sizes
(`prorate_parts_sum_to_total`, `prorate_last_part_absorbs_the_remainder`),
and each charge within half a unit of its exact share
(`a_months_charge_is_within_half_a_cent` and its siblings). Here they
hold for any number of parts and any sizes: `cum` never falls as the
share grows, never exceeds the total, and ends at it, so the parts
(differences of `cum` at each part's end) telescope to the total and are
each at least zero; each charge is within half a unit of its exact share,
so each part is within one unit of its days times the rate (`cum_near`,
`part_near`). The last section writes out the kernel's two formulas and
proves them `cum` (`chargeMid_eq_cum`, `chargeLast_eq_cum`), so these are
laws of the kernel's own arithmetic, not only of the model.
-/

namespace Wanderbooks.Prorate

/-- `num / den` rounded half up to a multiple of `unit`: the kernel's
`round_to_unit` (its `r >= step - r` is `2r >= step`, which is the floor of
`num / step + 1/2`). -/
def roundUnit (num den unit : Nat) : Nat :=
  unit * ((2 * num + den * unit) / (2 * den * unit))

/-- The charge of total `t` once `w` of the span's `V` has elapsed. -/
def cum (t unit V w : Nat) : Nat :=
  if w = V then t else min t (roundUnit (t * w) V unit)

/-- Rounding never falls as the amount rounded grows. -/
theorem roundUnit_mono (den unit : Nat) {a b : Nat} (h : a ≤ b) :
    roundUnit a den unit ≤ roundUnit b den unit := by
  unfold roundUnit
  apply Nat.mul_le_mul_left
  apply Nat.div_le_div_right
  omega

/-- The charge never exceeds the total. -/
theorem cum_le (t unit V w : Nat) : cum t unit V w ≤ t := by
  unfold cum
  split
  · exact Nat.le_refl t
  · exact Nat.min_le_left _ _

/-- Once the whole span has elapsed, the charge is the total exactly. -/
theorem cum_full (t unit V : Nat) : cum t unit V V = t := by
  simp [cum]

/-- Before anything has elapsed, nothing is charged. -/
theorem cum_zero (t unit V : Nat) (hV : 0 < V) (hu : 0 < unit) : cum t unit V 0 = 0 := by
  have hne : (0 : Nat) ≠ V := by omega
  have hlt : V * unit < 2 * V * unit := by
    have : 0 < V * unit := Nat.mul_pos hV hu
    rw [Nat.mul_assoc]
    omega
  simp [cum, hne, roundUnit, Nat.div_eq_of_lt hlt]

/-- **The charge never falls** as the elapsed share grows, up to the
whole span. -/
theorem cum_mono (t unit V : Nat) {w w' : Nat} (h : w ≤ w') (hw' : w' ≤ V) :
    cum t unit V w ≤ cum t unit V w' := by
  by_cases e' : w' = V
  · subst e'
    rw [cum_full]
    exact cum_le t unit w' w
  · have e : w ≠ V := by omega
    simp only [cum, e, e', ite_false]
    have r := roundUnit_mono V unit (Nat.mul_le_mul_left t h)
    omega

/-- The parts of a split: at each part's end (`w` its elapsed share), the
charge through it less the charge before it (the kernel's `out[i]`,
`prorate_by_days`). -/
def parts (t unit V : Nat) : Nat → List Nat → List Int
  | _, [] => []
  | prev, w :: ws => ((cum t unit V w : Int) - cum t unit V prev) :: parts t unit V w ws

/-- **The parts telescope**: they sum to the charge at the last part's end
less the charge where they start (the shape of `runPart_telescopes`). -/
theorem parts_telescope (t unit V : Nat) (prev : Nat) (ws : List Nat) :
    (parts t unit V prev ws).sum = (cum t unit V (ws.getLastD prev) : Int) - cum t unit V prev := by
  induction ws generalizing prev with
  | nil => simp [parts]
  | cons w rest ih =>
    simp only [parts, List.sum_cons, ih]
    cases rest with
    | nil => simp
    | cons _ _ => simp only [List.getLastD_cons]; omega

/-- **The parts sum to the total**, for any number of parts: a span
walked from nothing elapsed to all of it. -/
theorem prorate_total (t unit V : Nat) (hV : 0 < V) (hu : 0 < unit) (ws : List Nat)
    (hlast : ws.getLastD 0 = V) : (parts t unit V 0 ws).sum = t := by
  rw [parts_telescope, hlast, cum_full, cum_zero t unit V hV hu]
  simp

/-- The parts' ends rise: each at least the one before, none past the
span. -/
def Rising (V : Nat) : Nat → List Nat → Prop
  | _, [] => True
  | prev, w :: ws => prev ≤ w ∧ w ≤ V ∧ Rising V w ws

/-- **No part is negative** when the parts' ends rise, for any number of
parts. -/
theorem prorate_nonneg (t unit V : Nat) (prev : Nat) (ws : List Nat) (h : Rising V prev ws) :
    ∀ x ∈ parts t unit V prev ws, 0 ≤ x := by
  induction ws generalizing prev with
  | nil => simp [parts]
  | cons w rest ih =>
    obtain ⟨hpw, hwV, hrest⟩ := h
    intro x hx
    simp only [parts, List.mem_cons] at hx
    rcases hx with hx | hx
    · subst hx
      have := cum_mono t unit V hpw hwV
      omega
    · exact ih w hrest x hx

/-- **Each part is within one unit of its exact share** when each charge
is within half a unit of its exact one (Kani's
`a_months_charge_is_within_half_a_cent` and its three siblings prove
that of the kernel): a part is the difference of two charges over the same
denominator `D` (W, the span in MONTH_L-ths of a month), so its error is
at most the two halves. With `x - y` = total × days × MONTH_L / days in
the month and `D` = months × MONTH_L, dividing through by MONTH_L / days in
the month gives the plan's form: |part × months × days in month − total ×
days| <= unit × months × days in month. -/
theorem part_within_a_unit (a b D x y u : Int)
    (ha : -(u * D) ≤ 2 * (a * D - x) ∧ 2 * (a * D - x) ≤ u * D)
    (hb : -(u * D) ≤ 2 * (b * D - y) ∧ 2 * (b * D - y) ≤ u * D) :
    -(u * D) ≤ (a - b) * D - (x - y) ∧ (a - b) * D - (x - y) ≤ u * D := by
  rw [Int.sub_mul]
  omega

/-! ## The kernel's own formula

`prorated_through` charges in two ways. Through a part but the span's
last, the elapsed share is `e / W` (`W` = months × MONTH_L, `e` = before
+ k × MONTH_L / days in the month): `chargeMid`. Through `k` of the `d`
days of the last part, after `b` of `W`, it is `(b·d + (W − b)·k) / (W·d)`,
and the total itself on its last day: `chargeLast`. Both are `cum`, over
`W` and over `W·d` (`chargeMid_eq_cum`, `chargeLast_eq_cum`), and the two
meet where the last month starts (`chargeLast_start`, rounding being
unchanged when the amount and its denominator are scaled together). So
`cum`'s laws are the kernel's, at any size: through the last month the
charge never falls (`chargeLast_mono`), the split of any number of parts
sums to the total with none negative (`kernel_parts_total`,
`kernel_parts_nonneg`), and each part but the last is within one unit of
its days times the rate (`part_near`). A calendar walk keeps every part's
end short of the span's (rules.md B-15: the first month is at most one,
then whole months), which is the hypothesis `e < W`. -/

/-- **Rounding is within half a unit**, at any size (a tie goes up):
`2·den·r ≤ 2·num + den·unit` and `2·num < 2·den·r + den·unit`. -/
theorem roundUnit_near (num den unit : Nat) (hd : 0 < den) (hu : 0 < unit) :
    2 * den * roundUnit num den unit ≤ 2 * num + den * unit ∧
    2 * num < 2 * den * roundUnit num den unit + den * unit := by
  unfold roundUnit
  have hs : 0 < 2 * den * unit := Nat.mul_pos (Nat.mul_pos (by omega) hd) hu
  have hdm := Nat.div_add_mod (2 * num + den * unit) (2 * den * unit)
  have hml := Nat.mod_lt (2 * num + den * unit) hs
  generalize (2 * num + den * unit) / (2 * den * unit) = q at *
  generalize (2 * num + den * unit) % (2 * den * unit) = m at *
  have e : 2 * den * (unit * q) = 2 * den * unit * q := by
    rw [Nat.mul_assoc (2 * den) unit q]
  have hS : 2 * den * unit = 2 * (den * unit) := Nat.mul_assoc 2 den unit
  rw [e]
  generalize 2 * den * unit * q = P at *
  generalize 2 * den * unit = S at *
  generalize den * unit = DU at *
  omega

/-- Rounding is unchanged when the amount and its denominator are scaled
together. -/
theorem roundUnit_scale (n d u c : Nat) (hc : 0 < c) :
    roundUnit (c * n) (c * d) u = roundUnit n d u := by
  unfold roundUnit
  have h1 : 2 * (c * n) + c * d * u = c * (2 * n + d * u) := by
    rw [Nat.mul_add, Nat.mul_left_comm 2 c n, Nat.mul_assoc c d u]
  have h2 : 2 * (c * d) * u = c * (2 * d * u) := by
    rw [Nat.mul_left_comm 2 c d, Nat.mul_assoc c (2 * d) u]
  rw [h1, h2, Nat.mul_div_mul_left _ _ hc]

/-- **The charge is within half a unit of its exact share** `t·w / V`, at
any size, short of the span's end (where it is the total): the cap at the
total included. -/
theorem cum_near (t u V w : Nat) (hV : 0 < V) (hu : 0 < u) (hw : w < V) :
    2 * V * cum t u V w ≤ 2 * (t * w) + V * u ∧
    2 * (t * w) < 2 * V * cum t u V w + V * u := by
  have hne : w ≠ V := by omega
  simp only [cum, hne, ite_false]
  obtain ⟨h1, h2⟩ := roundUnit_near (t * w) V u hV hu
  have htw : t * w ≤ t * V := Nat.mul_le_mul_left t (Nat.le_of_lt hw)
  have hVu : 0 < V * u := Nat.mul_pos hV hu
  by_cases hc : t ≤ roundUnit (t * w) V u
  · rw [Nat.min_eq_left hc]
    have hm : 2 * V * t ≤ 2 * V * roundUnit (t * w) V u := Nat.mul_le_mul_left _ hc
    have e : 2 * V * t = 2 * (t * V) := by rw [Nat.mul_assoc, Nat.mul_comm V t]
    constructor <;> omega
  · rw [Nat.min_eq_right (by omega)]
    exact ⟨h1, h2⟩

/-- **Each part is within one unit of its exact share**, at any size: a
part `c' − c` of the charge (its ends `w ≤ w'` short of the span's end),
against its exact share `t·(w' − w) / V`:
`|(c' − c)·V − t·(w' − w)| ≤ u·V`, written without subtraction. Over
the span `V` = months × MONTH_L and `w' − w` = days × MONTH_L / days in
the month, this is the plan's "each part equals days × rate within one
unit" (plan/served.md P3 defect 9). -/
theorem part_near (t u V w w' : Nat) (hV : 0 < V) (hu : 0 < u) (hw' : w' < V)
    (h : w ≤ w') :
    cum t u V w' * V + t * w ≤ cum t u V w * V + t * w' + u * V ∧
    cum t u V w * V + t * w' ≤ cum t u V w' * V + t * w + u * V := by
  obtain ⟨a1, a2⟩ := cum_near t u V w' hV hu hw'
  obtain ⟨b1, b2⟩ := cum_near t u V w hV hu (by omega)
  have e1 : 2 * V * cum t u V w' = 2 * (cum t u V w' * V) := by
    rw [Nat.mul_assoc, Nat.mul_comm V]
  have e2 : 2 * V * cum t u V w = 2 * (cum t u V w * V) := by
    rw [Nat.mul_assoc, Nat.mul_comm V]
  have e3 : V * u = u * V := Nat.mul_comm V u
  constructor <;> omega

/-- **A total in whole units is charged in whole units**: every charge is
then a multiple of the unit, and so is every part (a difference of two). -/
theorem cum_dvd (t u V w : Nat) (h : u ∣ t) : u ∣ cum t u V w := by
  unfold cum roundUnit
  split
  · exact h
  · rcases Nat.le_total t (u * ((2 * (t * w) + V * u) / (2 * V * u))) with hc | hc
    · rw [Nat.min_eq_left hc]; exact h
    · rw [Nat.min_eq_right hc]; exact Nat.dvd_mul_right u _

/-- The kernel's charge through a part but the span's last
(`prorated_through`, `last = false`): `e` of the span's `W`. -/
def chargeMid (t u W e : Nat) : Nat := min t (roundUnit (t * e) W u)

/-- The kernel's charge through `k` of the `d` days of the span's last
part, after `b` of `W` (`prorated_through`, `last = true`). -/
def chargeLast (t u W b d k : Nat) : Nat :=
  if k = d then t else min t (roundUnit (t * (b * d + (W - b) * k)) (W * d) u)

/-- A part but the last, short of the span's end, is charged `cum`. -/
theorem chargeMid_eq_cum (t u W e : Nat) (h : e < W) : chargeMid t u W e = cum t u W e := by
  have : e ≠ W := by omega
  simp [chargeMid, cum, this]

/-- The span's last part is charged `cum` over `W·d`. -/
theorem chargeLast_eq_cum (t u W b d k : Nat) (hb : b < W) (hk : k ≤ d) :
    chargeLast t u W b d k = cum t u (W * d) (b * d + (W - b) * k) := by
  by_cases hkd : k = d
  · subst hkd
    have e : b * k + (W - b) * k = W * k := by
      rw [← Nat.add_mul, Nat.add_sub_of_le (Nat.le_of_lt hb)]
    simp [chargeLast, cum, e]
  · have hlt : (W - b) * k < (W - b) * d :=
      (Nat.mul_lt_mul_left (by omega : 0 < W - b)).mpr (by omega)
    have e : b * d + (W - b) * d = W * d := by
      rw [← Nat.add_mul, Nat.add_sub_of_le (Nat.le_of_lt hb)]
    have hne : b * d + (W - b) * k ≠ W * d := by omega
    simp [chargeLast, cum, hkd, hne]

/-- **The last month starts where the month before it ends**: its charge
through none of its days is the charge through the end of the part before
it. -/
theorem chargeLast_start (t u W b d : Nat) (hd : 0 < d) :
    chargeLast t u W b d 0 = chargeMid t u W b := by
  have hne : (0 : Nat) ≠ d := by omega
  have e1 : t * (b * d + (W - b) * 0) = d * (t * b) := by
    rw [Nat.mul_zero, Nat.add_zero, ← Nat.mul_assoc, Nat.mul_comm (t * b) d]
  have e2 : W * d = d * W := Nat.mul_comm W d
  simp only [chargeLast, chargeMid, hne, ite_false, e1, e2, roundUnit_scale _ _ _ _ hd]

/-- **The charge never falls through the span's last month**, at any size:
for days `k ≤ k'` of it. -/
theorem chargeLast_mono (t u W b d k k' : Nat) (hb : b < W) (hk : k ≤ k') (hk' : k' ≤ d) :
    chargeLast t u W b d k ≤ chargeLast t u W b d k' := by
  rw [chargeLast_eq_cum t u W b d k hb (by omega), chargeLast_eq_cum t u W b d k' hb hk']
  apply cum_mono
  · exact Nat.add_le_add_left (Nat.mul_le_mul_left _ hk) _
  · have : (W - b) * k' ≤ (W - b) * d := Nat.mul_le_mul_left _ hk'
    have e : b * d + (W - b) * d = W * d := by
      rw [← Nat.add_mul, Nat.add_sub_of_le (Nat.le_of_lt hb)]
    omega

/-- The kernel's split (`prorate_by_days`): each part but the last the
charge at its end (`es`, each short of the span's end, `chargeMid_eq_cum`)
less the charge before it; the last, the total less the charge at the end
of the part before it (its last day is charged the total). -/
def kernelParts (t u W : Nat) (es : List Nat) : List Int :=
  parts t u W 0 es ++ [(t : Int) - cum t u W (es.getLastD 0)]

/-- **The kernel's parts sum to the total**, for any number of parts. -/
theorem kernel_parts_total (t u W : Nat) (hW : 0 < W) (hu : 0 < u) (es : List Nat) :
    (kernelParts t u W es).sum = t := by
  simp only [kernelParts, List.sum_append, parts_telescope, cum_zero t u W hW hu,
    List.sum_cons, List.sum_nil]
  omega

/-- **No part of the kernel's split is negative**, for any number of
parts, when their ends rise within the span. -/
theorem kernel_parts_nonneg (t u W : Nat) (es : List Nat) (h : Rising W 0 es) :
    ∀ x ∈ kernelParts t u W es, 0 ≤ x := by
  intro x hx
  simp only [kernelParts, List.mem_append, List.mem_singleton] at hx
  rcases hx with hx | hx
  · exact prorate_nonneg t u W 0 es h x hx
  · subst hx
    have := cum_le t u W (es.getLastD 0)
    omega

end Wanderbooks.Prorate
