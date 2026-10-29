import Bookspec.Entry

/-!
# Balances and statements

Mirrors `kernel::normal_balance`, `kernel::columns`,
`Ledger::account_balance`, and `reports::trial_balance`; and the statement
of cash flows (`kernel::take`, `kernel::apply_cash`, `kernel::entry_flows`,
`kernel::constructive`, `cashflow::cash_flow`'s footing and reconciliation
checks).
-/

namespace Bookspec

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

/-! ## The statement of cash flows (rules.md B-14)

Mirrors `kernel::Flow`, `kernel::take`, `kernel::apply_cash` and
`kernel::entry_flows`, and the identities `cashflow::cash_flow` checks at
runtime: the sections sum to the cash accounts' change, a reversal's flows
are its original's negated, and net income reconciles to operating cash.

Lines are tagged per entry (`F : List Line → Line → Flow`), as the engine
tags them: a revenue or expense line is operating in one entry and
investing in another (rule 4), and a reversal is tagged by its chain's
root. The theorems hold for every such tagging; their hypotheses say what
the engine guarantees (the cash tag follows the account; revenue and
expense lines are operating or investing; a reversal's lines take its
original's tags). A line the engine splits between two sections (the bill
split, rule 12) is two lines here; a split keeps the entry's net. -/

/-- Where a line falls on the statement of cash flows (`kernel::Flow`). -/
inductive Flow where
  | cash
  | operating
  | investing
  | financing
  deriving DecidableEq, Repr

/-- The net movement of the lines `f` tags `k`. -/
def secNet (f : Line → Flow) (k : Flow) : List Line → Int
  | [] => 0
  | l :: ls => (if f l = k then l.signed else 0) + secNet f k ls

@[simp] theorem secNet_nil (f : Line → Flow) (k : Flow) : secNet f k [] = 0 := rfl
@[simp] theorem secNet_cons (f : Line → Flow) (k : Flow) (l : Line) (ls : List Line) :
    secNet f k (l :: ls) = (if f l = k then l.signed else 0) + secNet f k ls := rfl

/-- **Every line is in exactly one place**: an entry's net is its cash plus
its three sections' movements. -/
theorem net_splits_by_flow (f : Line → Flow) (ls : List Line) :
    net ls = secNet f .cash ls + secNet f .operating ls + secNet f .investing ls
      + secNet f .financing ls := by
  induction ls with
  | nil => rfl
  | cons l ls ih =>
    simp only [net_cons, secNet_cons, ih]
    cases f l <;> simp <;> omega

/-- The cash one section takes from what is left (`kernel::take`): only a
section moving against the cash, never more than its own movement. -/
def take (rem n : Int) : Int :=
  if 0 < rem ∧ n < 0 then min rem (-n) else if rem < 0 ∧ 0 < n then max rem (-n) else 0

/-- `kernel::apply_cash`: the cash applied to each movement in order, and
what is left. -/
def applyCash : Int → List Int → List Int × Int
  | rem, [] => ([], rem)
  | rem, n :: ns => (take rem n :: (applyCash (rem - take rem n) ns).1,
      (applyCash (rem - take rem n) ns).2)

/-- A sum of integers. -/
def isum : List Int → Int
  | [] => 0
  | x :: xs => x + isum xs

@[simp] theorem isum_nil : isum [] = 0 := rfl
@[simp] theorem isum_cons (x : Int) (xs : List Int) : isum (x :: xs) = x + isum xs := rfl

theorem isum_append (xs ys : List Int) : isum (xs ++ ys) = isum xs + isum ys := by
  induction xs with
  | nil => simp
  | cons x xs ih => simp only [List.cons_append, isum_cons, ih]; omega

theorem isum_neg (xs : List Int) : isum (xs.map (fun x => -x)) = -isum xs := by
  induction xs with
  | nil => rfl
  | cons x xs ih => simp only [List.map_cons, isum_cons, ih]; omega

/-- Applying cash invents and loses nothing: the parts and what is left
sum to the cash. -/
theorem applyCash_conserves (rem : Int) (ns : List Int) :
    isum (applyCash rem ns).1 + (applyCash rem ns).2 = rem := by
  induction ns generalizing rem with
  | nil => simp [applyCash]
  | cons n ns ih =>
    simp only [applyCash, isum_cons]
    have := ih (rem - take rem n)
    omega

theorem applyCash_length (rem : Int) (ns : List Int) :
    (applyCash rem ns).1.length = ns.length := by
  induction ns generalizing rem with
  | nil => rfl
  | cons n ns ih => simp [applyCash, ih]

theorem take_zero (n : Int) : take 0 n = 0 := by
  simp [take]

/-- With no cash, nothing is applied and nothing is left. -/
theorem applyCash_zero (ns : List Int) : (applyCash 0 ns).2 = 0 := by
  induction ns with
  | nil => rfl
  | cons n ns ih => simp [applyCash, take_zero, ih]

/-- Cash in is used up when the movements against it cover it. -/
theorem applyCash_exhausts_pos (rem : Int) (ns : List Int) (h0 : 0 ≤ rem)
    (h : rem + isum ns ≤ 0) : (applyCash rem ns).2 = 0 := by
  induction ns generalizing rem with
  | nil => simp [applyCash] at h ⊢; omega
  | cons n ns ih =>
    simp only [applyCash]
    simp only [isum_cons] at h
    by_cases hr : rem = 0
    · subst hr; simp [take_zero, applyCash_zero]
    · by_cases hn : n < 0
      · by_cases hle : -n ≤ rem
        · have ht : take rem n = -n := by
            simp only [take]; rw [ite_eq_left (by omega)]; omega
          rw [ht]; exact ih _ (by omega) (by omega)
        · have ht : take rem n = rem := by
            simp only [take]; rw [ite_eq_left (by omega)]; omega
          rw [ht, Int.sub_self]; exact applyCash_zero ns
      · have ht : take rem n = 0 := by
          simp only [take]; rw [ite_eq_right (by omega), ite_eq_right (by omega)]
        rw [ht, Int.sub_zero]; exact ih _ h0 (by omega)

/-- Cash out is used up when the movements against it cover it. -/
theorem applyCash_exhausts_neg (rem : Int) (ns : List Int) (h0 : rem ≤ 0)
    (h : 0 ≤ rem + isum ns) : (applyCash rem ns).2 = 0 := by
  induction ns generalizing rem with
  | nil => simp [applyCash] at h ⊢; omega
  | cons n ns ih =>
    simp only [applyCash]
    simp only [isum_cons] at h
    by_cases hr : rem = 0
    · subst hr; simp [take_zero, applyCash_zero]
    · by_cases hn : 0 < n
      · by_cases hle : rem ≤ -n
        · have ht : take rem n = -n := by
            simp only [take]; rw [ite_eq_right (by omega), ite_eq_left (by omega)]; omega
          rw [ht]; exact ih _ (by omega) (by omega)
        · have ht : take rem n = rem := by
            simp only [take]; rw [ite_eq_right (by omega), ite_eq_left (by omega)]; omega
          rw [ht, Int.sub_self]; exact applyCash_zero ns
      · have ht : take rem n = 0 := by
          simp only [take]; rw [ite_eq_right (by omega), ite_eq_right (by omega)]
        rw [ht, Int.sub_zero]; exact ih _ h0 (by omega)

/-- **Balanced, the cash is used up**: when the cash and the movements sum
to zero, nothing is left over. -/
theorem applyCash_exhausts (rem : Int) (ns : List Int) (h : rem + isum ns = 0) :
    (applyCash rem ns).2 = 0 := by
  rcases Int.le_total 0 rem with h0 | h0
  · exact applyCash_exhausts_pos rem ns h0 (by omega)
  · exact applyCash_exhausts_neg rem ns h0 (by omega)

theorem take_neg (r n : Int) : take (-r) (-n) = -take r n := by
  simp only [take]
  by_cases h1 : 0 < r ∧ n < 0
  · rw [ite_eq_right (by omega), ite_eq_left (by omega), ite_eq_left h1]; omega
  · by_cases h2 : r < 0 ∧ 0 < n
    · rw [ite_eq_left (by omega), ite_eq_right h1, ite_eq_left h2]; omega
    · rw [ite_eq_right (by omega), ite_eq_right (by omega), ite_eq_right h1, ite_eq_right h2]; omega

/-- Applying minus the cash to minus the movements gives minus the parts:
what lets a reversal mirror its original. -/
theorem applyCash_neg (rem : Int) (ns : List Int) :
    applyCash (-rem) (ns.map (fun x => -x))
      = ((applyCash rem ns).1.map (fun x => -x), -(applyCash rem ns).2) := by
  induction ns generalizing rem with
  | nil => rfl
  | cons n ns ih =>
    simp only [List.map_cons, applyCash, take_neg]
    have : -rem - -take rem n = -(rem - take rem n) := by omega
    rw [this, ih]

/-- An entry's cash: the net of the lines tagged cash. -/
def cashOf (F : List Line → Line → Flow) (e : List Line) : Int := secNet (F e) .cash e

/-- An entry's section movements, `[operating, investing, financing]`. -/
def moves (F : List Line → Line → Flow) (e : List Line) : List Int :=
  [secNet (F e) .operating e, secNet (F e) .investing e, secNet (F e) .financing e]

/-- An entry's cash flows per section (`kernel::entry_flows`). -/
def flows (F : List Line → Line → Flow) (e : List Line) : List Int :=
  (applyCash (cashOf F e) (moves F e)).1

/-- **An entry's cash flows sum to its cash** (`entry_flows_sum`): for a
balanced entry, the sections' cash is exactly the cash that moved. -/
theorem entry_flows_sum (F : List Line → Line → Flow) (e : List Line) (h : net e = 0) :
    isum (flows F e) = cashOf F e := by
  have hb := net_splits_by_flow (F e) e
  have hx : (applyCash (cashOf F e) (moves F e)).2 = 0 := by
    apply applyCash_exhausts
    simp only [moves, cashOf, isum_cons, isum_nil]
    omega
  have := applyCash_conserves (cashOf F e) (moves F e)
  simp only [flows]
  omega

theorem flows_length (F : List Line → Line → Flow) (e : List Line) : (flows F e).length = 3 := by
  simp [flows, applyCash_length, moves]

theorem secNet_reversal (f g : Line → Flow) (k : Flow) (ls : List Line)
    (h : ∀ l ∈ ls, g l.reverse = f l) : secNet g k (reversal ls) = -secNet f k ls := by
  induction ls with
  | nil => rfl
  | cons l ls ih =>
    simp only [reversal, List.map_cons] at ih ⊢
    simp only [secNet_cons, h l (by simp), Line.reverse_signed,
      ih (fun m hm => h m (by simp [hm]))]
    split <;> omega

/-- **A reversal's flows are its original's, negated** (`flows_reversal`),
when its lines take its original's tags: what lets an entry and its
reversal drop out of the statement together (rules.md B-14 rule 6). -/
theorem flows_reversal (F : List Line → Line → Flow) (e : List Line)
    (hF : ∀ l ∈ e, F (reversal e) l.reverse = F e l) :
    flows F (reversal e) = (flows F e).map (fun x => -x) := by
  simp only [flows, cashOf, moves]
  rw [secNet_reversal (F e) (F (reversal e)) .cash e hF,
    secNet_reversal (F e) (F (reversal e)) .operating e hF,
    secNet_reversal (F e) (F (reversal e)) .investing e hF,
    secNet_reversal (F e) (F (reversal e)) .financing e hF]
  have := applyCash_neg (secNet (F e) .cash e)
    [secNet (F e) .operating e, secNet (F e) .investing e, secNet (F e) .financing e]
  simp only [List.map_cons, List.map_nil] at this
  rw [this]

theorem isum_three (xs : List Int) (h : xs.length = 3) :
    isum xs = xs.getD 0 0 + xs.getD 1 0 + xs.getD 2 0 := by
  match xs, h with
  | [a, b, c], _ => simp; omega

/-! ### What a member or lender pays or receives for the company (rule 11)

`kernel::constructive` (D-42): what is left unsettled of an entry's
kinds that can pay or receive for the company (`s`, a financing liability
or a member's capital) settles what is left of the kinds they pay or
receive for (`u`), as if through the cash accounts. The engine sums the
parts by section into an entry's constructive triple; the entry theorems
below take any triple that nets to zero (and, for reversals, mirrors),
which `constructive_nets` and `constructive_neg` give the kernel's. -/

/-- The credits among some movements, `Σ max(-m, 0)`: what cash in can
settle. -/
def creditTotal : List Int → Int
  | [] => 0
  | m :: ms => max (-m) 0 + creditTotal ms

/-- The debits among some movements, `Σ max(m, 0)`: what cash out can
settle. -/
def debitTotal : List Int → Int
  | [] => 0
  | m :: ms => max m 0 + debitTotal ms

theorem creditTotal_nonneg (ms : List Int) : 0 ≤ creditTotal ms := by
  induction ms with
  | nil => simp [creditTotal]
  | cons m ms ih => simp only [creditTotal]; omega

theorem debitTotal_nonneg (ms : List Int) : 0 ≤ debitTotal ms := by
  induction ms with
  | nil => simp [debitTotal]
  | cons m ms ih => simp only [debitTotal]; omega

theorem creditTotal_neg (ms : List Int) :
    creditTotal (ms.map (fun x => -x)) = debitTotal ms := by
  induction ms with
  | nil => rfl
  | cons m ms ih => simp only [List.map_cons, creditTotal, debitTotal, ih]; omega

theorem debitTotal_neg (ms : List Int) :
    debitTotal (ms.map (fun x => -x)) = creditTotal ms := by
  induction ms with
  | nil => rfl
  | cons m ms ih =>
    simp only [List.map_cons, creditTotal, debitTotal, ih]

/-- Cash in is used up when the credits cover it (whatever the debits). -/
theorem applyCash_exhausts_in (rem : Int) (ns : List Int) (h0 : 0 ≤ rem)
    (h : rem ≤ creditTotal ns) : (applyCash rem ns).2 = 0 := by
  induction ns generalizing rem with
  | nil => simp [applyCash, creditTotal] at h ⊢; omega
  | cons n ns ih =>
    simp only [applyCash]
    simp only [creditTotal] at h
    by_cases hr : rem = 0
    · subst hr; simp [take_zero, applyCash_zero]
    · by_cases hn : n < 0
      · by_cases hle : -n ≤ rem
        · have ht : take rem n = -n := by
            simp only [take]; rw [ite_eq_left (by omega)]; omega
          rw [ht]; exact ih _ (by omega) (by omega)
        · have ht : take rem n = rem := by
            simp only [take]; rw [ite_eq_left (by omega)]; omega
          rw [ht, Int.sub_self]; exact applyCash_zero ns
      · have ht : take rem n = 0 := by
          simp only [take]; rw [ite_eq_right (by omega), ite_eq_right (by omega)]
        rw [ht, Int.sub_zero]; exact ih _ h0 (by omega)

/-- Cash out is used up when the debits cover it (whatever the credits). -/
theorem applyCash_exhausts_out (rem : Int) (ns : List Int) (h0 : rem ≤ 0)
    (h : -rem ≤ debitTotal ns) : (applyCash rem ns).2 = 0 := by
  induction ns generalizing rem with
  | nil => simp [applyCash, debitTotal] at h ⊢; omega
  | cons n ns ih =>
    simp only [applyCash]
    simp only [debitTotal] at h
    by_cases hr : rem = 0
    · subst hr; simp [take_zero, applyCash_zero]
    · by_cases hn : 0 < n
      · by_cases hle : rem ≤ -n
        · have ht : take rem n = -n := by
            simp only [take]; rw [ite_eq_right (by omega), ite_eq_left (by omega)]; omega
          rw [ht]; exact ih _ (by omega) (by omega)
        · have ht : take rem n = rem := by
            simp only [take]; rw [ite_eq_right (by omega), ite_eq_left (by omega)]; omega
          rw [ht, Int.sub_self]; exact applyCash_zero ns
      · have ht : take rem n = 0 := by
          simp only [take]; rw [ite_eq_right (by omega), ite_eq_right (by omega)]
        rw [ht, Int.sub_zero]; exact ih _ h0 (by omega)

/-- Which way the money went (`kernel::OnBehalf`). -/
inductive OnBehalf where
  | paid
  | received
  deriving DecidableEq, Repr

/-- The other way: what a reversal does. -/
def OnBehalf.mirror : OnBehalf → OnBehalf
  | .paid => .received
  | .received => .paid

/-- The cash the sources take, + in (`kernel::constructive`): paid, the
smaller of the sources' credits and the uses' debits; received, minus the
smaller of the sources' debits and the uses' credits. -/
def onBehalfCash : OnBehalf → List Int → List Int → Int
  | .paid, s, u => min (creditTotal s) (debitTotal u)
  | .received, s, u => -min (debitTotal s) (creditTotal u)

/-- `kernel::constructive`: the sources' parts and the uses' parts. -/
def constructive (w : OnBehalf) (s u : List Int) : List Int × List Int :=
  ((applyCash (onBehalfCash w s u) s).1, (applyCash (-onBehalfCash w s u) u).1)

/-- **What a member or lender handled for the company nets to zero**
(`constructive_nets`; Kani `constructive_nets_to_zero`): the sources take
exactly what the uses give, so an entry's flows still sum to its cash. -/
theorem constructive_nets (w : OnBehalf) (s u : List Int) :
    isum (constructive w s u).1 + isum (constructive w s u).2 = 0 := by
  have hs := applyCash_conserves (onBehalfCash w s u) s
  have hu := applyCash_conserves (-onBehalfCash w s u) u
  have cs := creditTotal_nonneg s
  have ds := debitTotal_nonneg s
  have cu := creditTotal_nonneg u
  have du := debitTotal_nonneg u
  have ⟨xs, xu⟩ : (applyCash (onBehalfCash w s u) s).2 = 0 ∧
      (applyCash (-onBehalfCash w s u) u).2 = 0 := by
    cases w with
    | paid =>
      simp only [onBehalfCash]
      exact ⟨applyCash_exhausts_in _ _ (by omega) (by omega),
        applyCash_exhausts_out _ _ (by omega) (by omega)⟩
    | received =>
      simp only [onBehalfCash]
      exact ⟨applyCash_exhausts_out _ _ (by omega) (by omega),
        applyCash_exhausts_in _ _ (by omega) (by omega)⟩
  simp only [constructive]
  omega

theorem onBehalfCash_mirror (w : OnBehalf) (s u : List Int) :
    onBehalfCash w.mirror (s.map (fun x => -x)) (u.map (fun x => -x))
      = -onBehalfCash w s u := by
  cases w <;> simp only [OnBehalf.mirror, onBehalfCash, creditTotal_neg, debitTotal_neg] <;> omega

/-- **The other way, minus the sources and uses give minus the parts**
(`constructive_neg`; Kani `constructive_mirrors`): a payment made for the
company and its reversal drop out together. -/
theorem constructive_neg (w : OnBehalf) (s u : List Int) :
    constructive w.mirror (s.map (fun x => -x)) (u.map (fun x => -x))
      = ((constructive w s u).1.map (fun x => -x), (constructive w s u).2.map (fun x => -x)) := by
  simp only [constructive, onBehalfCash_mirror]
  rw [applyCash_neg (onBehalfCash w s u) s, applyCash_neg (-onBehalfCash w s u) u]

/-- Three sections' figures, summed. -/
def sum3 (xs : List Int) : Int := xs.getD 0 0 + xs.getD 1 0 + xs.getD 2 0

@[simp] theorem sum3_cons (a b c : Int) : sum3 [a, b, c] = a + b + c := by
  simp [sum3]

/-- **The kernel's parts, by section, net to zero**: split the uses into
operating ones then investing ones (`kernel::constructive` takes them in
that order), and the triple (operating, investing, financing = the
sources) the engine adds to an entry's flows sums to zero. -/
theorem constructive_sections_net (w : OnBehalf) (s uo ui : List Int) :
    sum3 [isum ((constructive w s (uo ++ ui)).2.take uo.length),
      isum ((constructive w s (uo ++ ui)).2.drop uo.length),
      isum (constructive w s (uo ++ ui)).1] = 0 := by
  have h := constructive_nets w s (uo ++ ui)
  have hs := isum_append ((constructive w s (uo ++ ui)).2.take uo.length)
    ((constructive w s (uo ++ ui)).2.drop uo.length)
  rw [List.take_append_drop] at hs
  rw [sum3_cons]
  omega

/-- Passes add: two triples that net to zero add to one that does. -/
theorem sum3_add (xs ys : List Int) (hx : sum3 xs = 0) (hy : sum3 ys = 0) :
    sum3 [xs.getD 0 0 + ys.getD 0 0, xs.getD 1 0 + ys.getD 1 0, xs.getD 2 0 + ys.getD 2 0] = 0 := by
  rw [sum3_cons]
  simp only [sum3] at hx hy
  omega

/-- Two triples, added section by section. -/
def add3 (xs ys : List Int) : List Int :=
  [xs.getD 0 0 + ys.getD 0 0, xs.getD 1 0 + ys.getD 1 0, xs.getD 2 0 + ys.getD 2 0]

/-- An entry's flows on the statement: its cash flows, plus `C e`, what a
member or lender paid or received for it, by section (rule 11). -/
def stmtFlows (F : List Line → Line → Flow) (C : List Line → List Int) (e : List Line) :
    List Int :=
  add3 (flows F e) (C e)

/-- **An entry's statement flows sum to its cash** when what was handled
for it nets to zero: rule 11 moves no cash (`stmt_flows_sum`). -/
theorem stmt_flows_sum (F : List Line → Line → Flow) (C : List Line → List Int)
    (e : List Line) (h : net e = 0) (hC : sum3 (C e) = 0) :
    sum3 (stmtFlows F C e) = cashOf F e := by
  have he := entry_flows_sum F e h
  rw [isum_three _ (flows_length F e)] at he
  simp only [stmtFlows, add3, sum3_cons]
  simp only [sum3] at hC
  omega

theorem getD_neg (xs : List Int) (k : Nat) :
    (xs.map (fun x => -x)).getD k 0 = -xs.getD k 0 := by
  induction xs generalizing k with
  | nil => simp
  | cons x xs ih => cases k with
    | zero => simp
    | succ k => simp only [List.map_cons, List.getD_cons_succ, ih]

/-- **A reversal's statement flows are its original's, negated**, when its
lines take the original's tags and what was handled for it mirrors
(`constructive_neg`). -/
theorem stmt_flows_reversal (F : List Line → Line → Flow) (C : List Line → List Int)
    (e : List Line) (hF : ∀ l ∈ e, F (reversal e) l.reverse = F e l)
    (hC : C (reversal e) = (C e).map (fun x => -x)) :
    stmtFlows F C (reversal e) = (stmtFlows F C e).map (fun x => -x) := by
  simp only [stmtFlows, add3, flows_reversal F e hF, hC, getD_neg, List.map_cons,
    List.map_nil, List.cons.injEq, and_true]
  refine ⟨?_, ?_, ?_⟩ <;> omega

/-- A section's total over some entries: the sum of each entry's cash in
it (`k` = 0 operating, 1 investing, 2 financing), what a member or lender
handled for it included. -/
def sectionTotal (F : List Line → Line → Flow) (C : List Line → List Int) (k : Nat) :
    List (List Line) → Int
  | [] => 0
  | e :: es => (stmtFlows F C e).getD k 0 + sectionTotal F C k es

/-- The entries' cash, summed. -/
def cashTotal (F : List Line → Line → Flow) : List (List Line) → Int
  | [] => 0
  | e :: es => cashOf F e + cashTotal F es

/-- **The sections sum to the change in cash** (plan P3): over any balanced
entries, operating + investing + financing is the entries' cash, exactly,
whatever a member or lender handled for them as long as it nets to zero
within each entry (rule 11, `constructive_nets`). The statement's footing
(rules.md B-14 rule 9) can't fail on posted entries. -/
theorem cashflow_sections_sum_to_cash_change (F : List Line → Line → Flow)
    (C : List Line → List Int) (es : List (List Line)) (h : ∀ e ∈ es, net e = 0)
    (hC : ∀ e ∈ es, sum3 (C e) = 0) :
    sectionTotal F C 0 es + sectionTotal F C 1 es + sectionTotal F C 2 es = cashTotal F es := by
  induction es with
  | nil => rfl
  | cons e es ih =>
    simp only [sectionTotal, cashTotal]
    have he := stmt_flows_sum F C e (h e (by simp)) (hC e (by simp))
    simp only [sum3] at he
    have := ih (fun x hx => h x (by simp [hx])) (fun x hx => hC x (by simp [hx]))
    omega

/-- When the cash tag follows the account (a line is cash exactly when
its account is a cash account), an entry's cash is the movement of the
cash accounts. -/
theorem secNet_cash_is_movement (f : Line → Flow) (cash : List Nat) (hd : Distinct cash)
    (hf : ∀ l, f l = .cash ↔ l.account ∈ cash) (ls : List Line) :
    secNet f .cash ls = sumOver (fun a => movement a ls) cash := by
  induction ls with
  | nil => simpa using (sumOver_zero cash).symm
  | cons l ls ih =>
    simp only [secNet_cons, movement_cons]
    rw [sumOver_add, ← ih]
    by_cases hc : l.account ∈ cash
    · rw [ite_eq_left ((hf l).2 hc), sumOver_indicator l.account l.signed cash hd hc]
    · rw [ite_eq_right (fun e => hc ((hf l).1 e)), sumOver_indicator_absent l.account l.signed cash hc]

/-! ### Net income reconciled to operating cash (rules.md B-14 rule 8) -/

/-- Net movement of the lines on revenue and expense accounts (`pl`)
tagged `k` (`sel = true`), or on balance-sheet accounts (`sel = false`). -/
def plNet (pl : Nat → Bool) (sel : Bool) (f : Line → Flow) (k : Flow) : List Line → Int
  | [] => 0
  | l :: ls => (if pl l.account = sel ∧ f l = k then l.signed else 0) + plNet pl sel f k ls

/-- Net movement of every revenue and expense line. -/
def plAll (pl : Nat → Bool) : List Line → Int
  | [] => 0
  | l :: ls => (if pl l.account = true then l.signed else 0) + plAll pl ls

/-- Net income: revenue less expenses, `-(net of the P&L lines)`. -/
def netIncome (pl : Nat → Bool) (e : List Line) : Int := -plAll pl e

/-- G: the P&L lines rule 4 puts in investing (gains and losses). -/
def gainLoss (pl : Nat → Bool) (F : List Line → Line → Flow) (e : List Line) : Int :=
  plNet pl true (F e) .investing e

/-- W: minus the movement of the operating balance-sheet lines. -/
def balanceChange (pl : Nat → Bool) (F : List Line → Line → Flow) (e : List Line) : Int :=
  -plNet pl false (F e) .operating e

/-- X: what no cash, real or handled for the company, settled of the
operating movement. -/
def noncashOp (F : List Line → Line → Flow) (C : List Line → List Int) (e : List Line) : Int :=
  secNet (F e) .operating e + (stmtFlows F C e).getD 0 0

theorem secNet_split (pl : Nat → Bool) (f : Line → Flow) (k : Flow) (ls : List Line) :
    secNet f k ls = plNet pl true f k ls + plNet pl false f k ls := by
  induction ls with
  | nil => rfl
  | cons l ls ih =>
    simp only [secNet_cons, plNet, ih]
    cases pl l.account <;> by_cases h : f l = k <;> simp [h] <;> omega

theorem plAll_split (pl : Nat → Bool) (f : Line → Flow) (ls : List Line)
    (h : ∀ l ∈ ls, pl l.account = true → f l = .operating ∨ f l = .investing) :
    plAll pl ls = plNet pl true f .operating ls + plNet pl true f .investing ls := by
  induction ls with
  | nil => rfl
  | cons l ls ih =>
    simp only [plAll, plNet, ih (fun m hm => h m (by simp [hm]))]
    cases hp : pl l.account
    · simp
    · rcases h l (by simp) hp with hk | hk <;> simp [hk] <;> omega

/-- **Net income reconciles to operating cash**, entry by entry:
operating cash = NI + G + X + W, when revenue and expense lines are only
ever operating or investing (rules.md B-14 rule 8; ASC 230-10-45-28 to
45-30), whatever a member or lender handled for it (rule 11). -/
theorem operating_reconciliation_ties (pl : Nat → Bool) (F : List Line → Line → Flow)
    (C : List Line → List Int) (e : List Line)
    (h : ∀ l ∈ e, pl l.account = true → F e l = .operating ∨ F e l = .investing) :
    (stmtFlows F C e).getD 0 0
      = netIncome pl e + gainLoss pl F e + noncashOp F C e + balanceChange pl F e := by
  have hs := secNet_split pl (F e) .operating e
  have hp := plAll_split pl (F e) e h
  simp only [netIncome, gainLoss, noncashOp, balanceChange]
  omega

/-- The operating section's total over some entries. -/
def opTotal (pl : Nat → Bool) (F : List Line → Line → Flow) (C : List Line → List Int) :
    List (List Line) → Int
  | [] => 0
  | e :: es => netIncome pl e + gainLoss pl F e + noncashOp F C e + balanceChange pl F e
      + opTotal pl F C es

/-- **The reconciliation schedule ties to the operating section** over any
entries: the section's total is NI + G + X + W summed. -/
theorem operating_reconciliation_ties_all (pl : Nat → Bool) (F : List Line → Line → Flow)
    (C : List Line → List Int) (es : List (List Line))
    (h : ∀ e ∈ es, ∀ l ∈ e, pl l.account = true → F e l = .operating ∨ F e l = .investing) :
    sectionTotal F C 0 es = opTotal pl F C es := by
  induction es with
  | nil => rfl
  | cons e es ih =>
    simp only [sectionTotal, opTotal]
    rw [operating_reconciliation_ties pl F C e (h e (by simp)),
      ih (fun x hx => h x (by simp [hx]))]

end Bookspec
