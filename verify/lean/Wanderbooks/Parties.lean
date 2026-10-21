import Wanderbooks.Entry

/-!
# A party's part of an entry (rules.md B-35)

Mirrors `kernel::party_target` and `kernel::reverse_target`, and proves
what makes a reversal the exact opposite of what it reverses: the target
is odd (but for the debits fallback, where a sign means nothing, and a
member's entry whose cash isn't its money: an expense booked straight to
capital, a debt converted, a distribution still owed), reversing a target
twice gives it back, and a reversal's movement over any set of accounts
(a member's equity, say) is the original's negated.
-/

namespace Wanderbooks

/-- Which way an entry moved value with its parties (`kernel::Direction`). -/
inductive Direction where
  | paid
  | received
  | neither
  deriving DecidableEq, Repr

/-- Paid and received swap; neither stays neither. -/
def Direction.flip : Direction → Direction
  | .paid => .received
  | .received => .paid
  | .neither => .neither

@[simp] theorem Direction.flip_flip (d : Direction) : d.flip.flip = d := by
  cases d <;> rfl

/-- The direction of a signed amount (`Direction::of`). -/
def Direction.of (a : Int) : Direction :=
  if 0 < a then .paid else if a < 0 then .received else .neither

theorem Direction.of_neg (a : Int) : Direction.of (-a) = (Direction.of a).flip := by
  unfold Direction.of
  by_cases h1 : 0 < a
  · rw [ite_eq_right (show ¬ (0 < -a) by omega), ite_eq_left (show -a < 0 by omega), ite_eq_left h1]; rfl
  · by_cases h2 : a < 0
    · rw [ite_eq_left (show 0 < -a by omega), ite_eq_right h1, ite_eq_left h2]; rfl
    · rw [ite_eq_right (show ¬ (0 < -a) by omega), ite_eq_right (show ¬ (-a < 0) by omega), ite_eq_right h1,
        ite_eq_right h2]; rfl

/-- `kernel::PartyTarget`. -/
structure PartyTarget where
  amount : Int
  direction : Direction
  cash : Int
  inKind : Int
  deriving DecidableEq, Repr

/-- `kernel::reverse_target`: every amount negated, paid and received
swapped. -/
def reverseTarget (t : PartyTarget) : PartyTarget :=
  ⟨-t.amount, t.direction.flip, -t.cash, -t.inKind⟩

/-- **Reversing a reversal gives the original back** (B-9). -/
theorem reverseTarget_involutive (t : PartyTarget) :
    reverseTarget (reverseTarget t) = t := by
  cases t; simp [reverseTarget]

/-- A magnitude. -/
def magnitude (e : Int) : Int := if e < 0 then -e else e

theorem magnitude_neg (e : Int) : magnitude (-e) = magnitude e := by
  unfold magnitude
  by_cases h : e < 0
  · rw [ite_eq_right (show ¬ (-e < 0) by omega), ite_eq_left h]
  · by_cases h' : -e < 0
    · rw [ite_eq_left h', ite_eq_right h]; omega
    · rw [ite_eq_right h', ite_eq_right h]; omega

/-- The magnitude `m` with the sign of `like`. -/
def withSign (like m : Int) : Int := if like < 0 then -m else m

theorem withSign_neg (e m : Int) (he : e ≠ 0) : withSign (-e) m = -withSign e m := by
  unfold withSign
  by_cases h : e < 0
  · rw [ite_eq_right (show ¬ (-e < 0) by omega), ite_eq_left h]; omega
  · rw [ite_eq_left (show -e < 0 by omega), ite_eq_right h]

/-- What of a member's capital movement `e` counts, as (cash, in kind)
(`kernel::party_target` INVARIANTS 1-2): of a contribution (`e < 0`), the
property in it, at most `|e|`, in kind and the rest cash; of a
distribution, the money it paid (`net`, at most `e`) as cash, then the
property it gave, at most what is left, in kind. The rest of a
distribution is still owed and counts as nothing. -/
def capitalParts (e property net : Int) : Int × Int :=
  if e < 0 then
    (magnitude e - min (max property 0) (magnitude e), min (max property 0) (magnitude e))
  else
    (min (max net 0) (magnitude e),
      min (max property 0) (magnitude e - min (max net 0) (magnitude e)))

/-- The capital case of `kernel::party_target` (a member's entry moving
equity `e ≠ 0`), signed as `e`; neither when nothing counts. -/
def capitalTarget (e property net : Int) : PartyTarget :=
  ⟨e, if (capitalParts e property net).1 + (capitalParts e property net).2 = 0 then .neither
    else Direction.of e,
    withSign e (capitalParts e property net).1, withSign e (capitalParts e property net).2⟩

/-- The rest of `kernel::party_target`: money moved, else the parties'
own-account movement, else the debits; neither moves nothing. -/
def moneyTarget (net own debits : Int) : PartyTarget :=
  if net ≠ 0 then ⟨net, Direction.of net, net, 0⟩
  else ⟨if own ≠ 0 then own else debits, .neither, 0, 0⟩

/-- `kernel::party_target`. -/
def partyTarget (capital : Option (Int × Int)) (net own debits : Int) : PartyTarget :=
  match capital with
  | some (e, p) => if e ≠ 0 then capitalTarget e p net else moneyTarget net own debits
  | none => moneyTarget net own debits

/-- A member's entry whose cash is exactly its money: the money moved the
member's way (paid out on a distribution, received on a contribution) is
what isn't property. Not so for an expense booked straight to capital, a
debt converted, or a distribution still owed: the reversal arm, never the
lines, gives their reversals a target. -/
def cashIsMoney (e property net : Int) : Prop :=
  min (max (if 0 < e then net else -net) 0) (magnitude e) =
    magnitude e - min (max property 0) (magnitude e)

/-- Not vacuous: a cash contribution, a property contribution, and a
distribution of money and property each have their cash as their money. -/
example : cashIsMoney (-100) 0 (-100) ∧ cashIsMoney (-100) 100 0 ∧ cashIsMoney 100 40 60 := by
  unfold cashIsMoney magnitude
  decide

/-- Negating a member's entry whose cash is its money keeps what counts. -/
theorem capitalParts_neg (e p net : Int) (he : e ≠ 0) (h : cashIsMoney e p net) :
    capitalParts (-e) p (-net) = capitalParts e p net := by
  unfold cashIsMoney at h
  unfold capitalParts
  rw [magnitude_neg]
  by_cases hn : e < 0
  · rw [ite_eq_right (show ¬ (0 < e) by omega)] at h
    rw [ite_eq_right (show ¬ (-e < 0) by omega), ite_eq_left hn, h]
    congr 1
    omega
  · rw [ite_eq_left (show 0 < e by omega)] at h
    rw [ite_eq_left (show -e < 0 by omega), ite_eq_right hn, h]
    congr 1
    omega

/-- **A member's capital target is odd** whenever its cash is its money. -/
theorem capitalTarget_odd (e p net : Int) (he : e ≠ 0) (h : cashIsMoney e p net) :
    capitalTarget (-e) p (-net) = reverseTarget (capitalTarget e p net) := by
  unfold capitalTarget reverseTarget
  rw [capitalParts_neg e p net he h, withSign_neg _ _ he, withSign_neg _ _ he]
  congr 1
  by_cases hc : (capitalParts e p net).1 + (capitalParts e p net).2 = 0
  · simp [hc, Direction.flip]
  · simp [hc, Direction.of_neg]

/-- With money moved, or the parties' own accounts moved, the rest of the
target is odd too. -/
theorem moneyTarget_odd (net own debits : Int) (h : net ≠ 0 ∨ own ≠ 0) :
    moneyTarget (-net) (-own) debits = reverseTarget (moneyTarget net own debits) := by
  unfold moneyTarget reverseTarget
  by_cases hn : net = 0
  · have ho : own ≠ 0 := by omega
    have : ¬ (-own = 0) := by omega
    simp [hn, ho, this, Direction.flip]
  · have : ¬ (-net = 0) := by omega
    simp [hn, this, Direction.of_neg]

/-- **The target is odd** (rules.md B-35 item 3, B-9): negating an entry's
lines negates its capital movement, its money net and its own-account
movement, while the property beside a capital movement comes from the
other side of the lines and keeps its size; the result is the original's
`reverseTarget`, but for the debits fallback (nothing moved: a sign means
nothing there) and a member's entry whose cash isn't its money. The
engine's reversal arm, not the lines, gives every reversal its target. -/
theorem partyTarget_odd (capital : Option (Int × Int)) (net own debits : Int)
    (h : (∃ e p, capital = some (e, p) ∧ e ≠ 0 ∧ cashIsMoney e p net) ∨
      ((∀ e p, capital = some (e, p) → e = 0) ∧ (net ≠ 0 ∨ own ≠ 0))) :
    partyTarget (capital.map fun c => (-c.1, c.2)) (-net) (-own) debits =
      reverseTarget (partyTarget capital net own debits) := by
  cases capital with
  | none =>
    simp only [Option.map, partyTarget]
    apply moneyTarget_odd
    rcases h with ⟨_, _, hc, _⟩ | ⟨_, h⟩
    · cases hc
    · exact h
  | some c =>
    obtain ⟨e, p⟩ := c
    simp only [Option.map, partyTarget]
    by_cases he : e = 0
    · have : ¬ (-e ≠ 0) := by omega
      simp only [he, ne_eq, not_true_eq_false, ite_false, Int.neg_zero]
      apply moneyTarget_odd
      rcases h with ⟨e', p', hc, he', _⟩ | ⟨_, h⟩
      · simp at hc; omega
      · exact h
    · have : -e ≠ 0 := by omega
      simp only [ne_eq, he, this, not_false_eq_true, ite_true]
      rcases h with ⟨e', p', hc, _, hm⟩ | ⟨hz, _⟩
      · simp only [Option.some.injEq, Prod.mk.injEq] at hc
        obtain ⟨rfl, rfl⟩ := hc
        exact capitalTarget_odd e p net he hm
      · exact absurd (hz e p rfl) he

/-- **A reversal's movement over any accounts is the original's negated**
(so the equity movement a member's target reads flips with it): summed
over a set of accounts, `movement_reversal` account by account. -/
theorem sumOver_movement_reversal (as : List Nat) (ls : List Line) :
    sumOver (fun a => movement a (reversal ls)) as = -sumOver (fun a => movement a ls) as := by
  have : (fun a => movement a (reversal ls)) = (fun a => -movement a ls) := by
    funext a; exact movement_reversal a ls
  rw [this, sumOver_neg]

end Wanderbooks
