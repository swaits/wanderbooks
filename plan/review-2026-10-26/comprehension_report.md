# Stage 6: Comprehension review of `plan/served.md` (DetailedRange 766-2058)

Reviewer: `egm-comprehension`, 2026-10-26, as the entry-level engineer handed ONE unit in a worktree. Symbols checked under crates/, docs/, verify/, bookspec/, fuzz/.

**Verdict:** no remaining unit (P4b S4..S7, P4, P5..P13) can be re-implemented from the text alone without guessing. The P4b slices come closest (P4b.3-P4b.11 carry real contracts); P5-P13 are scope lists, not specifications.

## A. Cross-cutting gaps (every unit)
- **A1 Blocker. Numbered references that resolve to nothing.** "defect 2" (P4 title) is never described; "defects 4, 10, 20" (P6) only by a phrase; "decision 1, 8, 9, 11, 12" (P4b.5/7/8/14/17) do not match the numbered owner answers 1-5 (they come from a question-tool session not in the repo); "the private change log" (909, 1486, 1623, 641, 969) and `goldens/p4-arap-before/`, `goldens/p4-arap-sN/` (1477, 1495) are outside the repo (the only in-repo thing with that name is a journal fixture under `crates/ledger-store/tests/fixtures/`).
- **A2 Blocker. "Done" is not defined per unit.** P4b S4-S7 have no Accept block; P4b.15 lists ~55 tests for seven slices without saying which slice lands which (`an_invoice_from_issue_to_paid` spans S4, S5 and S6). P5-P9 Accepts are mostly live-session or private-data checks.
- **A3 Major. Stale source anchors.** Every `S:NNNN`, `ledger.rs ~NNNN`, `types.rs:NNNN`, `docs/mcp-interface.md (24-95, …)` anchor; "schema v12" vs `SCHEMA_VERSION = 15`.
- **A4 Major. "for one release" (83, 442, 1294, 1933) is undefined**; no release versioning scheme is stated.
- **A5 Major. Where "Needs attention" items live is undefined** for P4, S5, P6, P7, P8: `ledger-view` does not exist and no phase's scope creates it.
- **A6 Major. Dependencies and parallelism are never stated as a graph.** Hidden: S5/S6/S7 "take preview_token once P4 lands"; S7 adopt/move hits P4's gate; P5's `next_steps::decorate` needs the attention engine; P8 cross-checks need P6; P10 needs P6, P8, P9; P11's updater needs `books.job` and `wanderbooks rollback` (listed in P2 scope at 859 but never built). Shared files every slice edits: `server.rs`, `types.rs`, `scopes.rs`, `format.rs` KNOWN_FEATURES, `schema.rs`, `docs/mcp-interface.md`, `docs/spec/rules.md`, `docs/testing.md`.

## B. P4b S4 `documents_v2`
- **B1 Blocker (1002 vs 1215/1631).** Answer 3 requires `invoicing.payment_methods` list with QR; P4b.4/11/17 specify a single `invoicing.payment_instructions` string and "that key only" exempt. `qrcode` 0.14.1 already in `ledger-import`.
- **B2 Blocker (1289-1298, 1236).** Preview-then-confirm changes today's `create_invoice`/`create_bill` contract (`CreateDocumentParams {party_id, date, due_date, lines}`). Unstated: behaviour on books without `documents_v2`; whether `due_date` becomes optional; `terms? | due_date?` with both or neither; fallback order (party default → `invoicing.default_terms`); the type of `terms` (string to parse or object).
- **B3 Major (1159-1163).** `Terms.text` has no composer; `EarlyDiscount.days < net` meaningless for NetEom/DueOnReceipt; `due_date -> Result` error cases; `0..=3650` vs `u16`; reuse of `validate_due_date` (arap.rs:599) not said.
- **B4 Major (1194-1198, 1492).** Zero-pad width; behaviour at 10000; `first_number` "after the highest legacy id" while legacy invoices have no number; prefix change after first issue; preview number vs confirm number.
- **B5 Major (1170).** `quantity × unit_price == amount exactly`: decimal scale unspecified (3 × 33.333).
- **B6 Major (1291, 1371-1373).** Filing a server-generated PDF "in the same store call" needs a new store path (D-46: all attachments arrive by upload). File name, MIME, uploader principal, and how the DRAFT preview PDF is returned (HandedFile? counted against the 64 cap?) unstated.
- **B7 Major (1362, 1218).** Seller snapshot wants "email or website"; no `company.email`/`company.website` setting exists.
- **B8 Major (1296-1297).** `create_bill`: `vendor_ref` required but legacy bills lack one; omission of `upload_id | attachment_id` refused or allowed; `received_on` has no stated use.
- **B9 Major (1290, 1311).** `bill_to?` type; `update_party` `default_terms` one value for both customer and vendor sides?
- **B10 Major (1331-1337, 1595).** "The handbook procedure": no section id; `crates/ledger-mcp/handbook/` has 21 sections and 4.3's list (407) names different ids. B-39/B-40 need writing; B-38 already exists (rules.md:2872) so "new rules B-38 to B-41" is partly stale.
- **B11 Major (1268).** Which crate gates `settings.set` of `invoicing.*` behind the feature (`setting_feature` is `pub(crate)` in ledger-store; the registry is in ledger-core); the exact list of writes that must still work without the feature is not enumerated.
- **B12 Nit.** arap `Snapshot` collides with `ledger_store::Snapshot`. **B13 Nit.** `customer` name resolution when a name matches two (precedent `inv6_ambiguous_vendor_reference_refused`).
- **B14.** No Accept; candidate tests exist by name only.

## C. P4b S5 sent tracking
- **C1 Blocker (1003 vs 1185-1186/1269).** Answer 4 requires who sent and the channel's reference; `Sent`/`invoice.sent` carry neither.
- **C2 Major (1325).** Defaults for `on`, `via`, `kind`; `to` when the party has no email; mark_sent on Void/Paid/legacy; whether bills get sends.
- **C3 Major (1330).** "never sent (after 2 days)": from issue date or today, which timezone, calendar days; `reminder_days` is in S4's settings list.
- **C4 Major (1342).** mailto body "about 1,800 characters" is not a contract; S3 already built the mailto link; S5's delta unclear.
- **C5.** No Accept.

## D. P4b S6 paying on terms
- **D1 Blocker (1003 vs 1597/1244/1300).** The processor fee (gross to A/R, fee to expense, net deposited) is assigned to S6 by STATUS but appears in no S6 line, posting row or argument; no fee role/account in the role table.
- **D2 Major (1300, 1304).** Top-level `amount` vs Σ applications; whether application `amount` is cash only or cash+discount; `paid_in_full_marking?` type and slice.
- **D3 Major (1323).** `books.bills_due` output has no schema; "amount needed by each date" undefined.
- **D4 Major (1352-1353).** Aging CSV columns, tie-out line format, README text, xlsx/pdf templates; `detail?` and `control{}` on the aging tools have no slice.
- **D5 Major (1204-1210).** "Expense (credit balance)" has no account-type variant; roles "re-checked against the live chart" is the owner's private books.
- **D6 Major (1259).** Is `discount_reason` journaled (feature row names only `discount`)?
- **D7 Nit.** Bill flags `due_soon`/`discount_open`: S5 or S6?
- **D8.** No Accept.

## E. P4b S7 bad debts and legacy lines
- **E1 Blocker (1326).** `write_off_invoice` "needs evidence attached in the same call" but has no `upload_id`/`attachment_id` argument.
- **E2 Blocker (1327, 1272).** `adopt_control_line`: how the Invoice/Bill record is built (date, lines, customer/vendor creation), what `record_id` is, numbering, replay determinism.
- **E3 Blocker (1312, 1251, 1272).** `assign_role move_balance`: entry dates, description, party tag, evidence under P4's gate, unattached legacy lines, zero balance, atomicity.
- **E4 Major.** Cash-basis refusal: error code/guard name. **E5 Major (1249).** Recovery reversal vs P4's reversal-as-draft. **E6 Major (1208).** On-demand role creation when 6990 is taken. **E7 Nit.** B-41 and Lean `bounds_preserved` are names only.
- **E8.** No Accept.

## F. P4 drafts, reversal as draft, evidence gate, digest
- **F1 Blocker (971).** "defect 2" undefined; exemptions named by stale line numbers; the live handbook (`drafts-and-reversals.md:4`) says reversals need no receipt while I-5 says they are not exempt.
- **F2 Blocker (978, 66, 83, 512).** `preview_token`: "canonical draft state" undefined; "all confirm tools" not enumerated; 83 vs 512 contradict on whether `confirmed` without a token is accepted; `preview_entry`'s output unstated.
- **F3 Blocker (974, 1109, 1316, 1228).** Does `void_document` become a draft after P4? Draft-time vs post-time for `check_reversal`/`check_item_bounds`.
- **F4 Major (974).** `ledger_core::evidence::requirement` does not exist (gate is `ledger_config::ReceiptPolicy`); signature and return unstated; "exemption is sourced" names no source.
- **F5 Major (513, 574).** How `link_original` is journaled; what posting a lineless reversal draft journals; replay rebuilds lines from which original state.
- **F6 Major (976-977).** `list_drafts` shape; `as_of_status` on which reports; `strict` semantics; `posted_seq`/`entry_meta` do not exist; "zero-net touched accounts", "reconciliation_tracked", "never_reconciled" undefined.
- **F7 Major (981-985).** Lean draft-state notion absent; `Step.discard` obligations; `tests/model.rs` extension unstated.
- **F8 Major.** Attention location (A5). **F9 Major (993).** "0 unexplained drafts" is private. **F10 Nit.** "gains the new ops" lists none.

## G. P5 agent surface
- **G1 Blocker.** `ledger-view` never created by any scope while `next_steps::decorate` and `books.start` v2 need it.
- **G2 Blocker (1643, 411-423).** Prompt texts, argument validation and outputs unspecified; "golden render per prompt" impossible.
- **G3 Major (1644, 444-460).** Guard code list, per-tool next-step rules, merging with existing `warnings`; `books.start` already returns `next_steps`.
- **G4 Major.** `books.job` retention, id, polling, cancellation, which tools; `journal_history` output and `subject` grammar.
- **G5 Major (1646).** Source-only revision: no event name or feature. **G6 Major (1647).** URL-mode elicitation has no local use (D-28). **G7 Major (1649).** Agent-eval harness mechanics; depends on P4. **G8 Major.** Accept on a nonexistent fixture and manual sessions. **G9 Nit.** Which eight descriptions exceed 700 characters.

## H. P6 evidence pipeline
- **H1 Blocker.** Sandboxed `ledger-extract` contradicts D-32. **H2 Blocker (1682).** Migration tools' inputs are private scripts and files; formats unspecified.
- **H3 Major.** `inline_base64_requires_sha` contradicts D-46; `get_receipt` views `text`, `page_image` outputs unstated; `books.download_link` ambiguous under D-46.
- **H4 Major.** `attach_email`: no `eml` upload purpose; decision 5 defers email-in; Typst template unstated.
- **H5 Major.** `text_extracted` fields; backfill trigger; receipt numbering field; FIFO `reimbursed` inputs undefined; `annotate_attachment` validation.
- **H6 Major.** Consolidation event names absent from 5.2. **H7 Major.** Company record category list. **H8 Major.** Accepts are private data; hayro already present, not mentioned.

## I. P7
- **I1 Blocker.** Contradicts D-28, D-29, 3.6 as-built; which parts survive is undecided. **I2 Blocker.** Keyslot formats, `rekey`, HPKE crate; 4.1 still names `request_tin_entry`. **I3 Major.** Accept against private goldens. **I4 Major (644 vs D-31).** "No vault key material on disk" is false for backups.

## J. P8
- **J1 Blocker (1727-1734).** Each check is a noun phrase; no predicates, finding codes, workpaper schema or Typst template; "2100 aging" is a company-specific code in a generic engine.
- **J2 Blocker (1738-1742).** Attest tools have no argument lists; `policy.attestation_bar` has no value schema; which P6 fields the cross-check reads.
- **J3 Major.** `workpaper` category, `books.workpaper`, `books.attestations` not in scope. **J4 Major.** The refuse-without-audit override parameter is unnamed. **J5 Major.** "access-log tooling" in the title, none in scope. **J6 Nit.** Lean lemma has no statement.

## K. P9
- **K1 Blocker.** D-7 undecided, determines most of the code. **K2 Blocker.** Archive unit, seccomp/Landlock, `fetch` do not exist (D-31, D-32). **K3 Major.** `tunnel` command or doc; MCP App widget unspecified; `/h/inbox` built here or not. **K4 Major.** Replica traits, headers, events unnamed; `invite` sends mail though the books never send. **K5 Major.** No two-user CI job exists.

## L. P10
- **L1 Blocker.** Entirely on private books by live agents; depends on P6, P8, P9; "22 defect reproductions" stale (23 and 24 exist). **L2 Blocker.** "No TINs through the agent" contradicts D-29. **L3 Major.** `fetch`/`verify-archive` never existed. **L4 Major.** "100K tokens" has no measurement method.

## M. P11
- **M1 Blocker.** Dock built on the helper and confinement D-32 removed. **M2 Blocker.** Signing identities, repository keys, hardware token, GitHub Releases: outside the repo and the engineer's authority; the unit begins with research. **M3 Major.** `rollback`, `update.lock`, `update`, manifest format, staged layout unstated. **M4 Major.** Books-home file contents not given. **M5 Major.** Tests need clean VMs; no CI definition.

## N. P12
- **N1 Major.** Hardware external. **N2 Major.** "set time" never set; location of documented maxima. **N3 Nit.** `--jobserver-tasks` unverified. **N4 Nit.** §6 promises recipes for crates that don't exist.

## O. P13
- **O1 Blocker (1886).** The in-repo seed (`bookspec/`, 56 examples, checked by `cargo test`) is never mentioned; an engineer would start from zero. **O2 Blocker (1873).** `Engine` trait has no signature; TOML schema is prose. **O3 Major.** Rule identifier scheme; mapping from B-1..B-38; no done-line for a first edition; Lean consistency claim has no formalization hint. **O4 Major.** Claims list and the GnuCash run undefined.

## P. Contradictions
P1 reversal-as-draft vs `void_document` posts at once. P2 `payment_instructions` string vs `payment_methods` list. P3 `Sent` without sender/reference vs answer 4. P4 fee in STATUS/S6 vs absent from S6. P5 PDF stamps: three different lists; `invoice_pdf(&InvoiceDoc, &DocMeta)` vs built `invoice_pdf(&InvoiceDoc)`. P6 "slice S1 next" vs S1-S3 landed. P7 P7's scope vs D-28/D-29/3.6. P8 sandbox/archive unit/helper vs D-31/D-32. P9 `inline_base64_requires_sha`, `download_link` vs D-46. P10 P10 step 6 vs D-29. P11 `verify-archive`/`fetch`/`heads.jsonl` vs D-31. P12 `confirmed` without token accepted (83) vs required (512). P13 schema v12 vs 15. P14 "22 defects" vs 24. P15 P8 title vs scope. P16 4.3 handbook ids vs 21 files. P17 drop page: P2 scope vs 4.7 as-built vs P9 Accept. P18 "new B-38" vs existing. P19 `ledger-view` in P5 per P1 as-built vs absent from P5 scope. P20 `goldens/p4-arap-before/` vs private goldens vs same-named fixture.

## Q. Symbol verification
**Exists:** arap.rs (DocRef, Movement, DocTag, DocState, DocStatus, ControlCheck, aging_bucket, document_parts, validate_due_date, derive_status); `Ledger::stands_on`, `in_effect`, `check_reversal`, `check_item_bounds` (private), `control_check`, `void_document`, `invoice_state`/`bill_state`; `UntaggedControlLine`, `PaymentBeforeDocument`; KNOWN_FEATURES with `document_standing`, `document_line_projects`; `setting_feature` (pub(crate)), `ensure_feature`; `CreateDocumentParams`, `PaymentParams {instrument?, reference?}`; `invoice_pdf(&InvoiceDoc)`, `InvoiceStamp` (5 variants), `invoice_document`, `PackageBooks`; `Subject::Invoice/Bill`; `has_full_tin`/`redact_full_tins`; `company.duns` exemption, `company.name`, `company.address`, `books.accounting_basis`; `qrcode`, `hayro`; Lean `control_agrees`, `control_agrees_through`, `Step.document_unattached`, `cashflow_sections_sum_to_cash_change`, `draftsOpen`; `tests/model.rs`; fuzz targets; `local_server.rs`, `tests/common/mod.rs`, `projects_parity.rs`; `receipt_policy.rs`; `tools/{access,backup,files,instruments,kept,projects}.rs`; `books.start` with `next_steps`; docs/testing.md "For P12" sections; scheduled.yml; `bookspec/` seed; docs/oracle.md; rules.md B-38.
**Missing:** `Terms`, `TermsKind`, `EarlyDiscount`, `Sent`, `SentVia`, `SendKind`, arap `Snapshot`, `issued_pdf`, `vendor_ref`, `customer_ref`, `duplicate_reason`, `default_terms`, `check_document_line_account`, every `invoicing.*`/`bills.*` key, `company.email`/`company.website`; features `documents_v2`/`invoice_sent`/`settlements`/`write_offs`/`control_lines`; events `invoice.sent`/`invoice.written_off`/`control_line.adopted`/`role.balance_moved`; tools `mark_sent`/`bills_due`/`write_off_invoice`/`adopt_control_line`; `ledger_core::evidence`; `preview_token`/`preview_entry`/`list_drafts`/`set_draft_state`/`discard_draft`/`inherit_evidence`/`as_of_status`/`posted_seq`/`entry_meta`; `ledger-view`, `ledger-extract`, `lopdf`, `mail-parser`; `text_extracted`/`find_attachment` tool/`consolidate_duplicates`/`annotate_attachment`/`compare_reference`/`record_inventory`/`check_inventory`/`attach_email`; `journal_history`/`books.job`/`guards` envelope; `audit.rs`/`attest_*`/`workpaper`/`attestation_bar`; hpke/`keyring.json`/`request_tin_entry`/`set_w9_facts`/`/h/` routes; `fetch`/`tunnel`/`invite`/`verify-archive`/`rollback`/`update`/`update.lock`; replica routes; `scripts/*`; `goldens/*`; rules B-39/B-40/B-41; a handbook section for invoices.
