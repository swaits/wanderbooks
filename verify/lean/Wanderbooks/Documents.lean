import Wanderbooks.Entry

/-!
# Documents are the subledgers of their control accounts (rules.md B-38)

Every ledger line may carry the document it belongs to (an invoice or a
bill: `Ledger.line_docs`, `arap::DocTag`). A document's figures are the
sum of its tagged lines; a control account's balance is the sum of every
line on it. Mirrors `Ledger::doc_state` (outstanding), `control_check`,
the control guard (`check_control_lines`: every line a command posts on a
control account carries its document) and `reverse_entry` (a mirror line
copies its original's tag).

The headline, `control_agrees`: for every account and any lines at all,
the account's balance is the sum over the documents of what each carries
on it, plus its untagged ("unattached") lines. Both sides sum the same
lines, so it is an identity, not a reconciliation, and it holds through
any date, since a date is a filter on the lines. A document is one `Nat`
here: an injective coding of `DocRef` (invoices and bills in one id
space), which is all the sums need. Then, over the commands:
a document step (an invoice, a bill, a payment, a write-off, or any entry
whose control lines are tagged) never moves what is unattached; a reversal
moves each document, and what is unattached, by exactly the opposite of
the lines it mirrors (B-38 INVARIANT 3); so books built by these steps
alone have nothing unattached on a control account, ever
(`Reachable.unattached_zero`).

A write-off (S7, B-41) is a document step too: its control line carries the
document and its expense line is off the control accounts. And it never
takes a document below zero: `writeOff_within_bounds` reads the date-sum
before and after it.
-/

namespace Wanderbooks.Documents

open Wanderbooks

/-- A posted line, its day, and the document it carries (`none`: none). -/
structure TLine where
  line : Line
  date : Nat
  doc : Option Nat
  deriving DecidableEq

/-- A recorded send of an invoice (plan P4b S5, `Sent`, `invoice.sent`):
the invoice, the day, and a code for who sent it, how, to whom and the
channel's own reference. No figure reads it: a send moves no money. -/
structure Send where
  doc : Nat
  day : Nat
  code : Nat
  deriving DecidableEq

/-- A line's signed amount on account `a` (zero on another account). -/
def onAcct (a : Nat) (l : TLine) : Int := if l.line.account = a then l.line.signed else 0

/-- Account `a`'s balance over some lines (the general ledger). -/
def gl (a : Nat) : List TLine → Int
  | [] => 0
  | l :: ls => onAcct a l + gl a ls

/-- What document `d`'s lines on account `a` sum to (`doc_lines`). -/
def docSum (a d : Nat) : List TLine → Int
  | [] => 0
  | l :: ls => (if l.doc = some d then onAcct a l else 0) + docSum a d ls

/-- Account `a`'s lines that carry no document. -/
def unattached (a : Nat) : List TLine → Int
  | [] => 0
  | l :: ls => (if l.doc = none then onAcct a l else 0) + unattached a ls

@[simp] theorem gl_nil (a : Nat) : gl a [] = 0 := rfl
@[simp] theorem docSum_nil (a d : Nat) : docSum a d [] = 0 := rfl
@[simp] theorem unattached_nil (a : Nat) : unattached a [] = 0 := rfl

theorem gl_append (a : Nat) (xs ys : List TLine) : gl a (xs ++ ys) = gl a xs + gl a ys := by
  induction xs with
  | nil => simp
  | cons x xs ih => simp only [List.cons_append, gl, ih]; omega

theorem docSum_append (a d : Nat) (xs ys : List TLine) :
    docSum a d (xs ++ ys) = docSum a d xs + docSum a d ys := by
  induction xs with
  | nil => simp
  | cons x xs ih => simp only [List.cons_append, docSum, ih]; omega

theorem unattached_append (a : Nat) (xs ys : List TLine) :
    unattached a (xs ++ ys) = unattached a xs + unattached a ys := by
  induction xs with
  | nil => simp
  | cons x xs ih => simp only [List.cons_append, unattached, ih]; omega

/-- Every document some lines carry is among `ds`. -/
def Covers (ds : List Nat) (ls : List TLine) : Prop :=
  ∀ l ∈ ls, ∀ k, l.doc = some k → k ∈ ds

/-- **The control account agrees with its documents** (B-38 INVARIANT 1):
for any lines, any account, and any distinct document ids covering the
lines' tags, the balance is the documents' sum plus what is unattached. -/
theorem control_agrees (a : Nat) (ds : List Nat) (hd : Distinct ds) :
    ∀ ls : List TLine, Covers ds ls →
      gl a ls = sumOver (fun d => docSum a d ls) ds + unattached a ls := by
  intro ls
  induction ls with
  | nil => intro _; simp [sumOver_zero]
  | cons l ls ih =>
    intro hc
    have hc' : Covers ds ls := fun m hm => hc m (List.mem_cons_of_mem _ hm)
    simp only [gl, docSum, unattached]
    rw [sumOver_add, ih hc']
    cases h : l.doc with
    | none =>
      simp [sumOver_zero] <;> omega
    | some k =>
      have hk : k ∈ ds := hc l (List.mem_cons_self ..) k h
      have e : sumOver (fun d => if some k = some d then onAcct a l else 0) ds
          = sumOver (fun d => if k = d then onAcct a l else 0) ds :=
        sumOver_congr _ _ ds (fun d _ => by simp)
      simp only [e, sumOver_indicator k (onAcct a l) ds hd hk]
      simp <;> omega

/-- The lines dated on or before day `D`. -/
def through (D : Nat) (ls : List TLine) : List TLine := ls.filter (fun l => decide (l.date ≤ D))

theorem through_append (D : Nat) (xs ys : List TLine) :
    through D (xs ++ ys) = through D xs ++ through D ys := by
  simp [through, List.filter_append]

theorem Covers.through {ds : List Nat} {ls : List TLine} (h : Covers ds ls) (D : Nat) :
    Covers ds (through D ls) := fun l hl => h l (List.mem_filter.1 hl).1

/-- **The same through any day**: GL(A, D) = Σ_docs outstanding(d, D) +
unattached(A, D). -/
theorem control_agrees_through (a : Nat) (ds : List Nat) (hd : Distinct ds) (ls : List TLine)
    (hc : Covers ds ls) (D : Nat) :
    gl a (through D ls) = sumOver (fun d => docSum a d (through D ls)) ds
      + unattached a (through D ls) :=
  control_agrees a ds hd _ (hc.through D)

/-! ## The commands -/

/-- A line mirrored by a reversal: its side flipped, its day the
reversal's, its document kept (`reverse_entry_steps` copies the tag). -/
def mirror (D : Nat) (l : TLine) : TLine := { l with line := l.line.reverse, date := D }

theorem onAcct_mirror (a D : Nat) (l : TLine) : onAcct a (mirror D l) = -onAcct a l := by
  unfold onAcct mirror
  by_cases h : l.line.account = a
  · simp [Line.reverse, h]
    have := Line.reverse_signed l.line
    simp [Line.reverse] at this
    exact this
  · simp [Line.reverse, h]

theorem docSum_mirror (a d D : Nat) (ls : List TLine) :
    docSum a d (ls.map (mirror D)) = -docSum a d ls := by
  induction ls with
  | nil => simp
  | cons l ls ih =>
    simp only [List.map_cons, docSum, ih, onAcct_mirror]
    have : (mirror D l).doc = l.doc := rfl
    rw [this]
    by_cases h : l.doc = some d <;> simp [h] <;> omega

theorem unattached_mirror (a D : Nat) (ls : List TLine) :
    unattached a (ls.map (mirror D)) = -unattached a ls := by
  induction ls with
  | nil => simp
  | cons l ls ih =>
    simp only [List.map_cons, unattached, ih, onAcct_mirror]
    have : (mirror D l).doc = l.doc := rfl
    rw [this]
    by_cases h : l.doc = none <;> simp [h] <;> omega

/-- Every line of `ls` on a control account carries a document: what the
control guard requires of every entry a command posts. -/
def Tagged (control : List Nat) (ls : List TLine) : Prop :=
  ∀ l ∈ ls, l.line.account ∈ control → l.doc ≠ none

/-- A tagged list moves nothing that is unattached on a control account:
no line it adds on one carries no document. -/
theorem unattached_zero_of_tagged {control : List Nat} {e : List TLine} (h : Tagged control e)
    {a : Nat} (ha : a ∈ control) : unattached a e = 0 := by
  induction e with
  | nil => rfl
  | cons l ls ih =>
    simp only [unattached]
    rw [ih (fun m hm => h m (List.mem_cons_of_mem _ hm))]
    by_cases hn : l.doc = none
    · have hacc : l.line.account ≠ a := fun e => h l (List.mem_cons_self ..) (e ▸ ha) hn
      simp [hn, onAcct, hacc]
    · simp [hn]

/-- The two lines a write-off of `amount` posts on day `D` (plan P4b S7):
the document's control line credited, tagged with the document (the
receivable falls), and the same amount of bad-debt expense debited,
untagged (the deduction). -/
def writeOffLines (control expense d D : Nat) (amount : Int) : List TLine :=
  [ { line := { account := control, side := .credit, amount := amount }, date := D, doc := some d },
    { line := { account := expense, side := .debit, amount := amount }, date := D, doc := none } ]

/-- The books: the control accounts (the receivables and payables roles),
every posted line in order, and the sends recorded (each on its invoice;
plan P4b S5). -/
structure Books where
  control : List Nat
  lines : List TLine
  sends : List Send

/-- The documents some lines carry: the invoices a send can name. -/
def Books.docs (B : Books) : List Nat :=
  B.lines.filterMap (fun l => l.doc)

/-- What the engine allows (S1): a document command, or any entry whose
control lines are all tagged (`Step.document`); a reversal of posted
lines, mirror for mirror, each keeping its tag (`Step.reverse`), dated on
or after them (B-38, "Dating rules"); writing off a bad debt, for at most
what the document is owed (`Step.writeOff`, S7); assigning a document
role an account all of whose lines are documents' (`Step.assign`:
`check_assign_role` SCENARIO 6 refuses one holding an untagged line); and
an account leaving the control accounts once it holds no document's line
(`Step.release`: an assigned role moved off an account no document posted
to; an account holding a document's line stays guarded,
`control_role_of`). -/
inductive Step : Books → Books → Prop
  | document (B : Books) (e : List TLine) (h : Tagged B.control e) :
      Step B { B with lines := B.lines ++ e }
  | reverse (B : Books) (e : List TLine) (he : ∀ l ∈ e, l ∈ B.lines) (D : Nat) :
      Step B { B with lines := B.lines ++ e.map (mirror D) }
  | writeOff (B : Books) (control expense d D : Nat) (amount : Int)
      (hc : control ∈ B.control) (hne : control ≠ expense)
      (h : Tagged B.control (writeOffLines control expense d D amount))
      (hb : amount ≤ docSum control d (through D B.lines)) :
      Step B { B with lines := B.lines ++ writeOffLines control expense d D amount }
  | assign (B : Books) (c : Nat) (h : ∀ l ∈ B.lines, l.line.account = c → l.doc ≠ none) :
      Step B { B with control := c :: B.control }
  | release (B : Books) (c : Nat) (h : ∀ l ∈ B.lines, l.line.account = c → l.doc = none) :
      Step B { B with control := B.control.erase c }
  /-- `mark_sent` (`Ledger::mark_sent`, plan P4b S5): a send is recorded on
  an invoice the books hold, and nothing else changes. It moves no line,
  so every figure stays where it was. -/
  | markSent (B : Books) (s : Send) (h : s.doc ∈ B.docs) :
      Step B { B with sends := s :: B.sends }

/-- **A document step never moves what is unattached** on a control
account: no command adds an untagged control line. -/
theorem Step.document_unattached (B : Books) (e : List TLine) (h : Tagged B.control e)
    (a : Nat) (ha : a ∈ B.control) : unattached a (B.lines ++ e) = unattached a B.lines := by
  rw [unattached_append, unattached_zero_of_tagged h ha]; omega

/-- **A write-off never moves what is unattached**: its control line carries
the document (a `Step.document` in shape), and its expense line is off the
control accounts, so no control account gains an untagged line (B-41). -/
theorem Step.writeOff_unattached (B : Books) (control expense d D : Nat) (amount : Int)
    (h : Tagged B.control (writeOffLines control expense d D amount)) (a : Nat)
    (ha : a ∈ B.control) :
    unattached a (B.lines ++ writeOffLines control expense d D amount) = unattached a B.lines := by
  rw [unattached_append, unattached_zero_of_tagged h ha]; omega

/-- **A settlement is a document step**: its entry's every control line
carries its document (a payment or a discount), so the books' rules for
any entry whose control lines are tagged cover it; no new `Step`
constructor is needed (plan P4b S6; rules.md B-42). -/
theorem settle_posts_a_document_step (B : Books) (e : List TLine)
    (h : Tagged B.control e) : Step B { B with lines := B.lines ++ e } :=
  Step.document B e h

/-- **A settlement never moves what is unattached** on a control account:
every control line it posts is its document's (B-42, from
`Step.document_unattached`). -/
theorem settle_unattached (B : Books) (e : List TLine) (h : Tagged B.control e) (a : Nat)
    (ha : a ∈ B.control) : unattached a (B.lines ++ e) = unattached a B.lines :=
  Step.document_unattached B e h a ha

/-- **A reversal moves each document by exactly the opposite of its lines
it mirrors** (B-38 INVARIANT 3), and what is unattached likewise: only
reversing a legacy hand line changes it. -/
theorem Step.reverse_moves (B : Books) (e : List TLine) (D a d : Nat) :
    docSum a d (B.lines ++ e.map (mirror D)) = docSum a d B.lines - docSum a d e
      ∧ unattached a (B.lines ++ e.map (mirror D)) = unattached a B.lines - unattached a e := by
  rw [docSum_append, docSum_mirror, unattached_append, unattached_mirror]
  omega

/-- **Reversing a reversal restores every document** (B-10): the lines,
mirrored twice, sum as the originals did. -/
theorem reverse_twice (a d D D' : Nat) (e : List TLine) :
    docSum a d ((e.map (mirror D)).map (mirror D')) = docSum a d e := by
  rw [docSum_mirror, docSum_mirror]; omega

/-- **A send changes no figure** (plan P4b S5): recording one leaves every
control account's balance, every document's sum on it and what is
unattached exactly as they were, through any day. A send adds a record and
no line, so the engine's own statement (`Ledger::mark_sent`) that a
document's DocState is the same before and after is this, line for line. -/
theorem Step.markSent_changes_no_figure (B : Books) (s : Send) (_h : s.doc ∈ B.docs)
    (a d D : Nat) :
    gl a (through D ({ B with sends := s :: B.sends } : Books).lines)
        = gl a (through D B.lines)
    ∧ docSum a d (through D ({ B with sends := s :: B.sends } : Books).lines)
        = docSum a d (through D B.lines)
    ∧ unattached a (through D ({ B with sends := s :: B.sends } : Books).lines)
        = unattached a (through D B.lines) := by
  exact ⟨rfl, rfl, rfl⟩

/-- States reachable from books with no lines. -/
inductive Reachable : Books → Prop
  | empty (control : List Nat) : Reachable ⟨control, [], []⟩
  | step {B B' : Books} : Reachable B → Step B B' → Reachable B'

theorem Reachable.tagged {B : Books} (r : Reachable B) : Tagged B.control B.lines := by
  induction r with
  | empty => intro l hl; simp at hl
  | step _ s ih =>
    cases s with
    | document e h =>
      intro l hl hc
      rcases List.mem_append.1 hl with hl | hl
      · exact ih l hl hc
      · exact h l hl hc
    | reverse e he D =>
      intro l hl hc
      rcases List.mem_append.1 hl with hl | hl
      · exact ih l hl hc
      · obtain ⟨m, hm, rfl⟩ := List.mem_map.1 hl
        exact ih m (he m hm) (by simpa [mirror, Line.reverse] using hc)
    | writeOff control expense d D amount hc hne h hb =>
      intro l hl hc'
      rcases List.mem_append.1 hl with hl | hl
      · exact ih l hl hc'
      · exact h l hl hc'
    | assign c h =>
      intro l hl hc
      rcases List.mem_cons.1 hc with hc | hc
      · exact h l hl hc
      · exact ih l hl hc
    | release c h =>
      intro l hl hc
      exact ih l hl (List.mem_of_mem_erase hc)
    | markSent _ _ =>
      -- A send touches no line: the control lines are the ones it found.
      exact ih

/-- **Nothing is ever unattached on a control account** in books these
steps build, through any day, roles assigned and released included:
every control line is a document's. (Books an earlier version wrote, with
hand lines, are covered by the identity `control_agrees` alone; an
account becoming a control account by its first document, holding lines
from before, is outside the model: `control_check` lists those lines.) -/
theorem Reachable.unattached_zero {B : Books} (r : Reachable B) (a : Nat) (ha : a ∈ B.control)
    (D : Nat) : unattached a (through D B.lines) = 0 := by
  have ht := r.tagged
  have : ∀ ls : List TLine, (∀ l ∈ ls, l ∈ B.lines) → unattached a ls = 0 := by
    intro ls
    induction ls with
    | nil => intro _; rfl
    | cons l ls ih =>
      intro hs
      simp only [unattached]
      rw [ih (fun m hm => hs m (List.mem_cons_of_mem _ hm))]
      by_cases hn : l.doc = none
      · have hacc : l.line.account ≠ a :=
          fun e => ht l (hs l (List.mem_cons_self ..)) (e ▸ ha) hn
        simp [hn, onAcct, hacc]
      · simp [hn]
  exact this _ (fun l hl => (List.mem_filter.1 hl).1)

/-- **So the control account is its documents**, exactly, on every day:
GL(A, D) = Σ_docs outstanding(d, D). -/
theorem Reachable.control_is_documents {B : Books} (r : Reachable B) (a : Nat)
    (ha : a ∈ B.control) (ds : List Nat) (hd : Distinct ds) (hc : Covers ds B.lines) (D : Nat) :
    gl a (through D B.lines) = sumOver (fun d => docSum a d (through D B.lines)) ds := by
  rw [control_agrees_through a ds hd B.lines hc D, r.unattached_zero a ha D]; omega

/-! ## Writing off a bad debt (B-41, plan P4b S7) -/

/-- What a write-off of `amount` does to the document's own sum through
`D`: the control line it posts there is a credit tagged with the document,
and the expense line carries none and is on another account. -/
theorem docSum_writeOffLines (control expense d D : Nat) (amount : Int) :
    docSum control d (through D (writeOffLines control expense d D amount)) = -amount := by
  simp [writeOffLines, through, docSum, onAcct, Line.signed]

/-- **A write-off of at most what is owed leaves nothing owed below zero**
(B-41 INVARIANT 1, over the date-sum `docSum … through D`): the credit it
posts on the control line is the amount, so what the document owes through
the write-off's day is what it owed less the amount, and that is never
below zero. Reversing the write-off adds the amount back
(`reverse_twice`), which is the same-year recovery (D-49). -/
theorem writeOff_within_bounds (B : Books) (control expense d D : Nat) (amount : Int)
    (h : amount ≤ docSum control d (through D B.lines)) :
    0 ≤ docSum control d (through D (B.lines ++ writeOffLines control expense d D amount)) := by
  rw [through_append, docSum_append, docSum_writeOffLines]
  omega

end Wanderbooks.Documents
