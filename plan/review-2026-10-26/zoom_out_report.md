# Stage 5: Zoom-out review of `plan/served.md` (full file; focus P4b S4 – P13)

Two independent `egm-zoom-out` reviewers (A, B), 2026-10-26. Findings deduplicated; where both reported the same point it is marked (A+B). Line numbers are the file as it stood at `c1f9001c`.

## Blockers

### Z-1 (A+B). Section 8 has no "where we are / what is next / what runs in parallel" map
Scope: §8 preamble 766-774; P4b "Status" 997; P4 header 971.
Preamble still says every phase "runs mutation testing" (D-33 paused it). Line 997 says "slice S1 next" while S1-S3 are on main. P4 has no status line. P4b.3 calls `stands_on` "new code" though it is built (`ledger.rs:2106`). A fresh engineer cannot tell which of P4 and S4 is first. Fix: a status/dependency/ownership table at the top of §8; correct 997; "as built" blocks for S1-S3.

### Z-2 (A+B). P7 "Vault v2 and the human channel" builds what D-28, D-29, D-31, D-32 forbade
Scope: P7 1697-1723; 5.6 row 644; 4.9 509, 535; 4.1 370; risk 5 1907-1908; P10 step 6 1787.
`/h/tin`, `/h/w9`, `/h/unlock`, `request_tin_entry`, `set_w9_facts`, removal of `set_tin`/`set_w9`/`include_tins`, GUI passphrase unlock, HPKE keyslots, recovery key, `rekey`. D-28: TINs go through the agent into the vault, "no human web pages". D-29: TIN exports keep working for the agent, "no passphrase, no extra prompts". 3.6 as-built already says so. The live code agrees (those tools exist in `server.rs`; `/h/` appears once in the crate tree). Fix: shrink P7 to the known TIN-scanner defect (line 200: "123,45,6789") and a decision record closing vault v2; strike the superseded sentences elsewhere.

### Z-3 (A+B). P4 is unreconciled with the built P4b and the two reference each other in both directions
Scope: P4 971-993; P4b.3 1109; P4b.5 1236; P4b.8 1285, 1291; P4b.11 1291.
(a) P4 makes reversals drafts needing evidence; S1 built `books.void_document` as an immediate reversal. The plan never says which wins or what evidence a void needs. (b) S4 "takes `preview_token` once P4 lands" while P4 is listed after P4b; `preview_token` exists nowhere in the code. (c) P4 adds four new event types but names no format feature and no owner upgrade (D-39); decision 11 batches P4b's upgrades after S1 and S7, so S4-S6 writes are unusable on the live books until the S7 upgrade, which the plan never says out loud. (d) Every code pointer in P4 is stale (see Z-11). Fix: one order written down (recommended: S4-S7 with `confirmed` only, then P4 retrofits `preview_token` in one sweep); P4 states a void is a reversal with evidence inherited from the original (D-12 generalized); P4 names a feature and joins S7's upgrade batch.

## Major

### Z-4 (A+B). The attention engine: new crate `ledger-view` is YAGNI and arrives too late
Scope: 2.1 line 96; P1 as-built 832; P4 979; P4b.8 1330; P5 1640-1649; P8 1744.
No `ledger-view` exists. `attention::find` is in `ledger-gui/src/attention.rs:109` with helpers in `books.rs` and `pages/parties.rs`; the server's `next_steps` are hand-coded in `ledger-mcp/src/tools/kept.rs:764-832`. S4-S7, P4 and P8 each add attention items, so every item is written twice until the move. `ledger_package::PackageBooks` is already the shared read-only view over Store and Snapshot. Fix: move attention over `PackageBooks` (in `ledger-package` or `ledger-store`) as the first unit of remaining work; no new crate or trait.

### Z-5 (A+B). P6: `ledger-extract` sandboxed subprocess contradicts D-32 and adds a third PDF parser
Scope: P6 1664-1672, 1689; 3.1 128-129, 139; 2.1 94.
PDF text and page rasters already exist with hayro in `ledger-gui/src/receipts.rs` (`text_of` :467, raster :390) and `ledger-mcp/src/tin_files.rs:314`. Fix: one in-process `text_of`/`page_png` in `ledger-import`, called by server and viewer, journaling `attachment.text_extracted`; drop the subprocess, sandbox test and lopdf. Also stale: `inline_base64_requires_sha` (D-46), "the `company/` folder retired" (done), `attach_email` (unrequested; `.eml` is not an upload purpose).

### Z-6 (A+B). P6 migration tools and P10 step 5 build engine code for a one-time comparison whose inputs are private
Scope: P6 1682, 1695; 5.6 643; 4.9 507; P10 1786.
`compare_reference`, `record_inventory`, `check_inventory`, an exact-decimal XLSX reader, and journal events `inventory.*`/`reference.compared` replace scripts that now live only in the owner's private `legacy/`. The Sheet and the books as of 2026-10-11 are frozen; the final `verify-sheet` run is a filed workpaper. Fix: P10 step 5 cites the filed workpaper and records a decision; drop the tools. Dedup (defect 4) is partly done already (`attach_receipt` returns `deduplicated`).

### Z-7 (A+B). P8: `books.audit` re-implements existing checks; the attestation-bar machinery is heavy for a self-reported claim
Scope: P8 1724-1752; D-9 1949; risk 11 1930.
Integrity = `Store::verify`/`verify_blobs` + access-log chain; tie-out = `control_check` (already in `verify` and the digest); cash flow `reconciles` exists. Fix: compose `books.audit` from existing checks plus the genuinely new ones (closes in order with member deltas; schedules posted through), writing a workpaper (needs a `workpaper` document category: store categories today are receipts, formation, legal, tax, accounting, other). Attestations record who/when/what was claimed; the bar is a handbook convention; the CPA package warns, or refuses with an override per D-9. Drop `policy.attestation_bar` versioning and the Lean counting lemma. "Run as a job" depends on `books.job` (Z-9).

### Z-8 (A+B). P9 is three products in one phase, not on the P10 path, and still prescribes seccomp/Landlock units
Scope: P9 1754-1777; risk 8 1919; D-7 1988; 4.7 470.
In-house OAuth 2.1 server (security-critical, needs external review), replica viewer protocol, passkey invitations, MCP App widget, `openai/fileParams`. P10 already allows the claude.ai leg "not applicable". Fix: P9a = self-host recipe + `books adopt` with head check + static bearer over a tunnel; P9b = claude.ai via D-7(b) external IdP only when wanted; P9c = replica/invites not planned. Remove seccomp/Landlock from line 1759 (D-32).

### Z-9 (A+B). `books.job` and the 20 s rule are premature locally
Scope: 3.2 153; P5 1645; P8 1726; P9 1776.
No measured call approaches a client limit over the bridge. Build jobs in P9 where claude.ai's 240 s limit is real.

### Z-10 (A+B). P11 builds the dock on the helper and confinement D-32 removed, and fuses packaging with the dock
Scope: P11 1798-1833, esp. 1817, 1827, 1801-1812.
No `helper.rs` exists; 3.9 as-built says "No confinement, no helper". The dock spawns the agent as an ordinary child. Split: P11a packaging/updaters (Linux first; the format gate is already specified in 3.9), P11b dock + macOS/Windows signing. Within P11a the bespoke two-key manifest wraps cargo-packager's minisign: start with minisign + OS code signing (meets D-24); add the manifest only on an accepted threat.

### Z-11 (A+B). Stale line numbers, schema numbers and symbol names throughout the remaining phases
Scope: P4 974-976 (`ledger.rs:1355-1380`, `:1319`, `S:561-568`, `S:2540`, `receipt_policy.rs:6`, `books.rs:648`, "schema v12"); 5.2 588; P5 1648; risk 13 1932 (server.rs "8,529 lines": now 12,566).
`check_reversal` is at `ledger.rs:1988`, `reverse_entry_steps` at :2170; the gate is `ledger_config::ReceiptPolicy` via `ledger-mcp/src/policy.rs`, not `ledger_core::evidence::requirement`; `SCHEMA_VERSION` is 15 (shipped 14 per STATUS). Fix: symbol names and files, never line numbers.

### Z-12 (A+B). P10's steps and exit criteria name mechanisms that were never built or were superseded
Scope: P10 1782-1796.
`wanderbooks fetch`, `verify-archive --expect-head` (D-31: restore preview and drills instead); "TINs through the human link" (D-29: `wanderbooks download`); step 5 needs Z-6's tools; step 2 needs Z-7's bar machinery. Fix: restate P10 against what exists: `books.verify` head, backup drill, `books.control_check`, the 22 defect reproductions, the CPA package from both doors.

### Z-13 (A). `server.rs` is the merge hotspot for a parallel team and the promised split is not happening
Scope: risk 13 1932; 4.9 542.
12,566 lines; six files under `tools/`. Every remaining phase adds tools there and to `types.rs`, `scopes.rs`, `docs/mcp-interface.md`, `docs/testing.md`, `docs/spec/rules.md`. Fix: a single-owner pre-step splitting `server.rs` by tool group before fanning out; each unit states which files it owns.

### Z-14 (A+B). P4b carries two designs for the same thing and no "as built" markers
Scope: 995-1636.
Owner answer 3 (1002) specifies `invoicing.payment_methods` list with QR; P4b.4 (1215) and P4b.11 (1367) still say a single `invoicing.payment_instructions` string. `qrcode` with `svg` is already a dependency of `ledger-import`. Answer 4's processor fee (gross to A/R, fee to expense, net deposited) appears in no tool argument or slice; STATUS puts it in S6. S6 and S7 partly exist (`Movement::Discount`/`WriteOff` and their derivations shipped in S1). Fix: pick the list; put the fee in S6; "as built" for S1-S3; "to build" for S4-S7 scoped against what exists.

### Z-15 (A+B). Background sections a fresh reader would wrongly build
Scope: 4.1 370; 4.9 504-509, 535; 4.7 467, 476; 5.2 577-584; §6 663, 685-697; risks 1, 6, 12; 7.4 738; 7.6 751.
TIN human pages and tools, `books.inbox`, `books.attach_email`, `books.download_link`, `books.invite`, "Removed: `get_w9`", `/h/inbox`, P6/P7/P8 event rows, Landlock/seccomp, `ledger-view`, "small receipt files go straight through the chat". Fix: strike-through with the superseding decision, as 2.1's inline-bytes row already does.

## Nits
- Z-16 (A). P4b.8/P4b.9 weeds: mailto body cap, `party_id` alias "for one release", paper size, reminder day counts belong in slice briefs or GOAL blocks.
- Z-17 (A+B). P13: engine adapters for hledger/Beancount/QuickBooks are speculative; the move to its own repository must say how `just check` keeps running the examples (vendored copy or pinned dependency); keep in-tree until the harness is stable.
- Z-18 (A). P6 `reimbursed` derived by FIFO is a heuristic that will be wrong; `entry.settles` plus an attention item is simpler.
- Z-19 (B). `preview_token` as an HMAC with a key file buys little locally (I-6 admits it); a plain `preview_digest` detects staleness. Say whether forcing `preview_entry` is wanted.
- Z-20 (B). P12 says "every fuzz target, the old and the new": `fuzz_http_mcp_request` and `fuzz_upload_put` were never added. List what P12 must build versus run.

## Forced dependency order (reviewer B's synthesis, checked against A)
1. Pre-step: attention engine over `PackageBooks`; `server.rs` split by tool group (single owner).
2. P4b S4 → S6 (S6 needs `Terms`, `discount_by`, `max_discount`). S5 and S7 need only S1 and can run beside S4.
3. P4 after S4-S7 (or S4 without `preview_token`); P4 and S7 share one owner upgrade.
4. P5 after P4 (`books.start` v2 lists drafts by state).
5. P6 (extraction only) before P8's receipt cross-check and P10 step 2.
6. P5, P6, P8 before P10. P12 last, over everything.
Independent of the P10 path: P9 (whole), P11, P13's spec writing, the P7 scanner fix.
