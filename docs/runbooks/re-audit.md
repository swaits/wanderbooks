# The re-audit, through the MCP alone

The owner's acceptance of the whole programme (plan/served.md, P10). An
agent audits a company's books using only the books tools, the books' own
filed documents and the viewer's Export menu. Nothing here reads the data
root, a database file or the journal file. The engineering mirror of this
runbook is `crates/ledger-cli/tests/defects.rs` (every tool named below
exists; every defect has a named regression test); the run itself is the
owner's, on the owner's books.

Money is a decimal string everywhere. Every tool named is a tool in
`books.start`'s list; if one is missing, stop: the runbook is stale, not
the books.

## Before you start

- Two roles. The **auditor** runs `books.audit`, reads, and proposes fixes.
  The **verifiers** (at least two agents, not the auditor, not whoever made
  the entry) attest. The human confirms every correction.
- The bar is `policy.attestation_bar` (default `effort=high verifiers=2
  adversarial=true`, D-9). Read it with `books.settings`. A verifier's
  `method` is its own claim, said truthfully: `{ model, reasoning_effort,
  adversarial, verifier_agents }`. A reading that differs from the entry is
  recorded with `disagrees: true` and never counts; never bend a reading to
  fit.
- Context discipline. A receipt is read as text first
  (`books.get_receipt`, the default `meta` view is under 8 KiB, `view:
  "text"` gives the whole text), as one page picture (`view: "page_image"`,
  `page`, `max_px`) only when the text is empty or looks wrong, and never as
  `bytes`. Record each verifier's own token count for receipts; the exit
  bar is under 100K tokens per verifier.
- Start with `books.start` (rules, conventions, open items, its `next_steps`)
  and `books.handbook` section `attestations-and-audit`. The `audit` prompt
  (`scope`: `month`, `annual`, `cpa_handoff` or `full`) is the same
  procedure for a client that offers prompts.
- Audits and attestations need the books' `audits` feature (the owner's
  format upgrade). Without it the tools refuse with `format_upgrade_pending`:
  do the upgrade first, as its own step, with the owner.

## The two cases

- **T1, desktop only.** Claude Code and Codex on the owner's machine, through
  `wanderbooks connect`. Separation between the agent and the data root is
  advisory. The evidence that it held is the access log
  (`books.access_log`, `books:admin`) and the agents' transcripts, which never
  name the data root or a path into it.
- **T2, a self-host machine running** (D-30, D-31; `deploy/README.md`). The
  same steps, plus claude.ai through the tunnel once P9b exists. Separation
  is enforced by the machine's account split. Until P9b, T2 is T1 with the
  server on its own machine.

The steps are identical; only the evidence in step 8 differs.

## Step 1. The first audit and its fixes

1. `books.audit { scope: "full", through: <the last month end> }` without
   `confirmed`: it lists the checks and changes nothing. Show the human.
   Then run it with `confirmed: true` and the `preview` digest. Do the same
   for `{ scope: "cpa_handoff", through: <the fiscal year end> }`.
2. `books.workpaper { id }` for each run: the findings and the PDF as filed.
3. Fix every finding through journaled corrections, never an edit:
   - a wrong posted entry: `books.reverse_entry { entry_id, date,
     description }`, previewed, then `confirmed: true`; then the right entry
     through `books.draft_entry`, `books.add_line`, `books.post_entry`
     (preview, then `confirmed`);
   - a reversal with no receipt of its own (an old one, `legacy_reversal_exempt`):
     its evidence is its original's (D-12, D-47). Link the original's receipt
     to the reversal with `books.link_attachment { attachment_id, subject }`;
     `books.list_receipts { entry_id }` finds the receipt;
   - a duplicate file: `books.consolidate_duplicates` (preview, then
     `confirmed`); `books.find_attachment` to look one up by hash or number;
   - a receipt's facts: `books.annotate_attachment`;
   - an open draft: post it, or `books.discard_draft { reason }`;
   - an undecided 1099 vendor: `books.set_vendor_1099`;
   - a missing tax category: `books.set_tax_category`;
   - a judgment the books should remember: `books.record_decision`.
   Nothing is fixed without the human's confirmation.
4. Audit again at the new head. An audit is idempotent on the same books
   (`already_run`), so a new run means the books changed. Repeat until the
   `full` and `cpa_handoff` runs have no error findings; keep the final two
   run ids. Each journaled fix can be traced with `books.journal_history
   { subject }`.

## Step 2. Every receipt attested

For every receipt on a posted entry (`books.list_receipts`, entry by entry,
or `books.search_entries` then `books.get_entry`):

1. Verifier A reads `books.get_receipt` (text; a page image only if needed)
   and calls `books.attest_receipt { attachment_id, entry_id, amount, date,
   method, note? }` with the total and the date as read off the receipt.
2. Verifier B does the same independently, without seeing A's reading.
3. `books.attestations { kind: "receipt" }` shows each claim with
   `counted`. A claim below the bar, or with `disagrees: true`, does not
   count; the disagreement is a finding: reopen step 1 for that entry.

## Step 3. Every entry reviewed, every period attested

1. `books.review_entry { entry_id, method }` for every posted entry, by a
   verifier that did not make it.
2. `books.attest_period { period, method }` for each month since the
   company's formation (`books.list_periods`), each against a standing
   audit at the current head (`scope: "month"` or `"full"`), then once for
   the fiscal year against the `cpa_handoff` run. A period attestation is
   refused when the books changed since the audit: audit again, then attest.
3. `books.attestations` with no filter: every row has `counted: true` and
   none has `disagrees`.

## Step 4. The defect regressions

The 2026-10-16 audit found 20 defects; P0 found two more (21, 22) and P3
two (23, 24). Each has a named regression test in the workspace;
`crates/ledger-cli/tests/defects.rs` holds the list `(defect, test, file)`
and proves each name exists. Show them green, from a checkout of the build
under audit:

```
cargo test -p ledger-cli --test defects
```

then each test named in that list, by name, for example
`cargo test -p ledger-core --test spec_cash_flow
defect1_two_member_llc_first_year_regression`. Two defects have no code
fix to test: defect 2 (reversals without their own evidence) is closed by
D-12 and D-47 (its test shows the rule now) and defect 20 (the migration
scripts) was retired at the cutover (no tool exists; the test checks none
came back). Record the test run's output in the sign-off decision (step 7).

## Step 5. The Sheet comparison

The old Google Sheet and the books were frozen as of 2026-10-11 and
compared a final time before the cutover (P2, `verify-sheet`); that run is
filed as a document. There is no comparison tool and this
runbook builds none.

1. `books.find_attachment { text: "verify-sheet" }` finds the filed run
   (its extracted text names the tool); `books.get_receipt { attachment_id }`
   reads it (text view; its `links` and `sha256` identify it). Cite it by
   attachment id and sha256. If it is not found, stop: the comparison was
   not filed at the cutover and the owner decides what stands in for it.
2. `books.record_decision { subject: "Sheet comparison as of 2026-10-11",
   decision_md, decided_by: [<the owner>], decided_on, evidence_attachment:
   <that workpaper's id> }` stating that the frozen Sheet and the books
   agreed then, and nothing newer is claimed.

## Step 6. The CPA package from both doors

1. The agent's door: `books.export_cpa_package { as_of: <the year end>,
   include_tins: true }` (TINs are included by default, D-29). Without a
   counted audit it is refused (`audit_required`); never use
   `without_audit_reason` for the re-audit, the point is that it is not
   needed. Save the zip with the download command in its `file`.
2. The viewer's door: `wanderbooks gui <book>`, the Export menu, the CPA
   package as of the same day. It lands in the Downloads folder.
3. Compare the two zips file by file: byte-identical except for timestamps
   (the README's generated-at line and the zip entries' times). A difference
   in any figure is a defect: stop and report it.

## Step 7. Verify, back up, sign off

1. `books.verify { blobs: true }`: `status: "ok"` (not `healed`: a healed
   result means a torn tail was set aside; note it and look at
   `books.journal_history`'s `tail_repairs`).
2. `books.backup`: every target not `stale`, `last_error` empty, and
   `last_drill_at` set with no `drill_error`. If not, `books.back_up_now`
   (an admin action) and read it again. The drill is the restore test; a
   backup that was never restored is not a backup.
3. The owner's sign-off: `books.record_decision { subject: "Re-audit
   sign-off", decision_md, decided_by: [<the owner>], decided_on }` naming
   the final `cpa_handoff` and `full` run ids, the journal head
   (`books.verify`'s `head`), the defects run, and the CPA package's sha256.

## Step 8. The exit evidence

The re-audit passes only when all of these are true. The owner checks them;
the agent presents the proof for each.

| Exit bar | Proof |
|---|---|
| Zero failing workpaper checks | the final `full` and `cpa_handoff` workpapers (`books.workpaper`) |
| Cash flow difference 0.00 | `books.cash_flow` for the year; no `cash_flow_unfooted` finding |
| Every receipt and posted entry has a counted attestation, none disagrees | `books.attestations` (step 3.3) |
| Every action under the right principal | `books.access_log`: the auditor's token and each verifier's token appear on their own calls; no action by an admin token that an agent token did |
| No transcript names the data root | grep each agent's transcript for the root path before sign-off (T1); T2: the tenant root is not readable by the agent's account |
| No result carried a full TIN to a client that should not see one | `books.access_log` shows `include_tins` only on `books.export_cpa_package` and `books.get_w9`/`books.vendor_1099` calls by the owner's agent; the agents' exports may carry them (D-29), nothing else |
| Receipt reading stayed under 100K tokens per verifier | each verifier's own token count for its receipt calls (the harness's count, written into the attestation `note`) |
| The sign-off is journaled | `books.decisions` shows it |

When all are true the books are re-audited through the MCP alone.
