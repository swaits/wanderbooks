import Wanderbooks.Closing

/-!
# The ledger as a state machine

The engine's command surface (`Ledger`), modelled as the steps it allows:
add an account, start a draft, post, reverse, open a period, close one.
Each step carries the checks the engine makes before it mutates. The
theorems below hold in *every* state reachable from empty books, by any
sequence of commands, of any length.

Mirrors `Ledger::add_account`, `try_create_entry`, `post_entry`,
`reverse_entry`, `create_period`, and `close_period_to`; and, as one step
`record`, everything the books keep about how they are kept: settings,
conventions, decisions, open items and approvals (`apply_settings`,
`apply_convention`, `apply_decision`, `apply_open_item`,
`apply_approval`, and their retirements and resolutions), each a record
the step adds and nothing else; and an account's rename
(`rename_account`, `account.renamed`, rules.md B-31), since the model's
chart is codes and types and a name is no part of it.
-/

namespace Wanderbooks

/-- A journal entry: its id, its date (a day number), its lines. -/
structure Entry where
  id : Nat
  date : Nat
  lines : List Line
  deriving DecidableEq

/-- Every line of some entries, in order. -/
def allLines : List Entry → List Line
  | [] => []
  | e :: es => e.lines ++ allLines es

theorem allLines_append (xs ys : List Entry) : allLines (xs ++ ys) = allLines xs ++ allLines ys := by
  induction xs with
  | nil => rfl
  | cons x xs ih => simp [allLines, ih]

theorem mem_allLines {l : Line} {es : List Entry} : l ∈ allLines es ↔ ∃ e ∈ es, l ∈ e.lines := by
  induction es with
  | nil => simp [allLines]
  | cons e es ih => simp [allLines, ih]

/-- **Any collection of balanced entries nets to zero**: all of them, those
dated through some day, those in some period, any filter at all. -/
theorem net_allLines (es : List Entry) (h : ∀ e ∈ es, net e.lines = 0) : net (allLines es) = 0 := by
  induction es with
  | nil => rfl
  | cons e es ih =>
    simp only [allLines, net_append, h e (by simp), ih (fun x hx => h x (by simp [hx]))]
    rfl

/-- An accounting period: inclusive day range, open or closed. -/
structure Period where
  id : Nat
  start : Nat
  stop : Nat
  closed : Bool
  deriving DecidableEq

/-- Whether a day falls in a period. -/
def Period.contains (p : Period) (d : Nat) : Bool := decide (p.start ≤ d ∧ d ≤ p.stop)

/-- The books: chart, posted entries in posting order, drafts, periods,
and the record of how they are kept (settings, conventions, decisions,
open items, approvals), each record abstracted to a number. -/
structure Ledger where
  chart : Chart
  posted : List Entry
  drafts : List Entry
  periods : List Period
  records : List Nat

/-- Empty books. -/
def Ledger.empty : Ledger := ⟨[], [], [], [], []⟩

/-- Whether a closed period contains a day. -/
def Ledger.closedOn (L : Ledger) (d : Nat) : Prop :=
  ∃ p ∈ L.periods, p.closed = true ∧ p.contains d = true

/-- Entries dated on or before a day. -/
def through (d : Nat) (es : List Entry) : List Entry := es.filter (fun e => decide (e.date ≤ d))

/-- Mark one period closed. -/
def closeIn (p : Period) (ps : List Period) : List Period :=
  ps.map (fun q => if q = p then { q with closed := true } else q)

/-- The closing entry for period `p` crediting targets `ts`, if there is
anything to close. -/
def closingFor (L : Ledger) (p : Period) (ts : List (Nat × Int)) (id : Nat) : List Entry :=
  if closings (nominalCodes L.chart) (allLines (through p.stop L.posted)) = [] then []
  else [⟨id, p.stop, closingEntry (nominalCodes L.chart) ts (allLines (through p.stop L.posted))⟩]

/-- What the engine allows. -/
inductive Step : Ledger → Ledger → Prop
  /-- `add_account`: codes are unique. -/
  | addAccount (L : Ledger) (a : Nat) (t : AccountType) (h : a ∉ codes L.chart) :
      Step L { L with chart := (a, t) :: L.chart }
  /-- `try_create_entry` (and lines added to it): not in a closed period. -/
  | draft (L : Ledger) (e : Entry) (h : ¬ L.closedOn e.date) :
      Step L { L with drafts := e :: L.drafts }
  /-- `post_entry`: a draft, acceptable, on known accounts, not in a closed
  period. Appends to the log. -/
  | post (L : Ledger) (e : Entry) (hd : e ∈ L.drafts) (ha : Accepted e.lines)
      (hk : ∀ l ∈ e.lines, l.account ∈ codes L.chart) (hc : ¬ L.closedOn e.date) :
      Step L { L with posted := L.posted ++ [e], drafts := L.drafts.filter (· ≠ e) }
  /-- `reverse_entry`: a posted entry's lines, sides flipped, posted on a
  day not in a closed period. -/
  | reverse (L : Ledger) (e : Entry) (he : e ∈ L.posted) (r : Entry)
      (hr : r.lines = reversal e.lines) (hc : ¬ L.closedOn r.date) :
      Step L { L with posted := L.posted ++ [r] }
  /-- `create_period`: periods open. -/
  | openPeriod (L : Ledger) (p : Period) (h : p.closed = false) :
      Step L { L with periods := p :: L.periods }
  /-- `close_period_to`: an open period with no drafts in it, whose last day
  is in no other closed period, with no later period closed, crediting the
  net income to equity targets (not themselves revenue or expense) in
  parts that sum to it. -/
  | close (L : Ledger) (p : Period) (hp : p ∈ L.periods) (hopen : p.closed = false)
      (hdrafts : ∀ d ∈ L.drafts, p.contains d.date = false)
      (hother : ∀ q ∈ L.periods, q.closed = true → q.contains p.stop = false)
      (hlater : ∀ q ∈ L.periods, q.closed = true → q.stop ≤ p.stop)
      (ts : List (Nat × Int))
      (hts : ∀ x ∈ ts, x.1 ∈ codes L.chart ∧ x.1 ∉ nominalCodes L.chart)
      (hsum : total (ts.map Prod.snd)
        = net (closings (nominalCodes L.chart) (allLines (through p.stop L.posted))))
      (id : Nat) :
      Step L { L with posted := L.posted ++ closingFor L p ts id, periods := closeIn p L.periods }
  /-- `apply_settings`, `apply_convention`, `apply_decision`,
  `apply_open_item`, `apply_approval` and their retirements and
  resolutions, and the 1099 records `set_vendor_1099`,
  `set_vendor_w9_flag`, `add_instrument` and `set_entry_instrument`: a
  record of how the books are kept, and nothing else. `rename_account`
  too: a name is outside the chart the model keeps. -/
  | record (L : Ledger) (r : Nat) : Step L { L with records := r :: L.records }

/-- States reachable from empty books. -/
inductive Reachable : Ledger → Prop
  | empty : Reachable Ledger.empty
  | step {L L' : Ledger} : Reachable L → Step L L' → Reachable L'

/-- What holds in every reachable state. -/
structure Inv (L : Ledger) : Prop where
  distinct : Distinct (codes L.chart)
  accepted : ∀ e ∈ L.posted, Accepted e.lines
  known : ∀ e ∈ L.posted, ∀ l ∈ e.lines, l.account ∈ codes L.chart
  draftsOpen : ∀ d ∈ L.drafts, ¬ L.closedOn d.date

theorem closedOn_closeIn (L : Ledger) (p : Period) (d : Nat) :
    ({ L with periods := closeIn p L.periods } : Ledger).closedOn d →
      L.closedOn d ∨ (p ∈ L.periods ∧ p.contains d = true) := by
  rintro ⟨q, hq, hc, hd⟩
  simp only [closeIn, List.mem_map] at hq
  obtain ⟨q', hq', rfl⟩ := hq
  by_cases e : q' = p
  · subst e; right; simp at hd; exact ⟨hq', by simpa [Period.contains] using hd⟩
  · left; simp [e] at hc hd; exact ⟨q', hq', hc, hd⟩

theorem closingFor_lines (L : Ledger) (p : Period) (ts : List (Nat × Int)) (id : Nat) :
    ∀ e ∈ closingFor L p ts id,
      e.date = p.stop ∧ e.lines = closingEntry (nominalCodes L.chart) ts
        (allLines (through p.stop L.posted))
      ∧ closings (nominalCodes L.chart) (allLines (through p.stop L.posted)) ≠ [] := by
  intro e he
  unfold closingFor at he
  split at he
  · simp at he
  · simp at he; subst he; exact ⟨rfl, rfl, by assumption⟩

/-- **Every step preserves the invariant.** -/
theorem Step.inv {L L' : Ledger} (s : Step L L') (h : Inv L) : Inv L' := by
  cases s with
  | addAccount a t ha =>
    exact ⟨⟨ha, h.distinct⟩, h.accepted,
      fun e he l hl => List.mem_cons_of_mem _ (h.known e he l hl), h.draftsOpen⟩
  | draft e hc =>
    refine ⟨h.distinct, h.accepted, h.known, ?_⟩
    intro d hd
    cases hd with
    | head => exact hc
    | tail _ hd => exact h.draftsOpen d hd
  | post e hd ha hk hc =>
    refine ⟨h.distinct, ?_, ?_, ?_⟩
    · intro x hx
      simp only [List.mem_append, List.mem_singleton] at hx
      rcases hx with hx | rfl
      · exact h.accepted x hx
      · exact ha
    · intro x hx
      simp only [List.mem_append, List.mem_singleton] at hx
      rcases hx with hx | rfl
      · exact h.known x hx
      · exact hk
    · intro d hd
      exact h.draftsOpen d ((List.mem_filter.1 hd).1)
  | reverse e he r hr hc =>
    refine ⟨h.distinct, ?_, ?_, h.draftsOpen⟩
    · intro x hx
      simp only [List.mem_append, List.mem_singleton] at hx
      rcases hx with hx | rfl
      · exact h.accepted x hx
      · rw [hr, accepted_reversal_iff]; exact h.accepted e he
    · intro x hx
      simp only [List.mem_append, List.mem_singleton] at hx
      rcases hx with hx | rfl
      · exact h.known x hx
      · intro l hl
        rw [hr] at hl
        simp only [reversal, List.mem_map] at hl
        obtain ⟨l', hl', rfl⟩ := hl
        exact h.known e he l' hl'
  | openPeriod p hp =>
    refine ⟨h.distinct, h.accepted, h.known, ?_⟩
    rintro d hd ⟨q, hq, hc, hdq⟩
    cases hq with
    | head => simp [hp] at hc
    | tail _ hq => exact h.draftsOpen d hd ⟨q, hq, hc, hdq⟩
  | close p hp hopen hdrafts hother hlater ts hts hsum id =>
    refine ⟨h.distinct, ?_, ?_, ?_⟩
    · intro x hx
      simp only [List.mem_append] at hx
      rcases hx with hx | hx
      · exact h.accepted x hx
      · obtain ⟨-, hl, hne⟩ := closingFor_lines L p ts id x hx
        rw [hl]; exact closingEntry_accepted _ _ _ hsum hne
    · intro x hx
      simp only [List.mem_append] at hx
      rcases hx with hx | hx
      · exact h.known x hx
      · obtain ⟨-, hl, -⟩ := closingFor_lines L p ts id x hx
        intro l hl'
        rw [hl] at hl'
        simp only [closingEntry, List.mem_append] at hl'
        rcases hl' with hl' | hl'
        · exact nominalCodes_sub _ _ (closings_account _ _ l hl')
        · obtain ⟨y, hy, e⟩ := equityLines_account ts l hl'
          rw [e]; exact (hts y hy).1
    · intro d hd hcl
      rcases closedOn_closeIn L p d.date hcl with hcl | ⟨-, hc⟩
      · exact h.draftsOpen d hd hcl
      · simp [hdrafts d hd] at hc
  | record r => exact ⟨h.distinct, h.accepted, h.known, h.draftsOpen⟩

/-- **The invariant holds in every reachable state.** -/
theorem Reachable.inv {L : Ledger} (r : Reachable L) : Inv L := by
  induction r with
  | empty => exact ⟨trivial, by simp [Ledger.empty], by simp [Ledger.empty], by simp [Ledger.empty]⟩
  | step _ s ih => exact s.inv ih

/-! ## The books always balance -/

/-- **The books net to zero**: in every reachable state, the posted lines'
debits equal their credits. -/
theorem Reachable.books_net_zero {L : Ledger} (r : Reachable L) : net (allLines L.posted) = 0 :=
  net_allLines _ (fun e he => (r.inv.accepted e he).net_zero)

/-- The same through any day: the books as of any date balance too. -/
theorem Reachable.net_zero_through {L : Ledger} (r : Reachable L) (d : Nat) :
    net (allLines (through d L.posted)) = 0 :=
  net_allLines _ (fun e he => (r.inv.accepted e ((List.mem_filter.1 he).1)).net_zero)

theorem Reachable.covers {L : Ledger} (r : Reachable L) (es : List Entry)
    (hs : ∀ e ∈ es, e ∈ L.posted) : ∀ l ∈ allLines es, l.account ∈ codes L.chart := by
  intro l hl
  obtain ⟨e, he, hl⟩ := mem_allLines.1 hl
  exact r.inv.known e (hs e he) l hl

/-- **The trial balance balances**, in every reachable state. -/
theorem Reachable.trial_balance {L : Ledger} (r : Reachable L) :
    tbDebits (allLines L.posted) (codes L.chart) = tbCredits (allLines L.posted) (codes L.chart) :=
  trial_balance_balances _ _ r.inv.distinct (r.covers _ (fun _ h => h)) r.books_net_zero

/-- **The trial balance as of any date balances.** -/
theorem Reachable.trial_balance_through {L : Ledger} (r : Reachable L) (d : Nat) :
    tbDebits (allLines (through d L.posted)) (codes L.chart)
      = tbCredits (allLines (through d L.posted)) (codes L.chart) :=
  trial_balance_balances _ _ r.inv.distinct
    (r.covers _ (fun _ h => (List.mem_filter.1 h).1)) (r.net_zero_through d)

/-- **The accounting equation holds**, in every reachable state:
Assets = Liabilities + Equity + (Revenue - Expenses). -/
theorem Reachable.accounting_equation {L : Ledger} (r : Reachable L) :
    let ls := allLines L.posted
    typeTotal .asset ls L.chart
      = typeTotal .liability ls L.chart + typeTotal .equity ls L.chart
        + (typeTotal .revenue ls L.chart - typeTotal .expense ls L.chart) :=
  Wanderbooks.accounting_equation _ _ r.inv.distinct (r.covers _ (fun _ h => h)) r.books_net_zero

/-- **The balance sheet as of any date balances.** -/
theorem Reachable.accounting_equation_through {L : Ledger} (r : Reachable L) (d : Nat) :
    let ls := allLines (through d L.posted)
    typeTotal .asset ls L.chart
      = typeTotal .liability ls L.chart + typeTotal .equity ls L.chart
        + (typeTotal .revenue ls L.chart - typeTotal .expense ls L.chart) :=
  Wanderbooks.accounting_equation _ _ r.inv.distinct
    (r.covers _ (fun _ h => (List.mem_filter.1 h).1)) (r.net_zero_through d)

/-! ## Posted is forever -/

/-- **Posting only appends.** No step edits, reorders, or removes a posted
entry. -/
theorem Step.extends {L L' : Ledger} (s : Step L L') : ∃ t, L'.posted = L.posted ++ t := by
  cases s with
  | addAccount => exact ⟨[], by simp⟩
  | draft => exact ⟨[], by simp⟩
  | post e => exact ⟨[e], rfl⟩
  | reverse e _ r => exact ⟨[r], rfl⟩
  | openPeriod => exact ⟨[], by simp⟩
  | close p _ _ _ _ _ ts _ _ id => exact ⟨closingFor L p ts id, rfl⟩
  | record => exact ⟨[], by simp⟩

/-- Any number of steps. -/
inductive Steps : Ledger → Ledger → Prop
  | refl (L : Ledger) : Steps L L
  | tail {L M N : Ledger} : Steps L M → Step M N → Steps L N

/-- **Posted is forever**: after any sequence of commands, the old log is
a prefix of the new one. Every posted entry keeps its place and its
lines. -/
theorem Steps.extends {L L' : Ledger} (s : Steps L L') : ∃ t, L'.posted = L.posted ++ t := by
  induction s with
  | refl => exact ⟨[], by simp⟩
  | tail _ s ih =>
    obtain ⟨t, ht⟩ := ih
    obtain ⟨u, hu⟩ := s.extends
    exact ⟨t ++ u, by rw [hu, ht, List.append_assoc]⟩

/-! ## Closed periods are frozen -/

/-- Entries dated in a period. -/
def within (p : Period) (es : List Entry) : List Entry := es.filter (fun e => p.contains e.date)

theorem within_append (p : Period) (xs ys : List Entry) :
    within p (xs ++ ys) = within p xs ++ within p ys := List.filter_append ..

/-- A closed period stays closed. -/
theorem Step.keeps_closed {L L' : Ledger} (s : Step L L') (q : Period) (hq : q ∈ L.periods)
    (hc : q.closed = true) : q ∈ L'.periods := by
  cases s with
  | openPeriod p _ => exact List.mem_cons_of_mem _ hq
  | close p _ hopen =>
    have : q ≠ p := fun e => by subst e; simp [hopen] at hc
    exact List.mem_map.2 ⟨q, hq, by simp [this]⟩
  | _ => exact hq

/-- **A step never changes what a closed period contains.** -/
theorem Step.frozen {L L' : Ledger} (s : Step L L') (q : Period) (hq : q ∈ L.periods)
    (hc : q.closed = true) : within q L'.posted = within q L.posted := by
  have nothing : ∀ e : Entry, ¬ L.closedOn e.date → within q [e] = [] := by
    intro e he
    simp only [within, List.filter_cons, List.filter_nil]
    split
    · exact absurd ⟨q, hq, hc, by assumption⟩ he
    · rfl
  cases s with
  | post e _ _ _ hcl => rw [within_append, nothing e hcl, List.append_nil]
  | reverse e _ r _ hcl => rw [within_append, nothing r hcl, List.append_nil]
  | close p hp hopen hdrafts hother hlater ts hts hsum id =>
    rw [within_append]
    have : within q (closingFor L p ts id) = [] := by
      unfold closingFor
      split
      · rfl
      · simp only [within, List.filter_cons, List.filter_nil]
        rw [hother q hq hc]; rfl
    rw [this, List.append_nil]
  | _ => rfl

/-- **Closed periods are frozen**: once a period is closed, no sequence of
commands adds, removes, or changes an entry dated in it. -/
theorem Steps.frozen {L L' : Ledger} (s : Steps L L') (q : Period) (hq : q ∈ L.periods)
    (hc : q.closed = true) : within q L'.posted = within q L.posted ∧ q ∈ L'.periods := by
  induction s with
  | refl => exact ⟨rfl, hq⟩
  | tail _ s ih =>
    obtain ⟨h1, h2⟩ := ih
    exact ⟨(s.frozen q h2 hc).trans h1, s.keeps_closed q h2 hc⟩

/-! ## Closing -/

/-- **After a close, every revenue and expense account is zero through the
period's last day.** (The trial balance still balances: the close is a
step, and `Reachable.trial_balance` covers every step.) -/
theorem close_zeroes {L : Ledger} (r : Reachable L) (p : Period)
    (ts : List (Nat × Int)) (hts : ∀ x ∈ ts, x.1 ∈ codes L.chart ∧ x.1 ∉ nominalCodes L.chart)
    (id : Nat) (a : Nat) (ha : a ∈ nominalCodes L.chart) :
    movement a (allLines (through p.stop (L.posted ++ closingFor L p ts id))) = 0 := by
  have hd := nominalCodes_distinct _ r.inv.distinct
  have hta : ∀ x ∈ ts, x.1 ≠ a := fun x hx e => (hts x hx).2 (e ▸ ha)
  rw [through, List.filter_append, ← through, allLines_append]
  unfold closingFor
  split
  · rename_i hnil
    simp only [List.filter_nil, allLines, List.append_nil]
    exact closings_eq_nil _ _ hnil a ha
  · simp only [List.filter_cons, Nat.le_refl, decide_true, ite_true, List.filter_nil, allLines,
      List.append_nil]
    exact closingEntry_zeroes _ ts _ hd a ha hta

/-- **A close credits equity with exactly the period's net income**:
revenue minus expenses through its last day, split among the targets. -/
theorem close_credits_net_income (L : Ledger) (p : Period) (ts : List (Nat × Int))
    (hsum : total (ts.map Prod.snd)
      = net (closings (nominalCodes L.chart) (allLines (through p.stop L.posted)))) :
    let ls := allLines (through p.stop L.posted)
    total (ts.map Prod.snd) = typeTotal .revenue ls L.chart - typeTotal .expense ls L.chart := by
  intro ls
  rw [hsum, net_closings_is_net_income]

/-! ## Period statements add up -/

/-- Entries dated after day `b`, through day `c`. -/
def between (b c : Nat) (es : List Entry) : List Entry :=
  es.filter (fun e => decide (b < e.date ∧ e.date ≤ c))

theorem movement_split (a b c : Nat) (h : b ≤ c) (es : List Entry) :
    movement a (allLines (through c es))
      = movement a (allLines (through b es)) + movement a (allLines (between b c es)) := by
  induction es with
  | nil => rfl
  | cons e es ih =>
    simp only [through, between] at ih ⊢
    by_cases h1 : e.date ≤ b
    · have h2 : e.date ≤ c := by omega
      have h3 : ¬ (b < e.date ∧ e.date ≤ c) := by omega
      rw [List.filter_cons_of_pos (by simpa using h2), List.filter_cons_of_pos (by simpa using h1),
        List.filter_cons_of_neg (by simpa using h3)]
      simp only [allLines, movement_append, ih]; omega
    · by_cases h2 : e.date ≤ c
      · have h3 : b < e.date ∧ e.date ≤ c := by omega
        rw [List.filter_cons_of_pos (by simpa using h2), List.filter_cons_of_neg (by simpa using h1),
          List.filter_cons_of_pos (by simpa using h3)]
        simp only [allLines, movement_append, ih]; omega
      · have h3 : ¬ (b < e.date ∧ e.date ≤ c) := by omega
        rw [List.filter_cons_of_neg (by simpa using h2), List.filter_cons_of_neg (by simpa using h1),
          List.filter_cons_of_neg (by simpa using h3)]
        exact ih

theorem typeTotal_split (t : AccountType) (x y z : List Line)
    (h : ∀ a, movement a x = movement a y + movement a z) (c : Chart) :
    typeTotal t x c = typeTotal t y c + typeTotal t z c := by
  induction c with
  | nil => rfl
  | cons p c ih =>
    obtain ⟨a, t'⟩ := p
    simp only [typeTotal, h a, ih]
    split
    · cases t' <;> simp [balance, AccountType.normal, signed] <;> omega
    · omega

/-- **Period statements add up**: every account type's total (so revenue,
expenses, and net income) through day `c` is its total through day `b`
plus its total for the days after `b` through `c`. A year's P&L is the sum
of its months'. -/
theorem statements_add_up (t : AccountType) (b c : Nat) (h : b ≤ c) (es : List Entry)
    (chart : Chart) :
    typeTotal t (allLines (through c es)) chart
      = typeTotal t (allLines (through b es)) chart
        + typeTotal t (allLines (between b c es)) chart :=
  typeTotal_split t _ _ _ (fun a => movement_split a b c h es) chart

/-! ## The statement of cash flows foots (rules.md B-14 rule 9) -/

/-- Some entries' cash, summed, is the movement of the cash accounts over
their lines, when the cash tag follows the account. -/
theorem cashTotal_is_movement (F : List Line → Line → Flow) (cash : List Nat)
    (hd : Distinct cash) (hF : ∀ e l, F e l = .cash ↔ l.account ∈ cash) :
    ∀ xs : List Entry,
      cashTotal F (xs.map Entry.lines) = sumOver (fun a => movement a (allLines xs)) cash := by
  intro xs
  induction xs with
  | nil => simpa [cashTotal, allLines] using (sumOver_zero cash).symm
  | cons x xs ih =>
    simp only [List.map_cons, cashTotal, allLines, movement_append]
    rw [sumOver_add, ← ih, cashOf, secNet_cash_is_movement (F x.lines) cash hd (hF x.lines)]

/-- **The statement of cash flows foots**, in every reachable state: over
the posted entries dated after day `b` through day `c` (any selection of
them, `p`: the engine leaves out closing entries), operating + investing +
financing is the movement of the cash accounts over the same entries, for
any tagging whose cash tag follows the account, and whatever a member or
lender handled for each entry (`C`, rule 11) as long as it nets to zero
(`constructive_nets`). Every posted entry is accepted (`Reachable.inv`),
so each nets to zero. -/
theorem Reachable.cashflow_foots {L : Ledger} (r : Reachable L) (b c : Nat) (p : Entry → Bool)
    (F : List Line → Line → Flow) (C : List Line → List Int) (hC : ∀ e, sum3 (C e) = 0)
    (cash : List Nat) (hd : Distinct cash)
    (hF : ∀ e l, F e l = .cash ↔ l.account ∈ cash) :
    let xs := (between b c L.posted).filter p
    sectionTotal F C 0 (xs.map Entry.lines) + sectionTotal F C 1 (xs.map Entry.lines)
        + sectionTotal F C 2 (xs.map Entry.lines)
      = sumOver (fun a => movement a (allLines xs)) cash := by
  intro xs
  rw [cashflow_sections_sum_to_cash_change, cashTotal_is_movement F cash hd hF]
  · intro e he
    obtain ⟨x, hx, rfl⟩ := List.mem_map.1 he
    have hb := (List.mem_filter.1 hx).1
    exact (r.inv.accepted x (List.mem_filter.1 hb).1).net_zero
  · exact fun e _ => hC e

/-- **The sections are the change in the cash accounts' balances**: with
every entry between `b` and `c` counted, the three sections sum to the
cash accounts' movement through `c` less their movement through `b`
(their ending balance less their beginning balance). -/
theorem Reachable.cashflow_is_change_in_cash {L : Ledger} (r : Reachable L) (b c : Nat)
    (hbc : b ≤ c) (F : List Line → Line → Flow) (C : List Line → List Int)
    (hC : ∀ e, sum3 (C e) = 0) (cash : List Nat) (hd : Distinct cash)
    (hF : ∀ e l, F e l = .cash ↔ l.account ∈ cash) :
    let es := (between b c L.posted).map Entry.lines
    sectionTotal F C 0 es + sectionTotal F C 1 es + sectionTotal F C 2 es
      = sumOver (fun a => movement a (allLines (through c L.posted))) cash
        - sumOver (fun a => movement a (allLines (through b L.posted))) cash := by
  intro es
  have h := r.cashflow_foots b c (fun _ => true) F C hC cash hd hF
  have hall : (between b c L.posted).filter (fun _ => true) = between b c L.posted :=
    List.filter_eq_self.2 (fun _ _ => rfl)
  simp only [hall] at h
  rw [h]
  have hs : ∀ a, movement a (allLines (through c L.posted))
      = movement a (allLines (through b L.posted)) + movement a (allLines (between b c L.posted)) :=
    fun a => movement_split a b c hbc L.posted
  rw [sumOver_congr _ _ cash (fun a _ => hs a), sumOver_add]
  omega

/-- **A reversal and its original in one period drop out together**: for
an entry and its reversal (lines flipped, tagged as the original's, and
what was handled for it mirrored, `constructive_neg`), the sum of their
flows in each section is zero. -/
theorem cashflow_reversal_nets (F : List Line → Line → Flow) (C : List Line → List Int)
    (e : List Line) (hF : ∀ l ∈ e, F (reversal e) l.reverse = F e l)
    (hC : C (reversal e) = (C e).map (fun x => -x)) (k : Nat) :
    sectionTotal F C k [e, reversal e] = 0 := by
  simp only [sectionTotal, stmt_flows_reversal F C e hF hC, getD_neg]
  omega

/-! ## How the books are kept changes no figure -/

/-- **A record changes nothing the books hold**: a settings change, a
convention, a decision, an open item or an approval leaves the chart, every
posted entry, every draft and every period exactly as they were. (The
`record` step touches only `records` by definition: this pins the model.
The engine's side is checked in Rust by `settings_replay_is_pure`.
Settings that change how the same entries are presented, such as the
fiscal year, are outside the model.) -/
theorem Step.record_changes_no_figure (L : Ledger) (r : Nat) :
    let L' : Ledger := { L with records := r :: L.records }
    L'.chart = L.chart ∧ L'.posted = L.posted ∧ L'.drafts = L.drafts ∧ L'.periods = L.periods :=
  ⟨rfl, rfl, rfl, rfl⟩

/-- **Any number of records changes no figure**, and they keep the
invariant: a reachable state plus records is reachable, with the same
books. -/
theorem Reachable.records {L : Ledger} (h : Reachable L) (rs : List Nat) :
    Reachable { L with records := rs ++ L.records } := by
  induction rs with
  | nil => simpa using h
  | cons r rs ih => exact Reachable.step ih (Step.record _ r)

end Wanderbooks
