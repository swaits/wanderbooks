# Commands and steps: what the Lean model covers

bookspec's Lean proves its laws about the books over time for every state
`Reachable` from empty books by a `Step`. The engine changes the books
through 119 public commands on `Ledger`. This page maps each command to
the step or steps it is, so "every reachable state" can be read for what
it covers, and names what it does not.

The map is kept whole by a test
(`crates/ledger-core/tests/command_steps.rs`): a public command on
`Ledger` that takes `&mut self` and has no row here fails the build, and
so does a row for a command that is gone. A new command gets a row, and
if it is not one of the model's steps, a new `Step` in bookspec with its
preservation proof (AGENTS.md, formal methods, item 3).

## The model's steps

Two models run side by side (bookspec `lean/Bookspec/Exec/Doc.lean`,
"Composition"): the ledger (the chart, the posted log, drafts, periods)
and the documents (tagged lines, control accounts, sends).

| Step | Model | What it is |
|---|---|---|
| `addAccount` | ledger | a new account, its code not taken |
| `draft` | ledger | a draft entry (and a line added to one), not in a closed period |
| `post` | ledger | a draft posted: balanced, on known accounts, not in a closed period |
| `reverse` | ledger | a posted entry's mirror, at most once, never a close, not dated before the original |
| `openPeriod` | ledger | a period that nests with or lies apart from every other |
| `close`, `closeToRetained` | ledger | an open period closed, in order, with no draft in it |
| `discard` | ledger | a draft removed |
| `record` | ledger | a record of how the books are kept: no account, entry or period changes |
| `document` | documents | lines posted with their documents (an invoice, a bill, a settlement) |
| `doc.reverse` | documents | the mirror of lines that carry documents |
| `writeOff` | documents | a write-off, above zero and at most what is owed |
| `assign`, `release` | documents | an account takes or gives up a control role |
| `markSent` | documents | a send recorded on an invoice |

## What the map is, and is not

- **A reading, checked two ways, proved in neither.** No proof ties a
  Rust command to a Lean step. Two machine checks do: bookspec's examples
  (the engine answers each recorded command as the record says), and the
  differential test `crates/ledger-core/tests/bookspec/lean_model.rs`,
  which runs random command sequences through the engine and the Lean
  model and compares every figure and refusal. That test reaches an entry
  drafted and posted, a reversal, a close to retained earnings, an
  invoice and a bill. It does not reach a standalone `openPeriod`, a close
  with explicit targets, a draft left unposted (so `discard`), or an
  account added after the first event; those are by review.
- **`record` covers 71 commands.** They change nothing the ledger model
  keeps, so every law about balances, statements and periods holds across
  them with nothing to prove. Some keep a register with rules of its own
  (the note says which); see "Registers" below.
- **Several steps in one command** run inside `Ledger::atomically`: all
  of them happen or none does.

## Not covered

### No step: `renumber_account`

The model's chart is a list of codes, and its lines name accounts by
code. The engine's lines name accounts by id, and a renumber changes one
code and no entry. In the model that is a renaming of one code across the
chart and the log, which no step does. The engine refuses a taken code
(`check_renumber_account`), so codes stay distinct, and its tests check
that no balance moves; neither is a theorem. Closes with a `renumber`
step in bookspec: codes stay distinct, and every balance is the old one
under the new code.

### Doors: 17 commands that take what an older version wrote

A journal line an older version accepted must replay (BS-entry-4), so
each rule added later has a door beside the command: the same change
without the newer check. A store calls a door only for a line its journal
already holds. Most drop checks the model does not carry (a size bound, a
minor unit, a text rule). Four drop one the model's step requires:

| Door | The step's condition it does not check |
|---|---|
| `create_entry` | `draft`: the date is in no closed period |
| `replay_generate_recurring` | `draft`: the same |
| `replay_create_period` | `openPeriod`: no partial overlap |
| `replay_reverse_entry` | `reverse`: the reversal is not dated before the original |

So books replayed from a journal written before those rules can stand in
a state the model does not reach, and a caller of the engine that uses a
door for new work can put them there. What still holds, read from the
proofs: the preservation of `Inv.distinct`, `Inv.accepted` and
`Inv.known` (distinct codes, every posted entry balanced and on known
accounts) uses none of the four conditions, so the trial balance and the
accounting equation hold in those books too. `Inv.draftsOpen` (no draft
in a closed period) does not. That reading is not yet a theorem. Closes
with: a `ReplayStep` relation in bookspec (the steps without the four
conditions) and the proof that the three invariants hold in every state
it reaches.

`create_entry` is a door by its doc comment only: its name does not say
so, and it is public. The store calls `try_create_entry` first.

### Registers: state with rules of its own

These are `record` to the ledger model. Their arithmetic is proved as
functions (bookspec `Schedule`, `Prorate`, `Deductible`, `Payroll`,
`Apportion`); how they change over time is tested, not modelled:

- **Fixed assets**: registered, corrected, linked to an acquisition,
  depreciated by runs, disposed once.
- **Schedules**: a month posts at most once while its posting stands, and
  is due again when that posting is reversed.
- **Recurring entries**: an occurrence is generated at most once.
- **Payroll runs**: a run is recorded with the entry that posted it.

### The pair is one set of books by construction

bookspec says so itself (`Exec/Doc.lean`): that the documents' lines are
exactly the ledger's posted lines is kept by how the runner is built, not
proved. The engine's side of the same fact is `Ledger::atomically` and
the control check (`Ledger::control_check`), which are tested.

## Every command

In source order. "door" in a note marks a command that exists for replay.

| Command | Step | Note |
|---|---|---|
| `register_schedule` | record | register: schedules |
| `register_schedule_under` | record | register: schedules |
| `replay_register_schedule` | record | register: schedules |
| `replay_register_schedule_under` | record | register: schedules |
| `post_schedule_month` | draft, post, record | register: schedules (the month is marked posted) |
| `register_recurring` | record | register: recurring entries |
| `replay_register_recurring` | record | register: recurring entries |
| `generate_recurring` | draft, record | register: recurring entries (the occurrence is marked generated) |
| `replay_generate_recurring` | draft, record | door: no closed-period check; register: recurring entries |
| `add_account` | addAccount |  |
| `add_control_account` | addAccount |  |
| `replay_add_account` | addAccount | door: without the child-type and blank-text rules; a taken code is still refused |
| `renumber_account` | none | changes a code the chart holds; see "No step" |
| `rename_account` | record |  |
| `replay_rename_account` | record |  |
| `assign_role` | assign, release | `None` is the release |
| `set_account_tax` | record |  |
| `create_entry` | draft | door: no closed-period check (replay, and the engine's own entries, which post at once) |
| `try_create_entry` | draft |  |
| `add_line` | draft |  |
| `replay_add_line` | draft | door: without the account check and the size bound; the post still refuses |
| `add_line_fx` | draft |  |
| `replay_add_line_fx` | draft | door: without the account check and the size bound; the post still refuses |
| `post_entry` | post |  |
| `replay_post_entry` | post | door: without the minor-unit check |
| `set_draft_state` | record | a note on a draft: no line moves |
| `replay_set_draft_state` | record | a note on a draft: no line moves |
| `discard_draft` | discard |  |
| `replay_discard_draft` | discard |  |
| `note_entry_origin` | record | what posted an entry, and its journal line |
| `expect_posted_seq` | record | what posted an entry, and its journal line |
| `stamp_unstamped` | record | what posted an entry, and its journal line |
| `reverse_entry` | reverse, doc.reverse | the document step when a line carries a document |
| `replay_reverse_entry` | reverse, doc.reverse | door: without the date order and the document checks |
| `register_asset` | record | register: fixed assets |
| `replay_register_asset` | record | register: fixed assets |
| `set_asset_tag` | record | register: fixed assets |
| `set_asset_serial` | record | register: fixed assets |
| `set_asset_acquisition` | record | register: fixed assets |
| `replay_set_asset_acquisition` | record | register: fixed assets |
| `restore_asset_acquisition` | record | register: fixed assets |
| `set_asset_description` | record | register: fixed assets |
| `set_asset_basis` | record | register: fixed assets |
| `replay_set_asset_basis` | record | register: fixed assets |
| `replay_set_asset_tax_basis` | record | register: fixed assets |
| `correct_asset` | record | register: fixed assets |
| `replay_correct_asset` | record | register: fixed assets |
| `record_depreciation_run` | record | register: fixed assets |
| `dispose_asset` | record | register: fixed assets |
| `replay_dispose_asset` | record | register: fixed assets |
| `create_period` | openPeriod |  |
| `replay_create_period` | openPeriod | door: without the nesting rule and the calendar bound |
| `close_period` | addAccount, closeToRetained | the account only when retained earnings has none yet |
| `close_period_to` | addAccount, close | the account only when a target role has none yet |
| `add_customer` | record |  |
| `add_vendor` | record |  |
| `set_vendor_1099` | record |  |
| `replay_vendor_1099` | record |  |
| `set_vendor_w9_flag` | record |  |
| `add_instrument` | record |  |
| `set_entry_instrument` | record |  |
| `set_entry_vendor` | record |  |
| `add_party` | record |  |
| `update_party` | record |  |
| `set_party_role` | record |  |
| `set_party_w9` | record |  |
| `set_party_tin` | record |  |
| `set_party_creditor` | record |  |
| `set_party_w4` | record |  |
| `set_entry_party` | record |  |
| `set_entry_parties` | record |  |
| `replay_set_entry_parties` | record |  |
| `set_entry_no_party` | record |  |
| `create_invoice` | addAccount, draft, post, document | the account only when the control role has none yet |
| `create_invoice_with` | addAccount, draft, post, document | the account only when the control role has none yet |
| `create_bill` | addAccount, draft, post, document | the account only when the control role has none yet |
| `create_bill_with` | addAccount, draft, post, document | the account only when the control role has none yet |
| `mark_sent` | markSent |  |
| `replay_invoice_sent` | markSent |  |
| `settle` | addAccount, draft, post, document | a settlement is a document step (bookspec `settle_posts_a_document_step`) |
| `receive_payment` | addAccount, draft, post, document | a settlement is a document step (bookspec `settle_posts_a_document_step`) |
| `receive_payment_with` | addAccount, draft, post, document | a settlement is a document step (bookspec `settle_posts_a_document_step`) |
| `pay_bill` | addAccount, draft, post, document | a settlement is a document step (bookspec `settle_posts_a_document_step`) |
| `pay_bill_with` | addAccount, draft, post, document | a settlement is a document step (bookspec `settle_posts_a_document_step`) |
| `write_off` | addAccount, draft, post, writeOff |  |
| `void_document` | reverse, doc.reverse |  |
| `replay_invoice` | document | door: tags the lines of an entry its own events already replayed; nothing is bounded |
| `replay_invoice_with` | document | door: tags the lines of an entry its own events already replayed; nothing is bounded |
| `replay_bill` | document | door: tags the lines of an entry its own events already replayed; nothing is bounded |
| `replay_bill_with` | document | door: tags the lines of an entry its own events already replayed; nothing is bounded |
| `replay_invoice_payment` | document | door: tags the lines of an entry its own events already replayed; nothing is bounded |
| `replay_invoice_payment_at` | document | door: tags the lines of an entry its own events already replayed; nothing is bounded |
| `replay_invoice_write_off` | writeOff | door: as above |
| `replay_bill_payment` | document | door: tags the lines of an entry its own events already replayed; nothing is bounded |
| `replay_bill_payment_at` | document | door: tags the lines of an entry its own events already replayed; nothing is bounded |
| `mark_cleared` | record |  |
| `set_fx_rate` | record |  |
| `run_payroll` | addAccount, draft, post, record | register: payroll runs |
| `remit_payroll_taxes` | draft, post |  |
| `replay_payroll_run` | record | register: payroll runs |
| `apply_settings` | record |  |
| `annotate_attachment` | record |  |
| `replay_annotate_attachment` | record |  |
| `set_entry_settles` | record |  |
| `replay_set_entry_settles` | record |  |
| `apply_convention` | record |  |
| `apply_convention_source` | record |  |
| `apply_decision_source` | record |  |
| `apply_open_item_source` | record |  |
| `apply_retire_convention` | record |  |
| `apply_decision` | record |  |
| `apply_open_item` | record |  |
| `apply_resolve_item` | record |  |
| `apply_approval` | record |  |
| `add_class` | record |  |
| `set_line_class` | record | a note on a draft: no line moves |
| `replay_class` | record |  |
| `set_project` | record |  |
| `replay_set_project` | record |  |
