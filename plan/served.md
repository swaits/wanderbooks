# wanderbooks Served: the plan

## Decisions of 2026-10-16

Tobin Harlow made these decisions on 2026-10-16. Where anything below conflicts with them, they win, and the plan has been revised to match.

1. **The books live hidden away, as you.** Your computer gets no separate system account, no sudo and no containers. The books server runs as you, only while something you started is using it: the app, an agent session, or a terminal (amended 2026-10-18, D-21 and D-23: no background service). It keeps the books in a private folder that only your account can open, outside every project folder. Nothing the agent is told or shown ever names that folder. This means an agent cannot stumble into the books. An agent that went looking on purpose could still find them, and the books say so plainly ("same user, advisory"). Anyone who wants a hard wall runs the same server on another machine they control, such as a home server, a NAS or a rented server.
2. **The viewer reads the books directly, read-only, as it does today.** It never changes anything and never goes through the agent's connection. When the agent asks it to show you something, the request reaches the viewer through the books' own private folder, not through the agent's tools.
3. **No extra pop-ups.** Posting, closing and reconciling add no new permission prompts. The agent still shows you each entry before posting it, and a hidden check makes sure that what gets posted is exactly the preview the agent was given to show you.
4. **Your own server now, a paid hosted service later.** This plan builds the recipe for running the books on another machine you control, and claude.ai on the web can reach that machine. A hosted service for many customers is described separately at the end (section 11). This plan does not build it.
5. **Decided by the engineer from the standards:**
   - The company EIN is kept in the books' settings and shown masked.
   - Audits use the recommended review bar. The CPA package refuses to go out without a full counted audit unless a reason is given.
   - Odd-cent splits between members are tracked from the company's start.
   - New prepaid schedules are spread by the day.
   - During the re-audit, old reversals posted without their own receipt borrow the original entry's receipt.
   - The company's time zone is America/Denver.
   - Photographed receipts get no text recognition for now.
   - The CPA gets tax ID numbers through a one-time link that you send.
   - Emailing receipts in waits for the hosted service.
   - The test-only direct mode stays for tests and demos.
6. **Three questions wait for the move to the server (P2):**
   - Where should the automatic backups go?
   - Should the papers in the `company/` folder be filed into the books?
   - Should the old copies of the books in the project folder move into the private folder? This is recommended, and nothing is deleted.
7. **Light governance.** The two-member LLC gets no consent paperwork, spending limits, signed memos or amendments. Decisions are recorded with who made them and when. Getting your go-ahead before household reimbursements is a note, not a lock on posting. Approval rules exist in the engine for other companies, but they stay off unless a company turns them on.
8. **Plain language.** the owner reads the Summary and the User flows, so both are written in plain English.

---

Wanderbooks becomes a books server that owns its own data, and any agent can use the books from anywhere through the MCP. This plan starts from the winning proposal ("Served Books, built incrementally": `ledger-mcp` grows into the server in place). It adds the best ideas from the "Books as a Tenant" proposal (B) and the "One MCP, any agent" proposal (C), fixes every weakness and decision violation the three judges named, and states each resolution.

Paths are relative to `/home/swaits/Code/wanderbooks/crates/` unless they are absolute. `S:` means `ledger-mcp/src/server.rs`. These references were spot-checked against the source: `INSTRUCTIONS` S:57, `lock()` with a std `MutexGuard` at S:553, `open_vault` S:521, `redact_refusal` S:870, `today()` S:1943, `import_local_file` S:4511, `call_tool` S:8160, `get_info` S:8171, `cash_flow` `ledger-core/src/reports.rs:1231` with the "Owner distributions" label at :1437, `split_first` `ledger-core/src/shares.rs:197`, `apportion` `ledger-core/src/kernel.rs:309`, `built_in_gain` `ledger-core/src/assets.rs:204`, 20 `#[kani::proof]` in `ledger-core/src/proofs.rs`, and `verify/lean/Wanderbooks/{Apportion,Statements,Ledger}.lean`. This session's own copy of the wanderbooks instructions arrived cut off at "Prepaid expenses and deferred revenue: record the original… [truncated]", which confirms the 4,096-character client limit.

---

## 0. How to read this document

Written 2026-10-26, when the plan was reworked for a team of engineers working in parallel. Everything below it is the plan; this section says how its parts fit.

**What is where.** Sections 1 to 7 are the design: what the system is, the invariants it enforces, its architecture, the agent surface, how state migrates, the threat model, and the owner's flows. Section 8 is the work: 8.0 how it is organized (status, dependencies, shared files, the unit template, the recipes), 8.1 what has landed, 8.2 every remaining unit in merge order, 8.3 the P4b reference design, 8.4 what is deliberately not scheduled. Section 9 is cross-cutting risks, section 10 the decisions and the open questions, section 11 the hosted service this plan does not build. `plan/STATUS.md` says where the live books stand.

**Precedence.** The owner's decisions win: the eight at the top of this document (made 2026-10-16), then every D-n row in 10.1 in order of its number (a later decision amends an earlier one where they conflict, and says so). Then this section 8. Then sections 1 to 7, which describe the design as planned and carry **As built** notes and strike-throughs where the build or a decision departed from it. Where section 8 and an earlier section still disagree, section 8 is right and the earlier text is a documentation defect: fix the earlier text, never the unit.

**Superseded text.** A sentence a decision made false is struck through in place with the decision that did it: `~~the old words~~ → D-46`. A longer passage carries a bold bracket instead: **[Superseded → D-32: no confinement was built.]** An **As built (date, commit)** block under a section lists departures only and never rewrites the scope above it. A reader implementing anything reads the As-built block before the scope.

**Code references** are file paths and symbol names (`crates/ledger-core/src/ledger.rs::Ledger::stands_on`), never line numbers. Line numbers that remain in sections 1 to 7 (`S:553`, `ledger.rs:1355`) date from 2026-10-16 and are wrong today; treat them as "search for the symbol".

**Abbreviations.**
- **D-n**: a decision, 10.1 (settled), 10.2 (deferred); **Q-n**: an open question for the owner, 10.3; **A-n**: an owner answer about receivables and payables, 8.3.1.
- **I-n**: an invariant the code enforces, section 2. **B-n**: a rule in `docs/spec/rules.md` (B-1 double entry … B-38 documents; B-39 to B-45 are allocated in 8.0.3). **R-x**: a recipe in 8.0.5.
- **Pn**: a phase; **P4b Sn**: a slice of P4b (S1 to S7); **P2 slice n**: a slice of P2 (1 one executable, 2 the data root, 3 opening and closing the books, 4 access, 5 files in and out, 6 the viewer and `ledger-package`, 7 backups and restore, 8 the viewer's menu and setup, 9 the cutover). **U0a, U0b, U0c**: the pre-step units of 8.2.
- **S:**: `crates/ledger-mcp/src/server.rs` (its line numbers are stale; see above). **T1 / T2**: the local desktop deployment / the self-host deployment (3.8).
- **defect n**: a finding of the 2026-10-16 audit of the first company's books (1 to 20), of P0 (21, 22) or of P3 (23, 24): 1 cash flow (P3), 2 reversals posted without their own evidence (closed by D-12 and D-47), 3 receipts as blobs (P2 transport part; P6 text), 4 duplicate attachments (P6), 5 drafts (P4), 6 the digest (P4), 7 the IIF (P3), 8 tax categories (P3), 9 schedules (P3), 10 annotations (P6), 11 party TIN last-four (P3), 12 the 1099 worksheet (P3), 13 assets' bases (P3), 14 party shares (P3), 15 rename (P3), 16 period defaults (P3), 17 the odd cent (P3), 18 `not_reportable` (P3), 19 `import_local_file` (P0, P2), 20 the migration scripts (retired at the cutover; no tool), 21 the IIF ignores `as_of` (P3), 22 depreciation's final month (P3), 23 a vendor marked "no 1099" (P3), 24 accruals through other liabilities counted twice (P3).
- **GL** general ledger; **TB** trial balance; **A/R, A/P** accounts receivable, payable; **EOM** end of month; **EM** the engineering manager who assigns units and merges; **the first company**, **the owner's books**: the books the plan is proven on, whose figures never appear in this repository (5.6).
- `a | b` in a tool's arguments means exactly one of the two.

**What changed on 2026-10-26.** A nine-stage review (`plan/review-2026-10-26/`) found the plan sound as a design record and not executable as a hand-off. Section 8 was rewritten: 8.0 to 8.4 are new; P4 was rewritten against the P4b slices built after it and D-47 (reversals stay posted); P6, P7, P8, P9, P10, P11 were cut to what the owner's decisions ask for; superseded sentences in sections 1 to 7 and 9 were struck through; the owner's P4b answers were numbered; decisions D-47 to D-54 and questions Q-4 to Q-7 were added. The review's reports list every finding and what was done with it.

---

## 1. Summary

**Today.** Your books are a folder inside your project directory. That folder holds the journal, the receipts, the settings, a list of helper commands and the agent's instructions. The agent opens the books by starting a helper program itself, so it can also read every file in the folder directly. That is how an agent ended up reading the journal file instead of asking the books.

**After this plan.** The books move into a private folder that belongs to a small books server. The server runs as you, only while the app, an agent session or a terminal is using it. Agents are never told where the folder is, and they only talk to the server. In practice:
- You install once, with at most three commands and no administrator password (on Linux, the package manager asks as it always does). After that you just type `claude` in any folder. There is no project folder, there are no helper commands, and there is nothing to do at the start of a session: starting the agent opens the books.
- Claude Code and Codex reach the same books the same way. Later, claude.ai on the web does too.
- When an agent connects, the books tell it the rules, your company's conventions, what needs attention and what to do next. Step-by-step procedures also come from the books: setting up, the month-end close, handing off to the CPA, and the audit.
- Receipts reach the agent as readable text and pictures of the pages, not as giant blobs of data. Files go in and out through short-lived links.
- Social Security numbers and other tax ID numbers are typed and viewed only on a private page that asks for your passphrase. The agent only ever sees the last four digits.
- Backups and a tamper check run on their own.
- The viewer works as it does today. It reads the books directly and never changes them. Within a second it jumps to whatever the agent shows you.
- Posting, closing and reconciling bring up no new permission pop-ups. The agent still shows you each entry before it posts.

**How private is it?** Hidden, not locked. An agent cannot stumble into the books. Because the server runs as you, though, an agent that deliberately went hunting could find them, and the books say this plainly wherever privacy is reported. If you ever want a hard wall, run the same server on another machine you own. This plan includes that recipe, and the same setup is how claude.ai on the web reaches your books.

**Along the way**, the 20 problems found in the 2026-10-16 audit get fixed, and a 21st found while capturing the goldens in P0. The finish line is a full re-audit of the first company, done only through the books' own tools, in which every receipt and every entry is checked and signed off.

---

## 2. Principles: invariants the code enforces

| # | Invariant | Enforced by |
|---|---|---|
| I-1 | **The MCP is the only agent interface to the books.** Locally this is **advisory** (owner decision 2026-10-16): the server runs as the human, and the books are hidden, not locked. Mechanical enforcement is the self-host deployment on another machine, where agents have no access to the data root. | A data root outside every project directory (3.4), mode 0700, created with an explicit `set_permissions` call (not the umask). No tool, instruction, handbook text or result names a filesystem path (I-2, I-12). The stdio data-dir transport is gone (D-44): books are served only by the books server. `books.start`, `wanderbooks doctor` and the docs report `isolation: "same_user (advisory)"`. Landlock and seccomp confine the server itself where available; they do not restrict the agent and are never described as if they did. Test `agent_uid_cannot_read_tenant`, run in CI against the two-user self-host deployment, not the desktop default. |
| I-2 | **No tool takes or returns a server filesystem path.** | Schema lint `no_path_params`: the build fails on any parameter matching `*path*`, `save_to` or `*_file`. Landlock harness `no_tool_touches_outside_tenant`, run with fuzzed arguments. Static check: no `std::fs` in `ledger-mcp/src/tools/**` outside the ledger-store APIs. |
| I-3 | **The journal holds all books state.** Settings, conventions, decisions, draft states, evidence links, annotations, extraction records and attestations are journaled. Reports are pure functions of the journal. `wanderbooks.toml` is import/export text only. The server never reads a toml file. | `ledger-core::settings` replay. Property test `settings_replay_is_pure` (Store, Snapshot and the view are identical). Test `toml_round_trip_zero_events`. The daemon binary contains no toml file-read code path. |
| I-4 | **Money is exact Decimal, every entry balances, and posted entries are permanent.** Corrections are posted reversals; a reversal's evidence is its original's (D-47). ~~Corrections are reversal drafts that need evidence.~~ → D-47 | Existing kernel, Kani and Lean. New `Step.discard` (P4) with its preservation proof. ~~`Step.draftReversal`~~ → D-47 |
| I-5 | **Evidence gate.** One function, `ledger_core::evidence::requirement` (P4), decides whether a draft entry may post. Store, MCP and GUI all call it. Reversals are exempt: their evidence is the original's (D-47, D-12). Depreciation, amortization, closes and the entries the document, payroll, payout and disposal commands post are exempt by origin, and the exemption is sourced. ~~Reversals are not exempt.~~ → D-47 | The gate is called from `Store::post_entry` (P4 step 2). Test `gui_and_server_gate_agree`. Test `a_reversal_needs_no_receipt_of_its_own`. ~~`reversal_needs_evidence`~~ → D-47 |
| I-6 | **A confirmed commit is bound to the server's most recent preview of that exact draft or operation** (a stale or altered preview is refused). It does not prove that a human saw the preview; it detects drift (D-50). | `preview` = SHA-256 of the canonical preview payload (a draft's state and the settings seq; or the tool, its canonical arguments, the settings seq and the journal head seq), returned by every preview and required by every confirm from P4 on; a mismatch is refused with guard `stale_preview`. ~~`preview_token` = HMAC-SHA256 … keyed with a server secret held in `keys/`~~ → D-50 (a same-user key proves nothing; a digest detects the same drift). |
| I-7 | **A TIN passes through the agent only on its way into the vault; no result shows one except the TIN-bearing exports the owner chose (D-28, D-29).** ~~The agent never holds a full TIN or vault key material.~~ → D-28 | The v1 vault (`ledger-vault`, one key in `keys/legacy.key`, carried in every backup: D-31, D-48). A pattern-based TIN scanner (`has_full_tin`, `redact_full_tins`; P7 fixes its separators) fails closed on every result and masks the access log. The canary TIN tests in `crates/ledger-cli/tests/spec_vault.rs`. ~~HPKE, keyslots, human-page handlers, the GUI unlock path~~ → D-48 |
| I-8 | **The GUI never mutates and never uses the MCP.** Locally it reads the data root directly, read-only, as today. It finds the root from the platform's well-known location or from `wanderbooks gui <book>`, never from a project directory or `wanderbooks.toml`. `books.show` reaches it through `ops/show.jsonl` in the book's own data root, never through the MCP. Remote books (self-host) are viewed through a read-only replica stream (P9). | `ledger-gui/tests/read_only.rs`. Tests `gui_works_under_read_only_landlock` and `gui_finds_root_without_project_dir`. Test `replica_snapshot_equals_store_snapshot` (P9). The GUI writes nothing under the data root and leaves no files under `$HOME` except eframe preferences. |
| I-9 | **Every access through the server is logged, and logging fails closed.** | A hash-chained `access.jsonl` (own genesis). Property: exactly one record per tool call, upload, download, human action, remote replica connection and backup, refusals included (as built, slice 7: a backup's own runs are records of `ops/backup.jsonl`; `books.back_up_now`, a tool call, is in the access log). The local viewer's direct reads are not logged, because the GUI never writes; the docs say so. |
| I-10 | **Tamper evidence.** Every mutating result carries `head {seq, hash}`. The head is handed off when a session goes quiet. The customer can clone the archive and run `wanderbooks verify-archive --expect-head`. Backup and access records stay out of the journal, so they never advance the head they report. (As built, slice 7, D-31: each backup commit records its head; a restore's preview proves a clone and names its head, and weekly drills prove each target; no separate `verify-archive`.) | Test `backup_does_not_advance_head`. Test `clone_verify_expect_head` (as built: `a_restore_is_of_the_backup_previewed_only`, the drills). |
| I-11 | **No loss of ease of use.** Local setup is at most 3 commands on a clean machine, with no sudo, no system account and no container, beyond installing the app itself (D-24: on Linux the package manager asks for `sudo` as it does for every package; the dmg, the Windows installer and the AppImage ask for nothing). There is nothing to do per session, it works from any directory, the daily flow adds no client permission prompts (no `requiresUserInteraction`), and the viewer updates within 1 s of a post. | Acceptance criteria in P2 and P5. Tests `setup_needs_no_root` and `annotations_complete`. The agent-eval harness. |
| I-12 | **Guidance survives client truncation, and never names a path.** Connect text is at most 1,400 characters, and its first 512 stand alone and name `books.start`. Tool descriptions are at most 700 characters. No tool text mentions `just`, `wanderbooks.toml`, `~/`, git, GitHub PRs or any filesystem path, the data root included, and no result names one. | Tests `instructions_budget`, `tool_text_lint` and `no_result_names_a_path`. |
| I-13 | **Formal-methods bar.** Kani harnesses cover kernel arithmetic. Every new ledger command is a Lean `Step` with a preservation proof. A failing proof is a finding. All tools are pinned in `mise.toml`. Clippy reports zero warnings. Every mutation survivor is killed or documented as equivalent. | `just check` and `just prove` pass in every phase. |

### 2.1 Conflict resolutions (explicit)

| Question | Proposals | Resolution |
|---|---|---|
| Process shape | A: single process. B: gateway plus per-book worker. C: daemon, with a gateway when hosted. | **One `ledger-mcp serve` process** (A), both locally and on a self-host machine. Locally it runs as the human, only while something the human started holds it open: the app, an agent session, or a terminal. Nothing is installed to run in the background (owner decisions D-21 and D-23, 2026-10-18). Every local program is a mode of the one `wanderbooks` executable (D-22). It has no OAuth authorization server and only a minimal human-page surface. The multi-tenant front/worker split (B) belongs to the hosted service, which this plan does not build (section 11). |
| Isolation, and when it lands | A: P5. B: P6. C: P3. All three assumed a service user. | **Owner decision 2026-10-16: "you, hidden away."** The desktop gets no service user, sudo, system account or container. P2 moves the books into a server-owned data root (0700, outside every project directory) whose path no text ever names. This isolation is advisory and is labelled that way. Mechanical enforcement is the self-host deployment on another machine (P9). From P2 on, CI proves it with `agent_uid_cannot_read_tenant` against a two-user deployment. P2 needs journaled settings first (P1), because the server never reads toml. |
| Settings import | A: server reads legacy toml (`from_legacy:true`). B/C: agent passes text. | **No import tool** (D-44: `import_settings` is gone, owner 2026-10-23). No settings file is read by any server or the viewer: old books bring theirs in, once, by `books adopt`, and `books.set_setting` changes settings after. `books adopt` refuses a journal with no `settings.set` unless `--toml FILE` is given. The human's CLI reads that file and passes the text. |
| Before import | C: writes blocked by a `settings_not_imported` guard. | **Never block.** Daemon books always have journaled settings (enforced by `adopt` and `create`); the `legacy_files` mode of `dev-stdio` is gone (D-44). |
| Preview/post | A: `confirmed`. C: split tools plus `preview_token`. B: seven read-only twins. | **Keep `confirmed`, add `preview` (D-50).** Add one read-only twin, `books.preview_entry`, for the high-frequency post path. Every tool with `confirmed` returns `preview` when called without it and requires it when called with it, from P4 on, in one sweep (P4 step 4). ~~`preview_token` … `confirmed` without a token is accepted for one release~~ → D-50: a digest, no key, no grace period. |
| `requiresUserInteraction` | B: on every post. | **Not used anywhere in this plan** (owner decision 2026-10-16: no added friction). Post, reverse, close, reconcile, dispose, `run_*`, `purge_tin`, `consolidate_duplicates` and (until D-44 removed it) `import_settings` carry `destructiveHint` plus `preview` (D-50), and the client's normal permission rules apply. ~~Full TINs leave the agent's path through human pages: TIN-bearing outputs from P2, and all TIN handling in P7.~~ → D-28, D-29, D-48 |
| Harness deny rules | C: P0 `.claude/settings.json` deny rules. | **Rejected.** That is plan A, which is dropped. P0 corrects only the sentence in the server text and the generated template that caused the `cat` (`/home/swaits/Documents/the first company-Books/AGENTS.md:68-69`, "the journal is the evidence — read it"). This is a content fix, not a guard. |
| Defect 12 rule | A: everything through `financing_accounts` is excluded. | **Classified by instrument (C).** Payment-card and third-party-network payments are excluded (1099-K) and listed. A member's personal cash or check through 2100 stays reportable. An unknown instrument is `undetermined`. |
| Odd cent (17) | A: fair ties. C: rotating tie-break. | **Cumulative apportionment (B)**, extending `kernel::apportion` (kernel.rs:309), its existing Kani proofs, and `Apportion.lean`. |
| Cash flow (1) | A/C: per-entry heuristics. | **Structural partition plus direct-method lines (B).** Footing becomes an identity derived from trial-balance balance. A nonzero difference is an engine error. |
| Vault crypto | A: bespoke X25519/HKDF/XChaCha. | ~~**HPKE, RFC 9180** … The public key's sha256 is journaled.~~ → D-48: the v1 vault stays (XChaCha20-Poly1305 under one key). W-9 files are sealed into the vault and never become blobs (built). |
| Vault unwrap | A: model A first. C: client-side default. | ~~**Model A** … after the human types the passphrase.~~ → D-28, D-29, D-48: no passphrase and no human pages locally; the server opens the vault with the book's key for the tools the owner chose. Client-side unwrap (model B) is a requirement of the hosted service (section 11). |
| Human-session bootstrap | A: `wanderbooks open-link`, which a same-user-ID agent can mint. | **Rejected.** ~~Every `/h/` page needs a factor … That passphrase becomes vault keyslot 1 in P7.~~ → D-28, D-48: no `/h/` pages locally; the owner passphrase (P2) guards the format-upgrade consent only. Remote `/h/` pages are P9c, not planned (D-51). |
| Replica keyring route | A: `/replica/keyring` serves wrapped keyslots to `books:view`. | **Removed.** ~~Local GUI TIN click: … after the human types the passphrase~~ → D-29: the viewer copies a TIN to the clipboard with the book's key, no passphrase (3.6 as built). The replica itself is P9c, not planned (D-51). |
| TIN output scanner | A: match decrypted vault records. | **Pattern-based** (extend `has_full_tin`/`redact_full_tins` behind S:870 to every result, with the vault's last-four set as a hint). Fails closed. Documented as a pattern check. |
| Extraction | In-process (A/B). | ~~**Sandboxed subprocess** `ledger-extract` (C): rlimits, seccomp, no network, Landlock read-only on the input blob.~~ → D-32, D-53: in-process, in `ledger-import` (P6), with `catch_unwind` and size caps; no new crate. |
| Archive vs Landlock **[Superseded → D-31 (no archive process), D-32 (no Landlock, no seccomp); kept for the record]** | A: server shells out to git, which contradicts its own Landlock rules. | **A separate archive process, `ledger-mcp archive`,** that the host starts beside the server, after the server holds every book's lock, and stops with it (3.9); each confines itself on its own. The archive process has read-only Landlock on `books/<id>/{journal.jsonl,blobs}`. Locally, Landlock ABI ≥ 4 limits its TCP connect to the target's port (443 or 22); it cannot filter by host, and an unprivileged process has no host-level egress filter, so that exists only under a system unit on self-host (`IPAddressAllow`). Locally the server listens on Unix sockets (3.9); if the owner turns TCP on, it binds 127.0.0.1 first. Either way Landlock ABI ≥ 4 then denies all further TCP bind and connect, so the local server has no egress. As built (slice 7, D-31): no archive process; the server runs git itself (3.7 as built), so slice 8's Landlock must let it run git and reach the backup folder and repository. On self-host, a system unit adds `IPAddressDeny=any` (loopback allowed); if the built-in authorization server fetches CIMD documents (D-7 a), connect is allowed to port 443 only and `IPAddressDeny` narrows to private and link-local ranges (loopback still allowed), matching the CIMD SSRF guard. This confines the server only; it does not restrict the agent. |
| Attention engine location | A: `ledger-store/attention.rs`. B: wasm-clean `ledger-view`. | ~~**New crate `ledger-view`** … a `BooksView` trait implemented by Store and Snapshot.~~ → D-53: `ledger-package` already holds that trait (`PackageBooks`, over `Store` or `Snapshot`, built in P2 slice 6); Needs attention and `next_steps` move beside it (U0b). A pure-Rust evidence projection is deferred to the hosted service's web viewer (section 11). |
| Inline bytes | A: 256 KiB. B: 1 MiB without sha. C: 256 KiB plus sha. | ~~**256 KiB with a mandatory matching sha256.** A mismatch is refused with guard `base64_corrupt`, whose remedy is `books.begin_upload`.~~ **Superseded by D-46 (2026-10-23): no inline bytes at all, in or out; files come only by upload and go only by download.** `/mcp` body cap is 1 MiB. |
| Same-user mode | C: an announced tier. B: refuse to start without an acknowledgement flag. | **The local default, with no gate flag** (owner decision 2026-10-16). `books.start`, Needs attention, `wanderbooks doctor` and the docs report `isolation: "same_user (advisory)"`. |
| Viewer channel | A: replica over a peer-credential socket. | **Direct read-only disk access locally.** This is the owner's rule: "the GUI has r/o access to ALL state on disk; it never mutates; it doesn't go through the MCP." `books.show` requests reach the GUI through `ops/show.jsonl` in the book's own data root, which the server writes and the GUI watches. Presence is a shared `flock` that the GUI holds on a read-only descriptor of `ops/viewer.lock`. Locally there is no socket, no peer-credential check and no executable check. The replica protocol exists only for viewing remote books (P9). |
| Local token | A: literal header in the Claude config. | **OS keyring, read by the bridge** (D-23): Claude Code and Codex run `wanderbooks mcp --book X` over stdio, and the bridge reads the token itself, so no client config holds it. `headersHelper`/`http_headers_helper` running `wanderbooks token print` remain for clients that choose HTTP. |

---

## 3. Architecture

### 3.1 Processes

From P2 every local program here is a mode of the one `wanderbooks` executable (D-22): `wanderbooks server` is its server mode (the `ledger-mcp` crate), `wanderbooks gui` its viewer (`ledger-gui`), each run as its own process. ~~`ledger-extract` its extractor~~ → D-53. **[The figure and table below are the 2026-10-16 design; superseded parts: Landlock/seccomp and the archive process (D-31, D-32), `/h/*` and `/replica/*` (D-28, D-51), the extractor subprocess (D-53).]** The names below are the crates'. Claude Code and Codex reach a local book through the bridge, `wanderbooks mcp` (stdio, D-23), which speaks MCP to the server over a local socket (3.9).

```
                  agents (any directory, any client)                               human
   Claude Code (any dir)   Codex    claude.ai / ChatGPT    stdio-only       browser         ledger-gui
          |                  |      (P9: self-host or     (wanderbooks         |            (as the human)
          | wanderbooks mcp  | bridge  tunnel)            mcp --url)           | owner           |
          |  (stdio, lease)  |         | OAuth or token        |               | passphrase      |
          v                  v         v                       v               v                 |
   +---------------------------------------------------------------------------------+          |
   | ledger-mcp serve    local: open while held (D-23), as the human                 |          |
   |                     self-host (P9): same binary on another machine              |          |
   |   confines itself where available (Landlock, seccomp, no outbound TCP)          |          |
   |   /b/{book}/mcp   /b/{book}/u/{id} PUT   /b/{book}/d/{id} GET                   |          |
   |   /b/{book}/h/*  (human pages)    /b/{book}/replica/*  (P9, remote books only)  |          |
   +---------------------------------------------------------------------------------+          |
          | sole writer (0700; the path is never named)    ^ read-only Landlock                 | read-only,
          v                                                |                                    | direct
   <data root>/books/<id>/ journal.jsonl blobs/ ops/show.jsonl ... <-----------------------------+
          ^                                                |
          | sandboxed subprocess (rlimits, seccomp, no net) +-- ledger-mcp archive --------> git / s3 / dir / pull
   ledger-extract (pdf / xlsx / eml / page raster)
```

The data root is `$XDG_DATA_HOME/wanderbooks` on Linux (default `~/.local/share/wanderbooks`) and `~/Library/Application Support/wanderbooks` on macOS. It sits outside every project directory, and no tool, instruction, handbook section or result names it. Because the server runs as the human, this isolation is **advisory**: an agent cannot stumble into the books, but an agent that deliberately hunts for them could read them. The server's own Landlock and seccomp confinement limits what a compromised server can do; it protects nothing from the agent and is never described as if it did. Mechanical enforcement is the self-host deployment (3.8, P9).

| Process | Binary | Runs as | Reads tenant disk |
|---|---|---|---|
| Books server (local) | the server mode of `wanderbooks` (the `ledger-mcp` crate), listening on `run/mcp.sock` (and on 127.0.0.1:7411 only if the owner turns TCP on), started by the first opener (the app, an agent's bridge, or `wanderbooks serve`) through the host mode, and running while any opener holds it (D-21, D-23, 3.9; the host reads no books: it starts the server and the archive process, each confining itself, and stops both). TCP is off by default (3.9) | the human | yes, sole writer, holds the flock (`ledger-store/src/store.rs:74-89`) |
| Books server (self-host, P9) | the same binary under a system unit on another machine | a dedicated account on that machine | yes, sole writer |
| Archive pusher | `ledger-mcp archive --book <id>` (as built, slice 7, D-31: none; the server pushes) | the server's account, a separate process started beside the server | read-only journal and blobs |
| ~~Extractor~~ | ~~`ledger-extract` (subprocess)~~ → D-53: extraction runs inside the server (P6) | | |
| Viewer | `wanderbooks gui <book>`, the viewer mode (the `ledger-gui` crate; local, direct, the default; what opening the app shows); `wanderbooks gui --books <url>` (remote replica, P9) | the human | local: yes, read-only, never writes. Remote: none |
| CLI | `wanderbooks` (ledger-cli) | the human | **by design never**. Talks to `run/admin.sock` (peer-credential check: same user ID as the server). The admin socket cannot read books data; it can create or adopt books and mint agent tokens, admin-scope ones included. The peer-credential check keeps other local accounts out; any process running as the human, agents included, can use it (advisory, like the rest of the desktop deployment). |
| Agent bridge | `wanderbooks mcp --book <id>` (stdio to the agent; `--url` for a remote book, P9) | the agent's user | never; it has no data-dir flag. It may start the host (D-23), whose server then reads the disk as itself |

### 3.2 Transport (one code path, local and remote)

Replaces `StreamableHttpServerConfig::default()` in `serve_http` (`ledger-mcp/src/main.rs:154-180`) with an explicit configuration:
- `allowed_hosts` taken from `server.toml` (deployment config only).
- `enforce_origin_validation()` with an explicit list: loopback by default, plus `https://claude.ai` once OAuth exists in P9. Today's empty list turns the Origin check off.
- `json_response=true`.
- `legacy_session_mode=true` while Cursor and claude.ai stay on ≤ 2025-11-25.
- `max_request_body_bytes = 1 MiB`.

Both protocol eras are served: `initialize` and `server/discover` are both built from `get_info`, which calls `refresh` first, with `cacheScope: private` and `ttlMs: 60000`. rmcp features gain `elicitation` and `request-state` (`ledger-mcp/Cargo.toml:22`). Tools read the principal through `Extension<http::request::Parts>`. Long calls (CPA package, audit, consolidation) run under `spawn_blocking`. ~~and return `{job_id}` when they exceed 20 s; `books.job` polls. The 240 s claude.ai limit and Codex's 60 s are therefore never hit.~~ → D-53: no jobs locally; the extraction backfill is a command (P6).

**Concurrency.** One `Store` per book behind `tokio::sync::Mutex` (replacing the std mutex at S:553). Tool bodies use `blocking_lock` inside `spawn_blocking`. Readers use a `Snapshot` refreshed after each write, so reports never wait on the writer.

### 3.3 Auth and scopes

| Scope | Allows | Default holders |
|---|---|---|
| `books:read` | reads, reports, `start`, handbook, `get_receipt` text and images | all |
| `books:write` | drafts, lines, attach, annotate, upload, attest, settings preview | agent |
| `books:post` | confirmed post, close, reconcile, dispose, `run_*`, settings commit | agent (local default) |
| `books:admin` | tokens, access log, backup run, consolidate, invite | owner's agent (local default), owner |
| `books:view` | remote replica only (P9) | viewer client only, never agents |

~~No scope unwraps the vault. Unwrap happens only inside an `/h/` request that has just presented a human factor, or inside the local GUI process after the human types the passphrase (3.6).~~ → D-28, D-29: the server opens the vault for `set_tin`, `set_w9`, `get_w9`, `vendor_1099 include_tins` and `export_cpa_package`, which need books:post or books:admin as the As-built notes say; the viewer reads it with the book's key. *Until P7 moves TINs to the human pages, the three tools that can still put a full TIN out (`vendor_1099` with `include_tins`, `export_cpa_package`, `get_w9`) need books:admin, and sealing one (`set_tin`, `set_w9`) needs books:post (P2 slice 4, review).*

- **Local:** a static bearer `wb_` + 32 random bytes, stored only as a sha256, compared in constant time, kept in the OS keyring, minted by `wanderbooks connect`, and revocable with `wanderbooks token revoke`. Agent and server run as the same user, so the token identifies the client and keeps other local users and web pages out. It is not a wall against processes running as the human.
  - Claude Code: `wanderbooks connect claude-code --book <book>` (also the app's "Connect Claude Code") runs `claude mcp add --scope user wanderbooks-<book> -- <absolute path of wanderbooks> mcp --book <book>`: stdio, through the bridge (D-23). The bridge reads the token from the keyring itself, so Claude's config holds no token and no helper. It offers a `permissions.allow mcp__wanderbooks-<book>` line.
  - Codex: `wanderbooks connect codex --book <book>` writes:

    ```toml
    [mcp_servers.wanderbooks-acme]
    command = "<absolute path of wanderbooks>"
    args = ["mcp", "--book", "acme"]
    ```

    The bridge answers `initialize` within Codex's startup timeout, opening the books first if they are closed. A client that needs HTTP can be pointed at `http://127.0.0.1:7411/b/<book>/mcp` with `wanderbooks token print` as its header helper once the owner turns TCP on (3.9); it holds no lease and needs the books already open. No `default_tools_approval_mode` is written. Codex documents `writes` as prompting for every tool not marked read-only, which would prompt on every post, close and reconcile (against owner decision 3); leaving the key unset keeps Codex's normal rules.
- **Self-host, remote (P9a):** the same static bearer over TLS for clients that send headers (Claude Code, Codex). **[P9b, after Q-6 (D-51): claude.ai and ChatGPT need OAuth 2.1; the recommendation is an external identity provider (D-7 b), so the list below is the design only if the owner chooses D-7 a.]**
  - RFC 9728 PRM per book; 401 with `resource_metadata`.
  - 403 `insufficient_scope` for step-up.
  - RFC 8707 audience equal to the book URI.
  - PKCE S256, RFC 9207 `iss`, CIMD preferred (SSRF-guarded), DCR fallback.
  - Exact redirect match (`https://claude.ai/api/mcp/auth_callback`, loopback on any port).
  - `offline_access` only in the authorization server metadata.
  - 1 h access tokens with rotating refresh tokens.
  - Signing keys held through systemd-creds.
  - The self-hosted server is its own issuer (D-7 a), or it validates an external IdP's tokens (D-7 b). Codex uses `codex mcp login`.

**Human factor for `/h/`:** **[Superseded → D-28 (no local `/h/` pages), D-51 (remote `/h/` pages not planned). The owner passphrase (Argon2id, as built below) guards the format-upgrade consent only.]** ~~passkey with user verification … the session's `sub` equal the token's `sub`.~~

**As built (P2 slice 4), and where it departs:**
- **Tokens** are `wb_` and 64 hex digits (32 random bytes), one book and one name each (`agent` unless named), kept only as a SHA-256 in `auth/principals.jsonl`: a chain of `token.created` (with the token it replaces) and `token.revoked` records, each with its own hash besides the chain's (so a change to the last record is found too), replayed at start; one that can't be read refuses every agent with why (the list of books still answers). Checked in constant time against every live token. Made, listed, checked and revoked only through the admin socket: `wanderbooks token create --book ID [--name N] [--scopes read,write,post,admin] [--replace] | list | revoke | check | print`. A token made is kept in the keyring, and read back from it as kept, before anything is said of it, or revoked again; `print` is the only way its text is shown. `check` says whether the token kept here is the books' live one (exit 0), an old one, one the books no longer have, or none, asking the server by its SHA-256 alone (admin op `token_check`). `connect` (slice 8) will mint through the same path.
- **Kept in the OS keyring** (Secret Service on Linux, through keyring-core; macOS and Windows stores come with P11), under service `wanderbooks` and an account naming the root by a hash of its path, the book and the token's name. One account is one item (2026-10-22, after a bridge on the live books ran on a token the keyring no longer gave while `download` read a revoked one): keeping a token first removes every item for its account in every collection, then makes one and reads it back; reading finds the items in every collection and, finding several, refuses with how to keep one again (`token create --replace`) instead of giving one of them; a keyring that is locked or can't be reached is an error, never "no token". The bridge (`wanderbooks mcp --book ID [--token NAME]`) reads it before each new session (even with a token held: the keyring's is used, and a change is said on stderr; a slow keyring leaves the held one in use after 2 s) and each request until it has one, and again when the door refuses it, so a token made or replaced mid-session is used from the next call (with a new session: a session belongs to the token that opened it); without one, the server's refusal says how to make one, but when the keyring couldn't be read the agent is told that instead (locked or unreachable, the token itself fine; or several items), never to make a token. Tests keep tokens in files under the root's `run/` (the `file_keyring` knob): a folder per collection, which a test can lock, make unreachable, or plant a second item in.
- **The door** (`/b/{id}/mcp`): no token or an unknown one, 401 with `WWW-Authenticate: Bearer` (the scheme in any case); another book's, 403; a session is bound to its token when opened and used only with it (403), forgotten when it ends, is found gone (an idle one, when next used), or its token is revoked or replaced; an id the server never opened binds nothing; past 100,000 bindings an initialize is refused, never a binding dropped. Each refusal at the door is recorded in that book's access log (action `connect`).
- **Scopes** (`ledger-mcp/src/scopes.rs`): a table of every tool. Read-only tools need books:read; a tool with `confirmed` needs books:write to preview and books:post to act; tools that post at once, settings commits, voids, renumbering, corrections and role changes need books:post, and so do the controls on posting (setting an approval rule, retiring a convention, approving an entry) and sealing a TIN (`set_tin`, `set_w9`, which can replace one); `books.access_log` and the tools that can still put a full TIN out (`vendor_1099`, `export_cpa_package`, `get_w9`) need books:admin, and so do backing up now and choosing where backups go (a `backup.*` setting, by `set_setting` or an imported `[backup]` table; slice 7): a backup is the whole book. A token is offered only the tools it can call (the list is then cached per caller, `cacheScope: private`); a call beyond its scopes is refused, unrun, naming the scope. Reading a resource needs books:read. A local token gets all four unless told otherwise.
- **The access log** (`books/<id>/access.jsonl`, own genesis) records every tool call and resource read, refusals included: who (token id, name), the client as it named itself (cut to 64 characters, any full TIN masked), the request id, the tool or resource, a SHA-256 of the arguments' canonical JSON with what may be a TIN masked first (a value under a key naming one, any key or text holding one in a shape the TIN scanner knows, any number of eight digits or more: an unsalted digest of a TIN whose last four the books show is broken by 10^5 guesses; a file's base64 is hashed as it is, never spelling a TIN in the clear), what came of it (`ok`, `error`, or `refused` with a code), and the journal's head after the call's own last write, if it wrote, whatever it answered (taken under the store's lock as the call lets go). A call's record is written on its own blocking thread while it still counts as running (one that panics part way included, with what it wrote), so a close waits for it too, and before the answer goes; one that can't be written stops the book (no more requests until a restart) and the answer says the call ran (a call refused as not run, by a close or a stopped server, keeps its "not run" code). A tool no table names is refused unrun. Known gap: a crash of the process between a call's journal write and its record leaves the journal ahead of the log for that call. A damaged log, or one holding a record that isn't an access, keeps its book closed, with why. `books.access_log` (admin) reads it newest first, filtered by token, action or outcome, paged with `before`.
- **The owner passphrase** is an Argon2id hash (m=64 MiB, t=3, p=1, its own salt) in `auth/owner.json`, replaced whole; 12 to 1024 characters, no other rule (NIST SP 800-63B); a check refuses a hash weaker than that (changed by hand). `setup` at a terminal asks for it (twice); elsewhere it says to run `wanderbooks passphrase`, which works only at a terminal, unechoed, and asks for the one in force before a change. (Advisory, like everything local: a process running as the human can drive a pseudo-terminal and set the first one.) A stored hash of another Argon2 version, with a short salt or output, or asking a check for more than 1 GiB, 16 passes or 8 lanes is refused as changed by hand. The command line wipes token texts and passphrases from memory once used (the server and the bridge, which hold tokens for their lifetime, don't). `doctor` says whether it is set and lists the tokens. Its uses (`/h/` pages, consents) come with the slices that build them.
- **Known, product-wide (P7, 8.2):** the TIN scanner (`has_full_tin`) takes hyphens and spaces as separators, not commas or parentheses, so "123,45,6789" passes it in party fields and the access-log digest alike.
- **Departures:** rate limits (tower-governor) wait for P9: on a socket only this account can reach, nothing could be limited that couldn't read the files instead. Principals are per book, not lists of books. `initialize` (the connect text, composed from the book's settings) is given to any valid token and isn't recorded. Changing the token record holds its lock across one fsync, briefly delaying token checks.

### 3.4 Data layout (server-owned data root, 0700)

```
<root>/                         Linux: $XDG_DATA_HOME/wanderbooks (default ~/.local/share/wanderbooks)
                                macOS: ~/Library/Application Support/wanderbooks
                                self-host (P9): /srv/wanderbooks
  server.toml                   deployment only: listen, public_url, allowed_hosts/origins, archive targets, issuer
  auth/principals.jsonl         principals, sha256(token), scopes, labels; passkeys (hash-chained)
  run/admin.sock
  legacy/<id>-<date>/           old copies moved here at adoption if the owner agrees (Q-3); the server never deletes them
  books/<id>/                   == today's data dir layout, unchanged (Store/Snapshot/verify_and_heal untouched)
    journal.jsonl  blobs/aa/bb/<sha>  wanderbooks.v<N>.db(+wal/shm, one per schema; P3)  wanderbooks.lock
    vault/{tin,w9}-<party>.bin  sealed with the book's v1 key (D-48: no v2)
    keys/legacy.key             the book's vault key (D-31: travels in every backup; D-48: stays)
    (keys/keyring.json, keys/preview.key: not built → D-48, D-50)
    access.jsonl                hash-chained access log, own genesis "wanderbooks-access-v1"
    ops/backup.jsonl ops/heads.jsonl   not journaled
    ops/show.jsonl              books.show requests: the server appends (bounded, rotated), the GUI watches; not journaled
    ops/viewer.lock             created by the server; the GUI holds a shared flock on a read-only descriptor (presence)
    uploads/<id>.part           staging, swept after 15 min (a backup being restored is cloned into books/.restore-* instead: 3.7 as built)
    derived/<sha>/page-N.png    disposable page-raster cache (P6)
    archive.git/                bare; journal.jsonl + blobs/ + vault/ + keys/ + access.jsonl, as they are (D-31, not encrypted); destinations D-30
```

The server, the CLI and the GUI find the data root with one shared function (`ledger_store::root::Root::find`, built in P2 slice 2): `--root` if given, else the root `wanderbooks setup` recorded in `wanderbooks.location` beside the default, else the platform default under the account's home, taken from the password database on Unix, never `$HOME`. Finding it never reads the environment, the working directory, a project directory or a toml file; `$XDG_DATA_HOME` is honoured once, by `setup`, which records the result. The root and every folder made in it are created with an explicit 0700, not the umask, and a root that is a symlink, belongs to another user or that others can enter is refused (`setup` tightens one of ours). `setup` leaves a `.wanderbooks-root` marker, and a folder without one is never opened as a root nor, if it holds anything, made into one; every folder above the root must be one no other user can swap it in (owned by this account or the system, not writable by others unless sticky). Run again, `setup` keeps the root it recorded, and it never makes a second root or moves books while the current one holds any. Neither `setup` nor any refusal prints the root's location: nothing an agent is shown names it. Books are created and adopted one at a time, under `run/books.lock`. Windows (P11): `%LOCALAPPDATA%` from the known-folder API and an owner-only DACL; until then the Windows paths are read from the profile and carry no ACL. Extracted text is stored as a content-addressed blob, recorded in the journal by `attachment.text_extracted`, and archived with the books. Page rasters are disposable. Deleted assumptions: `books_dir = data_dir.parent()` (S:1155, `ledger-gui/src/books.rs:1474`, `ledger-mcp/src/main.rs:102`), the `.gitignore` check (S:531-543), and the `$XDG_RUNTIME_DIR` viewer file (`ledger-store/src/viewer.rs:98-130`), which `ops/show.jsonl` replaces.

### 3.5 Keys

| Key | Location | Usable by |
|---|---|---|
| Vault key (v1) | `keys/legacy.key` (0600), in every backup (D-31) | the server (sealing and the TIN-bearing exports the owner chose, D-29) and the viewer (clipboard copy) |
| ~~Vault public key / private key (keyslots)~~ | → D-48: not built | |
| ~~`preview.key`~~ | → D-50: not built | |
| Agent tokens | sha256 in `auth/`; plaintext in the user's keyring, which any process running as that user can read (advisory locally) | the agent's user |
| Archive credentials | OS keyring entry read by the archive process (local); systemd-creds for the archive unit (self-host). As built (slice 7): the owner's own ssh agent and git credential helpers, through the server | archive pusher |
| At-rest encryption | full-disk encryption (desktop); encrypted volume (self-host recipe) | protects against stolen disks |

### 3.6 GUI

- **Local: direct and read-only** (desktop default). This is the owner's rule: "the GUI has r/o access to ALL state on disk; it never mutates; it doesn't go through the MCP."
  - `wanderbooks gui <book>` runs `ledger-gui --root <root> --book <id>`. Without `--root`, the GUI resolves the platform data root itself (3.4). It never looks in a project directory or `wanderbooks.toml`.
  - It reads `journal.jsonl` and blobs into a `Snapshot` as today, tailing the journal so that a post appears within 1 s. Every open is read-only, and the GUI runs correctly with the data root under read-only Landlock (test).
  - **`books.show`:** the server appends `{seq, ts, target}` to `ops/show.jsonl` (bounded, rotated, not journaled). The GUI watches it with the same watcher it uses for the journal. This replaces `viewer::request` and the `$XDG_RUNTIME_DIR` viewer file (`ledger-store/src/viewer.rs`). A show request only navigates, so a stray writer can do no more than move the view.
  - **Presence:** the GUI holds a shared `flock` on `ops/viewer.lock`, opened read-only. The server probes the lock to report `viewer.open` and whether a show request was delivered.
  - Locally there is no socket, no peer-credential check and no executable check.
- ~~**Remote: replica**~~ **[P9c, not planned → D-51; the design is kept for the record]** (self-host books only), `ledger-gui --books <url>` with the static token or OAuth, and `books:view`:
  - It pulls `/replica/journal?after_seq=N` (SSE of raw canonical lines) into `Snapshot::from_lines(LineSource, BlobSource)`. This reuses the parse, chain-check and replay loop of `snapshot.rs:137-183`.
  - Blobs are fetched on demand, hash-verified, and kept in a 512 MiB in-memory LRU, never on disk. "Not fetched" is a separate state from "damaged" (fixes `books.rs:456-478`, `attention.rs:551-568`).
  - `/replica/events` carries `books.show` targets per principal, and presence.
  - The replica carries no keyslot material.
- **Settings come from the journal.** Deleted: `data.rs:76-87,157-162,188-195`, `books.rs:536-542`, `main.rs:47-53`, `--books-toml`.
- **Needs attention** comes from one function in `ledger-package` over `PackageBooks` (U0b; ~~`ledger_view::attention::find`~~ → D-53), so it is the same list as `books.start`.
- **Save a copy / Open with system app** on any document (`Snapshot::attachment_bytes`, snapshot.rs:328). Fixes the viewer half of defect 3.
- **TIN click:** ~~locally, the GUI prompts for the passphrase, reads the wrapped keyslot … then zeroizes.~~ → D-29 (as built below): a click copies the TIN to the clipboard with the book's key, no passphrase. This is not access-logged, because the GUI never writes; the docs say so. ~~Remotely, TINs appear only on `/h/` pages.~~ → D-51
- `[viewer] open_at_start` becomes a per-human eframe preference (`app.rs:833`), not journaled.
- **Color schemes** (owner, 2026-10-19: "full blown colorscheme support … all the top neovim style colorschemes", "on the order of 20 families, each with a flyout of all its variants", a light/dark/all filter; Ayu included). The button in the sidebar's lower corner opens a menu: the filter, then, alphabetically and all on screen at once, the viewer's own family (Wanderbooks Light and Dark) and twenty families of editor schemes (Ayu, Catppuccin, Cyberdream, Dracula, Everforest, GitHub, Gruvbox, Kanagawa, Material, Melange, Monokai Pro, Night Owl, Nightfox, Nord, One Dark, Oxocarbon, Rosé Pine, Solarized, Sonokai, Tokyo Night), each a flyout of all its variants, alphabetical, each a row with a strip of its colors that chooses it; a row rested on for 50 ms is worn live until the pointer moves on (owner, 2026-10-19), and only a click keeps it. The choice and the filter are per-human eframe preferences like `open_at_start`; nothing is journaled and no figure changes (`schemes.rs` paints nine pages in all 80 schemes and checks every painted word and figure is the same, in the same place, and readable on what is under it). Every palette keeps every text color (figures, gains and losses, warnings, links, the faintest text) at WCAG AA contrast (4.5:1) on every background it is drawn on: a scheme's own color that falls short keeps its hue and only changes lightness, and gains, losses and text stay plainly different to the eye. The scheme pins egui's light or dark theme, so the system flipping changes nothing. The palettes come from each scheme's official source (`docs/color-schemes.md`).

**As built (P2 slice 6), and where it departs:**
- **Finding the books.** `wanderbooks gui [BOOK]` (`ledger-gui --root PATH --book ID`): the data root from `--root`, else the one `setup` recorded (`Root::find`), never the working directory or a toml; the book named, else the root's only one (or the first by id). `--data-dir` still opened an older books dir's `books-data` (tests, and the first company's books until the cutover), and `--books-toml` its settings file, until D-44 removed both (owner, 2026-10-23): the viewer opens only books of the data root.
- **Settings come from the journal.** A book of the data root never reads `wanderbooks.toml` or `config.json`: books that keep no settings show the defaults. An older books dir whose journal kept none was read from its `wanderbooks.toml` and `config.json`, followed live, until D-44 removed that mode: the viewer reads every book's settings from its journal alone. The demo company journals its settings.
- **The vault.** For a book in the data root (`inside_a_root`), its own `vault/` and `keys/legacy.key`, whatever a setting says; for an older books dir, as before (`--vault-key-file`, the setting, the default), until D-44: now every book's vault is its own. No passphrase: TINs get no handling beyond the vault (D-29); a click copies one to the clipboard as before.
- **`books.show` and presence** (`ledger_store::ops`, which replaces `viewer.rs`): the server makes the book's `ops/` (0700) and `ops/viewer.lock` (0600) when it opens the books; a show request is a line of `ops/show.jsonl` (`{seq, at, target}`, at most 32, the file replaced whole); the viewer watches `ops/` once it exists and holds a shared `flock` on `ops/viewer.lock` through a read-only descriptor while it runs, let go of when it closes; `books.show` answers `viewer_open` and says so. `ops/` is not the books: the read-only tool check leaves it out.
- **Save a copy / Open** on every document, **Save the W-9** on a party whose W-9 is in the vault, and the **Export** menu (the CPA package as of today, the 1099 worksheet for this year or last, full TINs included): each a new file in the Downloads folder (`ledger_store::saving`: never over a file, written aside and linked in; the viewer's report PDFs now save the same way instead of overwriting), what came of it shown beside the button. The menu's files are built off the UI thread from a fresh read-only snapshot; a document's copy and a W-9 (one stored file each, hash-checked as read) are saved at the click. "Open" is a copy in Downloads opened with the system's app (the one folder the viewer may write once confined, slice 8).
- **One builder** (`ledger-package`): the CPA package, the 1099 worksheet and the report helpers the server's tools share moved out of `server.rs` into a crate both the server and the viewer use, over `Store` or `Snapshot` alike (`PackageBooks`), the TINs passed in. Proven unchanged: 91 tool responses, 19 packages and 366 entries byte-identical before and after (only the XLSX and zip timestamps, which differ between any two runs, aside). The package now names files of the same name `name (2).ext` instead of failing, its README names only what it holds, and a K-1 worksheet whose rows don't match the members is left out with the reason.
- **The agent's side (D-29):** see 4.7's slice 5 list; `export_cpa_package`, `vendor_1099` (`csv: true`) and `get_w9` hand their files over as exports (`GET /b/{book}/x/{export}`, `wanderbooks download ex_…`), never inline (D-44); no tool takes a path.
- **Not in this slice:** the menu's owner intents ("Close the books", "Back up now", "Restore…", "New books…", "Connect Claude Code / Codex") need the viewer's helper and the lease socket, and come with backups (slice 7) and setup (slice 8); so does running the viewer under read-only Landlock and Open-with through its helper.

### 3.7 Backup and tamper evidence

**[The three paragraphs below are the 2026-10-16 design; the As-built notes (D-30, D-31) are what exists: no archive process, no encryption, no `fetch`, no `verify-archive`, no `ops/heads.jsonl`.]** The archive process (`ledger-mcp archive`: locally started by the host after the server, and stopped with it, 3.9; a system unit on self-host) runs on a head-advance signal (debounced 30 s), on `period.closed`, on a timer while it runs, and once when it starts, for anything left unpushed. The books change only while they are open, so a backup is never missed while they are closed. "Back up now" in the viewer is the lease socket's `back_up_now` intent (3.9), opening the books first if they are closed. Each run:
1. Verifies the chain.
2. Commits the new journal lines and blobs to `archive.git` (one commit per push; the message is the seq range plus the head).
3. Pushes to the targets: git remote, S3, dir, or `pull` (the customer runs `wanderbooks fetch`).

It writes `ops/backup.jsonl`. `books.backup {status|run}` reports it, and Needs attention flags a target more than 24 h behind. Head handoff:
- every mutating result carries the head;
- `ops/heads.jsonl` records each 10-minute quiet period;
- the customer clones the archive and checks it (as built: a restore's preview, or the drills; no `verify-archive`).

Email or webhook head delivery and a witness log belong to the hosted service (section 11).

**Restore (owner, 2026-10-17: "these backups need to be restorable, both in existing and new installations").** A backup is complete: the journal, every blob, the vault records (sealed), and the head `{seq, hash}` at backup time. `wanderbooks books restore <backup> [--id X]` (and the viewer's "Restore…", which hands it to the server) works the same on a clean install and on an existing one. The server's Landlock keeps it inside the data root, so it never opens the backup by path: the CLI, or the viewer through its helper, stages the backup into the data root's `uploads/` (re-hashed on arrival), and the `restore` intent carries only that staged id.
1. **Proven before use.** The server replays the whole chain, re-hashes every blob and checks that the head equals the one the backup records. Any mismatch refuses the restore with a plain reason; nothing is half-restored.
2. **Never destructive.** On an existing install a restore arrives as its own book beside the current ones. Replacing a book moves the old one into `legacy/<id>-<date>/`, kept, never deleted, and only on the owner's explicit choice.
3. **The vault comes back only with its key.** The backup carries the vault sealed; the passphrase or the printed recovery key (P7) unlocks it on any machine. Until P7 the v1 key is a separate file: a backup counts as complete only when that key is saved too, and Needs attention says so when it isn't.
4. **Restore drills.** On a schedule (default weekly, or at the next start if the books were closed when it fell due) and after each close, the archive process restores the latest backup into a scratch folder, verifies it as in step 1, compares its reports with the live book's as of the backup head, and throws it away. A failed drill is a Needs-attention item; `books.backup status` reports the last drill.

**As built (P2 slice 7), and where it departs (D-30, D-31):**
- **No archive process, no encryption.** D-31 made a backup the book as it is, in git, so there is nothing to encrypt and nothing to segment: the server itself backs each book up (`ledger_store::backup`, `ledger-mcp backup.rs`). Each book keeps a bare `archive.git` beside its journal; a backup is one commit of its journal, `blobs/`, `vault/`, `keys/` and `access.jsonl` exactly as they are, made under the store's lock (so whole lines and a vault that matches), its message `seq N head H` with `book <id>` below, then pushed outside the lock, fast-forward only (a target that moved on is refused and said, never forced), to `backup.git_remote` and to `backup.folder/<book>.git` (made bare on first use; a folder that isn't there, a disk not connected, is said plainly). git runs with no system config, no hooks (in a repository on this computer it pushes to too, through `--receive-pack`), no prompts, signing, ignore or attributes files or file watchers, its own identity, only the file, ssh and https protocols (never `ext::` or `fd::`, whatever the owner's own configuration says), and a limit of five minutes without a word (pushes and clones report progress; a git stopped is stopped with all it started); the owner's own `~/.gitconfig` still applies otherwise, for credential helpers. The stored files are staged outside the store's lock (they never change; a failure there is only a lost head start) and the commit, under it, is checked to hold every file there is, by name; bytes are kept as they are because no attributes rule of the owner's, the system's or the archive's is read, and a `.gitattributes` in a book's folder refuses the backup. A repository on this computer (a folder target, or an address that is a path) is pushed to or read only when plain: a bare repository at that very path (never one git would find beside or inside it), nothing in it sending git to another repository (a `.git`, `commondir` or `gitdir` file, objects borrowed from elsewhere, a link where it or its own parts are, a `<path>.git` or `<path>.bundle` beside it), its configuration only what `git init --bare` writes (and, for a copy made with `git clone --bare`, where it came from), since a setting of its own could run a command on a push; repositories are made with the files ref format and SHA-1 whatever the owner's own defaults; one is read as a far one is (`--no-local`, so its copying reports progress). A folder target's repository is made only while the target never took a backup: one gone since is a disk not connected (a mount point with nothing on it), said, never made again where the disk should be. A lock a stopped git left in the archive is cleared once older than any git may run quiet. The host now passes the owner's `SSH_AUTH_SOCK` (only a socket the account owns, not a link to one) so a private repository over ssh is reachable. `backup.folder` must be absolute and `backup.git_remote` can't read as an option; setting either (or importing a `[backup]` table) needs books:admin, as backing up does (a backup is the whole book, vault and key included).
- **When.** When the books open; a while after they were last written (`--backup-quiet`, 30 s, the `backup_quiet` knob for tests); before they close (at most 60 s); and on `books.back_up_now`. A failed commit holds every run off for 15 minutes; a target whose push failed is left out of the backups after writes for 15 minutes and then tried again, written to or not, while the others carry on. `period.closed` needs no trigger of its own: a close is a write. No head handoff files (`ops/heads.jsonl`) yet: the backup's commits are the head record.
- **The record and the tools.** Every commit, push and drill is a record of `ops/backup.jsonl` (a chain log; a backup never moves the journal's head), each target's keyed by a digest of where it is (a new folder or address starts with no standing). Stale means behind, with the first try to back up what it lacks more than a day old (not a day since the last good push: quiet books aren't stale the moment they're written); a failed commit is every target's error until a push succeeds. The chain isn't verified at each run: the server verifies it when it opens the books, and drills prove what reached each target. Two tools, not `{status|run}`: `books.backup` (books:read) for the standing of each target (last good backup, through which seq, stale after a day behind, the last error, the last drill and why it failed) and `books.back_up_now` (books:admin, the one open-world tool). `books.start` adds a step when none is set up, or one is stale, failing or failed its drill. No error names a path in the data root.
- **Drills** run weekly per target that holds a backup, and at the next look after a failed one once 15 minutes have passed (looked for a minute after the books are first seen open, so also at the first start after one fell due, then hourly; `--drill-first`, the `drill_first` knob, for tests): clone, check the tree is plain files under the backed-up paths before any is written out (a link or submodule could otherwise point the checkout, or the store opened on it, at the live books), replay the chain, re-hash every stored file, check every TIN record the books name is there with its key, check the head against the commit's, and check the journal is a byte prefix of the live one (the reports then follow from the same journal, so no report comparison). Not after each close: weekly is the floor and a close changes nothing a drill would see.
- **Restore** is `wanderbooks books restore BACKUP [--id ID] [--replace] [--yes]` (without `--id`, the id of the books the backup holds, as its preview names them, else the repository's name), the admin socket's `restore`, previewed then confirmed against the previewed head. The server clones the backup into `books/.restore-*` (the address or folder crosses the owner-only admin socket as an adoption's path does; no staging through `uploads/`), proves it as a drill does (but for the live-journal check), one restore at a time, builds the book as an adoption builds (every stored file re-hashed, opened, verified, synced; the vault and its key come with it, D-31), gives it the backup's history as its own `archive.git` (so its next backups fast-forward), and renames it into place. Over existing books it is refused unless `--replace`: then the server lets go of the open books (their writer lock released; a session still holding them is refused, its client's next session sees the restored books), waits for their last backup run and starts no other until the restore is done (one in flight could otherwise push past the backup being restored), and moves them whole into `legacy/<id>-<time>/`; a restore that fails puts them back and opens them again. `books.purge_tin` now says backups made before keep their copies (D-29, D-31). Each backup commit names its books (`book <id>` in its message), so a restore knows whose books it brings back (and the command line, given no `--id`, restores them under that id, deciding after one preview, which only reports; a restore leaves `ops/restored-from` in the books, noted by the server when it opens them, so the note lands with the books): the same books count their targets' backups as their own (a backup folder gone since is a disk not connected, never made again); books restored beside the books they were, under another id, never push to those books' repository (it would stop their backups): it is paused for them, said by `books.backup`, until another is named. A damaged `ops/backup.jsonl` is set aside and begun again, never keeping the books shut, what it knew of whose backups the targets hold carried over; a backup folder that took the books' backups and holds none now says so (a disk not connected, or a new one: `books.back_up_now {new_disk: true}` begins it again, only when the human says so). One book's drill holds the others' backups until it ends; a backup at close waits at most a minute for it. No passphrase for a replacing restore (D-31: the old books are kept). Viewer and helper: "Restore…" and "Back up now" come with the menu intents (slice 8); the lease socket's `restore` intent now names the backup and the id, and both intents point to the terminal and the agent until then.
- **Tests:** `crates/ledger-cli/tests/backups.rs` (both targets after a write; a write not backed up before the quiet; close, and catching up at the next open; admin only and the head unmoved; a target that moved on; restore beside, on a clean install, of the previewed backup only, with its vault and key, refusing five kinds of damage and writing nothing, never overwriting and keeping the replaced books, a failed replace leaving the books open; a drill on open; a failed drill flagged), `open_while_held.rs` (`books restore` from the command line), `ledger-store` (backup unit tests: links and strangers refused before checkout, target hooks never run, ignore rules never leave a file out, stale locks, drills; `root.rs`: never moving books something holds), the knobs, the host's environment, the scopes, the settings' checks, and the loop's and the standing's unit tests. The suite passed nine of nine runs three at a time (the replace race above showed only under load).

### 3.8 Local vs self-host

| | Local (T1, the default) | Self-host (T2, P9) |
|---|---|---|
| Where | the human's computer | another machine the owner controls: home server, NAS or VPS |
| Processes | one server (as built, slice 7: which backs the books up itself), open while the app, an agent session or a terminal holds them (D-23) | the same binaries under system units, plus TLS (Caddy) and, optionally, `wanderbooks tunnel` |
| Runs as | the human | a dedicated account; agents have no account on that machine |
| Isolation | `same_user (advisory)`: hidden, not locked | `service_user (enforced)`: machine and user-ID boundary |
| Auth | static bearer in the keyring (no `/h/` pages: D-28) | static bearer over TLS for header-capable clients (P9a); OAuth 2.1 through an external identity provider for claude.ai (P9b, D-7 b recommended, Q-6) |
| Vault | the book's v1 key, no passphrase (D-29, D-48) | the same |
| Viewer | reads the data root directly, read-only | ~~replica over `books:view`~~ → D-51: a backup clone opened read-only |
| Residual risk | a process running as the human (for example an agent that deliberately goes hunting) can read the data root, the agent token and the vault key (D-29: TINs get no handling beyond the vault). ~~The server's Landlock and seccomp confine only the server~~ → D-32 | whoever administers that machine |

A multi-tenant hosted service is out of scope for this plan; see section 11.

---

### 3.9 How the books open and close (D-23)

The books are open while something holds them; there is no background service. This section is the mechanism; P2 builds it, P11 adds updates.

**Who holds them.** The app (the viewer), an agent session's bridge (`wanderbooks mcp`), a terminal running `wanderbooks serve`, and any CLI command that needs the server. Each holds a **lease**.

**Three local channels, all in `run/` under the data root** (0700 on Linux and macOS; on Windows `%LOCALAPPDATA%\wanderbooks`, never the roaming profile, with an owner-only DACL; named pipes with an owner-only DACL, `FILE_FLAG_FIRST_PIPE_INSTANCE`, and a check of the client's SID):
- `run/lease.sock`: served by the **host**, leases only. Each lease records its kind (app, bridge, serve, cli) and PID; the app and `wanderbooks doctor` list them, so a stray holder keeping the books open can be seen and closed. Besides holding, the socket accepts exactly five **owner intents**, which is how the viewer hands work to the server: `close`, `back_up_now`, `new_books`, `restore` (a backup, as its own book beside the others), and `consent_format_upgrade` (a book). `consent_format_upgrade` carries the owner passphrase typed in the viewer (a restore that replaces a book no longer does, D-31: the books it replaces are kept), checked by the server (Argon2id, as on `/h/`); no MCP tool can give either, so an agent never consents. Nothing else is accepted.
- `run/mcp.sock`: served by the **server**, MCP for the bridge. Local agents never use TCP, so no other account on the machine can squat a port and catch the token or the traffic. Both ends check the peer's user ID (`SO_PEERCRED`/`getpeereid`, the pipe's SID). TCP is off by default. The owner can turn it on (`server.toml`, or `wanderbooks serve --http`) for a client that needs HTTP; it then binds 127.0.0.1:7411, a failed bind is reported by `doctor` and is not fatal, and such clients hold no lease, so the books must already be open.
- `run/admin.sock`: served by the server, admin only (create, adopt, tokens), as in 3.1. Neither the viewer nor the bridge opens it.

**The server's process (P2 slice 3).** The server is the hidden mode `wanderbooks server`, started by the host with a pipe on its standard input; it runs until that input ends (the host let go of it, or died) or it gets SIGTERM, SIGINT or SIGHUP (watched from before the books are opened). It takes `run/server.lock` before anything else (a second server waits for it, touching nothing, and leaves if told to close first), opens and verifies every book (a book that won't open is listed with why, and the others are served), binds both sockets, and only then prints `ready` on its standard output. `run/mcp.sock` speaks Streamable HTTP over the Unix socket, `/b/{id}/mcp` per book, so local and remote share one code path (3.2); `run/admin.sock` takes one JSON line and answers one (`list`, `create`, `adopt`). A socket path longer than the system allows is reached through `/proc/self/fd` on Linux. Closing: new requests are refused, unrun (HTTP 503 at the door, or, for a call already past it, JSON-RPC error -32050 "closing: this call wasn't run"; the bridge sends either again once the books are back); every request already running finishes its work, however long, whether or not its client is still there; answers on their way out get 10 s more (a second signal skips that wait, never the first); then the sessions end and the sockets go. A request is at most 1 MiB (64 MiB until slice 5 gave files their own route). A book in the root keeps its vault in its own `vault/` and its v1 key in its own `keys/legacy.key` (0700 and 0600, made private whenever first written), and no tool writes a file anywhere in the root (the server keeps uploads on their way in in the book's own `uploads/`, slice 5). Adoption is a preview, then a confirm tied to the head the preview read; entries of a settings file the books don't know are refused unless skipped, its comments are shown, the vault key it names must be the one given, and the key must open every vault record. Open for slice 8: under Landlock the server can't read an adoption's source outside the root, so adoption needs another way in by then.

**The host's process (P2 slice 3).** `wanderbooks host` closes every descriptor above 2 it was handed, refuses to run confined, takes `run/host.lock` (waiting up to 30 s for a host that is closing; one that finds a live host exits at once), binds `run/lease.sock`, and starts the server from `/proc/self/exe`, naming the root on its command line only when the host itself was given one (tests): nothing in a process list names it. Ending leases, it tells each holder why and closes the server only once they have let go (at most 2 s), so holders stop sending before it closes; a call already on its way is refused at the door, unrun, and sent again once the books are back. A closing server is never killed, however long its last call runs; a new server waits for an old one still finishing (its host killed) rather than fail. A restart begins when the new version appears, not at a quiet moment: the server first finishes every call in progress (none is cut off), and calls arriving meanwhile wait for the new server (up to 60 s); ~~waiting for idle comes with jobs (`books.job`), which are what can run long.~~ → D-53 On a restart the old host hands over: it lets go of `host.lock` and keeps `update.lock` until the new host answers, so waiting holders find it. A holder starts a host only when none holds the lock; one that died at once is noticed within 2 s; a server that fails soon after opening counts as failing (only 60 s open clears the count). Lease records carry the holder's process as the kernel reports it. The lease reasons are `closed_by_owner`, `restarting`, `closing` (the host is closing because nothing held it: hold again, a new host will answer; the owner's close, by the menu, `wanderbooks close` or a signal to the host, is always `closed_by_owner`, and a holder that had held stays closed) and `server_failed`. The bridge never sends a call the agent cancelled before it was sent (MCP owes no answer to a cancelled request), answers a ping itself while the books open, refuses JSON-RPC batches request by request, passes no client offer it can't carry back (sampling, elicitation, roots), reports a call answered with nothing as possibly run, and carries the per-request protocol headers of the 2026-07-28 era. Sessions on the server time out only after 24 h without a message (rmcp's idle timer ignores a call in progress, so a shorter one could cut a long call off); the bridge ends its own when its stdin closes or it is signalled. `wanderbooks doctor` (a first form, completed in slice 8) says whether the books are open, what holds them, and the host's and server's last log lines, naming no folder. Tests shorten the timings with a hidden `--test-knobs` flag, passed down from holder to helper to host.

**Starting the host cleanly.** Every mode that confines itself (the viewer, the bridge) first starts a small unconfined **helper** (`wanderbooks helper`, spawned as its own process over an inherited socket pair before any thread, window or Cocoa start; never a fork of a running GUI), then confines itself. The helper accepts a fixed message set and nothing else: `start_host`; `open_with` an attachment id (the helper itself copies that blob, read-only, into a 0700 per-user directory, `$XDG_RUNTIME_DIR/wanderbooks-open/` or its per-system equivalent, swept when the helper exits, and hands it to the system's default app; it never accepts a path or a command line); `stage_backup`, whose file the helper asks the owner for in its own file dialog (never a path from the viewer), copying it into the data root's `uploads/` and returning a staged id for the `restore` intent (as built, slice 7: a backup is a repository, and the `restore` intent names its folder or address, which the server clones; 3.7); and `start_updater`, with no arguments. A confined mode starts the host only through its helper; unconfined holders (the CLI commands, `serve`) use the same clean start directly, and a restarting host starts its successor itself, already clean, in its own session and scope (P2 slice 3). The clean start is a re-exec: environment scrubbed to a fixed allow-list (keeping `XDG_RUNTIME_DIR` and `DBUS_SESSION_BUS_ADDRESS`, which `systemd-run --user` needs, only after checking they point at the host's own user's runtime directory), stdin, stdout and stderr to null (a log file under `run/`), a new session (`setsid`), a fixed umask. On Linux under systemd (uwsm on Omarchy puts every app in its own scope, which would take the host down with the app) it re-execs through a transient `systemd-run --user --scope`, which installs nothing, persists nothing, and disappears with the host; elsewhere `setsid` alone. On Windows, `CREATE_BREAKAWAY_FROM_JOB` where the job allows it, and `DETACHED_PROCESS`. The host resolves the data root itself from the user's account (`getpwuid` home, or the known folder on Windows), never from the opener's environment, unless `wanderbooks setup` recorded another root. It refuses to run if it finds itself confined (seccomp in `/proc/self/status`, a Landlock write probe). 

**One host, even in a race.** The host takes `run/host.lock` (an exclusive flock) before anything else; a second host exits and its opener attaches. The host starts the server, which takes every book's writer lock (as built, slice 7: and backs the books up itself, one run at a time per book, so two pushers never meet on one `archive.git`). An opener that finds no live host and an unlocked `host.lock` starts one; one that finds a host shutting down (the lease socket refuses during shutdown) waits on `host.lock`, then starts a new one.

**Closing.** When the last lease ends, the host waits 10 s; a lease arriving in that time cancels the close. Then: no new requests, the one in progress finishes, the server makes a last backup of each book (as built, slice 7, at most 60 s), every writer lock is released, and the host exits. "Close the books" (the app's menu, `wanderbooks close`) ends every lease at once; a bridge then tells its agent the owner closed the books and reopens them only in a new session. Before it ends leases the host says why on each one (`closed_by_owner`, `restarting`, `closing`, `server_failed`; P2 slice 3). A holder that sees the connection end with no reason (a crash, a kill) starts a new host and re-takes its lease, with backoff (1, 2, 4 … 60 s, at most five tries); a host that keeps failing to start (a damaged journal, a panic) becomes a Needs-attention and `doctor` item, and a bridge answers its agent with a clear error instead of retrying forever. `host_respawns` and `respawn_backs_off` test it.

**Restarting** (an update, or the executable replaced by a package manager) is not a close. What lives between calls survives it: upload grants, download links, `/h/` sessions and finished job results are kept in `run/state/` (0600, the session key with them), ~~and a running job delays the restart until it finishes (jobs, `books.job`, come later; until then~~ a restart begins at once and the server finishes every call in progress first (P2 slice 3; D-53: no jobs). The host takes `run/update.lock`, closes the books as above but keeps the lease socket's holders informed, swaps (or, for a package-managed install, finds the new executable already in place), and restarts from the new executable. Openers that arrive meanwhile wait on `update.lock`. Bridges reconnect and re-initialize upstream without the agent noticing. A package-managed upgrade is noticed by the host (its executable's inode changed) and restarts it as above.

**One version at a time.** Children (server, archive, extractor) are started from the executable the host itself is running (`/proc/self/exe` or `fexecve` on Linux), never by path, so a swap mid-run can't mix versions. Where that isn't available (macOS, Windows) every child, bridge and lease handshake carries the version, and a mismatch triggers a restart. The lease, helper and bridge protocols carry protocol versions (not release versions, since a staged update can skip releases); each accepts its current version and the one before, and outside that range refuses with a plain message ("restart this agent session").

**The bridge and slow starts.** The bridge waits up to 5 s for the server and returns its real `initialize` (the connect text composed from the book, 4.1). If the server isn't ready by then (a cold start, a projection rebuild), it answers `initialize` itself, within a client's startup timeout (Codex allows about 10 s), from the tool list built into the same executable and a short generic text that points at `books.start`, which returns the book's rules. Calls are queued until the server is ready; a queued call fails after 60 s with a clear error that names `wanderbooks doctor`.

**Format changes are the server's gate, not the updater's.** A server that would write a newer journal format than a book has opens that book read-only, with guard `format_upgrade_pending`, until the owner consents in the app ~~(or on a `/h/` page)~~ → D-28. Before it asks, it makes a verified local backup that includes the keys (`legacy.key`; D-48: for good). Consent comes only from the owner (the passphrase, through the viewer's `consent_format_upgrade` or a `/h/` page), never from an agent. Declining leaves the book as it was, and before consent the way back to a version that can write it is always there: a self-updating install keeps the previous verified app and offers "Go back to version X" (`wanderbooks rollback`); a package-managed one downgrades through its package manager (our apt and dnf repositories keep every release, and the AUR keeps old PKGBUILDs). This holds however the new version arrived: our updater, a package manager, or a hand install. **After consent** the previous version can no longer open that book, so "Go back to version X" refuses for it; the only way back is restoring the pre-upgrade backup, which would drop every entry since, so that restore says how many entries would be lost, arrives as its own book beside the current one (never replacing it), and needs the passphrase.

**As built (P3, the format upgrade, D-39), and where it departs:**
- **Waiting is a fact of the journal.** This version's format is the newest it reads with every feature it knows (`ledger_store::format::current`); a book declaring less waits (`JournalFormat::upgrade`), format 1 included, so the gate covers the step to format 2 as much as P3's features. New books declare the format in their second line (`Store::create`) and never wait; an adoption's copy is new books of this version and moves to it as it is made (the original keeps its own). The lazy declarations (`ensure_format` declared format 2 at the first setting) are gone: `ensure_format` and `ensure_feature` (the other P3 branches' call, same signature) now only refuse.
- **Served on the old format, not read-only.** The book opens and is served as before; only a write that needs the newer format or a feature it hasn't declared is refused, before anything is journaled (`StoreError::FormatUpgradePending`, guard `format_upgrade_pending`; the guards envelope is P5's, so the guard leads the error text and `books.start`'s `format_upgrade`). "Read-only" above is narrower than D-39's "declining never stops the books opening on the old format", which this follows.
- **The consent path.** The viewer's dialog and `wanderbooks books upgrade ID` (the owner passphrase typed at a terminal) send the lease intent `consent_format_upgrade`; the host, which reads no books, hands it to the server's admin socket (`upgrade_format`, the passphrase redacted from Debug), which checks the passphrase (Argon2id, `passphrase::verify`) before anything else. No MCP tool reaches either socket. No `/h/` page (P5).
- **Asked at every open.** The viewer asks once each time it opens waiting books, with a standing notice above every page and a Books-menu item; the owner's commands that open the books (`serve`, `books`, `token`, `connect`) ask once each run where the question is seen and answered (input and output a terminal, in the foreground: a command whose output goes to a file or a pipe would wait unseen, and mix the question into its output), and elsewhere print a note; `books list` and `doctor` show it; `books.start` puts it first.
- **The backup first.** Made at the go-ahead, not before asking (as above): a backup made when the owner is asked could be behind the books by the time they agree, and this one is the books exactly as they are moved. Under the books' lock throughout: the books are committed to their own `archive.git` (whatever targets the settings name, or none), the archive cloned and proven as a restore proves a backup, its head the books', its journal theirs byte for byte, its vault and keys file for file (`backup::prove_archive`); only then the one `journal.format_set`. Both are records of `ops/backup.jsonl` (`format_upgrade`, with the backup's commit and head); the targets take the pre-upgrade commit, as history, at the next backup after the write.
- **Not built:** "Go back to version X" (`wanderbooks rollback` doesn't exist yet: P11's updater); restoring the pre-upgrade backup as its own book that says how many entries it lacks (the backup is a commit in the archive's history; `books restore` brings back a repository's latest, so this needs a restore of a named commit, its own book id, the passphrase and the count: not small, left for P11 with the rollback).
- **Each projection schema in a file of its own** (the lead's decision, 2026-10-21), so going back to an older version stays possible until the owner consents: an older version refuses a database of a newer schema (`SchemaTooNew`), so a version that migrated the one file in place would have kept P2 from opening books that declined the upgrade. The projection's file is `wanderbooks.v{N}.db` for schema N (`ledger_store::db_file_for`, the name depending on the number alone, so the P3 branches' v13s merge to one name). A version opens or creates only its own; its own missing, it builds it from the journal (and catches it up at open, as ever); P2's `wanderbooks.db` is used only while it already holds this version's schema (no rebuild for nothing); any other schema's file is never migrated, written, refused or deleted. `books.verify` (`database`, `other_databases`) and `wanderbooks doctor` (`database: …`, with the others left there) say which; stale files are listed, never deleted. Old books dirs kept in git (before P2) ignore `wanderbooks.db`, not the new name: they come in by `books adopt`, which copies them into the data root.

**As built (P2 slice 8), and where it departs (D-32):**
- **No confinement, no helper.** No mode confines itself (no Landlock, no seccomp), so the helper is gone (`wanderbooks helper`, `ledger-host` helper.rs): the bridge, like every holder, starts the host itself with the clean start (start.rs). The host still refuses to run when it finds itself confined (started from a sandboxed app). Isolation is said as `same_user (advisory)` in `books.start` (`isolation`) and `wanderbooks doctor`.
- **The lease socket's intents are two:** `close` and `consent_format_upgrade`. Backing up, restoring and new books were intents only so a confined viewer could hand them over; the viewer now does them as the command line does, through the server's admin socket (`back_up_now`, `restore`, `create`), holding a lease while it asks.
- **The viewer holds the books open** (a lease of kind app) while it shows books of the data root, on a thread of its own, taken only once its own flags are good and its book is found (the window never waits; with no server it still opens and follows the books from disk). It lets go only when the owner closes the books; books that stop or won't open are tried again (a fresh try every 30 s after five failures). Its **Books** menu (ledger-gui owner.rs, beside Export): Back up now; Restore… (the backup's address or folder typed, checked first: what it would do is shown, then done for that backup at that head); New books…; Connect Claude Code, Connect Codex; Close the books (asked first). Each is done off the UI thread by the `Owner` ledger-cli gives the viewer, over the same ledger-host functions the command line uses; the viewer writes nothing of the books; the menu waits while one runs, and a restore's fields can't change while it is checked (a result for anything since edited is dropped). Older books dirs and the demo have no Books menu.
- **New commands:** `wanderbooks connect claude|codex --book ID` (the books' agent token kept in the keyring, made if none is, replacing the server's, which no one can be using; then the agent's own `claude mcp add --scope user` or `codex mcp add` of `wanderbooks-<book>` running `wanderbooks mcp --book <book>`; the agent's command found on the PATH, else where its installers put it under `$HOME`, or `/usr/local/bin`, `/opt/homebrew/bin`: a viewer started from the desktop has a short PATH), and `wanderbooks books back-up ID [--new-disk]` (the admin socket's `back_up_now`).
- **Scaffold deleted:** `wanderbooks new`/`init` and its templates (justfile, AGENTS.md, CLAUDE.md, wanderbooks.toml, .gitignore, .mcp.json, Claude and Codex settings), and its tests; the settings file's own export is ledger-config's (`books.export_settings`). `wanderbooks new`/`init` now says it is gone and what to run instead. `install.sh` installs the one executable, runs `setup`, and given a name starts those books and connects Claude Code: three commands to the first post, no sudo. The website (`site/index.html`) says the same; its built copy must be redeployed (`just deploy`) when this change is pushed: the old installer runs `wanderbooks new`. The first company's own books dir keeps its files until the cutover (slice 9).
- **Tests:** `ledger-gui/tests/owner_menu.rs` (each item through a fake owner, the restore checked first, nothing of the books written; busy states and a restore's fields frozen while checked; no menu for an older books dir; screenshots), `open_while_held.rs` (connect with fake agents and the command it registers reaching the books, a token the keyring lost, an agent that refuses; back up now from the command line, `--new-disk`; the viewer's hold, new books and close through the owner's functions; the two intents), `one_executable.rs` (the gone scaffold), `spec_kept.rs` (`isolation`).

## 4. The MCP surface after the change

### 4.1 Connect instructions (composed in `get_info`; replaces `INSTRUCTIONS` S:57 and the string replace at S:8179)

Hard cap 1,400 characters. The first 512 contain `books.start`. Composed from journaled state:

```
wanderbooks: the double-entry books of {company.name|"a new company"}. Call books.start first in every session: it returns the rules in force, this company's conventions, what needs attention, and your next steps. These tools are the only way to the books; never look for them in local files. Amounts are decimal strings ("123.45"). Entries: books.draft_entry, books.add_line, books.preview_entry, show the human the preview, then books.post_entry with its preview. Posted entries never change; books.reverse_entry posts a reversal (previewed first), whose evidence is the original's.
{evidence_rule}. No receipt yet? books.set_draft_state awaiting_evidence keeps the draft visible until it arrives. Files go in with books.begin_upload (never inline: D-46) and come out of books.get_receipt as text, page images or download links. Every change returns next_steps, guards and the journal head: follow them and show the human the affected balances. A full TIN goes only into the vault (books.set_tin, books.set_w9): never repeat one. Procedures are prompts (onboard, month_end_close, cpa_handoff, audit); the guide is books.handbook or books://handbook.
```

**[Amended 2026-10-26 (D-47, D-50, D-28): `preview` for `preview_token`; a reversal posts; TINs through the agent into the vault. The live connect text is composed in `crates/ledger-mcp/src/handbook.rs`; P5 finalizes it.]**

`{evidence_rule}` comes from `evidence::rule(&settings)`, which replaces `ReceiptPolicy::rule()` (`ledger-config/src/receipt_policy.rs:147`) and now names the required and exempt accounts. The first company example: "Every entry needs a receipt attached or linked before posting; depreciation, amortization and closes are exempt; a reversal needs none: the original carries the evidence" (D-47).

### 4.2 `books.start` (read-only, `_meta["anthropic/alwaysLoad"]=true`, ≤ 8 KB)

The v1 shipped in P1 with the fields marked † (as built: `crates/ledger-mcp/src/tools/kept.rs::start_out`). The full version ships in P5 (8.2).

Input: `{focus?: "daily"|"onboarding"|"close"|"audit", agent?: {model, reasoning_effort, harness}}`. The `agent` block is self-reported and goes to the access log.

```json
{ "book": {"id","name","url","timezone","today","fiscal_year":{"start","end"},"base_currency","basis"},   // †
  "you": {"principal","kind","scopes","client_id","verified":["principal","client_id"],"self_reported":["model","reasoning_effort"]},
  "isolation": "same_user (advisory)|service_user (enforced)",
  "head": {"seq","hash","last_handoff"},                                                                 // †
  "state": "needs_onboarding|active",                                                                    // †
  "rules": {"evidence","parties","capitalization","attestation_bar","approval_rules":"off"|[...]},       // † (evidence, parties)
  "settings_summary": [{"key","value","set_by","seq"}],                                                  // †
  "conventions": [{"id","kind","title","summary"}],                                                      // †
  "open_items": [{"id","title","due"}],
  "needs_attention": [{"id","severity","title","detail","tool","args","prompt"}],   // top 12 + counts by kind
  "drafts": {"building":n, "awaiting_evidence":[{"entry_id","date","amount","note"}]},   // no reversal_drafts (D-47)
  "reconciliation": {"tracked":[...], "never_reconciled":[...], "behind":[...]},
  "attestation": {"bar":{...}, "due":[...], "attested_through"},
  "viewer": {"open":bool, "how_to_open":"wanderbooks gui <book>"},                              // no remote replicas (D-51)
  "backup": {"last_at","through_seq","stale":bool},
  "handbook_sections": [{"id","uri","chars"}], "prompts": [...],
  "next_steps": [{"tool"|"prompt","why","args","required"}] }
```

When `state=needs_onboarding`, `next_steps = [{prompt:"onboard"}]`. Conventions are presented as company policy data, not as directives, which keeps the server eligible for directory review.

### 4.3 Handbook

`books.handbook {section?}` (read-only) and `books://handbook`, `books://handbook/{section}`. The source is `ledger-mcp/handbook/*.md`, compiled in with `include_str!`. It is generic and carries no the first company leaks ("FL-0001" and "Due to Shareholders" become neutral examples).

Sections as built (`crates/ledger-mcp/handbook/`, 21 files): `1099`, `attachments`, `backups`, `contract`, `control-roles`, `cpa-handoff`, `drafts-and-reversals`, `entries`, `evidence-and-receipts`, `files-in-out`, `fixed-assets`, `format-upgrade`, `parties-and-tins`, `prepaid-and-recurring`, `projects`, `reconciliation`, `revenue-and-payouts`, `rules-that-change`, `settings-and-conventions`, `tracing-figures`, `viewer`. To come: `invoices-and-bills` (S4), `attestations-and-audit` (P8), `self-host` (P9a); ~~`closing`~~ is covered by `entries` and the `month_end_close` prompt.

`tracing-figures` replaces "the journal is the evidence — read it" with "trace through `books.get_entry`, `books.search_entries`, `books.journal_history`". Each section ends with this company's conventions that apply to it, rendered from journaled `convention.set` events. The long tool descriptions move here: `export_cpa_package` (1,677 characters), `set_entry_party` (1,453), `set_w9` (1,378), `vendor_1099` (1,055).

### 4.4 Prompts (P5; arguments are single tokens)

**[As of 2026-10-26 no prompt exists: `opening.rs::capabilities()` enables tools and resources only. P5 adds the prompts capability, a prompt router and these prompts; struck rows are not built (D-46, D-28, D-17, Z-6).]**

| Prompt | Args | Drives |
|---|---|---|
| `onboard` | none | company facts → `set_setting` previews; chart template by entity type; members; timezone; evidence policy; attestation bar; opening entry; trial-balance check; offer `migrate`; offer to open the viewer |
| ~~`migrate`~~ | | → not built (8.4) |
| ~~`process_inbox`~~ | | → D-28, D-46: no drop page |
| ~~`receipts_from_email`~~ | | → D-17 |
| `reconcile` | `account=CODE month=YYYY-MM` | statement → `record_statement` → `mark_cleared` → `reconcile` |
| `month_end_close` | `month=YYYY-MM` | data-driven: tracked accounts, financing-account review, schedules, depreciation, recurring, `close_step` conventions, `books.audit scope=month`, `close_period`, month-end `attest_period` |
| `cpa_handoff` | `year=YYYY` | full audit at the bar → attestations → `export_cpa_package` (TINs by default, D-29; handed over as a download) |
| `annual_compliance` | `year=YYYY` | calendar by entity type, 1099 work, open items |
| `audit` | `scope=month\|annual\|cpa_handoff\|full` | the adversarial procedure: separate verifier agents, per-receipt extraction against page images, `attest_receipt`, `review_entry`, `attest_period` |

### 4.5 Resources (advertised templates; URIs repeated in `structuredContent` and in text)

| Resource | Contents |
|---|---|
| `books://start` | — |
| `books://handbook/{section}` | — |
| `books://conventions` | — |
| `books://settings` | — |
| `books://reports/{kind}{?start,end,as_of}` | today these take no dates (S:8189) |
| `books://digest{?since,as_of}` | the bare form works; the default is since the last session head (fixes `types.rs:1520`) |
| `books://entries/{id}` | — |
| `books://attachments/{id}` | metadata plus extracted text |
| `books://attachments/{id}/page/{n}` | PNG |
| ~~`books://attachments/{id}/file`~~ | → D-46: bytes only as a download (`books.get_receipt view=bytes`) |
| `books://journal/{seq}` | — |
| `books://workpapers/{id}` | — |

`books://receipts/{id}` is meta only (as built, slice 5); P6 adds the text to `books://attachments/{id}` and keeps the old URI as an alias until P5's doc rewrite.

### 4.6 Result envelope (single choke point in `call_tool`, S:8160)

```json
{ ...payload,
  "next_steps": [{"tool","args","why","required":bool}],
  "guards":     [{"code","message","remedy_tool","remedy_args"}],
  "warnings":   [...],
  "head":       {"seq","hash"} }           // on every mutating result
```

`next_steps::decorate(tool, &result, &view)` takes items from the attention function in `ledger-package` (U0b; ~~`ledger_view::attention`~~ → D-53) filtered to the touched subject, plus tool-specific steps. Examples:
- After a post: attach or attest evidence if it is missing, `books.show entry`, the affected balances.
- ~~After a reversal draft: add evidence.~~ → D-47
- After a close: `attest_period`.
- After `export_cpa_package`: the human download link.

Report results also carry `as_of_status {drafts_open_on_or_before, unreconciled, attested_through, settings_problems}`. Properties: every mutating result has a `head` and a non-empty `next_steps`; every guard names a remedy tool that exists.

### 4.7 Files in and out

**In:**
**[The planned lists; the As-built notes below are what exists. Struck items → D-28 (no drop page), D-46 (no inline bytes), D-17 (no email-in), D-51 (no App widget).]**
- `books.begin_upload {sha256, size, mime, file_name, purpose: receipt|document|import|w9}` returns `{upload_id, command, expires_at, max_bytes}` (as built). ~~`put_url`, `curl`, `human_url`~~
- `PUT /b/{book}/u/{upload_id}` is single-use, valid 15 minutes, bound to the principal, sha and size, capped at 25 MiB by default, and re-hashed before acceptance.
- ~~`human_url` is a drop zone on `/h/inbox` for clients with no shell (claude.ai).~~
- `attach_receipt`, `file_document`, `import_bank_csv`, `import_chart_csv` ~~and `attach_email`~~ take `upload_id | existing_attachment_id` ~~| inbox_id | sha256~~.
- ~~`books.inbox` / `books.discard_inbox_item` manage the drop zone.~~
- ~~`openai/fileParams` is declared on the attach tools (P9). An MCP App upload widget comes in P9.~~

**Out:**
- `books.get_receipt {attachment_id, view: meta (default; with text from P6) | text (P6) | page_image{page, max_px} (P6) | bytes}` (bytes and page images handed over as downloads, never inline: D-46). ~~`link`~~
- ~~`books.download_link`~~ → D-46: every file is handed over as a `HandedFile` with its `wanderbooks download` command (`hand_over`); there are no URLs locally.
- `export`, `asset_labels` and `export_cpa_package` hand their files over (as built, slice 6).
- ~~TIN-bearing outputs come only from `books.request_human_download`, which returns an `/h/` link.~~ → D-29: they come from the same tools, TINs included, as downloads.

**Typical receipt flow (Claude Code, the daily case in 7.2), as planned; slice 5 built it with a command instead of `curl` (below):** `sha256sum` → `books.begin_upload` → `curl -T` the file to `put_url` → `attach_receipt {upload_id}` → `get_receipt` (text) → `draft_from_template` or `draft_entry` → `preview_entry`, shown to the human → on "yes", `post_entry` with its `preview` (D-50) → `next_steps` and the head come back, and `books.show` moves the viewer to the entry. No step raises a client permission prompt beyond the client's normal rules. "Save it as a draft" is `set_draft_state awaiting_evidence`. ~~On claude.ai, files go through `human_url` and the `process_inbox` prompt~~ → D-28, D-51 (claude.ai is P9b; its file path is decided then).

**As built (P2 slice 5), and where it departs:**
- **No path on the server's computer is read.** `books.import_local_file`, the import roots and `--import-root` are gone, and so is `set_w9`'s `w9_file_path`; a test (`no_tool_takes_a_path`) keeps any path-like parameter out, except the three outputs with full TINs (`export_cpa_package` and `get_w9` `save_to`, `vendor_1099` `save_csv_to`), which keep theirs, written only outside the books, until the viewer takes them over in slice 6 (D-28: the viewer reads and downloads; it never writes).
- **An upload, for books served from the root:** `books.begin_upload {sha256, size, mime_type, file_name, purpose: receipt|document|import|w9}` returns `{upload_id, command, expires_at, max_bytes}`. No `put_url`, `curl` or `human_url`: the server listens on a Unix socket, so the agent runs `command`, `wanderbooks upload --book ID UPLOAD FILE`, which reads its token from the keyring (`--token NAME` is in the command when the caller's token isn't the default `agent`) and PUTs the file to `/b/{book}/u/{upload}` (D-28: the agent does it all; no `/h/inbox` drop zone, no `inbox_id`; the hosted server of P9 adds a URL). The bytes stream to `uploads/<id>.part` and are kept only if exactly the size and SHA-256 declared, by the token that began it, once (409 after); 25 MiB at most; 15 minutes; at most 16 under way per token; lapsed ones swept when the books open, whenever a grant is made, and every 10 seconds (owner, 2026-10-19: a W-9 or a file with a TIN "shouldn't just linger there forever... EVENTUALLY cleaned up is fine"). One lock per book orders every change to its uploads (grants, sweeps, a send's start and end, a take, a give-back), and a grant is written aside and renamed. Refusals carry their code (`no_such_upload`, `not_yours`, `already_sent`, `in_use`, `not_as_declared`, `stopped`), and a refused body is read and dropped (up to 25 MiB) so the client gets the answer. Every PUT to books the server holds open is in the access log (action `upload`, with the upload's id, refusals at the door included; a request for books it doesn't hold has no log to go in), written in the same blocking task as the bytes, so a client hanging up never leaves bytes kept without their record, and bytes whose record can't be written are thrown away. An upload's id is `up_` and 32 letters `a`-`p`: with no digits, the access log's TIN masking never hides one.
- **Filed once, and only when filed.** A tool takes the upload by renaming its bytes aside (so two calls never both have it), hashes them again, and spends it only once they are in the books (an import: once its rehearsal passed and the writing begins); a call refused after taking it, or one that panics, gives it back, and a server stopped mid-call gives it back when the books next open. A tool takes only an upload of its own purpose (receipt or document: `attach_receipt`, `file_document`; import: the two CSV imports; w9: `set_w9`). `existing_attachment_id` is offered only for a receipt or document, and never names a withdrawn attachment.
- **Inline: removed by D-46 (2026-10-23).** It was `content_base64` (≤ 256 KiB) with a required `sha256`; now every argument that carried a file's bytes or text (`content_base64`, `sha256`, `csv_text`, `w9_content_base64`, `w9_sha256`) is refused as unknown before the tool runs, pointing to `books.begin_upload`, and the filing tools' schemas take no other argument. Names and types are checked when a file is filed: a name of 1 to 255 bytes, no folder in it, no control or invisible characters; a MIME type of visible ASCII. `attach_email` and `inbox_id` aren't built. `set_w9` takes `w9_upload_id`, judged a PDF, PNG or JPEG by its first bytes, with an optional `w9_file_name`. A request on `/b/{book}/mcp` is at most 1 MiB now that files have their own route (it was 64 MiB).
- **Out:** `books.get_receipt {attachment_id, view: meta (default) | bytes}`: since D-46, bytes hands the file over (a `HandedFile`, whatever its size), as `books.export` (every format) and `books.asset_labels` do; before it, bytes came inline for a file ≤ 256 KiB. The stored file also comes by `wanderbooks download --book ID ATTACHMENT --to FOLDER`, which GETs `/b/{book}/a/{attachment}` with the agent's token (books:read; `Content-Disposition` with the name in ASCII and RFC 8187 UTF-8, `nosniff`, and the SHA-256 as an RFC 9530 `Repr-Digest`, which the command checks before it saves), into a new file beside any of that name (`name (1).pdf`), never over one: written to a hidden file and linked in whole, so the name never holds part of it. The name it saves under is the server's made plain (no folder, no leading dot, nothing invisible, at most 200 bytes), else `attachment-N`. The read and its record are one blocking task, as for uploads. The `books://receipts/{id}` resource is meta only. Not built yet: `text`, `page_image` and `link` views, `books.download_link`, and links from `export`, `asset_labels` and `export_cpa_package` (slice 6, with the viewer's downloads).

### 4.8 Drafts waiting on evidence

- Draft states are `building | awaiting_evidence{note}` (P4). ~~plus reversal drafts~~ → D-47
- Open drafts appear in `books.start.drafts`, the digest, Needs attention ("Draft #52 awaiting receipt: Figma annual plan, noted 2026-10-16") and every report's `as_of_status`.
- A close is refused while a draft in the period is open (existing Lean `hdrafts`). `discard_draft` is the way out.
- `export_cpa_package` refuses while a draft dated on or before `as_of` is open, unless given `allow_open_drafts {reason}`.

### 4.9 Tool changes

**New:**

| Area | Tools |
|---|---|
| session | `books.start`, `books.handbook` (built), `books.journal_history {after_seq?, types?, subject?, limit}` (P5; the only way to "read the journal"). ~~`books.job`~~ → D-53; ~~`books.viewer_status`~~ → `books.show` reports it |
| settings / conventions | `books.settings` (from the journal, with provenance and schema), `books.set_setting`, `books.export_settings`, `books.conventions`, `books.set_convention`, `books.retire_convention`, `books.record_decision`, `books.decisions`, `books.open_items`, `books.raise_item`, `books.resolve_item`, `books.draft_from_template {template_id, ...}` |
| drafts (P4) | `books.preview_entry` (read-only), `books.list_drafts {as_of?, state?}`, `books.set_draft_state`, `books.discard_draft {reason}`. `books.inherit_evidence {entry_id, from_entry, reason}` stays the re-audit's annotation (D-12) |
| chart | `books.rename_account` |
| evidence | `books.begin_upload`, `books.add_instrument`, `books.instruments`, `books.set_entry_instrument` (built); `books.find_attachment {sha256\|receipt_number\|text}`, `books.annotate_attachment`, `books.consolidate_duplicates` (P6). ~~`books.inbox`, `books.discard_inbox_item`, `books.attach_email`, `books.download_link`~~ → D-28, D-17, D-46 |
| assurance (P8) | `books.audit {through, scope}` (deterministic workpaper; ~~job~~ → D-53), `books.workpaper`, `books.attest_receipt`, `books.review_entry`, `books.attest_period`, `books.attestations` |
| ops | `books.backup`, `books.back_up_now` (admin), `books.access_log` (admin) (built). ~~`books.invite`~~ → D-51 |
| ~~migration~~ | ~~`books.compare_reference`, `books.record_inventory`, `books.check_inventory`~~ → not built (8.4) |
| reports | `export` kinds `members_equity`, `general_ledger`, `general_journal` (JSON with `entry_id`) |
| ~~vault (human links)~~ | ~~`books.request_tin_entry`, `books.request_human_download`, `books.set_w9_facts`~~ → D-28, D-29, D-48: `books.set_tin`, `books.set_w9`, `books.get_w9` stay |

**Changed:**
- `post_entry` requires `preview` (D-50; P4); the gate moves into Store (P4); the result carries `posted_seq`.
- ~~`reverse_entry` creates a reversal draft, with `evidence: "link_original" | {attachment_ids} | "later"`.~~ → D-47: it previews, then posts, with the S1 bounds guard.
- `attach_receipt` and `file_document` deduplicate by sha (P6): identical bytes link the existing record, so "deduplicated" becomes true. Text is extracted at attach (P6).
- `get_receipt`: views as in 4.7.
- `digest` (defect 6).
- Reports: one default range, `as_of_status`, and `strict`.
- `cash_flow`: `reconciles`, `noncash_disclosures`, `operating_reconciliation`.
- `vendor_1099`: instrument-aware; `include_tins` and `save_csv_to` dropped.
- `set_vendor_1099` takes an explicit `not_reportable` plus a reason.
- `register_asset`: `in_service_date` (with `purchase_date` kept as an alias for one release) and basis fields; it reports `acquired_date`, which comes from the acquisition entry it is linked to (settled in design, 2026-10-20), so it takes none.
- `list_assets`: `accumulated_posted` and `accumulated_scheduled_through`.
- `list_schedules`: start, end, convention, next, remaining.
- `parties`: last four from `party.tin_set`.
- `add_account`: tax-category warning, or refusal under the policy.
- `show`: refuses `start > end`; appends the target to `ops/show.jsonl` in the book's data root (locally) or to the replica event channel (P9, remote viewers), and reports whether a viewer is open and was reached. The result never names a path.
- `verify`: also checks the control tie-out (built, S1); P8's audit checks `access.jsonl` and extraction blobs. ~~the vault public-key sha~~ → D-48
- `export_cpa_package`: handed over as a download, TINs by default (D-29, D-46). ~~no-TIN package returned as a link; becomes a job.~~

**Annotations (P0):** every tool gets a `title` and `readOnlyHint` or `destructiveHint`, plus `idempotentHint` where true. Corrections: `export_cpa_package` is no longer marked read-only; `verify`, `list_periods`, `ar_aging` and `ap_aging` are annotated. No tool gets `requiresUserInteraction` (2.1; owner decision 2026-10-16).

**Removed:**
- `import_local_file` (S:4511): restricted in P0, deleted in P2.
- ~~`get_w9`.~~ → D-29: kept.
- `set_w9`'s `w9_file_path`: deleted in P2. ~~`set_tin`'s `tin` and `set_w9`'s `tin`: deleted in P7.~~ → D-28: kept.
- Every `save_to`/`save_csv_to` (`types.rs:368, 2563, 2686, 2758-2764`): deleted in P2.
- ~~`include_tins`.~~ → D-29: kept.
- The `INSTRUCTIONS` const.
- `viewer::request` (replaced by `ops/show.jsonl`).
- `--books-toml`/`--config` in serve mode.
- Every mention of `just` or `wanderbooks.toml` in tool text: S:340, 390, 525, 546, 637, 640, 1902, 3061, 3251, 3374, 4060, 4315, 4568, 5194, 5829.
- `server.rs` (12,566 lines on 2026-10-26; six groups in `tools/` so far) is split by tool group in one go (U0a) before the remaining units open.

The tool count is 118 (2026-10-26) and grows by about 15. Only `books.start` and `books.handbook` load upfront under tool search.

---

## 5. State migration

### 5.1 Format gate (P0, before any new event)

- An unknown type is no longer `JournalCorrupt` (`store.rs:4419-4423`, `:4640`, Snapshot, GUI). It becomes `StoreError::NewerJournal {seq, type, need}`, shown as "update wanderbooks".
- A one-time `journal.format_set {format: 2, features: [...]}` is written before the first v2 event.
- Fixture journals from every phase are added to `books_written_under_older_rules_still_open`.
- There is one executable (D-22), so an update never mixes versions; the host starts its children from its own executable (3.9). `wanderbooks update` updates a self-updating install (dmg, Windows installer, AppImage) and refuses on a package-managed one, saying to use the package manager. The local GUI reads the journal directly, so it hits the same `NewerJournal` gate. ~~the replica protocol (P9) carries a minimum version header.~~ → D-51

### 5.2 New events (all additive; legacy events replay unchanged)

Every event may carry an additive `actor {principal, client_id, kind}`, which replay ignores.

| Event | Phase | Replay target |
|---|---|---|
| `settings.set {key, value, effective_from?, source{decided_by, decided_on, citation?, note?}}` / `settings.cleared` | P1 | `ledger_core::Settings` (Lean `Step.setSetting`: balances and drafts unchanged) |
| `convention.set {id, kind: posting_template\|approval_rule\|policy\|close_step\|calendar\|mapping\|note, title, body_md, structured?, source, supersedes?}` / `convention.retired` | P1 | `Conventions` |
| `decision.recorded {id, subject, decision_md, decided_by[party], decided_on, evidence_attachment?, supersedes?}` | P1 | `Conventions` |
| `open_item.raised` / `open_item.resolved` | P1 | `Conventions` |
| `account.renamed {account_id, code, name, previous, reason}` (keyed by id, as `account.renumbered`: a code can change, an id can't; `code` is informational; the journal feature `account_renamed` is declared before the first) | P3 | chart |
| `instrument.added {id, kind: bank\|business_card\|member_card\|member_cash_or_check\|platform, label, last4?, owner_party?, account?}` / `entry.instrument_set` | P3 | `Ledger` (1099) |
| `asset.basis_set {asset_id, rule: purchase\|carryover_723\|personal_use_conversion, contributor_adjusted_basis, fmv_at_contribution, sec704c_method, source, tax_basis_723, depreciable_basis, built_in_gain_704c}` (the facts; the figures for readers, never recomputed at replay; a purchase's `depreciable_basis` null; the journal feature `asset_basis` declared before the first; legacy `asset.tax_basis_set` still replays) | P3 | assets |
| `schedule.registered` + `convention` (absent = `monthly_even`) | P3 | schedules |
| `vendor.1099_set` value `not_reportable{reason}`; legacy `null` replays as `undetermined` | P3 | vendors |
| `period.closed` with `close_to: MembersCumulative{since_seq}` (old variants replay as before) | P3 | periods |
| `entry.draft_state_set {id, state, note}` / `entry.draft_discarded {id, reason}` | P4 | `Ledger` drafts (`Step.discard`; `draftsOpen` at `Ledger.lean:133`) |
| ~~`entry.reversal_drafted`~~ | ~~P4~~ | → D-47: not built |
| `entry.evidence_inherited {entry_id, from_entry, attachment_ids, reason}` | P10 (the re-audit's annotation, D-12) | evidence |
| `entry.posted` + `actor` | P4 | ignored by replay (`posted_seq` is the line's own `seq`, read at replay: no field) |
| `attachment.text_extracted {id, text_sha256, extractor, version, pages, status: ok\|none\|failed}` | P6 | evidence |
| ~~`attachment.rendered`~~ | ~~P6~~ | → D-17: not built |
| `attachment.annotated {id, fields{receipt_number?, payee?, amount?, date?, instrument?, category?, source?, reimbursable_to?}, note}` | P6 | evidence |
| `entry.settles {entry_id, settles[]}` | P6 | derived `reimbursed` |
| ~~`inventory.recorded`, `reference.compared`~~ | ~~P6~~ | → not built (8.4) |
| ~~`vault.created`, `vault.rotated`, `vault.keyslot_*`, `vault.resealed`~~ | ~~P7~~ | → D-48: not built |
| `invoice.created`/`bill.created` with the `documents_v2` fields; `invoice.sent`; `invoice.payment_received`/`bill.paid` with `line`, `discount`, `fee`; `invoice.written_off` | P4b S4-S7 | `arap` (8.3.7) |
| `convention.source_revised`, `decision.source_revised`, `open_item.source_revised` | P5 | `Conventions` |
| `audit.run {id, through, scope, program_version, through_seq, tb_hash, findings, workpaper}` | P8 | assurance |
| `receipt.attested`, `entry.reviewed`, `period.attested {..., method{model, reasoning_effort, adversarial, verifier_agents}, verified{principal, client_id}}`. `counted` is **derived** against `policy.attestation_bar` as it stands when asked; never stored (8.2 P8). | P8 | assurance |

**Not journaled:** the access log, backup runs, head handoffs, viewer preferences, page rasters, and any key material or key path.

**Projection (`schema.rs::SCHEMA_VERSION`, 15 on 2026-10-26; numbers taken at merge, 8.0.3):**
- `entries` gains `created_seq, posted_seq, posted_at (= ts of the posting line), draft_state, posted_by`.
- New tables: settings, conventions, decisions, open_items, instruments, annotations, extractions, attestations, audits.
- Core keeps a side map `Ledger.entry_meta: id → {created_seq, posted_seq, posted_at}`, filled during replay, so the Snapshot sees it.
- `projection_rows`/`compare_projections` include all of them.

### 5.3 Setting keys (typed registry `ledger-core/src/settings.rs`; validators moved from `ledger-config/src/lib.rs:434-622`)

Each key has a validator, a docstring (which becomes the `books.settings` schema) and a sensitivity class: `public`, `identity` (the full value appears only in reports and packages; `books.start` shows it masked), or `never` (cannot be set). TIN-shaped values are refused in every key except `company.ein`.

| Area | Keys |
|---|---|
| company | `company.name`, `.entity_type`, `.tax_classification`, `.tax_form`, `.fiscal_year_start`, `.timezone` (drives `today()`, replacing `chrono::Local` at S:1943), `.legal{formation_state, formed, state_entity_number, address, registered_agent, partnership_representative, irs_name_control, duns}`, `.ein` (identity; D-5, decided), `.cpa` (party), `.members` (effective-dated `[{party, share, capital_account, distribution_account, files_jointly_with}]`) |
| books | `books.accounting_basis`, `.cash_accounts`, `.financing_accounts`, `.investing_accounts`, `.asset_tag_prefix`, `.close_into`, `.close_sweep`, `.schedule_convention` |
| policy | `policy.evidence{threshold, required_accounts, exempt_accounts}` (replaces config.json `receipt_policy`), `policy.parties`, `policy.capitalization{de_minimis, twelve_month_rule, sources}`, `policy.require_tax_category: warn\|refuse` (default `warn`; the journal feature `require_tax_category` is declared before the first change naming it), `policy.attestation_bar` (default per D-9), `policy.cpa_package_requires_audit: refuse\|warn` (default `refuse`, overridable per export with a reason; D-9), `policy.audit_cadence`, `policy.approval_rules: off\|enforce` (default `off`: `approval_rule` conventions are recorded but never block a post) |

`base_currency` stays in `store.created`. `[books] data_dir`, `[vault] key_file` and `[viewer]` are dropped on import with a note. Unknown keys are reported as problems instead of being silently ignored.

### 5.4 toml import/export

`ledger-config` shrinks to a codec:
- `toml_to_events(&str, &Settings) -> Result<Vec<Event>, Problems>` emits only the diff.
- `settings_to_toml(&Settings, &Conventions)` writes provenance as comments.

The `WANDERBOOKS_TOML` template (`ledger-cli/src/main.rs:237-347`) becomes the export format. `ledger-cli/tests/init.rs:305,354` become round-trip properties: export then import yields zero events, and every example imports. The `config_parse` fuzz target is kept, and `fuzz_settings_replay` is added.

### 5.5 Migrating existing books (generic)

- `wanderbooks books adopt <data-dir> --id X [--toml FILE] [--config-json FILE]` runs as the human, with no sudo, through the admin socket. The server, which also runs as the human, does the copy:
  1. Takes the source flock, so it refuses while another program holds it.
  2. Verifies the chain and records head H.
  3. Copies `journal.jsonl` and `blobs/` into a staging folder (a cut-off last line or a link among the blobs is refused), opens the copy (rebuilding the projection), checks that the head equals H, verifies it end to end and re-hashes every blob; only then is it renamed into `books/X/`, and on any failure the staging folder is removed and nothing appears. A source inside the root, or holding it, is refused.
  4. Imports settings from the given texts (preview printed, confirmation at the TTY) unless the journal already has `settings.set` events.
  5. Copies the vault's records into `books/X/vault/`, and the v1 key into `keys/legacy.key` (0600). Copied, not moved (P2 slice 2): adoption never changes the source, so a refused or abandoned adoption leaves it exactly as it was; step 7 is what moves the old copies away.
  6. Seeds `archive.git` from the source's git history, filtered to the journal and blob paths, and records the old repository's last commit hash. The push destination is the owner's answer to Q-1.
  7. If the owner agrees (Q-3), moves the source data dir, its vault and the project folder's git history into `<root>/legacy/X-<date>/`. Nothing is deleted. If the owner says no, the source stays where it is, read-only, and Needs attention lists it as an exposed old copy.
- `ledger-cli` `init`/`new` stop scaffolding and call `books create`.
- (`dev-stdio` kept `legacy_files` mode for unadopted data dirs until D-44 removed it; old books come in by `books adopt`.)
- Legacy semantics kept:
  - `entry.reversed` stays a posted reversal. During the re-audit the original's receipts are linked with `inherit_evidence` (D-12, decided); any not yet linked are reported as `legacy_reversal_exempt`.
  - Old close variants keep their rounding.
  - Old schedules stay `monthly_even`.
  - A `null` 1099 type replays as `undetermined`.
  - Duplicate attachment records stay until consolidated.

### 5.6 the first company (staged; reversible until Q-3)

| When | Step | Acceptance |
|---|---|---|
| P0 | Install new binaries (still stdio). Capture **golden JSON** of TB, P&L, BS, CF, members' equity, GL, journal, vendor_1099, parties, assets, schedules and digest, as of 2026-10-11, every month-end, FY-to-date and today, through the old binary over MCP. By decision (2026-10-16) the first company's goldens are kept privately in its own books dir, not in this public repo: `export_cpa_package` zips plus a canonical JSON form made by that dir's `goldens/canon.py` (which normalizes the text each export varies, such as its timestamp), with the comparison protocol in its README. | Goldens stored |
| P1 | The agent reads `/home/swaits/Documents/the first company-Books/wanderbooks.toml` (and `books-data/config.json` only by asking the owner to state its values, since it sits under `books-data/`) and calls `import_settings`. Expected diff:<br>• the 15 `[company]` keys; EIN as identity<br>• members 50/50 → 3000/3001 with `files_jointly_with`<br>• `accounting_basis=accrual`, `policy.evidence.threshold=0.00` (the owner, 2026-10-12)<br>• `cash_accounts=["1000"]`, `financing_accounts=["2100"]`; the citation is corrected to ASC 230-10-45-14(b)<br>• `parties=require` (2026-10-15), `timezone=America/Denver` (D-13, decided)<br>• `policy.approval_rules` stays `off`<br><br>Conventions are proposed in one batch preview from `AGENTS.md:74-227`:<br>• 9 posting templates: joint contribution as two entries; expense from joint personal funds → 2100; expense from Quillmere; household reimbursement (`Dr 2100 / Cr 1000`) with a `note` that the owner gives the go-ahead before money moves, recorded with its source and not an enforced gate; 50/50 distribution; revenue 4000; card 2050 expense and payment; cash entries dated by bank posting date<br>• policies: numbering, one file per receipt (PDF preferred), email as PDF, capitalization (`policy.capitalization` plus decisions 2026-10-11 and 2026-10-16)<br>• `close_step`: "ask the owner whether a member paid anything personally"<br>• `calendar`: Utah annual report around Oct 4, renewal fee to taxes & licenses<br>• memory-only rules become conventions: post when ready; decisions to the owner go one question at a time; TINs only in the vault<br><br>Decisions: one file per receipt (10-11), threshold (10-12), cutover with Sheet frozen as of 10-11 and reference attachment 37 (10-14), renumber 6100→6050 and card at 2050 (10-14), TINs in vault (10-15); light governance, meaning no consent paperwork, spending limits, signed memos or amendments for the two-member LLC, with decisions recorded with their source (10-16); local isolation "you, hidden away", the viewer reading the disk directly, no added permission prompts, and self-host now with hosting later (10-16).<br><br>Open items:<br>• IRS name control "WALN" discrepancy; letter = attachment 70, mailing pending<br>• deferred revenue to be raised with the CPA before the first subscription<br><br>Per-entry notes: entries 4 and 5 dated Oct 7 to match the Sheet. | Every golden report and export is byte-identical before and after the import. GUI shows the first company correctly with `wanderbooks.toml` renamed away. No `approval_rule` convention is created, and every daily-flow post still succeeds without an approval |
| P2 (pre-cutover) | Run `verify-sheet` and `drive-manifest verify` a final time and file their outputs as `workpaper` documents. Recapture goldens. | Both pass |
| P2 cutover (about 10 min) | 1. Ask the owner the two open cutover questions (section 10.3: Q-1 backups, Q-3 old copies; Q-2 is answered), one at a time, in plain English.<br>2. Quit the agent sessions and the GUI.<br>3. `wanderbooks setup`, with no sudo: it creates the data root (0700) and asks for the owner passphrase. Nothing is installed to run in the background (D-21).<br>4. Nothing to start: each command below opens the books while it runs, and afterwards the app or Claude opens them (D-23).<br>5. `wanderbooks books adopt /home/swaits/Documents/the first company-Books/books-data --id fernhill` (archive target from Q-1; old copies moved per Q-3).<br>6. `wanderbooks connect claude-code --book fernhill` (removes the old stdio registration made by `just mcp-add`, adds the bridge's stdio registration, `wanderbooks mcp --book fernhill`).<br>7. `wanderbooks connect codex --book fernhill`.<br>8. `wanderbooks gui fernhill`.<br>9. Remove `justfile`, `.claude/settings.json` hook, `.codex/hooks.json`, `scripts/` (archived under the data root's `legacy/`); `AGENTS.md`/`CLAUDE.md` become a 3-line pointer ("connect the wanderbooks MCP; call books.start").<br>10. The `company/` records are already filed (Q-2, D-20); the local copies stay until P6. | `books.verify` over MCP equals H. Goldens byte-equal. `books.start` works from Claude Code started in `~` and in `/tmp/empty`, and reports `isolation: "same_user (advisory)"`. No connect text, tool text, handbook section or result in a recorded cutover session names the data root. No step asked for sudo. If Q-3 is "move": the project folder holds no journal, blobs, vault or git history containing them. Viewer updates within 1 s. With every holder closed the books close cleanly (`books.verify` ok with the same head at the next open), and a Claude Code session started with the books closed opens them by itself, with no reconnect. **Rollback (nothing posted on the server):** close the books (`wanderbooks close`), move `legacy/fernhill-<date>/` back to the project folder (or use the untouched source if Q-3 was "no"), and re-register the P1 binary's stdio server as before cutover. **Rollback (entries posted):** `wanderbooks books export-legacy fernhill` writes a data dir with the same head check, then the same re-registration |
| P3 | Defect fixes change reports (cash flow, 1099 sections, asset fields, digest). Each difference is hand-adjudicated in the P3 change log. 1099 classification and 704(c) fields are shown to the CPA. | Change log signed off |
| P4b (one upgrade, D-52) | After S7 and P4 land: `wanderbooks books upgrade` (a verified backup first); then the owner sets `invoicing.*` (prefix, first number, default terms, payment methods), each party's email and terms, and the goldens are compared (the figures must not move). | `books.verify` ok; `books.control_check` agrees; the goldens unchanged to the cent |
| P4 | `list_drafts`. Leftover drafts are discarded or set to `awaiting_evidence`. | Zero unexplained drafts |
| P6 | • `consolidate_duplicates` resolves the 11 redundant records (preview first).<br>• Extraction is backfilled for all 66 attachments.<br>• Receipt numbers 1000–1029 are annotated from the "Receipt ID" in each description. Anomalies become annotations: 1009 unissued; 1011/1020 byte-identical pairs (merged); 43/45/47 have no ID. The next number is derived as 1030.<br>• Instruments: Quillmere, joint Amex, member cards, Chase Ink 2050.<br>~~• `verify-sheet.py`'s tables become a `mapping` convention, then `compare_reference` runs against attachment 37 as of 2026-10-11.<br>• `drive-manifest.json` text goes to `record_inventory`; the Drive connector listing goes to `check_inventory`.~~ → not built (8.4): the final `verify-sheet` run is a filed workpaper (P2 pre-cutover) and P10 cites it. | Every attachment's `get_receipt` default answer under 8 KiB; the 11 duplicates consolidated; receipt numbers annotated |
| ~~P7~~ | ~~`wanderbooks vault init` … `keys/legacy.key` shredded~~ → D-48: nothing; the v1 key stays in the books and in every backup (D-31) | — |
| P10 | Re-audit (8.2 P10; `docs/runbooks/re-audit.md`). | the exit criteria there |

---

## 6. Security and threat model

This section covers the local (T1) and self-host (T2) deployments. Threats specific to a multi-tenant hosted service (other tenants, a hostile operator, SSN compliance) are in section 11.

**Adversaries and controls**

| Adversary | Goal | Controls |
|---|---|---|
| The agent (careless or prompt-injected; locally it runs as the human's user ID, the same as the server) | read or alter books outside the MCP; see TINs | I-2, I-7, I-12: no text ever names the data root, which sits outside every project directory. Every call is access-logged. Destructive hints are set. `preview` (D-50). **Locally this is advisory:** an agent that deliberately hunts can read the data root, just as the human can, and `books.start`, `wanderbooks doctor` and the docs say so. ~~TIN plaintext stays sealed even then, because unwrap needs the passphrase.~~ → D-29: the vault key is in the data root too (advisory). Enforcement is the self-host deployment, proven by `agent_uid_cannot_read_tenant`. Where a company turns on approval-rule conventions (off by default), they are asserted by the agent (`approval{by_party}`): a policy, not proof, which the access log records |
| Network attacker or browser | DNS rebinding, CSRF against localhost | Host allowlist; Origin enforced (403); bearer tokens only on `/mcp`; `/h/` cookies `__Host-` and SameSite=Strict, with CSRF tokens; no tokens in query strings |
| Compromised OAuth client (self-host remote path, P9b; D-7 b: an external identity provider) | over-broad access | PKCE, exact redirect match, `iss`, CIMD SSRF guard (public IPs, 5 s, 64 KiB), consent per client per book, minimal base scopes with step-up, 1 h tokens with rotating refresh |
| History rewrite or fork (any deployment) | alter or fork the books unnoticed | head in every result; a backup's commits record each head; a restore's preview and the weekly drills prove each clone (D-31; ~~`verify-archive --expect-head`~~) |
| Stolen disk or backup | read books | full-disk encryption on the desktop, an encrypted volume in the self-host recipe; ~~archive encrypted with age~~ → D-31: the backup destinations are trusted to keep it private; the vault records are ciphertext, the key beside them (D-31) |
| Untrusted files (PDF/XLSX) | parser exploit inside the data-owning process | pure-Rust parsers (hayro, quick-xml) with size and page caps and `catch_unwind` (P6); fuzz targets `fuzz_extract_pdf`, `fuzz_extract_xlsx`. ~~the `ledger-extract` subprocess sandbox~~ → D-32, D-53: accepted residual risk, stated |
| A compromised server process | reach beyond the data root into the human's account | ~~Landlock, seccomp, rlimits; a further sandbox for extraction~~ → D-32: no confinement; the residual is stated as `same_user (advisory)` |
| Other processes running as the human, on the desktop | read the books; drive the viewer | no mechanical control locally (same user). A same-user process can alter anything the GUI reads (journal, blobs, keyslots). Journal tampering is detected by the hash chain and head handoff, not prevented; a vault public-key swap fails `books.verify`; a forged show request can only navigate. The GUI never writes. Stated as the advisory residual in `books.start`, `doctor` and the docs; the self-host deployment is the enforced option |

**Immediate hole (P0).** `import_local_file` (S:4511) does an unrestricted `std::fs::read`, so it can file `~/.config/wanderbooks/vault.key` (`ledger-vault/src/lib.rs:216-222`), and `get_receipt` then returns the key as base64. P0 changes this:
- An explicit `--import-root` allowlist (default: `$XDG_DOWNLOAD_DIR` only).
- Canonicalize, then refuse anything under the data dir, the books dir, the vault dir or the key directory, symlink escapes, and non-regular files.
- Test `import_local_file_refuses_key_and_journal`, plus a property over generated paths.
- No `requiresUserInteraction` is added (owner decision 2026-10-16). The interim TIN-bearing tools (`get_w9`, `set_tin`, `set_w9` with `tin`, `vendor_1099 include_tins`, `export_cpa_package` with TINs) keep today's behaviour (S:1198-1266, S:6111-6117).
- P2 deleted the path parameters. ~~and makes TIN-bearing outputs human-only. P7 deletes full TINs from the MCP entirely.~~ → D-28, D-29, D-48: the TIN tools stay; TINs go through the agent into the vault and come out in the exports the owner chose.

**Access log record:**

```
{seq, ts, prev, principal{id, kind: agent|human|viewer|cpa|system, label},
 client{client_id (verified), name, version (asserted)}, model? (asserted), request_id,
 action: tool|upload|download|connect|token.created|backup.run   (as built; human.* and replica.* → D-28, D-51; keyslot.changed → D-48),
 subject, args_digest (sha256 of canonical args), result: ok|refused(code), head_after?}
```

Each record is fsynced, and a failed write fails the request. Readable through `books.access_log` (admin) and the viewer's Access page. The CPA package carries a summary. Its head is part of the head handoff. `replica.connect` occurs only for remote viewers (P9). The local viewer's direct reads, including its in-process TIN unlock, are not logged, because the GUI never writes.

**Security test suite** (`ledger-cli/tests/security_*.rs`, driving a real books server as `tests/common` does, D-44):
- auth on every route; wrong audience → 401; bad Origin → 403; bad Host → 400; body over limit → 413
- expired or replayed grant → 410; upload sha mismatch refused
- `scope_step_up_403`; `handles_bound_to_sub`; ~~`tin_entry_requires_same_sub`~~ → D-28
- ~~`unwrap_only_in_human_handler`~~, ~~`vault_pubkey_swap_detected_by_verify`~~ → D-48
- the TIN canary tests of `crates/ledger-cli/tests/spec_vault.rs`: a canary TIN never appears where D-29 forbids it (results that are not the owner's TIN exports, errors, server logs, access-log records)
- `no_result_names_a_path`: the same session fuzz with the data root as the canary, across connect text, tool text, handbook, prompts, results and errors
- `access_log_one_record_per_call`, refusals included
- `isolation_label_is_advisory`: a local install reports `same_user (advisory)` in `books.start` and `doctor`
- `agent_uid_cannot_read_tenant`, run against the two-user self-host deployment in CI (P9a builds the job; the desktop default is advisory and is not claimed to pass it); ~~`no_tool_touches_outside_tenant` (Landlock harness)~~ → D-32

Fuzz targets: nine exist (`fuzz_config_parse`, `fuzz_core_serde`, `fuzz_entry_validate`, `fuzz_import_csv`, `fuzz_journal_line`, `fuzz_op_sequence`, `fuzz_settings_replay`, `fuzz_store_open`, `fuzz_store_replay`); to add: `fuzz_http_mcp_request`, `fuzz_upload_put` (P12), `fuzz_extract_pdf`, `fuzz_extract_xlsx` (P6). ~~`fuzz_eml`, `fuzz_replica_stream`~~ → D-17, D-51. Mutation recipes to add in P12: `ledger-host`, `ledger-package`, `ledger-vault`, `ledger-cli` (~~`ledger-view`, `ledger-mcp/http`~~ → D-53).

**Retention:** 7 years. A local or self-hosted book keeps SSNs, sealed, on a machine the owner controls.

---

## 7. User flows

What you will see and do. The technical details are in sections 3 and 4.

### 7.1 Setting up on your computer

1. You install Wanderbooks the usual way for your system: a `.dmg` on a Mac, an installer on Windows, your package manager on Linux (D-24). No developer tools, and no administrator password (on Linux your package manager asks for yours, as it does for everything). On first open it:
   - sets up the books server. It installs nothing that runs in the background, and it does not ask for an administrator password;
   - asks you to choose a passphrase. You need it only to agree to a books-format upgrade after an update (D-39). ~~You need it to type or see someone's Social Security or tax ID number … the page where you drop a big file;~~ → D-28, D-29: tax ID numbers go through the agent into the sealed vault and come out in the CPA package you ask for, with no passphrase.
   - creates your company's books;
   - connects Claude Code to the books, and Codex too if you have it;
   - tells you how the books open: opening Wanderbooks, or starting an agent, opens them; when the last of those closes, so do the books. Nothing runs in the background.
2. You type `claude` in any folder and say "set up my books". The agent sees that the books are new and walks you through setup: company facts, accounts, members, opening balances, and a check that everything balances. Every answer is recorded in the books.

### 7.2 Day to day

- You say: "The receipt is in Downloads, inv-883.pdf, the Figma annual plan." The agent sends the file to the books, reads it, drafts the entry, and shows you exactly what it will post. You say yes, it posts, and the viewer jumps to the new entry. Posting brings up no extra permission pop-up.
- You say: "Save it as a draft, I'll find the receipt." The draft stays on the to-do list until the receipt arrives. You see it in the viewer's Needs attention, and the agent sees it at the start of every session.
- You say: "Show me Q3 against last year." The viewer opens that report.
- Backups happen on their own while the books are open. Every change the agent makes comes back with a fingerprint of the books, so any tampering would show.

**Compared with today:** no helper commands, no project folder, it works from any folder, you answer the same number of questions, and installing needs no administrator password (on Linux, the package manager asks as it always does).

### 7.3 How private your books are

- The books live in a private folder that only your account can open, outside every project folder. Nothing the agent is told or shown names that folder, so an agent cannot wander in by accident.
- It is a curtain, not a lock. The server runs as you, so an agent that deliberately went looking could find the folder. The books say so whenever they report on privacy ("same user, advisory"), and `wanderbooks doctor` shows it too.
- Social Security and tax ID numbers are sealed in the books' vault. ~~Opening one needs your passphrase.~~ → D-29: the agent and the viewer show them where you asked for them, with no passphrase.
- For a real lock, run the books on another machine you own (7.4). The agent then has no way to reach the files at all.

### 7.4 Your own server, and claude.ai on the web

- You install the same programs on a home server, a network drive (NAS) or a rented server by following the recipe. Two commands move your books there, one on each machine, and the books' fingerprint must match before and after the move.
- One command gives that server a web address claude.ai can reach. Anyone can find the address, but only you can sign in. In claude.ai you go to Settings → Connectors → Add custom connector, paste the address and sign in.
- Then you chat as usual, for example "what needs attention?"
- ~~Small receipt files go straight through the chat. For bigger ones, you drop them on a private upload page and say "process my inbox".~~ → D-46, D-51: files go in through the upload command; how claude.ai sends a file is decided with P9b.
- ~~Downloads come as one-time links.~~ → D-46: every file comes as a download command.
- ~~When someone's tax ID number is needed, the agent gives you a link to a private page and you type the number there. The chat only ever sees "last four 1234 recorded".~~ → D-28: you tell the agent the number and it seals it in the vault.
- ~~The viewer on your computer can show the books on that server too. It streams a copy that it cannot change.~~ → D-51: the viewer opens a backup copy of the books instead.

### 7.5 Codex

The installer offers to connect Codex, or you can run `wanderbooks connect codex --book <name>` later. After that you run `codex` from any folder. Codex gets the same rules and procedures as Claude Code.

### 7.6 Your CPA

1. On your computer, you send the CPA the package the agent prepares. ~~With your own server (7.4), you can also invite the CPA to look at the books directly … all read-only.~~ → D-51: not planned.
2. The CPA package is an ordinary download, tax ID numbers included (D-29): the agent makes it, or the viewer's Export menu does. ~~The version with tax ID numbers comes only through a one-time link. You open it, unlock it with your passphrase, and send it on.~~ Every download is recorded in the access log (the agent's) or not at all (the viewer's, which never writes).
3. If the CPA wants, they can connect their own agent with a read-only token to your own server (7.4).

### 7.7 What stays easy (acceptance)

- At most three commands from a new computer to the first posted entry, with no administrator password beyond the Linux package manager's usual one.
- Nothing to do at the start of a session.
- Everything works from any folder.
- The viewer catches up within a second.
- No new permission pop-ups in daily work.
- `wanderbooks doctor` shows on one screen: the server, your books, the connected apps, the viewer, the last backup, the books' fingerprint, and how private the books are.
- A test proves that an agent that knows nothing about the books can finish the everyday jobs just by following what the books tell it to do next.

---

## 8. Phased milestones

Phases are labelled P0 to P13, with P4b (receivables and payables) between P4 and P5. The labels are load-bearing in commits, `plan/STATUS.md` and the decisions, so they never change; the order of work does, and 8.0 is where it is stated. A reader who wants to know what to build next reads 8.0 and then the unit's own section in 8.2. Section 8.1 is history. Section 8.3 is the P4b reference design the S4 to S7 units point into. Section 8.4 is what is deliberately not scheduled.

### 8.0 How the work is organized

#### 8.0.1 Status and dependency table (2026-10-26)

One row per unit of work. A unit is what one engineer takes into one git worktree and brings back as one or a few reviewed pull requests. "Depends on" lists the units that must be merged into `main` first. "Beside" lists units that may run at the same time in other worktrees because they share no exclusive file (8.0.3 lists the shared files every unit appends to by rule). "Go-ahead" is the owner's format upgrade a unit's writes need on the live books (D-39); merging never waits for it.

| Unit | Title | Status | Depends on | Beside | Go-ahead | Size (engineer-days) |
|---|---|---|---|---|---|---|
| P0 | Guard rails | landed 2026-10-17 | | | | |
| P1 | Journal-native settings and conventions | landed 2026-10-17 | | | | |
| P2 | One executable, open while held, the cutover | landed 2026-10-20 (slices 1-9) | | | | |
| P3 | Engine correctness | landed 2026-10-22 | | | | |
| Projects | Projects as classes with facts (D-43) | landed 2026-10-22 | | | | |
| P4b S1 | Documents read their figures from standing lines | landed 2026-10-24 (`d7fa16e9`, `07f213c9`); feature `document_standing` declared on the live books 2026-10-24 | | | | |
| P4b S2 | Read tools and viewer pages | landed 2026-10-24 (`7d2bf0b7`, `b7b7f171`) | | | | |
| P4b S3 | The invoice PDF | landed 2026-10-24 (`c1f9001c`) | | | | |
| U0a | Split `server.rs` by tool group | landed 2026-10-27 (`45396bdc`) | none | U0b, U0c, S4.1, S4.2, S7.1 (core steps only) | none | 3-4 |
| U0b | The attention engine over `PackageBooks` | not started | none | U0a, U0c, S4.1, S4.2 | none | 4-6 |
| U0c | `docs/testing.md` sections for S1-B, S2, S3; `docs/mcp-interface.md` cleanup of stale P4b notes | landed 2026-10-26 (`198aa2a1`) | none | everything | none | 0.5-1 |
| P4b S4 | `documents_v2`: numbers, terms, line accounts, snapshots, preview and confirm, the issued PDF filed, party email and terms, invoicing settings | in progress: S4.1 landed (`13282cf8`), S4.2 landed 2026-10-27 (`bf91c5a7`); S4.3-S4.5 open | S4.1 and S4.2 need nothing; S4.3 needs S4.1, S4.2; S4.4 needs U0a, S4.3; S4.5 needs S4.4 | S5 (after S4.2), S7 (core after S1), P6, P9a, P11a, P13 | `documents_v2` (one upgrade after S7, D-52) | 12-16 |
| P4b S5 | Sent tracking | not started | S1; S4.2 for the defaults (works without it, see the unit) | S4.3 onward, S6 | `documents_v2` | 3-4 |
| P4b S6 | Paying on terms: multi-document payments, discounts, the processor fee, bills due, aging exports | not started | S4.1 (`Terms`), U0a | S5, S7, P6 | `documents_v2` | 8-10 |
| P4b S7 | Bad debts (write-offs) | not started | S1; S4.3 for the same-call evidence attach; U0a | S4.4 onward, S5, S6 | `documents_v2` | 4-6 |
| P4 | Draft states, the evidence gate in one place, the preview digest, the digest tool | not started | S4-S7 merged (one sweep over every confirm tool), U0b | P6, P9a, P11a, P13 | `drafts_v2` (same upgrade as `documents_v2`, D-52) | 8-10 |
| P5 | Agent surface complete: the result envelope, prompts, `books.start` v2, journal history, agent evals | not started | P4, U0b; the envelope step is single-owner and merges when no other MCP branch is open | P6, P9a, P11a, P13 | `source_revisions` | 10-12 |
| P6 | Evidence pipeline: extraction, dedup, annotations, company record categories | not started | U0a, U0b | S4-S7, P4, P5 (not its envelope step), P9a, P11a, P13 | `attachment_text` | 8-10 |
| P7 | The TIN scanner's separators; vault v2 closed (D-48) | landed 2026-10-26 (`3e65c83f`) | none | everything | none | 1 |
| P8 | Audit and attestations | not started | P6 (extraction), P4 (drafts), U0b | P9a, P11a, P13 | `audits` | 10-12 |
| P9a | Self-host recipe, `books adopt` move with the head check, static bearer over a tunnel, the two-user CI job | not started | none (code); a second machine to try it on | everything | none | 3-4 |
| P9b | claude.ai through an external identity provider (D-7 b, Q-6) | not planned until Q-6 is answered | P9a | | | |
| P9c | Remote viewer replica, invitations | not planned (D-51) | | | | |
| P10 | The re-audit runbook and its CI mirror; the owner runs the re-audit | not started | P5, P6, P8 | P11a, P13 | none | 1 (+ owner days) |
| P11a | Packaging and updates: the trial, `wanderbooks update`/`rollback`, Linux packages, signing, macOS and Windows bundles | not started | none (code); certificates and keys from the owner for the signing steps; Q-4 before the manifest step | everything | none | 20-25 |
| P11b | The agent dock and books homes | not started | P11a (an installed app to dock into) | P12 | none | 8-10 |
| P12 | Max testing, again | not started | everything else merged | | none | 2-3 (+ machine time) |
| P13 | bookspec: checker crate, `Engine` trait, rule ids, the spec text, the runner | in progress: steps 1-3 landed 2026-10-26 (`579d0eed`); steps 4-6 open | none for steps 1-4; the engine stable for step 5 | everything | none | 15-20 |

#### 8.0.2 Dependency graph and merge order

Forced order (an arrow means "merge first"):

```
U0a ─┬─> S4.4 ─> S4.5 ─┐
     ├─> S6 ───────────┤
     ├─> S7.2 ─────────┤
     └─> P6 ───────────┼─> P4 ─> P5 ─> P8 ─> P10
U0b ─┬─> P4 (attention items)                ^
     └─> P5, P6, P8                           │
S4.1 ─> S4.3 ─> S4.4 ; S4.1 ─> S6 ; S4.2 ─> S4.3 ; S4.2 ─> S5 (defaults) ; S4.3 ─> S7.2
P6 ─> P8 ; P4 ─> P8
P9a, P11a, P13 (steps 1-4), P7: independent of the chain above
P11a ─> P11b
everything ─> P12
```

The shortest path to P10 (the owner's goal: the whole re-audit through the MCP alone) is U0a, U0b → S4 (S5, S6, S7 beside it) → P4 → P5 → P6 (can start earlier) → P8 → P10. P9a, P11a, P13 and P7 run beside that chain whenever an engineer is free. P11b and P12 come last.

Recommended staffing for a team of five: engineer A on U0a then S4.4/S4.5; B on U0b then P6; C on S4.1/S4.3 then S6; D on S4.2 then S5 then S7; E on P13 steps 1-3, then P9a, then P11a. The EM merges in the order above and holds the three single-owner moments: U0a (before any MCP-touching branch opens), P4's one sweep over the confirm tools, and P5's envelope step.

#### 8.0.3 Shared files, owners, and allocated names

These files are edited by nearly every unit. Each has one owner (the EM, or a named engineer) who merges changes to it; every other engineer appends by the rule in the right column, never reorders, and rebases before merging.

| File | What every unit adds | Rule |
|---|---|---|
| `crates/ledger-mcp/src/server.rs` and `crates/ledger-mcp/src/tools/<group>.rs` | tool handlers | after U0a, a unit adds handlers only in its own group file (`tools/documents.rs`, `tools/drafts.rs`, `tools/evidence.rs`, `tools/audit.rs`, ...); `server.rs` keeps the router and the shared helpers |
| `crates/ledger-mcp/src/types.rs` | params and output structs | append at the end, under a `// <unit>` comment |
| `crates/ledger-mcp/src/scopes.rs` | one row per tool and the array length | append at the end of `TABLE`; bump the length |
| `crates/ledger-store/src/format.rs` | `KNOWN_FEATURES`, `EVENT_TYPES`, `setting_feature` arms | feature and event names are the ones allocated below; append |
| `crates/ledger-store/src/schema.rs` | one migration per unit that changes a table | numbers are taken at merge time: the unit writes its migration as the last entry; the second unit to merge rebases and takes the next number. Provisional numbers below are a guide, not a reservation |
| `crates/ledger-store/src/store.rs` | replay and projection arms, new commands | arms go beside the arms of the same noun (`invoice.*` together) |
| `crates/ledger-store/tests/format.rs`, `fuzz/fuzz_targets/fuzz_store_replay.rs` | the new event type's generator | append |
| `crates/ledger-core/src/settings.rs` | `KEYS` entries and `Kind` arms | append under a `// <unit>` comment |
| `crates/ledger-core/src/kernel.rs`, `crates/ledger-core/src/proofs.rs` | kernel functions and their Kani harnesses | append; a harness slower than a few minutes is named in the justfile's `kani_slow` |
| `verify/lean/Wanderbooks/*.lean`, `verify/lean/Audit.lean` | new `Step` constructors, theorems, `#print axioms` lines | one unit at a time (the EM serializes Lean merges) |
| `crates/ledger-mcp/handbook/*.md`, `handbook.rs::SECTIONS` | a section per procedure | ids allocated below |
| `docs/mcp-interface.md`, `docs/spec/rules.md`, `docs/testing.md`, `docs/parity.md`, `docs/verification.md`, `docs/architecture.md`, `docs/receipt-store.md` | the unit's rows and paragraphs | append in the unit's own section; never rewrite another unit's |
| `plan/served.md` (this file) | the unit's **As built** block and its Status | the engineer writes the As-built block in the PR; the EM updates 8.0.1 |

**Allocated names** (no engineer invents another; a unit that needs one not listed asks the EM, who adds it here first):

| Kind | Name | Unit |
|---|---|---|
| journal feature | `documents_v2` | S4, S5, S6, S7 (one feature for the four; D-52) |
| journal feature | `drafts_v2` | P4 |
| journal feature | `source_revisions` | P5 |
| journal feature | `attachment_text` | P6 |
| journal feature | `audits` | P8 |
| event type | `invoice.sent` | S5 |
| event type | `invoice.written_off` | S7 |
| event type | `entry.draft_state_set`, `entry.draft_discarded` | P4 |
| event type | `convention.source_revised`, `decision.source_revised`, `open_item.source_revised` | P5 |
| event type | `attachment.text_extracted`, `attachment.annotated`, `entry.settles` | P6 |
| event type | `audit.run`, `receipt.attested`, `entry.reviewed`, `period.attested` | P8 |
| new fields on existing events | `invoice.created`/`bill.created`: `number`, `terms`, `bill_to`, `seller`, `customer_ref`, `memo`, `vendor_ref`, `received_on`, `duplicate_reason`; lines: `account`, `quantity`, `unit_price` | S4 |
| new fields on existing events | `party.updated` field `email`, `default_terms_as_customer`, `default_terms_as_vendor` | S4 |
| new fields on existing events | `invoice.payment_received`/`bill.paid`: `line`, `discount`, `discount_line`, `discount_reason`, `fee`, `fee_line` | S6 |
| projection schema (provisional) | 16 S4, 17 S5, 18 S6, 19 P4, 20 P6, 21 P8 | taken at merge, 8.0.3 rule |
| setting keys | `invoicing.number_prefix`, `invoicing.first_number`, `invoicing.default_terms`, `invoicing.payment_methods`, `invoicing.footer`, `invoicing.unsent_days`, `invoicing.reminder_days`, `bills.remind_days`, `bills.discount_warn_days`, `company.email`, `company.website` | S4 (S5 and S6 read the day counts) |
| setting key | `policy.attestation_bar` | P8 |
| roles | `bad_debts` (6990), `purchase_discounts` (6995), `sales_allowances` (4100, existing account), `payment_processing_fees` (the account payouts' fee lines already use) | S7, S6 |
| MCP tools, new | `books.mark_sent` (S5); `books.bills_due` (S6); `books.write_off_invoice` (S7); `books.preview_entry`, `books.list_drafts`, `books.set_draft_state`, `books.discard_draft` (P4); `books.journal_history` (P5); `books.find_attachment`, `books.annotate_attachment`, `books.consolidate_duplicates` (P6); `books.audit`, `books.workpaper`, `books.attest_receipt`, `books.review_entry`, `books.attest_period`, `books.attestations` (P8) | |
| MCP tools, changed | `books.create_invoice`, `books.create_bill`, `books.update_party` (S4); `books.receive_payment`, `books.pay_bill`, `books.ar_aging`, `books.ap_aging`, `books.export` (S6); `books.post_entry`, `books.reverse_entry`, `books.digest`, every tool with `confirmed` (P4); `books.start`, `books.set_convention`, `books.record_decision`, `books.raise_item` (P5); `books.get_receipt`, `books.attach_receipt`, `books.file_document` (P6); `books.export_cpa_package`, `books.verify` (P8) | |
| handbook sections | `invoices-and-bills` (S4; S5, S6, S7 add their steps to it), `drafts-and-reversals` rewritten (P4), `evidence-and-receipts` extended (P6), `attestations-and-audit` (P8), `self-host` (P9a) | |
| rules.md | B-39 terms (S4), B-40 numbers and the issued copy (S4), B-41 write-offs (S7), B-42 settlements and the fee (S6), B-43 drafts and evidence (P4), B-44 extraction and dedup (P6), B-45 audits and attestations (P8) | |
| test files | `crates/ledger-cli/tests/documents.rs` (S4-S7 over the wire), `documents_parity.rs` (viewer equals server), `crates/ledger-core/tests/spec_documents.rs` (exists; grows), `crates/ledger-cli/tests/agent_evals/` (P5), `crates/ledger-cli/tests/audit.rs` (P8) | |

#### 8.0.4 Every unit's bar, and the unit template

Every unit, before its last pull request merges:
- passes `just check` (format, clippy with `-D warnings`, rustdoc with `-D warnings`, every test) and `just prove` (Kani with no vacuous cover; Lean with standard axioms only, no `sorry`); a unit that adds a kernel function or a `Ledger` command adds its Kani harness and Lean step (AGENTS.md, "Formal methods");
- updates, in the same pull request as the code: `docs/parity.md`, `docs/spec/rules.md` (SOURCE / CONVENTION / DECISION lines), `docs/mcp-interface.md` (a test fails otherwise), `docs/testing.md` (a **For P12** paragraph: D-33 paused mutation testing until P12, so each unit records what P12 must mutate), `docs/verification.md` (every new harness and theorem), `docs/architecture.md` where a crate's shape changes;
- builds its feature whole: core, store, MCP, viewer, exports, handbook, tests; a unit is not done with the MCP tool and no viewer page, or the viewer page and no test;
- is reviewed until a review round comes back clean (the reviewer reads the diff against the unit's section here and against AGENTS.md; findings are fixed, not argued away);
- writes its **As built** block into this file (the template's last field) and the EM moves its row in 8.0.1 to "landed".

The owner's live books (the first company's) keep working between units: a unit's writes that need a new journal feature are refused on books that have not declared it, with the `format_upgrade_pending` remedy, until the owner's go-ahead (D-39); everything else in the unit works at once. Merging never waits for the go-ahead.

**The template.** Every remaining unit in 8.2 has these eight fields, in this order; a field with nothing to say reads "none".

1. **Status**: not started / in progress (branch, engineer) / landed (commit, date).
2. **Depends on**: units merged first; the owner go-ahead, if any.
3. **Feature and schema**: the allocated feature name(s), event type(s) and provisional schema number, or none.
4. **Build**: numbered steps, each a thing that compiles and passes `just check` on its own; each bullet ends with where it lives, as `crate/path/file.rs::symbol (new | changed)`. Core, store, MCP, viewer, docs sub-steps appear only where the unit has them.
5. **Files**: owns (exclusive) / also touches (the 8.0.3 files, by their rules).
6. **Tests and proofs**: every test named, with the file it goes in; the Kani harnesses and Lean theorems; the bookspec example ids.
7. **Done when**: a numbered list the EM can check without the owner's books.
8. **As built** (date, commit): departures from Build, one bullet each, citing the decision or review that caused it. The Build text is struck through where it changed (`~~text~~ → As built`), never rewritten.

#### 8.0.5 Recipes (the mechanical steps every unit repeats)

**R-A A new journal feature.** (1) `crates/ledger-store/src/format.rs`: `pub const NAME: &str = "name";` with a doc comment saying what an older version would misread, in the style of `DOCUMENT_STANDING`; append to `KNOWN_FEATURES` (`Store::ensure_feature` has `debug_assert!(KNOWN_FEATURES.contains(..))`). (2) A unit test `name_is_its_own_feature` beside `document_standing_is_its_own_feature`: `declared_knowing` with every earlier feature gives `NewerJournal`; `require_feature` gives `FormatUpgradePending`; `upgrade().adds()` names it. (3) Every `Store` command that writes a line the feature marks calls `self.ensure_feature(NAME)` before `rehearse` and before anything is journaled; a store test `<write>_waits_for_<name>` modelled on `repaying_after_a_reversal_waits_for_document_standing` (`crates/ledger-store/tests/arap_standing.rs`): refused, journal length unchanged, succeeds after `upgrade`, replay equals live. (4) `crates/ledger-cli/tests/format_upgrade.rs::books_p2_served_cut_over_whole`: one entry in its `refused` list and the same call made after consent. (5) `crates/ledger-mcp/handbook/format-upgrade.md` and rules.md B-27 list the feature.

**R-B A new event type.** (1) `format.rs::EVENT_TYPES` (alphabetical; `crates/ledger-store/tests/format.rs::every_event_the_store_replays_passes_the_gate` scans store.rs for every `"x.y" =>` arm and fails on a mismatch in either direction). (2) A payload reader in `crates/ledger-store/src/meta.rs` in the style of `project_set_of`: a missing or mistyped key → `corrupt(..)`; an enum value this version doesn't know → `format::newer_content(..)` (B-27: never "corrupt" for newer). (3) A `replay_line` arm in `store.rs` calling a `Ledger::replay_*` method (`line.seq` is available, as `apply_settings` uses it). (4) An `apply_line_rows` arm (projection SQL); a migration in `schema.rs` and `SCHEMA_VERSION` + 1 only when a table or column changes (8.0.3's numbering rule); `crates/ledger-store/tests/schema_files.rs` proves the old database file is left alone and a new one built. (5) `projection_rows` and `compare_projections` list any new table; `Ledger::differs_from` gains the new state (the `("line_docs", ..)` pattern), or Store-vs-Snapshot parity cannot see it. (6) `fuzz/fuzz_targets/fuzz_store_replay.rs::TYPES` gains the type (`the_replay_fuzzer_writes_every_event_type` fails otherwise). (7) A `describe` arm in `crates/ledger-gui/src/pages/journal.rs`. (8) The event table in `docs/receipt-store.md` and the rule in `docs/spec/rules.md`.

**R-C A new or changed MCP tool** (one commit; three tests fail on a partial state). (1) `crates/ledger-mcp/src/types.rs`: params and output structs with `JsonSchema` and doc comments (they are the schema descriptions the agent reads). (2) The handler in the unit's `tools/<group>.rs` with `#[tool(name = "books.x", title = "...", description = "...", annotations(read_only_hint, destructive_hint, open_world_hint))]` (`annotations_complete` checks every tool). (3) `crates/ledger-mcp/src/scopes.rs::TABLE`: a row `("books.x", <Needs>)` using the same `Needs` value the neighbouring rows use for that kind of tool, and the array length constant (a compile error otherwise). (4) `docs/mcp-interface.md`: an entry in the shape `` `books.x { a, b? } → { c }` `` under the right "Tools —" heading (`every_tool_is_documented_and_every_documented_tool_exists`). (5) The description at most 700 characters (`tool_descriptions_fit_700_characters` pins how many exceed it today; never raise that number) and `tool_text_lint` (no `just`, no toml, no path). (6) An over-the-wire test in `crates/ledger-cli/tests/<unit>.rs` through `common::Books::new(tag)` → `.wire()` → `.call("books.x", json!({..}))` → `answer_of` (the harness in `crates/ledger-cli/tests/common/mod.rs`; files by `upload_args` and `Wire::upload`, handed files by `Wire::fetch`). (7) A `docs/parity.md` row where the tool is a feature.

**R-D A new setting key.** (1) `crates/ledger-core/src/settings.rs::KEYS`: a `Key { name, toml, kind, sensitivity, doc }` entry (pattern `company.duns`). A nine-digit (TIN-shaped) exemption is a `Kind` arm of its own, never a key-name match: `no_tin` runs for every text kind, and `Kind::Duns` escapes by having its own arm. (2) A `setting_feature` arm in `crates/ledger-store/src/format.rs` naming the feature the key needs (`Store::journal_settings` calls it; the `"settings.set"` replay arm refuses an unknown key as `newer_content`). (3) `ledger-config`'s `toml_round_trip_zero_events` and `codec_props.rs` pick the key up; `crates/ledger-core/tests/settings_units.rs` for the kind's canonical form. (4) The settings.rs module INVARIANTS block, where the key has a rule of its own.

**R-E A new handbook section.** `crates/ledger-mcp/handbook/<id>.md` plus a tuple in `crates/ledger-mcp/src/handbook.rs::SECTIONS` (`include_str!`); text that differs before and after the owner's upgrade uses a placeholder rendered by `section(..)`, as `{document_projects}` does through `document_projects_rule(declared)`. The tests in handbook.rs check every section renders and every id is unique. `books.start` lists the sections and `books://handbook/{id}` serves them.

**R-F A new Needs-attention item.** After U0b: one function in `ledger-package`'s attention module, read by the viewer and by `books.start`'s next steps alike; the parity test `crates/ledger-gui/tests/view_model.rs` and a screenshot through `shot()` (`crates/ledger-gui/tests/invoices.rs` shows how). Before U0b (only U0a and U0c may land before it): do not add attention items.

**R-G The For-P12 paragraph.** In `docs/testing.md`, under the unit's heading, `**For P12 (mutation targets):**` naming each new function and the arms a mutant would flip (pattern: the P4b S1 section's paragraph). D-33.

#### 8.0.6 Before a journal-touching unit merges

A unit that adds a feature, an event type or a field on an event (S4, S5, S6, S7, P4, P5, P6, P8) proves, before merge, that the owner's live books cannot be hurt by it:
1. `crates/ledger-store/tests/store.rs::books_written_under_older_rules_still_open` gains a hand-written line of the new shape through `JournalWriter`, and still opens every older fixture.
2. A new fixture directory under `crates/ledger-store/tests/fixtures/old-books/` written by the version *before* the unit (the `old-books-writer` pattern), with its `expected.json`; the unit's version opens it and reports the same figures.
3. The `books_p2_served_cut_over_whole` entry (R-A step 4): the write is refused before consent and accepted after.
4. The `_is_its_own_feature` test (R-A step 2) and the `tests/format.rs` arms (R-B).
5. Where a figure could move (a report, an aging, a status), a `figures.json` golden captured by the previous version (pattern: `crates/ledger-store/tests/fixtures/p4-arap-before` and `arap_standing.rs::write_the_p4_arap_fixture`), and a named test that adjudicates every difference. S4 to S7 and P4 move no figure: a golden change is a stop, not an update.
6. The statement, in the PR description: "merge does not need the owner's go-ahead; the live books refuse the new write until he gives it."

#### 8.0.7 Working in a worktree

- One worktree per unit (`git worktree add ../wb-<unit> -b <unit>`), branched from `main` on the day it starts; rebase on `main` before each PR.
- Conventional commits, one logical change each, the message explaining the accounting (AGENTS.md). The unit's id in the subject: `feat(arap): terms and due dates (P4b S4.1)`.
- A PR is one step or a few steps of the unit's Build list; its description names the steps, the tests added, the docs touched, and the 8.0.6 checklist where it applies.
- The engineer never runs `cargo mutants` (D-33), never edits a golden file without recomputing it by hand and saying so in the commit, never touches the owner's books (the only books an engineer uses are the test fixtures and books made by `wanderbooks setup` in a scratch root), and never changes `plan/served.md` beyond the unit's own As-built block.
- Stuck for more than an hour on a question the plan does not answer: write the question and the two or three answers considered into the PR, pick the one that keeps the books right under the principles in section 2, and flag it for the EM. Do not invent a journal feature, event type, setting key or tool name: ask the EM, who adds it to 8.0.3.

### 8.1 Landed

History, kept for the record. Each landed phase keeps its original Scope / Tests / Accept text and its **As built** notes; the Status line is the only addition. New readers need only the Status lines and `plan/STATUS.md`.

#### P0: Guard rails (days)

**Status:** landed 2026-10-17 (`391634a`).

**Scope:**
- `import_local_file` allowlist (defect 19, interim).
- `NewerJournal` and `journal.format_set` reader support.
- `title` plus a read-only or destructive hint on all 93 tools, with the corrections in 4.9. No `requiresUserInteraction` on any tool (owner decision 2026-10-16: zero added friction; TIN handling moves to human pages in P2 and P7).
- `get_info` calls `refresh()`.
- Bare `books://digest` works.
- `INSTRUCTIONS` is replaced by an interim connect text (≤ 1,400 characters) plus `books.handbook` and handbook resources holding the scrubbed old text by section.
- `tokio::sync::Mutex` plus `spawn_blocking` (S:553).
- Pin `cargo-mutants` and `cargo-fuzz` in `mise.toml:21-22`.
- Remove the duplicate Transports section (`docs/mcp-interface.md:78/:728`) and the bearer-token claim.
- Template text fix ("trace through tools").
- Capture the first company goldens (5.6).

**Tests:** `import_local_file_refuses_key_and_journal` (property); `annotations_complete` (every tool has a title and a hint, and none carries `requiresUserInteraction`); `instructions_budget`; `tool_text_lint`; `digest_resource_bare_reads`; `old_reader_gets_newer_journal`.

**Accept:**
- A live Claude Code session receives the connect text untruncated.
- The first company's daily flow is unchanged, with no new permission prompts.

#### P1: Journal-native settings and conventions (B, part 1)

**Status:** landed 2026-10-17; live on the owner's books the same day. **[Scope below is the 2026-10-16 design; `ledger-view` → D-53 (U0b, over `PackageBooks`).]**

**Scope:**
- `ledger-core/src/settings.rs` and `conventions.rs`.
- Events `settings.*`, `convention.*`, `decision.recorded`, `open_item.*`.
- `Ledger` fold, so Snapshot and Store share `Settings`.
- `ledger-config` becomes the codec.
- Tools: `settings`, `set_setting`, `import_settings`, `export_settings`, conventions, decisions, open items, `draft_from_template`.
- Approval-rule conventions: supported by the engine for companies that want them, off by default (`policy.approval_rules = off`). They are enforced at post only after a company turns them on. The the first company import creates none (owner decision 2026-10-16: light governance; decisions are recorded with their source instead).
- `books.start` v1.
- `company.timezone` drives `today()`.
- New crate `ledger-view`: `attention::find` moved from `ledger-gui/src/attention.rs:68-571` together with its helpers (`books.rs:643-650` `missing_receipt`, `depreciation_due`, `w9_status`, `reconciliation_due`, `party_infos`), over a `BooksView` trait.
- GUI reads settings from the Snapshot; journal-page `describe` arms (`pages/journal.rs:825`); Company page shows provenance; copy fixes `company.rs:59`, `cash.rs:51`, `attention.rs:562`; the "due to" heuristic uses `financing_accounts` (`attention.rs:349`).
- (`dev-stdio` kept `legacy_files` mode, until D-44.)
- The first company P1 steps.

**Defects:** none directly. P1 enables 8 and 16.

**Proofs and tests:**
- Lean `Step.setSetting` preservation (balances and `draftsOpen` unchanged).
- `settings_replay_is_pure` (2,000 cases, Store = Snapshot = view).
- `toml_round_trip_zero_events`.
- `fuzz_settings_replay`.
- `attention_server_equals_gui` (golden on `fixture_company`).
- Sensitivity: a TIN-shaped value is refused outside `company.ein`.
- `approval_rules_off_by_default`: with an `approval_rule` convention present and the default policy, a post succeeds without an approval; with `enforce`, it is refused until one is given.

**Accept:**
- The first company goldens are byte-identical before and after the import.
- The GUI is correct with the toml renamed away.
- `books.start` v1 is at most 8 KB.

**As built (2026-10-17), and where it departs from the scope above (engineering calls):**
- One event per change set: `settings.set {changes: [{key, value|null}], source}` is all or nothing (an import is one line); there is no separate `settings.cleared` (a null value clears). `effective_from` is not built: no key needs it yet, and effective-dated members are the varying-interest rule (§706(d)), which changes figures and belongs with the P3 allocation work. A later format adds it.
- Keys are flat (`company.formation_state`, `company.irs_name_control`, ...), not one `company.legal{}` object: each fact gets its own source, which is what "facts carry their source" asks. `company.members` keeps member names (matched to parties as before); `company.cpa` is text. `policy.capitalization`, `require_tax_category`, `attestation_bar`, `cpa_package_requires_audit`, `audit_cadence` and `books.schedule_convention` arrive with the phases that use them (P3, P8).
- ~~`ledger-view`~~ → D-53, U0b (`attention::find` moved out of the viewer) ~~moves to P5~~, where its first user (`books.start`'s needs_attention, `next_steps`) lands; P1 has no server-side consumer, and moving 700 lines of the viewer for nothing would be risk without use. `attention_server_equals_gui` goes with it.
- The "due to" heuristic keeps reading account names: `financing_accounts` also holds bank loans, which aren't "paid personally".
- Approval rules are checked by the engine (`Ledger::approvals_missing`) and enforced by the store: `Store::post_entry` refuses a covered draft until approved, and the operations that post at once (invoices, bills, payments, payroll, remittances, disposals) are refused when a rule covers the entry they would post (`approval_gate`: draft it instead). Reversals, closes, depreciation and amortization are exempt. An approval records the draft's fingerprint (date, description, lines, classes); any change voids it. A rule names accounts that take postings, and an account a rule names can't be renumbered until the rule is revised.
- `books.import_settings` took only the file the server read, whole (compared key by key with the server's own reading), never turned approval rules on, and on books that keep their own settings changed nothing (review round 1: an agent-transcribed text could drop the EIN). D-44 removed it.
- `books.start` v1 leaves `book.id`, `book.url`, `head.last_handoff`, the `agent` input and `alwaysLoad` for P5 (they need the served book identity and tool search).
- Posting templates take slots (`{expense}`): an account each draft names, so "an expense a member paid: Dr <expense> / Cr 2100" is one template.
- A vault key file is never a books setting: `--vault-key-file` on the server and the viewer (both gone with D-44: a book keeps its own key), and an adoption is refused unless the key it names is the one given.
- The connect text's lead names `books.start` first, then the handbook.

#### P2: One executable, open while held, hidden away, and the first company's cutover

**Status:** landed 2026-10-18 to 2026-10-20 (slices 1-9); the cutover 2026-10-20. **[Scope below is the 2026-10-16 design; D-31 (no archive process, no encryption), D-32 (no confinement, no helper) and the As-built notes in 3.3, 3.6, 3.7, 3.9 and 4.7 are what exists.]**

**Scope:**
- `ledger-mcp serve --root` with multiple books at `/b/{book}/mcp`; explicit transport config (3.2); bearer tokens and scopes; rate limits (tower-governor).
- **One executable (D-22).** `wanderbooks` is the only program: a thin dispatcher over the existing crates whose modes are the viewer (`gui`), the books server, the host, the agent bridge (`mcp`), the archive process, the extractor, and the CLI commands. Each mode that runs as a process confines itself at start, after forking its unconfined helper (3.9) and before it opens anything. The viewer, on Linux, confines itself read-only on the data root, with writes allowed only to its preferences folder and the Downloads folder (exports, Save a copy); Open-with and the updater go through its helper. A `gui` cargo feature, off in the self-host server build (P9), leaves the window libraries out. P11 then only packages it.
- **Open while held, no service, no sudo (D-21, D-23).** `wanderbooks setup` creates the data root (3.4) with an explicit `set_permissions(0o700)` (an owner-only DACL on Windows) and sets the owner passphrase. The books open when something holds them and close when nothing does, as section 3.9 describes: leases on the host's `run/lease.sock`, the helper's clean start of the host, `host.lock`, the 10 s close, "close the books", restarts, one version at a time, the bridge's own `initialize`, and the server's `format_upgrade_pending` gate. Nothing is installed or persistent: no systemd unit file, no LaunchAgent, no socket activation, no login autostart (the transient scope of 3.9 disappears with the host), and nothing runs after the last holder closes. There is no system account, no container and no root step.
- **Honest isolation label.** `books.start`, Needs attention, `wanderbooks doctor` and the docs report `isolation: "same_user (advisory)"`. There is no `--same-user` flag and no acknowledgement gate.
- **Self-confinement.** Where available: on the server, Landlock (filesystem limited to the data root; on ABI ≥ 4, all TCP bind and connect denied once it has bound its own listening socket on 127.0.0.1) and seccomp; on the archive process, read-only Landlock on the journal and blobs, TCP connect limited to the target's port (443 or 22) on ABI ≥ 4, and seccomp. An unprivileged process cannot filter egress by host. If they cannot be applied, `doctor` reports it; it is not fatal. The docs describe them as protecting the human's account from a compromised server, not the books from the agent.
- Access log plus `books.access_log`.
- Owner passphrase for `/h/` (Argon2id).
- **Files transport:** `begin_upload`, the PUT route, the `/h/inbox` drop page plus `books.inbox` and the `process_inbox` prompt, `download_link`.
- **Every server-path parameter deleted:** `import_local_file`, `save_to`, `save_csv_to`, `w9_file_path`, get_w9 `save_to`. W-9 upload moves to `/h/w9/{party}` (sealed with the v1 key until P7). TIN-bearing outputs come only through `request_human_download` → `/h/`, behind the owner passphrase. This closes defect 19.
- `get_receipt view=link|bytes` (defect 3, transport part; bytes is a download since D-46, never inline).
- **Viewer, direct and read-only (3.6):** data-root discovery through `ledger-store::paths::data_root()` or `--root` from `wanderbooks gui <book>`, never a project directory or `wanderbooks.toml`; `books.show` through `ops/show.jsonl` (delete `ledger-store/src/viewer.rs`); presence through the `ops/viewer.lock` shared flock; Save-a-copy and Open-with (defect 3, viewer part). Locally there is no replica; the viewer's only channels are the lease socket's owner intents and its helper (3.9).
- Archive process and module; `books.backup`; `ops/heads.jsonl`; `verify-archive`; `fetch`. (As built, slice 7, D-31: the server's backup module, `books.backup` and `books.back_up_now`; no archive process, `ops/heads.jsonl`, `verify-archive` or `fetch`: a backup is a repository, cloned and proven by a restore's preview and the drills.)
- **Restore (3.7, D-19):** `books restore`, proven before use, never destructive, onto a new or an existing install; restore drills; the vault and its key travel in the backup (D-31).
- **Viewer menu (D-19):** "Export for CPA" (done by the viewer, into Downloads); "Back up now", "Restore…", "New books…", "Close the books" and format-upgrade consent handed over as the lease socket's owner intents (3.9); "Connect Claude Code" and "Connect Codex". Nothing in the viewer writes a book.
- `wanderbooks mcp`, the agent bridge (D-23, 3.9): stdio to the agent, MCP over `run/mcp.sock` to the local server (or HTTPS with `--url`, P9), the token from the keyring. It opens the books if they are closed and holds a lease for the session. Across a server restart (an update) it reconnects and re-initializes upstream without the agent noticing, sending `notifications/tools/list_changed` only if the tool list changed.
- CLI subcommands and modes: `setup`, `serve`, `close`, `mcp`, `host`, `helper`, `rollback`, `books create|adopt|list|export-legacy`, `connect`, `token`, `gui`, `doctor`, `verify-archive`, `fetch`, `update`. `books export-legacy <id>` (rollback only) writes a P1-format data dir outside the data root, verifies its head equals the server's, and stops the server's writes to that book.
- **Scaffold deleted:** `JUSTFILE`, `AGENTS_MD`, `CLAUDE_SETTINGS`, `CODEX_HOOKS`, `GITIGNORE` (`ledger-cli/src/main.rs:43-521`); `install.sh` rewritten, with no sudo.
- The test harness drives HTTP (`mcp_journeys.rs` keeps the bridge).
- **Two-user CI deployment:** a container that runs the server as a dedicated user and the agent as another, in the configuration P9 turns into the self-host recipe. It is the target of `agent_uid_cannot_read_tenant`.
- The first company cutover, preceded by the two open cutover questions (10.3).

**Defects:** 19 (closed), 3 (partial).

**Tests:**
- `agent_uid_cannot_read_tenant` (the two-user CI deployment; not the desktop default).
- `setup_needs_no_root` (setup, serve, adopt, connect and gui as an unprivileged user in a clean container; nothing is left installed to run in the background).
- `books_close_cleanly` (the last lease ending, "close the books", SIGTERM and SIGHUP of the host, all during a burst of posts: every request either completes or is refused, the head verifies at the next open, the lock is released); `openers_share_one_server` (the app, `serve`, a CLI command and two bridges started together: one host, one writer); `books_close_with_last_holder` (each kind of holder ending, `kill -9` included: closed 10 s after the last, not before); `bridge_opens_closed_books` and `bridge_survives_server_restart` (the agent's session carries on, no reconnect); `close_now_ends_every_lease`; `host_starts_clean` (started from a confined viewer or bridge with a hostile environment, `XDG_DATA_HOME` and `LD_PRELOAD` included, the host is unconfined, finds the real root, and holds no inherited descriptor); `host_respawns` (kill the host: a holder starts a new one); `host_survives_app_scope` (under a systemd app scope, as uwsm sets up, closing the app that started the host leaves it running for the other holders); `no_local_agent_tcp` (the bridge never connects to TCP; TCP is off by default; with it on, a squatter on 7411 makes `doctor` report a failed bind and sees no bridge traffic); `lease_socket_accepts_only_owner_intents` (exactly the five; consent without the passphrase refused); `restart_is_not_close` (bridges carry on across a restart; openers wait on `update.lock`); `children_match_host_version` (replace the executable mid-run: no child of the new version joins the old host); `format_upgrade_waits_for_consent` (read-only until the owner's consent, a backup with the keys made first, no MCP tool can consent, declining leaves the book openable after `wanderbooks rollback`); `no_rollback_after_consent` ("Go back" refuses for an upgraded book; the pre-upgrade restore arrives beside it, states the entries it lacks, and needs the passphrase); `restore_reads_only_staged_backups` (superseded by D-31: the `restore` intent names the backup's folder or address); `bridge_initializes_during_cold_start`; `archive_catches_up_at_start` (a push that failed before a close is made at the next open).
- `one_executable` (every mode from one binary; the viewer mode under read-only Landlock opens and follows a book); `server_build_has_no_gui` (the self-host build links no window libraries).
- `viewer_shows_the_engines_numbers` (the viewer's two musts, owner 2026-10-18: it never writes the books, and every figure it shows is correct): on random books, every report page, account register, entry and Needs-attention count the viewer renders equals the engine's report for the same books and dates, extending `tests/view_model.rs` from the overview to every page. No mutation runs on the viewer.
- `viewer_without_server` (with the server stopped or failing to start, the viewer still opens and follows the books from disk; "Restore…" and "New books…" say why they can't run).
- `isolation_label_is_advisory`.
- `no_path_params` schema lint; `no_result_names_a_path`.
- `no_tool_touches_outside_tenant`.
- `security_*` (auth, Origin, Host, body limit, grants, upload sha).
- `access_log_one_record_per_call`.
- `gui_works_under_read_only_landlock`; `gui_finds_root_without_project_dir` (started in an empty directory with no toml); `show_request_reaches_gui` (≤ 1 s); `viewer_presence_reported`.
- `clone_verify_expect_head`; `backup_does_not_advance_head`.
- `restore_on_clean_install` and `restore_beside_existing_books` (same head, identical goldens); `restore_refuses_damaged_backup` (a flipped byte in the journal, a missing blob, a head that doesn't match: each refused, nothing written); `restore_never_overwrites` (replace moves the old book to `legacy/`); `restore_drill_failure_is_flagged`; `restored_vault_needs_its_key`.
- `viewer_menu_never_writes_a_book` (backup and export leave the data root byte-identical; restore and new books go through the server).
- `http_concurrency` extended to 3 books × 6 clients while the GUI reads.
- `upload_round_trip_byte_exact` (20 MiB).
- GUI `read_only`: no writes under the data root, and no files under `$HOME` except eframe preferences.
- `fuzz_http_mcp_request`, `fuzz_upload_put`.

**Accept:**
- The 5.6 P2 checks.
- A clean machine reaches the first post in at most 3 commands, and no step asks for sudo.
- Claude Code from `~`, and Codex, each recorded under its own principal.
- A Codex post, close and reconcile raise no approval prompt beyond Codex's defaults (no `default_tools_approval_mode` in the generated config).
- Two concurrent agents post 12 entries each with no lost writes.
- Viewer updates within 1 s.
- A backup of the first company restores on a clean machine and beside the live books, each with the same head and byte-identical goldens.

#### P3: Engine correctness (defects 1, 7, 8, 9, 11 to 18, 21 to 24)

**Status:** landed 2026-10-20 to 2026-10-22; live on the owner's books 2026-10-22.

**Defect 1, cash flow** (`ledger-core/src/reports.rs:1231-1560`):
- **Partition.** Every non-cash balance-sheet account belongs to exactly one section: investing (fixed-asset cost and accumulated depreciation, plus `investing_accounts`), financing (equity, plus `financing_accounts`), or operating (all others, including P&L).
- **Scope of the body.** Only entries that touch a cash account. Entries with no cash line go to `noncash_disclosures` (ASC 230-10-50-3/4), for example a member's property contribution (Dr equipment / Cr capital).
- **Section totals.** Each section's total is minus the movement of its accounts within cash-touching entries. Gain or loss on disposal of investing assets is attributed to investing (named rule, test `disposal_proceeds_all_investing`).
- **Lines.** Gross direct-method lines, one per cash-touching entry and section, including credits, so disposal proceeds count.
- **Reversals.** An original and its in-period reversal net out of the gross lines. Reversal lines are labelled "Reversal of #N", never "Owner distributions" (:1437).
- **Supplementary schedule.** `operating_reconciliation` (net income to operating cash), required when the direct method is used (ASC 230-10-45-30; sourced in `rules.md`).
- **Footing.** `reconciles {sections_sum, net_change, difference}`. A nonzero difference is `CashFlowDoesNotFoot`, an engine error that is never output.
- **Description.** Tool text (S:2202) is corrected.
- **Proof.** Lean `cashflow_sections_sum_to_cash_change` in `Statements.lean`, derived from the per-entry balance law. Property test (5,000 cases). Kani on the section-sum kernel function.
- **The first company.** FY2026 must foot (it didn't: its figures are in the company's private change log); the golden is recomputed by hand.
- **As built (2026-10-20).** `ledger-core/src/cashflow.rs` (re-exported from `reports`), the kernel's `take`, `apply_cash`, `entry_flows`, `constructive` and `exchanged`. The section rule is refined by ASC 230-10-50-5 ("only the cash portion"): each entry's cash is applied to the sections moving against it, operating, investing, financing, each up to its own movement (CONVENTION: the order), and the rest is disclosed (rules.md B-14 rule 3); the plan's wording holds exactly for entries whose cash settles all they move (`prop_pure_entries_sections_are_movements`). D-35's layout (operating by class, an entry's interest its own class; investing and financing by kind with each kind's total, each transaction under its kind), the bill split and the warnings are built; reversals pair along the chain from its root, and exchanges inside one section are disclosed (the review's two highs). What a member or a lender pays or receives for the company is cash borrowed (or contributed) and spent, on its date, per D-42 (rules.md B-14 rule 11, the kernel's `constructive`, Kani `constructive_nets_to_zero` and `constructive_mirrors`, Lean `constructive_nets`, `constructive_neg`): ASC 230 doesn't address it and the firm guides present it so; an expense a member pays into capital is a constructive contribution (BDO 3.4's parent paying a subsidiary's expenses; the K-1 worksheet already calls it cash); property a member owned, the seller's own credit, and interest or guaranteed payments charged to what the company owes stay noncash. D-35's noncash presentation was replaced the same day. The tool text is at `ledger-mcp/src/server.rs` (`books.cash_flow`). The first company's change log is kept with its goldens, outside this repository (5.6).

**Defect 7, IIF** (`ledger-import/src/iif.rs:169,243`): DOCNUM = entry_id. Output encoded in Windows-1252; characters outside it are transliterated (deunicode) and listed in `warnings`.

**Defect 21, CPA package IIF and as_of** (found in P0, 2026-10-16): the CPA package's `general-journal.iif` ignores `as_of` (an as-of package includes entries dated after it); its xlsx and csv files respect it. The IIF export takes the same `as_of`, and a test pins every file in an as-of package to it.

**Defect 22, depreciation through an asset's final month** (found in P0 review round 4, 2026-10-17): the depreciation schedule plugs to the full depreciable amount whenever the date it is computed through falls in the asset's final month, even mid-month. A disposed asset's schedule therefore shows full depreciation in its disposal month, and mid-final-month figures are overstated. Resolve from the standard (straight-line, partial months prorated by days, as the rest of the schedule already does), then adjudicate the changed figures in the P3 change log. Related, found in the same round and fixed in P0: the final-month true-up compared the month with chrono's `%Y`, so assets whose life ran past 9999 (or before year 0) never trued up. Assets whose life runs past 9999 also can't post months after 9999-12 (periods are bounded to years 1-9999); decide in P3 whether to refuse such lives at registration. Also noted: the viewer recomputes a very long-life asset's whole schedule every frame (slow; pre-existing).

**Defect 23, a vendor marked "no 1099" looks undecided** (found on the first company's books, 2026-10-18): `books.set_vendor_1099` with `form_1099: "none"` stores and returns `null`, the same as a vendor nobody has decided on, so `books.list_vendors`, the 1099 worksheet and Needs attention can't tell "the owner decided: no 1099" from "never asked". Store the decision as its own value (`none`, with who decided and when), and let Needs attention ask about vendors with no decision. P3 (it changes what the 1099 worksheet says).

**Defect 8, tax categories:** `add_account` warns on P&L accounts without a tax category. Needs attention flags all P&L accounts regardless of activity. `policy.require_tax_category` makes it a refusal.

**Defect 9, schedules:**
- `convention: daily|monthly_even`. New schedules use `books.schedule_convention`, default `daily` (D-11, decided). Legacy schedules stay `monthly_even`.
- Shared kernel function `prorate_by_days` with depreciation. Kani: parts sum to the total; each part equals days × rate within 0.01; the last part absorbs the remainder.
- `list_schedules` exposes start, end, convention, next and remaining.

**Defect 11:** `parties` takes `tin_last4` from `party.tin_set` or W-9 facts.

**Defect 12, 1099 worksheet** (`ledger-core/src/form1099.rs`):
- Instruments are added.
- Payments by card or third-party network are excluded and listed under `excluded_card_payments` (IRS Instructions for Forms 1099-MISC and 1099-NEC, payment-card exception; vendored in `docs/spec/sources/`).
- A member's personal cash or check through a financing liability stays reportable, because the entity is the payer.
- An unknown instrument on a financing-routed payment is `undetermined` and raised in Needs attention.
- README wording fixed. The untagged note is quieter: one count plus a tool hint.

**Defect 13, assets:**
- `accumulated_posted` vs `accumulated_scheduled_through(as_of)`.
- `in_service_date` + `acquired_date`.
- `asset.basis_set`:
  - `tax_basis_723` is the carryover basis (IRC §723);
  - `depreciable_basis` = lesser of FMV and adjusted basis for personal-use conversion (Treas. Reg. §1.168(i)-4(b));
  - `built_in_gain_704c = fmv_at_contribution − tax_basis_723` (IRC §704(c), Treas. Reg. §1.704-3), with §704(c)(1)(C) noted when the result is negative;
  - `sec704c_method`.
- `built_in_gain()` (`assets.rs:219`) is redefined to match.

**Defect 14, party shares:** `resolved` uses the line sign convention (debit-positive) plus an explicit `direction`. Totals take `basis: cash|all`, default cash, with the basis labelled.

**Defect 15:** `rename_account`; `show` refuses `start > end`.

**Defect 16:** one `period_defaults(kind, settings, today)` used by tools, the GUI and generated descriptions.

**Defect 17, odd cent:**
- `CloseTo::MembersCumulative`. Each member's cumulative allocation = round(cumulative net × share); each close posts the delta.
- Extends `kernel::apportion` (kernel.rs:309) and replaces `split_first` (`shares.rs:197`) for closes.
- Kani: exact sum; |cumulative − exact| < 0.01.
- Lean `Apportion.lean`: bounded-drift theorem.
- Posted closes never change. The cumulative count starts at inception (D-10, decided), so the first close after the upgrade carries any historical true-up of a cent or two, listed in the P3 change log.

**Defect 18:** `not_reportable{reason}` is distinct from `undetermined`.

**Settled in design (2026-10-20; each design's reviewed recommendation, told to the owner, who may revisit any):** party totals count a member's entry by what moved their capital (so "all" equals the members' equity statement and "cash" means what it means on the K-1); money is cash, cards, the members' financing and loans, and a liability credit never counts as paying that liability's own creditor (defect 24, found in design: accruals through other liabilities counted twice, fixed here); a report asked for without dates covers this fiscal year so far, everywhere, and an account's register opens on it with "Show all dates" one click away; P&L accounts without a tax category warn by default (header accounts exempt), and the first company's books are fully categorized at the P3 cutover and then set to refuse; a new account named like a sibling (ignoring capitals), or holding a nine-digit number, is refused; `books.show` refuses options its target ignores; an asset's acquired date comes from the entry it is linked to; a month of depreciation skipped is caught up in the month the run is made (an immaterial miss; a material one is restated, ASC 250).

**Tests and accept:**
- A named regression test per defect, built from its 2026-10-16 reproduction (anonymized fixture).
- Each of those regression tests is also written as a bookspec example (P13, D-34) as it is built: the rule it shows, its source, the setup, the events and the exact expected results. They are the spec's first examples.
- Goldens recomputed by hand.
- GnuCash differential re-adjudicated.
- 0 unexplained mutation survivors in `ledger-core` and `ledger-import` (run in P12, D-33).
- The the first company change log is adjudicated.


#### Projects: classes with facts (D-43)

**Status:** landed 2026-10-22 (`321b9456`, `3eda95eb`, `528c0706`, `6c587086`). A project is a class with facts (rules.md B-37), four tools (`books.add_project`, `books.update_project`, `books.projects`, `books.project`) in `crates/ledger-mcp/src/tools/projects.rs`, a project page in the viewer with a snapshot of its figures, invoice and bill lines that take a project (feature `document_line_projects`), the IIF carrying each line's project as CLASS. `plan/STATUS.md` "Projects" has the detail.

#### P4b S1: documents read their figures from standing lines (`document_standing`)

**Status:** landed 2026-10-24, part A `d7fa16e9` (core, store, viewer, Lean), part B `07f213c9` (MCP). The owner declared `document_standing` on the live books 2026-10-24.

**As built (2026-10-23 to 2026-10-24):**
- `crates/ledger-core/src/arap.rs`: `DocRef`, `Movement { Origin, Payment, Discount, WriteOff, Adopted, RoleMove }`, `DocTag`, `DocState`, `DocStatus { Open, Partial, Paid, WrittenOff, Void, Overpaid, Unattached }`, `doc_status(..)`, `ControlCheck`, `DocAttention`, `UnattachedLine`, `aging_bucket`, `document_parts`, `validate_due_date`, `validate_lines`.
- `crates/ledger-core/src/ledger.rs`: `Ledger.line_docs` (the tag map), `stands_on(entry, as_of)`, `in_effect`, `doc_state`/`invoice_state`/`bill_state` (the signed sum of the document's tagged lines dated on or before the day; `stands_on` decides only the Void flag), `check_item_bounds` (private, run from `check_reversal` and the payment path), `payment_room`, `check_payment_account`, `check_control_lines`/`check_hand_line` (the control guard with the reversal-mirror exemption), `control_role_of`, `control_check`, `check_void_document`, `void_document`; `reverse_entry_steps` copies each line's tag onto its mirror; `Invoice.amount_paid`, `.owed`, `.status` kept as derived caches (`refresh_doc`, never journaled).
- `crates/ledger-core/src/kernel.rs`: `running_bounds`, `bounds_kept`; Kani `bounds_check_matches_definition`, `bounds_kept_is_the_definition_on_books_in_bounds`.
- Lean `verify/lean/Wanderbooks/Documents.lean`: `control_agrees`, `control_agrees_through`, `Step.document_unattached`, `Step.reverse_moves`, `reverse_twice`, `Reachable.control_is_documents`.
- Store: `ensure_payment_features` (declares `document_standing` for a payment accepted only because an earlier one was reversed); replay derives tags from `invoice.created`, `bill.created`, `invoice.payment_received`, `bill.paid` ("the first untagged control line for the amount", rules.md B-38 item 1); `Store::void_document`.
- MCP: `books.void_document { kind, id, date, reason }`, `books.control_check { as_of? }`, `instrument` and `reference` on `books.receive_payment` and `books.pay_bill`; the tie-out in `books.verify`; `control_problems` in `books.digest`; `documents_void_and_tie_out_over_the_wire`.
- Goldens: `crates/ledger-store/tests/fixtures/p4-arap-before` (written by `0df7f52a`), adjudicated in `legacy_reversal_now_counts`.
- Rules: `docs/spec/rules.md` B-38 (new), B-18 and B-23 amended. Tests: `crates/ledger-core/tests/spec_documents.rs`, `crates/ledger-store/tests/arap_standing.rs`, `arap_props.rs`, `bounds.rs`.
- Departures from the plan: the plan's "stored fields removed" became derived caches; the plan's "`outstanding` over entries that stand" is not what was built (the date-sum is, and it is the right definition: a mirror carries the same tag and cancels; 8.3.3 says so now).

#### P4b S2: read tools and the viewer's Invoices & Bills

**Status:** landed 2026-10-24 (`7d2bf0b7` MCP, `b7b7f171` viewer). No format change.

**As built:** `books.documents { kind, status?, party?, due_by?, as_of? }` and `books.document { ref, as_of?, pdf? }` (`crates/ledger-mcp/src/server.rs`; `parse_doc_ref("invoice:7")`, `doc_ref_text`, `doc_label`, `movement_key`); the viewer's Money in and Money out pages, a document's page with its timeline (reversed items struck through), overview tiles ("Owed to you", "You owe"), `Route::Invoice(id)`/`Route::Bill(id)` (`crates/ledger-gui/src/pages/invoices.rs`); attention items for overdue and due-soon documents; `crates/ledger-gui/tests/invoices.rs` screenshots.

#### P4b S3: the invoice PDF

**Status:** landed 2026-10-24 (`c1f9001c`). No format change.

**As built:** `crates/ledger-import/src/invoice_pdf.rs::invoice_pdf(&InvoiceDoc) -> Result<Vec<u8>, PdfError>`, `InvoiceDoc { number, issue_date, due_date, terms: Option<String>, seller: InvoiceParty, bill_to: InvoiceParty, items: Vec<InvoiceItem { description, amount }>, total, settled: Vec<(String, Decimal)>, balance, stamp: Option<InvoiceStamp>, stamp_line, note }`, `InvoiceStamp { Paid, PartiallyPaid, PastDue, Void, Draft }`, `templates/invoice.typ` (seller block, "INVOICE" and the number, bill-to, dates, a Terms row, the lines, Total, each standing payment with its date, method and reference, Balance due, the stamp, the note; US Letter); `crates/ledger-package/src/invoice.rs::invoice_document(ledger, company, id, as_of, today) -> Result<InvoiceDoc, String>` (the one builder the server and the viewer share); `books.document { pdf: true }` hands the file over (`HandedFile`); the viewer's Save invoice PDF; a legacy invoice's copy is re-created and says so (`note`). Deterministic bytes (`an_invoice_renders_deterministically`). The copy issued is kept from S4 on.

### 8.2 Remaining units, in merge order

Each unit follows the 8.0.4 template. "A-n" is an owner answer numbered in 8.3.1; "D-n" a decision in section 10; "B-n" a rule in `docs/spec/rules.md`; "I-n" an invariant in section 2; "R-x" a recipe in 8.0.5. Code is named by file and symbol, never by line number. A `|` between two argument names means exactly one of them.

#### U0a: split `server.rs` by tool group

1. **Status:** not started.
2. **Depends on:** none. Single owner; merges before any other MCP-touching branch opens (S4.4, S6, S7.2, P4, P5, P6, P8 wait for it).
3. **Feature and schema:** none.
4. **Build:**
   1. Move each tool group's handlers out of `crates/ledger-mcp/src/server.rs` into `crates/ledger-mcp/src/tools/<group>.rs`, following the existing `tools/projects.rs` and `tools/kept.rs` pattern (`use super::*;` and the `#[tool(..)]` attributes unchanged). Groups: `documents.rs` (invoices, bills, payments, void, documents, document, control check, agings), `entries.rs` (draft, line, post, reverse, get, search), `reports.rs` (statements, show, export), `assets.rs`, `schedules.rs` (schedules and recurring), `periods.rs`, `reconciliation.rs`, `parties.rs` (parties, vendors, customers, 1099, TIN vault), `attachments.rs` (receipts, documents filed, links), `settings.rs` (settings, conventions, decisions, open items), `chart.rs`, `payroll.rs`, `payouts.rs`, `tax.rs`. `server.rs` keeps `BooksServer`, the router (`tool_router`), `call_tool`, `get_info`, the shared helpers (`as_of`, `hand_over`, `resolve_vendor`, `parse_doc_ref`, `preview_lines_for`, ...) and the tests that span groups. Behaviour and tool names unchanged. `crates/ledger-mcp/src/server.rs (changed)`, `crates/ledger-mcp/src/tools/*.rs (new)`.
   2. Move the shared helpers a group needs into `tools/mod.rs` or a `helpers.rs`, so no group file reaches into another group's.
   3. `docs/architecture.md`: the `ledger-mcp` layout paragraph names the groups.
5. **Files:** owns `crates/ledger-mcp/src/server.rs`, `crates/ledger-mcp/src/tools/*`; touches nothing else.
6. **Tests and proofs:** no new tests; every existing test passes unchanged (`cargo test -p ledger-mcp`, `cargo test -p ledger-cli`). `every_tool_is_documented_and_every_documented_tool_exists`, `annotations_complete`, `tool_text_lint` and `tool_descriptions_fit_700_characters` prove the surface is identical.
7. **Done when:** (1) `server.rs` is under 3,000 lines and holds no `#[tool(..)]` handler that belongs to a group; (2) `BooksServer::tool_router().list_all()` names the same 118 tools, in the same order, with identical descriptions and annotations (write a one-off comparison against `main` in the PR and paste the result); (3) `just check` green; (4) the architecture doc names the groups.
8. **As built:** (2026-10-26, branch `U0a`) Build steps 1-3 as written. The 98 handlers `server.rs` still held moved to `tools/documents.rs`, `tools/entries.rs`, `tools/reports.rs`, `tools/assets.rs`, `tools/schedules.rs`, `tools/periods.rs`, `tools/reconciliation.rs`, `tools/parties.rs`, `tools/attachments.rs`, `tools/settings.rs`, `tools/chart.rs`, `tools/payroll.rs`, `tools/payouts.rs` and `tools/tax.rs`; `books.add_class` and `books.list_classes` joined the existing `tools/projects.rs` (a project is a class with its facts), and `books.handbook`, `books.settings` and `books.set_fx_rate` make up `settings.rs` (`books.verify` and `books.digest` are in `reports.rs`). `server.rs` is under 2,000 lines (1,909) and holds no `#[tool(..)]` handler; `tool_router` composes the group routers, and `core_router` is gone because `server.rs` has no tools left. The helpers only one group uses moved into that group's file; the ones several groups share moved to `tools/helpers.rs`, which `server.rs` re-exports (`pub(crate) use helpers::*;`), and `server.rs` keeps the named shared helpers (`as_of`, `hand_over`, `resolve_vendor`, `parse_doc_ref`, `preview_lines_for`) and the request machinery. Two departures: (1) `settings.rs` is the module `settings_tools`, because `crate::settings` already names the live settings; (2) the test bodies sit in `crates/ledger-mcp/src/server_tests.rs`, a `#[path]` module of `server.rs`, with their fixtures in `server::test_support`, and each group's own tests moved into its group file: `server.rs` keeps the tests module (and the cross-group ones are the server's) but reaches 3,000 lines only with the bodies beside it. Done-when 2's count is stale: the surface is 122 tools, not 118, and a one-off dump of every tool's name, title, description, input and output schema and annotations, in `list_all()` order, diffs empty against `main`'s.

#### U0b: the attention engine over `PackageBooks`

1. **Status:** not started.
2. **Depends on:** none. Merges before P4, P5, P6, P8 and before any unit adds a Needs-attention item (R-F).
3. **Feature and schema:** none.
4. **Build:**
   1. Read `crates/ledger-gui/src/attention.rs::find(b: &Books) -> Vec<Attention>` and the GUI's `Books` builder (`crates/ledger-gui/src/books.rs`, from `Books::load` on): list every field `find` reads (the per-entry `origin` label, `missing_receipt`, `depreciation_due`, `reconciliation_due`, `party_infos`, `w9_status`, drafts, documents overdue). This is a 700-line refactor, not a file move: the builder today is written over `ledger_store::Snapshot`.
   2. Add an `attention` module to `crates/ledger-package` (`crates/ledger-package/src/attention.rs (new)`): `pub fn find(books: &impl PackageBooks, today: NaiveDate, settings: &Settings) -> Vec<Attention>`, with `Attention { severity, kind, title, detail, target: Target, tool: Option<(String, serde_json::Value)> }` where `Target` is an enum the viewer maps to its `Route` and the server maps to a `next_steps` item. Generalize the helpers it needs from `Snapshot` to `&impl PackageBooks` (`PackageBooks` already gives `ledger()`, `journal_seq()`, `all_attachments()`, `links_of()`, `attachment_bytes()`; add `settings()` if the trait lacks it, implemented for `Store` and `Snapshot` in `crates/ledger-package/src/books.rs`).
   3. `missing_receipt` is computed by the policy the server uses (`ledger_config::ReceiptPolicy::receipt_required`, reachable from ledger-package through ledger-config) plus the same origin rule the GUI applies today (`matches!(origin, "Journal entry" | "Recurring") && reversal_of.is_none()`), so the list is identical before and after. P4 replaces that rule with the one gate; U0b does not change it.
   4. The viewer's `attention::find` becomes a thin call that maps `Target` to `Route` (`crates/ledger-gui/src/attention.rs (changed)`); its GOAL block's INVARIANT list moves to the package module's.
   5. `books.start`'s hand-coded next steps in `crates/ledger-mcp/src/tools/kept.rs::start_out` take their attention-derived items from the same function (`next_steps` keeps its own order rule: format upgrade first, then onboarding, then attention by severity, then focus, then the handbook contract and the digest). The hand-coded items that are not attention (format upgrade, onboarding, handbook contract) stay.
   6. `docs/architecture.md`: `ledger-package` is "the shared read-only view: packages, the invoice PDF, and Needs attention, over `Store` or `Snapshot`".
5. **Files:** owns `crates/ledger-package/src/attention.rs`, `crates/ledger-gui/src/attention.rs`; touches `crates/ledger-package/src/books.rs`, `crates/ledger-gui/src/books.rs`, `crates/ledger-mcp/src/tools/kept.rs`, `docs/architecture.md`.
6. **Tests and proofs:** `crates/ledger-gui/tests/view_model.rs` passes unchanged (the viewer's list is the engine's); new `crates/ledger-cli/tests/attention_parity.rs::the_viewer_and_books_start_see_the_same_attention` (on the demo books and on a random book: the viewer's items equal the server's, item for item; pattern `projects_parity.rs`); the existing `spec_kept.rs` tests of `books.start` pass unchanged.
7. **Done when:** (1) `ledger-gui` no longer computes any attention item itself; (2) `books.start` lists the same items the viewer shows, proven by the parity test; (3) `just check` green; (4) no new crate and no new trait beyond `PackageBooks`.
8. **As built:** none yet.

#### U0c: testing notes for S1-B, S2, S3, and stale notes in the tool docs

1. **Status:** not started.
2. **Depends on:** none.
3. **Feature and schema:** none.
4. **Build:**
   1. `docs/testing.md`: three sections in the existing style ("P4b S1, part B", "P4b S2", "P4b S3"), each with its **For P12 (mutation targets)** paragraph: S1-B the `void_document` and `control_check` handlers and `PaymentDetails::{check, description}`; S2 `books.documents`' status filter arms and ~~the timeline's struck-through items~~ → As built: labelled; S3 `InvoiceStamp` arms and `invoice_document`'s legacy branch.
   2. `docs/mcp-interface.md`: the A/R and A/P section's text matches the shipped tools (it already does for S1-S3; check `books.document pdf` and the status list against `DocStatus::key()`), and a one-line note at the head of the section: "S4 changes `books.create_invoice` and `books.create_bill`; see plan 8.2 S4".
5. **Files:** owns nothing; touches `docs/testing.md`, `docs/mcp-interface.md`.
6. **Tests and proofs:** `every_tool_is_documented_and_every_documented_tool_exists` green.
7. **Done when:** the three sections exist with their paragraphs; `just check` green.
8. **As built** (2026-10-26, `e9febc56`, `feae51e4`): the three sections are under `docs/testing.md`'s P4b S1 part A section, in its style; the A/R and A/P note is under that heading. Departures:
   - The timeline's reversed items are labelled, not struck through: "Payment reversed" for the reversal and "Payment (reversed later)" for the payment, which is what the S2 paragraph names as P12's targets (`crates/ledger-gui/src/pages/invoices.rs`). Verified against the shipped page, not against plan 8.1 S2's "struck through".
   - Build item 2 assumed the A/R and A/P text already matched S1-S3. It does not in one place: `books.documents`' `party` was said to take a customer's or vendor's name or id, while the handler matches that name (ignoring case) or the document's own id, never the customer's or vendor's id. The doc now says the name; the schema's own description (`crates/ledger-mcp/src/types.rs`, `DocumentsParams::party`) still offers an id, which is a code finding (resolve the id, or narrow the schema) for a unit that may change code, and this one may not.

#### P4b S4: `documents_v2`

1. **Status:** not started.
2. **Depends on:** S1-S3 (landed). Inside the unit: S4.1 and S4.2 need nothing and may run in two worktrees; S4.3 needs both; S4.4 needs U0a and S4.3; S4.5 needs S4.4. S6 may start when S4.1 has merged; S5 when S4.2 has; S7.2 when S4.3 has. Go-ahead: `documents_v2`, one upgrade after S7 (D-52; Q-5 asks the owner whether to upgrade earlier). Until the go-ahead, on the owner's books a document carrying any new field is refused with `format_upgrade_pending`; a document carrying none posts exactly as before.
3. **Feature and schema:** feature `documents_v2` (R-A; shared with S5, S6, S7, so S4 creates the constant and the test and the later slices add their call sites). New fields on `invoice.created` and `bill.created` and on their lines, and on `party.updated`, as 8.0.3 lists; no new event type. Provisional schema 16: columns on the documents and document lines tables, a `parties.email` and two terms columns.
4. **Build:**

   **S4.1 Core: terms, numbers, line accounts** (`crates/ledger-core`).
   1. `arap.rs (changed)`: `pub enum TermsKind { DueOnReceipt, Net(u16), NetEom(u16), Custom }`, `pub struct EarlyDiscount { percent_bp: u32 /* basis points, 1..=9999 */, days: u16 }`, `pub struct Terms { kind, discount: Option<EarlyDiscount>, text: String }`. `Terms::due_date(date) -> Result<NaiveDate, LedgerError>`: DueOnReceipt = date; Net(n) = date + n days; NetEom(n) = the last day of date's month + n days; Custom = the date given (the caller passes it); every result goes through `arap::validate_due_date(date, due)` (which calls `calendar::check`, so a date past 9999-12-31 is a refusal, never a panic); `n` is 0..=3650. `Terms::discount_by(date) -> Option<NaiveDate>` = date + discount.days, and `discount.days < the net days` (for DueOnReceipt or NetEom a discount is refused: `LedgerError::TermsDiscountNeedsNetDays`). `Terms::describe() -> String` composes `text` ("Net 30", "Net 15 EOM", "2/10 net 30", "Due on receipt", "Due 2026-11-15") so no caller supplies it. `Terms::parse(&str)` accepts exactly those spellings, case-insensitive, and refuses anything else (`LedgerError::TermsUnreadable`).
   2. `kernel.rs (changed)`: `pub fn discount_max<T: Amount>(total: T, percent_bp: u32) -> Option<T>` = total × bp / 10,000, exact, no rounding (the caller's cents are what is taken; the kernel only bounds them); `pub fn line_amount<T: Amount>(quantity: T, unit_price: T) -> Option<T>` = the exact product. Kani in `proofs.rs`: `discount_max_is_bounded` (0 ≤ result ≤ total; `kani::cover!` a nonzero result), `line_amount_is_exact`. Bridge properties in `crates/ledger-core/tests/kernel_bridge.rs` for both on `Decimal`.
   3. `arap.rs`: `DocumentLine` gains `account: Option<AccountId>` (None = the role's account, which is also what every legacy line means), `quantity: Option<Decimal>`, `unit_price: Option<Decimal>` (both or neither; `kernel::line_amount(q, u) == amount` compared as `Decimal` with no rounding, else `LedgerError::LineArithmetic`). `document_parts` keys by `(Option<AccountId>, Option<ClassId>)`; `Ledger::add_document_parts`, `create_invoice_steps`, `create_bill_steps`, `replay_invoice`, `replay_bill` and `Store::create_invoice`/`create_bill`'s posting vectors all take the account from the part (CD-A2: the posting is built in two places today and compared on replay; both change together). `Ledger::check_document_line_account(kind, account)` beside `check_line_account`: an invoice line takes a revenue account or a liability holding no role (2400 Deferred Revenue); a bill line takes an expense account or an asset that is not cash (`books.cash_accounts`) and holds no role (1500 Equipment, 1300 Prepaid); placeholders and role accounts (`control_role_of`) refused.
   4. `arap.rs`: `pub struct PartySnapshot { name, address: Option<String>, email: Option<String> }` (not `Snapshot`: that name is the books' read-only view). `Invoice` gains `number: Option<String>`, `terms: Option<Terms>`, `bill_to: Option<PartySnapshot>`, `seller: Option<PartySnapshot>`, `customer_ref: Option<String>`, `memo: Option<String>`, `sends: Vec<Sent>` (the type comes in S5; S4 adds the empty field with `#[serde(default)]` so S5 changes no struct shape). `Bill` gains `vendor_ref: Option<String>`, `received_on: Option<NaiveDate>`, `terms: Option<Terms>`, `memo: Option<String>`. Every new field `#[serde(default, skip_serializing_if = "Option::is_none")]` exactly as `class_id` on `DocumentLine`.
   5. `ledger.rs`: `next_invoice_number(prefix, first_number) -> String`: `prefix` + the number zero-padded to four digits (a fifth digit widens it: 9999 → 10000, never truncated); the number is `first_number` + the count of numbered invoices issued so far; stored on the invoice at issue and never recomputed; legacy invoices (`number: None`) are shown as "Invoice {id}" and are not counted. `check_vendor_ref(vendor, vendor_ref, duplicate_reason)`: `(vendor, normalized vendor_ref)` (lower-case, spaces and hyphens removed) unique among bills whose entry stands; a duplicate is refused (`LedgerError::DuplicateVendorRef`) unless `duplicate_reason` is given, in which case it is journaled on the bill.
   6. Tests in `crates/ledger-core/tests/spec_documents.rs`: `terms_derive_due_dates` (2026-01-31 Net 30 → 2026-03-02; 2026-10-23 Net 15 EOM → 2026-11-15; a leap day; the 9999-12 bound refused), `discount_bounds_and_late_reason` (the bound, the date, DueOnReceipt refused), `terms_describe_and_parse_round_trip`, `bill_lines_post_to_their_accounts`, `invoice_line_to_deferred_revenue_posts_to_2400`, `qty_times_price_must_equal_amount`, `document_line_account_refuses_roles_cash_and_placeholders`, `numbers_run_and_voids_keep_theirs`, `number_widens_past_9999`, `duplicate_vendor_ref_refused_unless_a_reason_is_given`. `legacy_reversal_now_counts` and every S1 test unchanged.

   **S4.2 Settings and party fields** (`crates/ledger-core/src/settings.rs`, `parties.rs`; may run beside S4.1).
   7. R-D for each key: `invoicing.number_prefix` (Text, default "INV-", 1..=12 visible ASCII characters); `invoicing.first_number` (a new `Kind::Count`, a positive integer; settable only while no numbered invoice exists, checked by the store at `set_setting`: `LedgerError::FirstNumberLocked`); `invoicing.default_terms` (a new `Kind::Terms`, canonical form `Terms::describe()` of `Terms::parse`; default "Net 30"); `invoicing.payment_methods` (a new `Kind::PaymentMethods`: a JSON list of `{kind: ach|wire|link|check|p2p|crypto|other, label, lines: [String], link?: String, qr?: bool}`, at most 8 methods, each line at most 120 characters; the `no_tin` check is skipped for `lines` of `ach` and `wire` methods only, because a routing number is nine digits (A-3); every other text in it is scanned); `invoicing.footer` (LongText); `invoicing.unsent_days` (Count, default 2), `invoicing.reminder_days` (Count, default 14), `bills.remind_days` (Count, default 7), `bills.discount_warn_days` (Count, default 3); `company.email`, `company.website` (Text; the seller snapshot reads them through `Settings::company().profile`). `setting_feature` arms: every `invoicing.*` and `bills.*` key and the two `company.*` keys → `documents_v2`.
   8. `parties.rs (changed)`: `Party` gains `email: Option<String>`, `default_terms_as_customer: Option<Terms>`, `default_terms_as_vendor: Option<Terms>` (`#[serde(default)]`); `PartyField` gains `Email(Option<String>)`, `DefaultTermsAsCustomer(Option<Terms>)`, `DefaultTermsAsVendor(Option<Terms>)`; `Ledger::check_update_party` runs `parties::check_text` on the email (and a plain shape check: one `@`, no spaces) and `has_full_tin` covers it. `Store::update_party` calls `ensure_feature(DOCUMENTS_V2)` for the three new variants. The `"party.updated"` replay arm in `store.rs` maps an unknown `field` to `format::newer_content(..)`, not `JournalCorrupt` (B-27; today it says corrupt: fix it in this step and add `an_unknown_party_field_is_newer_not_corrupt` to `crates/ledger-store/tests/format.rs`).
   9. Tests: `crates/ledger-core/tests/settings_units.rs` (each new kind's canonical form and refusals: a routing number allowed in an `ach` method's lines, a nine-digit number refused in a `link` method's label), `crates/ledger-config` round-trip tests pick the keys up; `crates/ledger-store/tests/parties.rs::a_party_takes_an_email_and_terms_under_documents_v2`.

   **S4.3 Store: journaling the new fields, the issued PDF** (`crates/ledger-store`; needs S4.1 and S4.2).
   10. R-A: `DOCUMENTS_V2` constant, `KNOWN_FEATURES`, `documents_v2_is_its_own_feature`. `Store::create_invoice`/`create_bill` take the new fields; `ensure_document_fields` (beside `ensure_document_lines`) calls `ensure_feature(DOCUMENTS_V2)` only when any new field is `Some`; `document_line_json` and the document event's payload emit a key only when set (so a document with none of the new fields is journaled byte for byte as before: `no_new_fields_journals_byte_for_byte` in `tests/arap_standing.rs`, asserting the payload equals the pre-S4 shape field for field, like `a_payment_journals_its_method_and_reference`); `parse_document_lines` and the replay arms read them back; `replay_invoice`/`replay_bill` derive the posting with the line accounts. `documents_v2_waits_for_the_upgrade` (R-A step 3). Projection: columns for the new fields (schema 16, provisional); `differs_from` compares them.
   11. The issued PDF, filed in the issuing call: `Store::create_invoice_issued(params, pdf_bytes: Vec<u8>) -> Result<(InvoiceId, EntryId, AttachmentId), StoreError>`. The server renders the PDF first (a Typst failure returns before the store is touched). Inside one `atomically` step the store journals the entry lines, `invoice.created`, then `receipt.attached` (through the existing `AttachRequest`: `entry_id: Some(entry)`, file name `INV-0007.pdf`, MIME `application/pdf`, description "Issued invoice"), then `attachment.linked` to `Subject::Invoice(id)`. There is **no** `issued_pdf` field on `invoice.created` (CD-A6: the attachment id is allocated after the entry exists); the issued copy is found through the link, which `books.document` and the viewer already read (`attachments_for(Subject::Invoice(id))`), and its SHA-256 is the `blob_sha` in `receipt.attached`. A failure anywhere leaves the journal unchanged: `issued_pdf_is_filed_with_the_invoice_or_nothing_is` (`tests/arap_standing.rs`).
   12. `Store::preview_invoice(params) -> Result<InvoicePreview, StoreError>` runs `Ledger::create_invoice` on a clone (the `rehearse` pattern) and returns the posting, the total, the number it would take and the due date, journaling nothing. The same for bills.
   13. 8.0.6 checklist items 1-4 and 6.

   **S4.4 MCP** (`crates/ledger-mcp/src/tools/documents.rs`; needs U0a and S4.3). R-C for each.
   14. `books.create_invoice { customer, date, terms? | due_date?, lines: [{ description, amount, quantity?, unit_price?, account?, class? }], customer_ref?, memo?, bill_to?: { name?, address?, email? }, confirmed? }` → without `confirmed`: `{ preview: true, number, due_date, terms, total, lines: [{ account, side, amount, class }], warnings, pdf: HandedFile (stamped DRAFT) }`; with `confirmed`: `{ id, number, total, due_date, terms, entry_id, pdf: { attachment_id, sha256 }, mailto, warnings, notes }`. `customer` is a customer id, a party id or a name (new `resolve_customer` mirroring `resolve_vendor`; a name matching two is refused, as `inv6_ambiguous_vendor_reference_refused` shows). Due date: `due_date` if given; else `terms` if given; else the party's `default_terms_as_customer`; else `invoicing.default_terms`; else refused naming the three ways to give one. `terms` is a string `Terms::parse` reads. `bill_to` omitted = the party's name, address and email at issue. The DRAFT preview PDF counts against the token's 64 handed files like any export. `party_id` is accepted as an alias of `customer` until P5's `docs/mcp-interface.md` rewrite removes it (that is the concrete end of "one release"). The result's `mailto` is a convenience link ("Invoice INV-0007 from {company}", the body URL-encoded, truncated to 1,800 bytes at the last whole word); the owner or a delegate sends (A-4).
   15. `books.create_bill { vendor, date, received_on?, vendor_ref, terms? | due_date?, lines: [{ description, amount, account, class? }], memo?, upload_id | existing_attachment_id, duplicate_reason?, confirmed? }`. Under `documents_v2`, `account` is required on every line (A-11) and the file is required (`upload_id` through the same helper `books.attach_receipt` uses in `tools/files.rs`, or `existing_attachment_id`), filed on the bill's entry as a receipt and linked `Subject::Bill`; on books without the feature, lines without `account` post to 6999 as before and the file is optional. `vendor_ref` is required for a new bill (legacy bills have none and are shown by id). Output adds `discount_by?`, `max_discount?`. Preview as for invoices (no PDF).
   16. `books.update_party` gains `email`, `default_terms_as_customer`, `default_terms_as_vendor` (`UpdatePartyParams` in `types.rs`; one `party.updated` per field as today).
   17. Over-the-wire tests, `crates/ledger-cli/tests/documents.rs (new)`: `an_invoice_from_issue_to_paid` (S4's leg: the preview journals nothing and hands a DRAFT PDF; confirm; the attachment's sha equals `books.get_receipt`'s; the entry carries the PDF as its evidence; the number is INV-0001 then INV-0002), `a_bill_with_its_vendor_invoice_and_terms`, `duplicate_vendor_invoice_refused`, `a_line_account_must_be_allowed`, `terms_fall_back_in_order`, `documents_v2_refused_until_consent` (in `format_upgrade.rs`), `party_id_alias_still_works`.

   **S4.5 PDF, viewer, handbook** (needs S4.4).
   18. `crates/ledger-import/src/invoice_pdf.rs` and `templates/invoice.typ (changed)`: `InvoiceDoc` gains `items[].quantity`, `items[].unit_price` (printed as "3 × 33.33" when given), `customer_ref`, `memo`, `footer`, `payment_methods: Vec<PaymentMethodDoc { label, lines, qr_svg: Option<String> }>` (the QR through `labels::qr_svg`, placed with the same `image(bytes(..), format: "svg")` idiom `labels.typ` uses), seller email and website, `terms` text, the issue copy (no stamp, no settled rows: `InvoiceVersion::Issued`) beside the current copy (`InvoiceVersion::Current { as_of }`, whose "Re-created on DATE" note prints `as_of`, never the wall clock, so bytes stay deterministic). Every string is data (no Typst markup is ever interpreted); every amount passes through `report::amount_parts`. `crates/ledger-package/src/invoice.rs::invoice_document` fills the new fields from the invoice's snapshots (never from today's party record when a snapshot exists).
   19. Tests: `an_invoice_renders_deterministically` (its golden sha updated once, the recomputation stated in the commit), `invoice_pdf_prints_what_it_should` in `crates/ledger-import/tests/spec_import.rs` (reads the PDF back with the hayro text reader beside `pdf_is_deterministic_and_labels_are_data`: the number, both dates, the total, the bill-to, each payment method's label and lines; a CJK name and an RTL name; no account code, project or id on the page), `labels_are_data` (an item description `#import "x"` prints literally), `two_hundred_lines_paginate`.
   20. Viewer (`crates/ledger-gui/src/pages/invoices.rs (changed)`): the document page shows the number, terms, bill-to and PO; the Money in list shows numbers; Save invoice PDF offers the issued copy when one is filed and the current copy always. Screenshot `shot("invoice-numbered")` in `crates/ledger-gui/tests/invoices.rs`; `view_model.rs` parity for the new fields.
   21. R-E: `crates/ledger-mcp/handbook/invoices-and-bills.md`, the "Invoices and bills" procedure: (1) invoice: preview, confirm, download the PDF, the owner or a delegate sends it, mark it sent (S5 adds the tool); (2) collect: `books.receive_payment` with the method and the processor's reference; (3) pay bills on their terms (S6 adds `books.bills_due`); (4) void, write-off or reverse a payment: when to use which (void = the document should never have existed; write-off = it will not be collected, S7; reverse a payment = it bounced or was recorded wrongly); (5) the month-end tie-out (`books.control_check`); (6) the CPA's list from 8.3.2. A `{documents_v2}` placeholder renders "your books have not declared `documents_v2` yet: numbers, terms and line accounts wait for the owner's upgrade" before the go-ahead. Rules: `docs/spec/rules.md` B-39 (terms), B-40 (numbers, the issued copy, the duplicate-bill control) from the spec blocks in 8.3.13, marked CONVENTION until their sources are vendored (8.4). `docs/mcp-interface.md`, `docs/parity.md` (A/R and A/P rows gain "numbers, terms, line accounts, the issued PDF"), `docs/testing.md` For-P12 (R-G: `Terms::due_date`'s arms, `next_invoice_number`'s padding, `check_vendor_ref`'s normalization, `check_document_line_account`'s arms, `ensure_document_fields`'s `Some` test).
5. **Files:** owns `crates/ledger-core/src/arap.rs` (S4.1), `crates/ledger-mcp/src/tools/documents.rs` (S4.4), `crates/ledger-import/src/invoice_pdf.rs`, `crates/ledger-import/templates/invoice.typ`, `crates/ledger-package/src/invoice.rs`, `crates/ledger-gui/src/pages/invoices.rs`, `crates/ledger-mcp/handbook/invoices-and-bills.md`, `crates/ledger-cli/tests/documents.rs`; touches the 8.0.3 files.
6. **Tests and proofs:** the tests named in each step; Kani `discount_max_is_bounded`, `line_amount_is_exact`; bridge properties for both; no Lean change (no new `Ledger` command: the document steps are the existing ones with more fields). bookspec (added to `bookspec/examples/` now; the loader in `crates/ledger-core/tests/bookspec.rs` gains `due_date` and `terms` on `InvoiceSpec`/`BillSpec`): `payment-terms-due-date`, `invoice-numbers-run-and-voids-keep-theirs`.
7. **Done when:** (1) every test in steps 6, 9, 10, 11, 17, 19, 20 exists and passes; (2) the S1 tests named in 8.1 (`legacy_documents_replay_with_tags`, `legacy_reversal_now_counts`, `a_payment_journals_its_method_and_reference`, `several_payment_events_on_one_entry_tag_its_lines_in_order`) and the viewer's `the_list_shows_the_ledgers_figures` pass unchanged; (3) `no_new_fields_journals_byte_for_byte` proves an old-shape document is byte-identical; (4) the 8.0.6 checklist is in the PR; (5) `just check` and `just prove` green; (6) the handbook section renders; (7) no figure in any golden changed.
8. **As built:** none yet.

#### P4b S5: sent tracking (`invoice.sent`)

1. **Status:** not started.
2. **Depends on:** S1. S4.2 gives the defaults (`to` from the party's email; `invoicing.unsent_days`, `invoicing.reminder_days`); if S5 merges before S4.2, `to` is required and the day counts are the constants 2 and 14, and both paths are tested. MCP step needs U0a. Go-ahead: `documents_v2` (D-52).
3. **Feature and schema:** feature `documents_v2` (S5 adds its `ensure_feature` call site; the constant is S4's, or S5 creates it per R-A if S5 merges first); new event type `invoice.sent { invoice_id, on, by, via, to: [String], kind, reference?, note? }` (R-B); provisional schema 17: table `invoice_sends`.
4. **Build:**
   1. `crates/ledger-core/src/arap.rs (changed)`: `pub enum SentVia { Email, Mail, Portal, Hand, Other(String) }`, `pub enum SendKind { Original, Reminder }`, `pub struct Sent { on: NaiveDate, by: String /* the owner, or a named delegate such as an agent: A-4 */, via: SentVia, to: Vec<String>, kind: SendKind, reference: Option<String> /* the channel's own id: a message id */, note: Option<String> }`. `Invoice.sends: Vec<Sent>` (the field S4 added empty). `Ledger::mark_sent(invoice, Sent) -> Result<(), LedgerError>`: refused on a Void invoice (`LedgerError::DocumentVoid`), allowed on Open, Partial, Paid, WrittenOff (a receipt copy); `to` non-empty; each `to` and `reference` through `parties::check_text`. A send changes no figure: `DocState` is identical before and after. Bills get no sends (we receive them).
   2. Store (R-A, R-B): `Store::mark_sent` → `ensure_feature(DOCUMENTS_V2)` → `rehearse` → `append_journal("invoice.sent", ..)` → `ledger.mark_sent`; `meta::invoice_sent_of`; replay arm; projection table `invoice_sends (invoice_id, on, by, via, to_json, kind, reference, note)`; `differs_from` compares `sends`. Tests in `crates/ledger-store/tests/arap_standing.rs`: `mark_sent_waits_for_documents_v2`, `sends_replay_and_project`, `a_send_changes_no_figure`, `mark_sent_on_void_refused`; `arap_props.rs` gains a `MarkSent` op in `status_is_a_function_of_journal_and_date`.
   3. MCP (R-C): `books.mark_sent { invoice, on?, by?, via?, to?: [String], kind?, reference?, note? }` → `{ invoice, sends: [..] }` (W scope). Defaults: `on` = today in the company's time zone (`BooksServer::as_of(None)`); `by` = the token's name (the agent) unless given; `via` = email; `kind` = original for the first send, reminder after; `to` = the invoice's `bill_to.email`, else the party's email, else required. `books.documents` rows gain `last_sent?`; `books.document`'s timeline shows each send. Over-the-wire: `documents.rs::mark_sent_records_who_and_how` (two sends, the second a reminder with a message id; `documents` shows `last_sent`).
   4. Attention (R-F, after U0b): "invoice never sent" when `status ∈ {Open, Partial}`, `sends` empty, `number` is Some (legacy invoices exempt), and today (company time zone) > issue date + `invoicing.unsent_days` calendar days; "overdue invoice not reminded" when overdue and the last send is older than `invoicing.reminder_days`. Both in the viewer and `books.start` by the one function. Screenshot.
   5. Handbook step 1 of `invoices-and-bills.md` gains the mark-sent sentence; `docs/mcp-interface.md`; `docs/parity.md`; `docs/testing.md` For-P12 (`mark_sent`'s refusals, the `kind` default, the two attention predicates' comparisons).
5. **Files:** owns the `Sent` types and `Ledger::mark_sent`, the `invoice.sent` arms, `books.mark_sent`; touches the 8.0.3 files.
6. **Tests and proofs:** named above; no kernel change, no Lean change (a send moves no money). 8.0.6 items 1-4, 6.
7. **Done when:** (1) the tests pass; (2) a send on a fixture book changes no report and no `DocState` (`a_send_changes_no_figure`); (3) an older version stops at the `invoice.sent` line with `NewerJournal` (the `tests/format.rs` arm); (4) `just check` green; (5) attention parity holds.
8. **As built:** none yet.

#### P4b S6: paying on terms (multi-document payments, discounts, the processor fee, bills due, aging exports)

1. **Status:** not started.
2. **Depends on:** S4.1 (`Terms`, `discount_by`, `kernel::discount_max`), U0a. Go-ahead: `documents_v2` (D-52).
3. **Feature and schema:** feature `documents_v2` (call site in `Store::ensure_payment_features`); no new event type: `invoice.payment_received` and `bill.paid` gain `line` (the index of the control line this event tags), `discount`, `discount_line`, `discount_reason`, `fee`, `fee_line` (8.0.3). A payment entry that settles N documents is N events naming the same entry, each with its `line`; legacy events without `line` keep the positional rule ("the first untagged control line for the amount", rules.md B-38 item 1, test `several_payment_events_on_one_entry_tag_its_lines_in_order`). Provisional schema 18 if a column is needed for discounts and fees in the projection's payments table.
4. **Build:**
   1. Kernel (`crates/ledger-core/src/kernel.rs (changed)`): `pub fn settle<T: Amount>(cash: &[T], discounts: &[T], fee: T) -> Option<Settlement<T>>` returning `{ applications_total = Σcash + Σdiscounts, deposit = Σcash − fee }` with checked arithmetic (`None` on overflow or a negative deposit). Kani `settle_is_exact` on fixed arrays with `kani::cover!` (a nonzero discount and a nonzero fee); bridge property in `tests/kernel_bridge.rs`. `payout_plan` in `payouts.rs` is left as it is (IZ-1e).
   2. Core (`ledger.rs (changed)`): `Ledger::settle(SettleParams { date, description, cash_account, party, applications: Vec<Application { doc, cash, discount, discount_reason }>, fee: Option<Fee { amount, account }>, instrument, reference })` replaces the internals of `receive_payment_with`/`pay_bill_with` (which become one-application calls of it). Rules: one party per settlement (every document names the same customer or vendor); each application's `cash + discount ≤ payment_room(doc, date)` (the existing bound); a discount ≤ `kernel::discount_max(doc.total, terms.discount.percent_bp)` and dated ≤ `terms.discount_by(doc.date)`, else `discount_reason` required and journaled (a late discount a vendor honours); on a bill whose lines include an asset or prepaid account a discount is refused with the remedy "record the bill net of the discount, or reclassify by an entry Dr 6995 / Cr the asset account" (D-49: no apportioning across asset lines); the fee allowed on `receive_payment` only (a processor's fee on money received), posted Dr `payment_processing_fees` for the fee / Dr deposit account for the net / Cr A/R `[d, Payment]` for the gross (the same gross presentation `payouts.rs` uses, A-4), the fee line's party tag the processor's vendor if `fee.vendor` is given, else untagged. Posting rows per 8.3.6: payment lines in application order, then discount lines, then the fee line, then the cash line, so `line` indices are stable. The entry carries the party tag, `instrument` and `reference` as today.
   3. Store: `Store::settle` journals `journal_auto_lines` (the N-line variant `create_invoice` uses) then one `invoice.payment_received`/`bill.paid` per application with `line`, `discount`, `discount_line`, `discount_reason`, `fee`, `fee_line` (the fee on the first application's event only); `ensure_payment_features` adds `ensure_feature(DOCUMENTS_V2)` when there is more than one application, a discount or a fee; replay reads `line` when present and tags that line, else the positional rule; a line it cannot match is listed by `control_check` as unattached (B-28: replay never refuses). Tests in `arap_standing.rs`: `a_settlement_names_its_lines_and_replays`, `settlement_waits_for_documents_v2`, `legacy_payment_events_keep_the_positional_rule`; `arap_props.rs` gains multi-document, discount and fee ops (the identity and bounds hold after each). 8.0.6 checklist, with a `figures.json` from the pre-S6 version over a fixture with payments (no figure moves).
   4. Roles: `purchase_discounts` (6995, expense; tax category `other_deductions`), `sales_allowances` (4100, existing: `returns_allowances`), `payment_processing_fees` (the expense account payouts' fee lines use today; if payouts have no role for it, make one with that account as its default) created on demand by the same function the receivables and payables roles use (`ensure_control_account`'s pattern, in `store.rs`); chart templates under `crates/ledger-import/templates/` gain the codes; `crates/ledger-import/src/templates.rs::no_template_puts_another_account_at_a_control_code` stays green.
   5. MCP (R-C): `books.receive_payment { date, deposit_to, instrument?, reference?, invoice | applications: [{ invoice, amount, discount?, discount_reason? }], fee?: { amount, account?, vendor? }, paid_in_full_marking?: bool, confirmed? }` → `{ entry_id, documents: [{ ref, number, outstanding, status }], deposited }`; `amount` (single form) or the applications' `amount`s are cash only; the discount is beside it. `paid_in_full_marking: true` adds a warning quoting UCC §3-311 (cashing a check marked payment in full of a disputed amount may settle the claim) and records nothing else. An overpayment is refused showing the excess and the remedy: record the excess as a plain entry to a customer-deposits liability (2300, no role) and apply it later (A-13, D-49). `books.pay_bill { date, paid_from, instrument?, reference?, bill | applications: [{ bill, amount, discount?, discount_reason? }], confirmed? }`. `document_id`/`cash_account` stay as aliases until P5's doc rewrite. `books.bills_due { through? }` → `{ bills: [{ ref, vendor, vendor_ref, due_date, outstanding, discount_by?, max_discount?, pay_by_discount_date_amount? }], by_date: [{ date, amount /* the running total of what falls due through that date */ }] }` (R). `books.ar_aging`/`books.ap_aging` gain `detail?: bool` (one row per document) and `control: { account, balance, documents_total, unattached: [..], difference }`. Over-the-wire (`documents.rs`): `one_payment_settles_two_invoices`, `bill_on_terms_with_discount`, `late_discount_needs_a_reason`, `a_fee_posts_gross_to_receivables_and_net_to_the_bank`, `card_paid_bill_is_not_1099_nec`, `equipment_bill_payment_is_investing_cash`, `discount_on_an_asset_bill_is_refused_with_the_remedy`, `bills_due_orders_by_due_and_discount`, `multi_document_payment_counts_once_for_1099_and_party_totals`, `overpayment_refused_with_the_excess`.
   6. Exports: `books.export` kinds `ar_aging`, `ap_aging` in csv, xlsx, pdf (the `hand_over_export`, `layout_statement`, `report::{format_amount, amount_parts}`, `pdf::statement_pdf` seams); columns `number, party, date, due_date, terms, total, paid, discounted, outstanding, bucket, days_overdue`; a tie-out line `TOTAL,,,,,,,,<Σ outstanding>,GL <account> <balance>,difference 0.00`. The CPA package (`crates/ledger-package/src/package.rs`) gains `ar_aging.csv` and `ap_aging.csv`, and its README states the B-23 tie-out, the direct write-off method (S7) and the gross discount method. Tests: `crates/ledger-import/tests/spec_import.rs` (deterministic pdf, labels are data), `crates/ledger-package/tests/package.rs::the_aging_csvs_tie_to_the_control_check`.
   7. Viewer: the Money out list gains a discount-deadline column and `due_soon`/`discount_open` flags; bucket totals and the tie-out badge; screenshot; `view_model.rs`.
   8. Handbook steps 2, 3 and 5 of `invoices-and-bills.md`, plus the two procedures D-49 records: a replacement payment received before a bounce is known (park it Dr bank / Cr 2300, reverse the bounced payment dated the bounce day, then `receive_payment { deposit_to: 2300, date ≥ the bounce }`), and a discount on a bill with an asset line (the refusal's remedy). `docs/spec/rules.md` B-42 (settlements: exactness, one party, the discount bounds, the fee's gross presentation; SOURCE: ASC 606-10-32-5 to 32-9 noted as the strict treatment, the gross method a DECISION per A-9). `docs/mcp-interface.md`, `docs/parity.md`, `docs/testing.md` For-P12 (`settle`'s arms, `discount_max`, the late-discount test, the asset-line refusal, the `line` replay branch). bookspec: `payment-terms-early-discount`, `arap-settlement-exact`, `processor-fee-gross-presentation` (loader: `receive_payment`/`pay_bill` events with `applications`, `discount`, `fee`; queries `ar_aging`, `ap_aging`, `control_check`).
5. **Files:** owns `Ledger::settle` and its tests, the payment events' new fields and arms, `books.bills_due`, the aging exports; touches the 8.0.3 files, `crates/ledger-mcp/src/tools/documents.rs` (after S4.4; coordinate with the S4 engineer on `tools/documents.rs`: S6 adds its handlers in a second file `tools/settlements.rs` if S4.4 is still open).
6. **Tests and proofs:** named above; Kani `settle_is_exact`; Lean: none new (the posting step is `Step.document` with more lines; `control_agrees` covers it because every control line is tagged).
7. **Done when:** (1) the tests pass; (2) `several_payment_events_on_one_entry_tag_its_lines_in_order` still passes (the legacy rule); (3) the 1099 worksheet and party totals count a two-invoice payment once (`multi_document_payment_counts_once_for_1099_and_party_totals`); (4) the CPA package's aging CSVs tie to `control_check` on the demo books; (5) 8.0.6 in the PR, no golden figure moved; (6) `just check`, `just prove` green.
8. **As built:** none yet.

#### P4b S7: bad debts (`write_off_invoice`)

1. **Status:** not started.
2. **Depends on:** S1; S4.3 for the same-call attach (the write-off needs its evidence filed in the same store call); U0a for the MCP step. Go-ahead: `documents_v2`.
3. **Feature and schema:** feature `documents_v2`; new event type `invoice.written_off { invoice_id, entry_id, date, amount, reason }` (R-B). No schema change expected (the entry and its tag carry the figures; a column only if the projection's documents table needs `written_off`).
4. **Build:**
   1. Core: `Ledger::write_off(invoice, date, amount, reason) -> Result<EntryId, LedgerError>`: refused on cash-basis books (`Company.basis == "Cash"` from `Settings::company()`; `LedgerError::WriteOffOnCashBasis`, IRC §166 and Treas. Reg. §1.166-1(e): a debt never counted as income has no basis); `0 < amount ≤ outstanding(invoice, date)` (partial allowed, §1.166-3(a)); the date in an open period and not before the invoice; posts Dr `bad_debts` (6990) / Cr A/R `[d, WriteOff]` with the customer's party tag; `Movement::WriteOff` and `DocStatus::WrittenOff` already exist (S1), as does the derivation in `doc_state`. A recovery is **not** a new movement (D-49): same tax year, reverse the write-off (the document reopens) then `receive_payment`; a later year, a plain entry Dr deposit account / Cr a "Bad debt recoveries" revenue account (tax category `other_income`), memo naming the invoice, the document staying Written off (§111). Tests in `spec_documents.rs`: `half_paid_rest_written_off_is_written_off`, `write_off_refused_on_cash_basis`, `write_off_above_outstanding_refused`, `write_off_then_recovery_same_year`.
   2. Lean (`verify/lean/Wanderbooks/Documents.lean (changed)`): `Step.writeOff` as a `Step.document` whose lines are tagged (it is one: a tagged control line and an expense line), so `control_agrees` and `Reachable.control_is_documents` hold without new proof; state and prove `writeOff_within_bounds`: a write-off of at most what is owed leaves `0 ≤ owed` (over the date-sum `docSum … through D`). List in `Audit.lean`. `just prove`.
   3. Store (R-A call site, R-B): `Store::write_off_invoice(invoice, date, amount, reason, evidence: Evidence { upload bytes | existing_attachment_id })` in one `atomically`: `ensure_feature(DOCUMENTS_V2)`, the entry lines, `invoice.written_off`, `receipt.attached` on the write-off's entry (the evidence of worthlessness: collection correspondence, a bankruptcy notice; §1.166-2), `attachment.linked` to `Subject::Invoice`. Tests: `write_off_waits_for_documents_v2`, `write_off_replays_and_projects`, `write_off_without_evidence_is_refused`; `arap_props.rs` gains `WriteOff`.
   4. MCP (R-C): `books.write_off_invoice { invoice, date, amount?, reason, upload_id | existing_attachment_id, confirmed? }` → `{ entry_id, document: { ref, status, paid, written_off, outstanding } }` (destructive; preview without `confirmed`). `books.reverse_entry` on a write-off's entry works as on any entry (the document reopens). Over-the-wire: `documents.rs::write_off_then_recovery`, `write_off_needs_evidence`.
   5. Attention (R-F): "write-off awaiting the CPA's note" (a written-off invoice in the current year, listed until year end). Viewer: the timeline shows the write-off, struck through if reversed; screenshot.
   6. Handbook step 4 (when to void, write off, or reverse) and the recovery procedures; `docs/spec/rules.md` B-41 from 8.3.13's block (SOURCE: IRC §166(a), §111; Treas. Reg. §1.166-1(e), (f), -2(a), -3(a); ASC 326-20 noted; DECISION: direct write-off, A-6); `docs/mcp-interface.md`; `docs/parity.md`; `docs/testing.md` For-P12. bookspec: `bad-debt-direct-write-off`, `arap-hand-control-posting-refused` (the latter uses the existing `older = true` and `refused` fields; the loader's refusal vocabulary gains `UntaggedControlLine`).
5. **Files:** owns `Ledger::write_off`, the event's arms, `books.write_off_invoice`, `Step.writeOff`; touches the 8.0.3 files.
6. **Tests and proofs:** named above; Lean `writeOff_within_bounds`; no kernel change.
7. **Done when:** (1) the tests pass; (2) `just prove` green with the new theorem in `Audit.lean`; (3) 8.0.6 in the PR; (4) the handbook says when to void, write off or reverse; (5) `just check` green.
8. **As built:** none yet.

**Deferred from S7 (D-49, recorded in 8.4):** `books.adopt_control_line` and `books.assign_role { move_balance }`. 8.3.14 item 4 already gives two remedies for a legacy hand line on 1200 or 2000 that need no code: reverse it and re-enter it as a document while its period is open; or leave it, record a decision, and let the CPA README footnote it (the identity's `unattached` term exists for exactly this). Build the tools only if `books.control_check` on the owner's books lists a line neither remedy can take; then the EM writes a unit for them with a Lean `Step.adopt` and a per-account `owed_on` identity (8.4 has the notes).

#### P4: draft states, the evidence gate in one place, the preview digest, the digest tool

1. **Status:** not started. Rewritten 2026-10-26 against P4b S1-S3 and D-47 (reversals stay posted; no reversal drafts).
2. **Depends on:** S4-S7 merged (step 4 is one sweep over every confirm tool, including S4's and S6's); U0b (attention items). Go-ahead: `drafts_v2`, declared in the same owner upgrade as `documents_v2` (D-52).
3. **Feature and schema:** feature `drafts_v2`; new event types `entry.draft_state_set { id, state, note }`, `entry.draft_discarded { id, reason }` (R-B); provisional schema 19: `entries.draft_state`, `entries.posted_seq`.
4. **Build:**
   1. **Draft states.** `crates/ledger-core/src/journal.rs (changed)`: `JournalEntry` gains `draft_state: DraftState { Building, AwaitingEvidence { note } }` (meaningful while `status == Draft`); `Ledger::set_draft_state`, `Ledger::discard_draft(id, reason)` (removes the draft from `entries`; refused on a posted entry; `Ledger::differs_from` compares drafts and their states). `Ledger::posted_seq(entry) -> Option<u64>`: an `entry_id → seq` map filled in the `"entry.posted"` and `"entry.reversed"` replay arms (the precedent is `apply_settings(.., line.seq)`), so the Snapshot sees it; no new event. Lean (`Ledger.lean`): `Step.discard` (a draft removed; `draftsOpen` shrinks; balances unchanged), preservation proved, listed in `Audit.lean`. Store (R-A, R-B): `Store::set_draft_state`, `Store::discard_draft`, both `ensure_feature(DRAFTS_V2)`; replay and projection arms; `posted_seq` and `draft_state` columns. Tests: `crates/ledger-core/tests/invariants.rs::discard_unblocks_close` (a period with an open draft can't close; discarded, it can: the existing Lean `hdrafts`), `crates/ledger-store/tests/store.rs::draft_states_replay_and_project`, `drafts_v2_waits_for_the_upgrade`, `posted_seq_is_the_posting_lines_seq`.
   2. **The evidence gate in one place** (I-5 as amended by D-47). Move the policy evaluation out of `crates/ledger-config/src/receipt_policy.rs` into `crates/ledger-core/src/evidence.rs (new)`: `pub fn requirement(ledger: &Ledger, entry: &JournalEntry, settings: &Settings) -> Requirement { Required { reason }, Exempt { source } }`; exemptions by what `Ledger` already knows: a closing entry (`closing_entry_ids`), a schedule posting (`schedule_month_of`), a depreciation run, a reversal (`reversal_of.is_some()`: D-47, D-12: a reversal's evidence is its original's), and entries the document, payroll, payout and disposal commands post (they carry their own evidence or are posted at once by design; their origin is known to the command that posts them and recorded as today); the policy's thresholds and account lists come from `policy.evidence.*` in `Settings`. `ReceiptPolicy` in `ledger-config` becomes a thin re-export or goes (its parsing stays only as far as adoption of an old `wanderbooks.toml` needs it). `Store::post_entry` calls `evidence::requirement` and refuses `Required` with the receipt-required error the MCP builds today (`ReceiptRequiredError { code: "receipt_required", remedy_tool: "books.attach_receipt", .. }` moves to the store's error and the MCP maps it). `BooksServer::post_entry`, `digest`, `dispose_asset`, `handbook::evidence_rule` and `tools/kept.rs` `RulesOut.evidence` call the one function. The viewer's `missing_receipt` (`crates/ledger-gui/src/books.rs`, the `matches!(origin, "Journal entry" | "Recurring")` string match) is deleted and the attention module (U0b) calls `evidence::requirement` over the Snapshot. The at-once tools keep posting at once with their evidence attached right after, as the live instructions say; nothing about them changes in P4. Tests: `crates/ledger-cli/tests/drafts.rs::gui_and_server_gate_agree` (on random books, the viewer's missing-evidence set equals the server's, through the parity harness), `legacy_reversed_replays_identically`, `a_reversal_needs_no_receipt_of_its_own` (D-47), `the_gate_moved_and_the_mcp_error_is_unchanged`.
   3. **`books.list_drafts { state?, as_of? }`** → `{ drafts: [{ entry_id, date, description, total, state, note, missing_evidence }] }` (R); **`books.set_draft_state { entry_id, state, note? }`** (W); **`books.discard_draft { entry_id, reason, confirmed? }`** (destructive); **`books.preview_entry { entry_id }`** (R: the full preview `books.post_entry` shows without `confirmed`, plus `preview`). Reports gain `as_of_status { drafts_open_on_or_before, unreconciled, settings_problems }` where a draft dated on or before the report's end is open; `strict: true` on a report refuses instead of warning. `books.export_cpa_package` refuses while a draft dated on or before `as_of` is open unless `allow_open_drafts { reason }` is given (journaled as a decision). Over-the-wire: `drafts.rs::a_draft_waits_for_its_receipt_and_posts`, `discard_unblocks_the_close_over_the_wire`, `report_warns_on_draft_before_as_of`, `cpa_package_refuses_open_draft`.
   4. **The preview digest (D-50, amending I-6).** Every tool called without `confirmed` returns `preview: String`, the SHA-256 of the canonical JSON of its preview payload (for a draft entry: the entry id, date, description, lines, classes, party, instrument, and the evidence links, plus the settings `seq`; for an operation without a draft, a close, a reconciliation, a disposal, a run, a document create or a settlement: the tool name, its canonical arguments, the settings `seq` and the journal head `seq`). Every call with `confirmed: true` takes `preview` and recomputes it; a mismatch is refused with guard `stale_preview` and a fresh preview (a post by another session in between moves the head and forces a re-preview, which is the intended behaviour; owner decision 3). Computed in one helper called from the result path in `server.rs` (`Preview::of(&payload)`), not per tool; no key file (`keys/preview.key` is not built). Required from this unit on: there is no "accepted without it for one release". Tools with `confirmed` today: `books.post_entry`, `books.reverse_entry`, `books.run_schedules`, `books.run_depreciation`, `books.run_recurring`, `books.dispose_asset`, `books.reconcile`, `books.close_period`, `books.void_document`, `books.create_invoice`, `books.create_bill`, `books.receive_payment`, `books.pay_bill`, `books.write_off_invoice`, `books.discard_draft` (grep `confirmed` in `crates/ledger-mcp/src/tools/` to get the list the day this starts). The journey tests (`mcp_journeys.rs`, `oracle_mcp.rs`, `spec_mcp.rs`) are updated to pass `preview`. Test: `drafts.rs::stale_preview_refused` (a post between preview and confirm; a changed argument), `every_confirm_tool_takes_a_preview` (iterates the tool list for `confirmed` and asserts `preview` is in its schema).
   5. **`books.digest` (defect 6).** `since` stays by entry date; new `since_seq` selects entries whose `posted_seq` is after it (what was posted since a session's head, backdated entries included); `as_of`; touched accounts whose net movement is zero are listed under `zero_net_accounts`; `drafts_by_state`; `reconciliation_tracked`, `never_reconciled` (the accounts in `books.cash_accounts` with or without a statement ever recorded). Test `digest_since_seq_is_posting_time` (an entry backdated but posted today appears).
   6. Attention (R-F): "draft awaiting evidence" and "draft not posted" as two items. Handbook `drafts-and-reversals.md` rewritten (it says reversals need no receipt: that stays true, D-47, and now says why, and how a draft waits on evidence). `docs/spec/rules.md` B-43 (drafts and the evidence gate; SOURCE: Pub. 583 "supporting documents", Rev. Proc. 98-25; DECISION D-47). `docs/mcp-interface.md` (the drafts tools; `preview` on every confirm tool; `as_of_status`); `docs/parity.md`; `docs/verification.md` (`Step.discard`); `docs/testing.md` For-P12 (`requirement`'s exemption arms, `Preview::of`'s canonicalization, `digest`'s `since_seq` comparison). `fuzz_op_sequence` gains `SetDraftState`, `DiscardDraft`.
5. **Files:** owns `crates/ledger-core/src/evidence.rs`, `crates/ledger-mcp/src/tools/drafts.rs`, `crates/ledger-cli/tests/drafts.rs`, `handbook/drafts-and-reversals.md`; touches `journal.rs`, `ledger.rs`, `store.rs`, every tool with `confirmed` (the sweep), `crates/ledger-gui/src/books.rs`, `crates/ledger-package/src/attention.rs`, the 8.0.3 files.
6. **Tests and proofs:** named above; Lean `Step.discard`; no kernel change.
7. **Done when:** (1) every tool with `confirmed` takes `preview` and refuses a stale one (`every_confirm_tool_takes_a_preview`, `stale_preview_refused`); (2) one function decides evidence and the GUI's string match is gone (`gui_and_server_gate_agree`); (3) reversals still post at once with no receipt of their own (`a_reversal_needs_no_receipt_of_its_own`); (4) the drafts tools and the digest fields exist and are documented; (5) 8.0.6 in the PR (a `figures.json` from the pre-P4 version over a fixture holding reversals and drafts: no figure moves); (6) `just check`, `just prove` green.
8. **As built:** none yet.

**Struck from P4 (D-47):** ~~reversal as a draft (`entry.reversal_drafted`, `Step.draftReversal`, `evidence: link_original | {attachment_ids} | later`, "reversal draft needs evidence", `reversal_needs_evidence`, `inherited_evidence_posts`)~~; ~~`preview_token` as an HMAC with `keys/preview.key`~~ → D-50; ~~`ledger_core::evidence::requirement` used by "every path" including the at-once tools~~ → the at-once tools are unchanged (IZ-1h); ~~"defect 2"~~ → the 2026-10-16 audit's defect 2 (a reversal posted with no evidence of its own) is closed by D-47 and D-12, not by code.

#### P5: agent surface complete

1. **Status:** not started.
2. **Depends on:** P4 (`preview` and the drafts tools are in `books.start` v2 and the evals), U0b (attention). Step 1 (the envelope) is single-owner and merges when no other MCP-touching branch is open; the EM schedules it. Go-ahead: `source_revisions` (its one new event type).
3. **Feature and schema:** feature `source_revisions`; event types `convention.source_revised`, `decision.source_revised`, `open_item.source_revised { id, source, reason }` (R-B). No schema change (sources are read from the journal into `Conventions` as today).
4. **Build:**
   1. **The result envelope** (4.6): in `server.rs`'s result path, every tool result gains `next_steps: [{ tool | prompt, args?, why, required }]`, `guards: [{ code, message, remedy_tool, remedy_args? }]`, `warnings: [..]` (a tool's own `warnings` field is merged into it), and, on every mutating result, `head: { seq, hash }` (the store's `head()` after the call's last write). `next_steps` come from the attention module (U0b) filtered to the subject the call touched, plus tool-specific steps (after a post: attach evidence if missing, `books.show entry`, the affected balances; after a close: `attest_period` (P8); after `export_cpa_package`: the download command). A guard registry (`crates/ledger-mcp/src/guards.rs (new)`): an enum of every guard code with its remedy tool (`stale_preview`, `format_upgrade_pending`, `receipt_required`, `approval_required`, `untagged_control_line`, `duplicate_vendor_ref`, `first_number_locked`, ...). Tests: `every_mutating_result_has_head_and_next_steps`, `every_guard_names_an_existing_tool` (both in `crates/ledger-mcp/src/server.rs` tests, iterating the tool router).
   2. **`books.start` v2** (4.2): the built `StartOut` (`crates/ledger-mcp/src/tools/kept.rs`) gains `book.id`, `book.url` (the local socket's book path, never a filesystem path), `you { principal, kind, scopes, client_id }`, `drafts { building, awaiting_evidence: [..] }`, `reconciliation { tracked, never_reconciled, behind }`, `backup { last_at, through_seq, stale }`, `prompts`, `needs_attention` (top 12 by severity plus counts by kind) and `next_steps` from the one attention function; `_meta["anthropic/alwaysLoad"] = true` on the tool; the `START_MAX` fit loop keeps it under 8 KiB. `agent?: { model, reasoning_effort, harness }` is accepted and written to the access log, nothing else.
   3. **Prompts and resource templates** (4.4, 4.5). Today `opening.rs::capabilities()` enables tools and resources only and there is no prompt router (CD-D2): add `enable_prompts()`, a `tools/prompts.rs` with `ServerHandler::list_prompts`/`get_prompt`, and `list_resource_templates` beside `list_resources`; the bridge's cold-start `opening_initialize` (`crates/ledger-host/src/bridge.rs`) advertises the same capabilities. Prompts: `onboard`, `reconcile { account, month }`, `month_end_close { month }`, `cpa_handoff { year }`, `annual_compliance { year }`, `audit { scope }` (P8 fills its body; P5 ships it pointing at `books.verify`, `books.control_check` and the digest). ~~`migrate`, `process_inbox`, `receipts_from_email`~~ → not built (D-46, D-28, D-17: no drop page, no email-in, no migration tools; 8.4). Each prompt's text lives in `crates/ledger-mcp/prompts/<name>.md` (`include_str!`), with a golden render per prompt on the `p2-served` fixture (`crates/ledger-mcp/tests/prompts.rs`). Resource templates: `books://entries/{id}`, `books://attachments/{id}` (meta plus extracted text once P6 lands), `books://journal/{seq}`, `books://reports/{kind}{?start,end,as_of}`, `books://digest{?since,since_seq,as_of}`.
   4. **`books.journal_history { after_seq?, types?, subject?, limit }`** → `{ lines: [{ seq, ts, type, summary, subject, entry_id? }] }` (R): the only way to "read the journal". Its `summary` comes from the describer in `crates/ledger-gui/src/pages/journal.rs` (`describe` arms), moved to `crates/ledger-store/src/describe.rs (new)` so the viewer and the server share one; it reads lines with `journal::read_journal_and_format` and lists `Store::tail_repairs`. `subject` is `entry:N | invoice:N | bill:N | account:CODE | party:N`.
   5. **Correcting a record's source**: `books.set_convention`, `books.record_decision`, `books.raise_item` accept a call that changes only `source` (citation, who decided, when) with a `reason`; journaled as the `*.source_revised` events (today `set_convention` with only a new source is refused as "already says exactly that": `Ledger::check_convention`). Tests `a_source_only_revision_is_journaled`, `source_revisions_wait_for_the_upgrade`.
   6. **Descriptions, lints, docs.** Drive `tool_descriptions_fit_700_characters`'s pinned count to 0 (the long descriptions' text moves into the handbook, 4.3) and delete the allowance; `instructions_budget` (the connect text ≤ 1,400 characters, `books.start` in the first 512: `handbook::CONNECT_MAX`, `CONNECT_LEAD`) stays green with the final connect text (4.1 as amended: no `preview_token`, no reversal drafts, no `request_tin_entry`). Remove the `party_id`, `document_id`, `cash_account` aliases (S4, S6). Rewrite `docs/mcp-interface.md`'s overview sections to the surface as it is (the per-tool sections are already kept current by the test).
   7. **Agent evals.** `crates/ledger-cli/tests/agent_evals/*.json (new)`: scripted transcripts, no LLM: `[{"call": "books.start", "args": {}}, {"expect_next_steps_contains": "books.set_setting"}, {"call_next_step": 0}, ...]`, run by `agent_evals_follow_only_next_steps` through `common::Books` on a fresh book: onboarding to a first post; a receipt-backed post; a draft awaiting a receipt that posts when it arrives; a reversal; a reconciliation; a month-end close. Each transcript may only call `books.start` first and then a tool named in the previous result's `next_steps`.
5. **Files:** owns `crates/ledger-mcp/src/guards.rs`, `tools/prompts.rs`, `prompts/*.md`, `crates/ledger-store/src/describe.rs`, `crates/ledger-cli/tests/agent_evals/`; touches `server.rs` (the result path), `tools/kept.rs`, `opening.rs`, `bridge.rs`, `handbook.rs`, the 8.0.3 files, `docs/mcp-interface.md`.
6. **Tests and proofs:** named above; no kernel or Lean change.
7. **Done when:** (1) every mutating result carries `head` and non-empty `next_steps`; (2) every guard's remedy tool exists; (3) `books.start` ≤ 8 KiB on the `p2-served` fixture and on the demo books; (4) the prompts list and render; (5) `tool_descriptions_fit_700_characters` pins 0; (6) the six evals pass; (7) `just check` green. The live check (a fresh Claude Code session in an empty folder, and a Codex session, each posting an entry by following `next_steps`) is the EM's, recorded in the As-built block.
8. **As built:** none yet.

**Struck from P5:** ~~`books.job` and the 20 s rule~~ → D-53 (no measured call approaches a client limit over the bridge; jobs come with P9b if claude.ai's 240 s limit is ever real); ~~URL-mode elicitation~~ → nothing local to elicit (D-28); ~~`ledger-view` crate~~ → U0b (`PackageBooks`); ~~`books.viewer_status`~~ → `books.show` already reports `viewer_open`.

#### P6: evidence pipeline

1. **Status:** not started. Rewritten 2026-10-26 (D-32: no sandbox; D-46: no inline bytes; D-17: no email-in; no migration tools).
2. **Depends on:** U0a, U0b. Independent of P4b and P4. Go-ahead: `attachment_text`.
3. **Feature and schema:** feature `attachment_text`; event types `attachment.text_extracted { attachment_id, text_sha256, extractor, version, pages, status: ok | none | failed }`, `attachment.annotated { attachment_id, fields { receipt_number?, payee?, amount?, date?, instrument?, category?, source?, reimbursable_to? }, note? }`, `entry.settles { entry_id, settles: [entry_id] }` (R-B); `attachment.withdrawn` gains `duplicate_of?`. Provisional schema 20: tables `extractions`, `annotations`.
4. **Build:**
   1. **Extraction in-process, in `ledger-import`** (`crates/ledger-import/src/extract.rs (new)`): `pub fn pdf_text(bytes: &[u8]) -> Result<String, ExtractError>` promoted from the test helper in `crates/ledger-import/tests/spec_import.rs` (it reads glyph runs through `hayro_interpret`; `hayro-interpret` moves from dev-dependencies to dependencies); `pub fn page_png(bytes: &[u8], page: usize, max_side_px: u32) -> Result<Vec<u8>, ExtractError>` moved from the viewer's private `pdf_first_page_png`/`render_pdf` (`crates/ledger-gui/src/receipts.rs`), which then call it; `pub fn text_of(bytes, mime_type, file_name) -> Option<String>` moved from `crates/ledger-mcp/src/tin_files.rs` and extended to call `pdf_text` for PDFs, so the filing TIN scan reads a PDF's text too (D-20's own-EIN exemption applies there as well); XLSX cells as exact decimal strings from the sheet XML (zip plus quick-xml, never floats) and CSV/plain text as today. Caps: 25 MiB in, 30 pages, 2 MiB of text; a failure is `status: failed`, never a panic out of the server (`std::panic::catch_unwind` around the hayro call; the process is not isolated, D-32, and the plan says so). No `lopdf`, no `mail-parser`, no subprocess. Tests: `extraction_deterministic` (same bytes, same text and same PNG bytes), `xlsx_cells_are_decimal_strings` (property), `a_pdf_that_crashes_the_reader_is_failed_not_fatal`; fuzz targets `fuzz_extract_pdf`, `fuzz_extract_xlsx` in `fuzz/fuzz_targets/` (listed in `.github/workflows/scheduled.yml`).
   2. **Journaled extraction** (R-A, R-B): `Store::extract_attachment(id)` stores the text as a content-addressed blob and journals `attachment.text_extracted` (idempotent on `(attachment_id, extractor, version)`); `attach_receipt`/`file_document` extract at attach time; the backfill is `wanderbooks books extract ID` over the admin socket (`crates/ledger-host/src/commands.rs`, `admin::Request::Extract`), one attachment at a time, resumable. `Store::verify` checks the text blob's presence and hash and never re-derives it (an extractor upgrade journals a new record, never replaces one). Tests: `extraction_is_journaled_once_per_version`, `extract_waits_for_attachment_text`.
   3. **`books.get_receipt { attachment_id, view: meta | text | page_image { page, max_px } | bytes }`** (R-C): `meta` (default) now includes `text` (the first 8 KiB, with `text_truncated`) and `pages`; `text` the whole text (capped 64 KiB, else a `HandedFile`); `page_image` a PNG as a `HandedFile` (D-46); `bytes` unchanged. Resources `books://attachments/{id}` (meta plus text) and `books://attachments/{id}/page/{n}`. Test: `get_receipt_default_has_text_and_no_bytes`, `page_image_is_a_png_of_the_page`.
   4. **Dedup** (defect 4). Today `Store::attach` allocates a new attachment for identical bytes and only reports `deduplicated` (INVARIANT 3: "each filing is its own attachment"). P6 changes that invariant: `attach_receipt`/`file_document` with bytes already stored return the existing attachment's id and add a link (`attachment.linked`) to the new subject, with `deduplicated: true`; legacy duplicates stay as written. `books.find_attachment { sha256? | receipt_number? | text? }` (R). `books.consolidate_duplicates { confirmed? }` (destructive): for each set of records with one `blob_sha`, link the first record to every subject the others have (`Store::link_attachment`), unlink the others (`unlink_attachment`/`unlink_receipt`), then `withdraw_attachment(dup, "duplicate of #N")` with `duplicate_of`; refused if any entry would be left with no evidence (the gate of P4 step 2 is re-evaluated for every touched entry before the journal append). Tests: `same_bytes_one_record_many_links` (property), `consolidation_never_removes_an_entrys_only_evidence`, `legacy_duplicates_replay_as_written`.
   5. **Annotations** (defect 10): `books.annotate_attachment { attachment_id, fields, note? }` (W; the closed field list above; `amount` a decimal string; `date` a date; `instrument` an instrument id or label); engine receipt numbers: `Ledger::next_receipt_number()` = max annotated `receipt_number` + 1; `reimbursed` for a member-paid entry is derived from `entry.settles` only (`books.set_entry_settles { entry_id, settles }`): where none is recorded, Needs attention asks (no FIFO guess: IZ-18). Tests: `annotations_replay_and_project`, `receipt_numbers_run_from_the_max`, `reimbursed_is_derived_from_settles_only`.
   6. **Company records (D-20):** `file_document` categories gain `notices`, `correspondence`, `plans`, `history` (the list validated in one place, `BooksServer::file_document`; the store's doc comment on `Attachment.category` lists them all, with `statement` and `import_source` which the descriptions already use); the viewer's Documents page groups by category and subject.
   7. Attention (R-F): "extraction failed" (a filed PDF whose text failed, so a human must read it); "member-paid entry with no settlement recorded". Handbook `evidence-and-receipts.md` gains the text/page-image reading order (text first; a page image only when the text is empty or suspicious; never `bytes` into context) and the dedup and annotation procedures. `docs/spec/rules.md` B-44 (extraction is derived and journaled, never trusted over the bytes; dedup by hash; SOURCE: Rev. Proc. 98-25 on electronic records' audit trail; CONVENTION). `docs/mcp-interface.md`, `docs/parity.md`, `docs/receipt-store.md` (the new events and tables), `docs/testing.md` For-P12.
5. **Files:** owns `crates/ledger-import/src/extract.rs`, `crates/ledger-mcp/src/tools/evidence.rs`, the two fuzz targets; touches `crates/ledger-gui/src/receipts.rs`, `crates/ledger-mcp/src/tin_files.rs`, `store.rs` (`attach`), `crates/ledger-host/src/commands.rs`, the 8.0.3 files.
6. **Tests and proofs:** named above; no kernel or Lean change (no money moves).
7. **Done when:** (1) every attachment of the `p2-served` fixture and of the demo books gives a `get_receipt` default answer under 8 KiB; (2) the viewer and the TIN scan read PDFs through the one `extract` module (no `hayro` use left in `ledger-gui` beyond the call, none in `ledger-mcp`); (3) the dedup property and the only-evidence refusal pass; (4) the backfill command runs on a fixture and is idempotent; (5) 8.0.6 in the PR; (6) `just check` green and the two fuzz targets run clean for 10 minutes locally.
8. **As built:** none yet.

**Struck from P6:** ~~`ledger-extract` sandboxed subprocess (rlimits, seccomp, Landlock), `extractor_sandboxed`~~ → D-32, D-53; ~~`lopdf`~~ → hayro; ~~`attach_email`, `attachment.rendered`, `fuzz_eml`~~ → D-17 (email-in waits for the hosted service; a receipt that is an email is printed to PDF by the human, as the first company's convention says); ~~`compare_reference`, `record_inventory`, `check_inventory`, `inventory.recorded`, `reference.compared`~~ → the Sheet and the books as of 2026-10-11 are frozen and the final `verify-sheet` run is a filed workpaper; P10 step 5 cites it (IZ-3b, Z-6); ~~`inline_base64_requires_sha`~~ → D-46; ~~`reimbursed` by FIFO~~ → `entry.settles` only; ~~"the books dir's `company/` folder retired"~~ → done at the P1 and P2 cutovers.

#### P7: the TIN scanner's separators; vault v2 closed

1. **Status:** not started. Rewritten 2026-10-26: D-28 and D-29 keep TINs in the v1 vault with no passphrase, no human pages and no extra handling, so vault v2 (HPKE, keyslots, recovery key, `/h/tin`, `request_tin_entry`, `set_w9_facts`, the GUI passphrase unlock) is closed by D-48 and nothing of it is built.
2. **Depends on:** none.
3. **Feature and schema:** none.
4. **Build:**
   1. `crates/ledger-core/src/parties.rs (changed)`: `has_full_tin` and `redact_full_tins` take commas, periods, parentheses and any run of spaces as separators between the groups (today hyphens and spaces only, so "123,45,6789" passes); the own-EIN exemption (B-26) unchanged. Property test `tin_shapes_with_any_separator_are_found` in `crates/ledger-core/tests/properties.rs` (generated separators; no false positive on a nine-digit D-U-N-S in the `company.duns` key, a routing number in an `ach` payment method's lines, or an amount).
   2. The access-log digest's masking (`crates/ledger-store/src/access.rs`) and the filing scan (`tin_files.rs`) use the same function and gain a test each (`access_log_masks_a_comma_separated_tin`, `a_file_with_a_comma_separated_tin_is_refused`).
   3. `docs/spec/rules.md` B-26 amended; `docs/testing.md` For-P12 (the separator arms).
5. **Files:** owns `has_full_tin`/`redact_full_tins`; touches `access.rs`, `tin_files.rs`, the two docs.
6. **Tests and proofs:** named above.
7. **Done when:** the property passes; the two integration tests pass; `no_mcp_output_contains_full_tin`-style canary tests (the existing TIN canary in `crates/ledger-cli/tests/spec_vault.rs`) still pass; `just check` green.
8. **As built** (2026-10-26, `0176214`, `fd9f678`): vault v2 stays closed (D-48): nothing of HPKE, keyslots, a recovery key or `/h/` pages was built. Departures from Build:
   - Step 1's separators are read as the unit's own guard requires: a run of spaces counts once however long it is (`tin_gap`), and a comma or a parenthesis joins only a 3-2-4 or a 2-7 group (`grouped_tin_runs`), so `1,234,567.89` (1-3-3-2) stays an amount and a nine-digit D-U-N-S number stays `company.duns`'s kind. The truth table and the ASCII-equality property in `crates/ledger-core/tests/spec_parties.rs` pinned the old separator set (`291, 12, 2846` and `12    3456789` were false); they were updated in the same commit, and the property is now `has_full_tin_never_forgets_a_tin` (everything found before is still found).
   - Step 1's ACH payment-method exemption is P4b S4's `invoicing.payment_methods` (`Kind::PaymentMethods`), which is not built; the property pins the two exemptions the books have today (`company.duns`, an amount) and says the S4 kind keeps a routing number out of the rule when it lands.
   - Step 2's "the filing scan uses the same function" is not how `tin_files.rs` is built: B-26 makes its text check deliberately narrow so a bank export's routing numbers and amounts can be filed. It kept its own shapes and gained the comma renderings in `tin_shaped` and `holds_printed_tin` rather than calling `has_full_tin`, which would refuse `an_ordinary_bank_export_passes`.
   - `docs/spec/rules.md` B-26 and `docs/testing.md` P7's For-P12 paragraph are updated; no other doc names this unit.

#### P8: audit and attestations

1. **Status:** not started. Rewritten 2026-10-26: composed from the checks that exist; one bar setting evaluated at query time; one `audit.run` event.
2. **Depends on:** P6 (extracted text for the receipt cross-check), P4 (drafts), U0b. Go-ahead: `audits`.
3. **Feature and schema:** feature `audits`; event types `audit.run { id, through, through_seq, scope, program_version, tb_hash, findings_sha256, workpaper_attachment }`, `receipt.attested { attachment_id, entry_id, by, at, method { model?, reasoning_effort?, adversarial?, verifier_agents? }, amount_checked, date_checked, note? }`, `entry.reviewed { entry_id, by, at, method, note? }`, `period.attested { period_id, by, at, method, audit_run_id }` (R-B; `method` is self-reported and said so, risk 11). Provisional schema 21: tables `audits`, `attestations`. Setting `policy.attestation_bar { reasoning_effort: high, verifier_agents: 2, adversarial: true }` (a new `Kind::AttestationBar`, R-D; default per D-9). Setting `policy.cpa_package_requires_audit: refuse | warn` exists in the registry's plan (5.3) and is added here if missing.
4. **Build:**
   1. **`Finding`** (`crates/ledger-core/src/audit.rs (new)`): `pub struct Finding { code: &'static str, severity: Severity, subject: Subject, message: String }` and `pub fn statements(ledger: &Ledger, through: NaiveDate) -> Vec<Finding>`: the trial balance balances; A = L + E; P&L articulates to equity; `cash_flow(..).reconciles` (code `cash_flow_unfooted`); `control_check(through)` (`control_difference`, `unattached_control_line`, `document_overpaid`); closes in order and each close's member deltas equal `kernel::apportion_cumulative`'s (`close_out_of_order` cannot occur: the engine refuses it; `close_split_drifts`); schedules and depreciation posted through `through` (`Schedule::due(through)`, `FixedAssetRegister::unposted_depreciation()`: `schedule_behind`, `depreciation_behind`); assets' register equals the cost accounts (`register_differs`); every posted entry on or before `through` meets `evidence::requirement` (P4) or is `legacy_reversal_exempt` (informational: listed, not a failure); no draft dated on or before `through` (`draft_open`); tax categories on active P&L accounts (`tax::uncategorized_accounts`); 1099 payees with a decision (`vendor_1099_undecided`, from `form1099`); settings problems (the server's `settings_problems` moves beside it). Each check is one function with a unit test that seeds the defect through `Store` and asserts exactly that finding (`crates/ledger-core/tests/audit.rs`, one test per code; `audit_detects_each_seeded_defect` iterates the code table).
   2. **Integrity** (`crates/ledger-store/src/audit.rs (new)`): `Store::integrity_findings() -> Vec<Finding>` composing `verify_and_heal`/`verify` (chain, projection, `differs_from`), `verify_blobs`, the access log's chain (`access::read`), extraction blobs present (P6), backup standing (`Backups::standing`, stale after a day: `backup_stale`, `drill_failed`).
   3. **`books.audit { through, scope: month | annual | cpa_handoff | full, confirmed? }`** (R-C; P scope; preview lists what it will check): runs 1 and 2, writes the workpaper (JSON, and a PDF through Typst `templates/workpaper.typ`, deterministic bytes), files it with `Store::file_document("workpaper", ..)` (a new category; `Subject::Period` link for a month or year scope), journals `audit.run`. Idempotent on `(through, through_seq, program_version)`: the same head gives the same findings and the same sha (`audit_is_deterministic`). "Current head" for attestations means: no figure-bearing event (an entry, a document, a close, a settings change, a role change) after the audit's `through_seq`; audit and attestation events do not count (IZ-1k). `books.workpaper { id }` (R) returns the JSON and hands the PDF over. `books.verify` keeps its own checks and its output; `books.audit` is the whole programme.
   4. **Attestations:** `books.attest_receipt { attachment_id, entry_id, amount, date, method?, note? }`: the engine compares `amount` and `date` with the entry's cash line and date; a mismatch is recorded as `disagrees: true` and never counts. `books.review_entry { entry_id, method?, note? }`. `books.attest_period { period, method?, note? }`: refused unless an `audit.run` exists at the current head (as defined above) for a scope covering the period. `books.attestations { through?, kind? }` (R) lists them with `counted: bool`, evaluated against `policy.attestation_bar` as it stands when asked (no bar versioning; D-9's bar; a bar change changes what counts, prospectively and retrospectively alike, and the handbook says so). `books.export_cpa_package` refuses when `policy.cpa_package_requires_audit = refuse` and no `audit.run` with scope `cpa_handoff` or `full` at the current head has its period attested, unless `without_audit_reason` is given (journaled as a decision, D-9).
   5. Attention (R-F): "month-end attestation due" after each close without one; "full audit due before the CPA handoff" when the fiscal year has closed; "annual audit due". The `audit` prompt (P5) gets its body: separate verifier agents, per receipt `get_receipt` text then `page_image`, `attest_receipt` by each, `review_entry`, `attest_period`. Handbook `attestations-and-audit.md` (R-E): the bar, what counts, what the CPA package needs. `docs/spec/rules.md` B-45 (an audit is deterministic over the journal; attestations are claims by an authenticated client; SOURCE: Pub. 583, Rev. Proc. 98-25; DECISION D-9). `docs/mcp-interface.md`, `docs/parity.md`, `docs/receipt-store.md`, `docs/testing.md` For-P12.
5. **Files:** owns `crates/ledger-core/src/audit.rs`, `crates/ledger-store/src/audit.rs`, `crates/ledger-mcp/src/tools/audit.rs`, `templates/workpaper.typ`, `crates/ledger-cli/tests/audit.rs`, `handbook/attestations-and-audit.md`; touches the 8.0.3 files, `tools/prompts.rs`, the CPA package builder.
6. **Tests and proofs:** `audit_is_deterministic`, `audit_detects_each_seeded_defect`, `amount_mismatch_never_counts`, `below_bar_not_counted`, `stale_head_period_attestation_refused`, `cpa_package_refuses_without_a_counted_audit_unless_a_reason_is_given`, `audits_wait_for_the_upgrade`, `workpaper_pdf_is_deterministic`; no kernel change (the audit calls kernel functions; it adds none); Lean: none (the laws it checks are already theorems).
7. **Done when:** (1) the code table has one row per check with its predicate, code, seeding fixture and severity, and `audit_detects_each_seeded_defect` iterates it; (2) a fixture with an unfooted cash flow, a missing receipt, an open draft and a duplicate blob produces exactly those findings; (3) the same head gives the same workpaper sha twice; (4) `export_cpa_package` refuses and the override is journaled; (5) 8.0.6 in the PR; (6) `just check` green.
8. **As built:** none yet.

**Struck from P8:** ~~bar versioning (`counts` derived against the bar in force at that seq; `bar_change_is_prospective`), the Lean counting lemma~~ → IZ-3d; ~~"vault public-key sha"~~ → D-48; ~~"access-log tooling"~~ → `books.access_log` exists (P2); ~~`books.audit` run as a job~~ → D-53.

#### P9a: self-host recipe, the move, a static bearer over a tunnel

1. **Status:** not started. P9 was split on 2026-10-26 (D-51): P9a is the owner's own use of a second machine; P9b (claude.ai through an external identity provider) waits for Q-6; P9c (a remote viewer replica, invitations) is not planned.
2. **Depends on:** none in code. To try it: a second machine or VPS the owner controls, a Tailscale account (or cloudflared), a domain and TLS.
3. **Feature and schema:** none.
4. **Build:**
   1. `deploy/wanderbooks-server.service (new)`: a system unit running `wanderbooks serve --root /srv/wanderbooks` as a dedicated account (`DynamicUser=no`, `User=wanderbooks`), `ProtectSystem=strict`, `ReadWritePaths=/srv/wanderbooks`, `NoNewPrivileges=yes`, `IPAddressDeny=any` with `IPAddressAllow=localhost` (the server listens on its Unix socket and, for the tunnel, on 127.0.0.1 with `--http`); no seccomp or Landlock lines (D-32 applies to the program; the unit's own hardening is the operator's). `deploy/README.md` (new): the recipe in numbered steps: install the one executable; `wanderbooks setup` as that account; move the books (`wanderbooks books adopt <copy> --id X` on the server from a backup clone or a copied data dir, then `books.verify` and compare the head with the desktop's: `commands::adopt` and `admin::Request::AdoptPreview` already do the head check; ~~`wanderbooks fetch`~~ → D-31: a backup clone is the transport); make a token (`wanderbooks token create --book X --name laptop`); Caddy for TLS; the tunnel (Tailscale Funnel or cloudflared) to the local HTTP port; on the desktop, `claude mcp add --transport http wanderbooks-X https://<host>/b/X/mcp` with the header helper `wanderbooks token print` (3.3), and Codex likewise. `books.start` reports `isolation: "service_user (enforced)"` when the server finds itself under such a unit (a `--isolation enforced` flag the unit passes; the server never guesses).
   2. `.github/workflows/two-user.yml (new)`: a job that installs the executable in a container, runs the server as one user and the test client as another, and runs `crates/ledger-cli/tests/two_user.rs::agent_uid_cannot_read_tenant` (new: the client's user cannot read `/srv/wanderbooks`, every tool still works over HTTP with the bearer, `books.start` says enforced) and `self_host_adopt_same_head` (the moved books verify to the same head).
   3. Handbook `self-host.md` (R-E): how the owner moves books and connects; `docs/architecture.md` 3.8 updated (as built).
5. **Files:** owns `deploy/`, `.github/workflows/two-user.yml`, `crates/ledger-cli/tests/two_user.rs`, `handbook/self-host.md`; touches `books.start`'s `isolation` (`tools/kept.rs`), `crates/ledger-host/src/commands.rs` (the flag).
6. **Tests and proofs:** `agent_uid_cannot_read_tenant`, `self_host_adopt_same_head`; `isolation_label_is_enforced_under_the_unit`.
7. **Done when:** (1) the CI job passes; (2) the recipe, followed on a clean VM by someone other than its author, reaches `books.start` from a desktop Claude Code in under 30 minutes (recorded in the As-built block); (3) `just check` green.
8. **As built:** none yet.

**P9b (planned, not scheduled; after Q-6):** claude.ai and ChatGPT need OAuth 2.1. The recommendation is D-7 (b): an external identity provider (Authentik, Ory or Keycloak) issues tokens and the server only validates them (audience = the book URI, `iss`, expiry), so no authorization server is written in-house and no external security review is needed; the Origin allowlist gains `https://claude.ai`; `books.job` and the 240 s rule come here if a call is measured over the limit. **P9c (not planned, D-51):** the remote viewer replica (`/replica/*`, `Snapshot::from_lines`, the blob LRU), `books.invite`, the `/h/` pages for remote people, the MCP App upload widget and `openai/fileParams`. The viewer on another machine reads a backup clone (D-30, D-31) read-only instead.

#### P10: the re-audit, through the MCP alone

1. **Status:** not started. P10 is the owner's acceptance, run on the owner's books by the owner's agents; the engineering deliverable is small.
2. **Depends on:** P5, P6, P8 (and P4b, P4 through them).
3. **Feature and schema:** none.
4. **Build:**
   1. `docs/runbooks/re-audit.md (new)`: the steps, each naming the tool as it exists: (1) `books.audit { scope: full, through: <month end> }` and `{ scope: cpa_handoff }`; every finding fixed through journaled corrections (reversals with the original's evidence per D-12 and D-47, annotations, decisions); (2) every receipt attested from `get_receipt` text plus `page_image` by a pair of verifier agents at the bar (D-9), no bytes in context; (3) `review_entry` for every posted entry; `attest_period` monthly since formation and `cpa_handoff` for the year; (4) each defect regression test of the 24 (the 2026-10-16 audit's 1-20, P0's 21 and 22, 23 and 24 found in P3) named and shown green; (5) the Sheet comparison as of 2026-10-11: cite the filed `verify-sheet` workpaper (P2 pre-cutover) and record `decision.recorded` that the frozen Sheet and the books agreed then; no new comparison tool; (6) the CPA package from both doors (the agent's `export_cpa_package` with TINs by default, and the viewer's Export menu: D-29), byte-identical but for timestamps; (7) `books.verify` ok and a backup drill clean (`books.backup` shows the head and the drill), the owner's sign-off journaled as `decision.recorded`. The two cases: T1 (desktop only: Claude Code and Codex; separation advisory; evidence = the access log and transcripts that never name the data root) and T2 (a self-host machine running: add claude.ai through the tunnel when P9b exists; separation enforced).
   2. `crates/ledger-cli/tests/defects.rs::every_defect_reproduction_is_a_named_test (new)`: asserts the 24 named regression tests exist in the workspace (a list of `(defect, test name, file)`; the test greps for each).
5. **Files:** owns `docs/runbooks/re-audit.md`, `crates/ledger-cli/tests/defects.rs`.
6. **Tests and proofs:** the one test.
7. **Done when (engineering):** the runbook exists and names only tools and commands that exist; the test passes. **Exit (the owner's run):** zero failing workpaper checks; cash flow difference 0.00; every receipt and posted entry carries a counted attestation and none disagrees; the access log shows every action under the right principal and no transcript names the data root; no result carried a full TIN to a client that should not see one (D-29: the agent's exports may); agent context on receipts stayed under 100K tokens per the harness's own token count; the sign-off journaled.
8. **As built:** none yet.

#### P11a: packaging and updates (D-19, D-22, D-24, D-25, D-26)

1. **Status:** not started. P11 was split 2026-10-26 (D-54): P11a is installers and updates; P11b the agent dock and books homes.
2. **Depends on:** none in code. From the owner before the signing steps: the Apple Developer ID certificate and notarization credentials as CI secrets; the Windows signing identity (Azure Artifact Signing if eligible, else an OV certificate); GitHub Releases enabled on the public mirror; the release hardware token; clean VMs or runners for the install tests. Q-4 (keep the two-key release manifest of D-25, or minisign alone) before step 5.
3. **Feature and schema:** none.
4. **Build:**
   1. **The trial** (2-3 days): build all six formats with `cargo packager` (pinned in `mise.toml`: `cargo-packager`, `cargo-packager-updater`, `cargo-generate-rpm`) on clean VMs; record in `docs/packaging.md (new)` what each produced, what failed, and the decision for each format (a table: format | built by | installs as | updated by | signed by). Anything the tooling cannot meet is written there before anything is built around it.
   2. **`wanderbooks update` and `rollback`** (`crates/ledger-host/src/commands.rs (changed)`, a new `update.rs`): check the feed (`releases/latest/download/manifest.json` on GitHub Releases, D-26), download and verify (step 5's signatures), stage under `<root>/updates/<version>/`, swap the executable atomically, then trigger the restart the host already has (`host.rs` INVARIANT 7: `run/update.lock`, `Ending::Restart`, `restart()`; `bridge.rs` `with_list_changed`: these exist; the updater only triggers them). `rollback` swaps back to the kept previous executable and refuses for a book that consented to a newer format (3.9). A package-managed install (AUR, apt, dnf) refuses `update` and says to use the package manager; the host notices the replaced executable and restarts when idle (exists). The format gate is unchanged (3.9). Tested against a `file://` feed: `update_is_atomic`, `failed_update_rolls_back`, `unsigned_update_refused` (a bad signature, a sha that differs from the manifest, each refused, nothing changed), `bridge_survives_update`, `update_waits_for_a_running_call`, `package_managed_install_refuses_update`.
   3. **Linux packages** in CI containers (unsigned first): `.deb` with an apt repository index and `.rpm` with a dnf index served from GitHub Pages; the AUR `PKGBUILD`; the AppImage with self-update. `linux_packages_install_and_upgrade` (Arch, Debian, Fedora containers: install, open, upgrade, the host restarts, the books' head unchanged).
   4. **Updates staged, applied at restart**: the host checks daily while the books are open, downloads, verifies, stages; applied at the next restart the owner accepts in the app, or before the host next starts; never during a running call (3.9).
   5. **Signing**: minisign through `cargo-packager-updater` plus OS code signing (Developer ID and notarization; the Windows identity). The two-key release manifest (two Ed25519 keys, rotation statements, a monotone version, an expiry) per the owner's answer to Q-4; D-25 and D-26 as recorded say to build it; if the owner answers "minisign only", D-25 is amended and this step is the minisign step alone.
   6. **macOS and Windows bundles**: the `.dmg` (translocated-app detection: refuse to connect agents until moved out of Downloads), the per-user NSIS `.exe`; `install_needs_no_admin` on clean VMs; `translocated_app_refuses_to_connect`. The viewer's menu gets "Check for updates" (`Owner::check_for_updates`, over the admin socket like the other items).
   7. `docs/architecture.md`, the website's install page, `docs/parity.md`.
5. **Files:** owns `crates/ledger-host/src/update.rs`, `deploy/packaging/*`, `.github/workflows/release.yml`, `docs/packaging.md`; touches `commands.rs`, `crates/ledger-gui/src/owner.rs`, `mise.toml`.
6. **Tests and proofs:** named above.
7. **Done when:** (1) on a clean Linux VM the AppImage installs, opens, onboards and posts an entry with no terminal and no admin password, and updates itself from a test feed; (2) on Arch, Debian and Fedora the packages install and upgrade with the head unchanged; (3) an update leaves every book's head and goldens unchanged and a bridge session carries on across it; (4) a backup made on one system restores on another; (5) macOS and Windows: the same as (1), once the signing prerequisites exist (record which are done in the As-built block); (6) `just check` green.
8. **As built:** none yet.

#### P11b: the agent dock and books homes (D-27)

1. **Status:** not started.
2. **Depends on:** P11a (an installed app). Independent of the P10 chain.
3. **Feature and schema:** none.
4. **Build:**
   1. **Books homes.** `commands::books_home(book) -> PathBuf` (new): `~/Documents/Wanderbooks/<Company>/` (the user's Documents folder on Windows), created by `New books…` and by `wanderbooks connect`; it holds exactly: `AGENTS.md` and `CLAUDE.md` (three lines: "These are <Company>'s books. Connect the wanderbooks MCP and call `books.start`. The books live in the app's private folder, not here."), `.mcp.json` (the project-level MCP server entry: `wanderbooks mcp --book <id>`), `.claude/settings.json` allowing `mcp__wanderbooks-<book>__*` so the daily flow doesn't prompt. No books data, no key, no token, no path into the data root. `connect_agent` is unchanged (it registers the user-scope server as today); the books home adds the project-level entry. Whether `connect codex` may mark the folder trusted is asked of the owner when built (Q-7, placeholder).
   2. **The dock.** A terminal panel along the bottom of the viewer's window (`crates/ledger-gui/src/dock.rs (new)`, `alacritty_terminal` over a PTY; ConPTY on Windows), resizable and collapsible; the header picks Claude Code (default) or Codex, remembered as a viewer preference. The agent is started as an ordinary child process of the viewer's `Owner` (`Owner::start_agent(Agent, book) -> Child`, over the same ledger-host functions the command line uses) in the book's books home; ~~"started by the viewer's helper … the viewer stays confined"~~ → D-32: there is no helper and no confinement; the viewer is read-only by design and by test, which the dock does not change (the agent writes through the server, not through the viewer). The CLIs are found or offered through their vendors' installers, never bundled.
   3. Tests: `dock_runs_the_chosen_agent` (a fake `claude` on the PATH: started in the books home, its output reaches the panel, Ctrl-C reaches it, resize redraws), `books_home_holds_no_books_data` (only the four files; none names the data root), `agent_missing_offers_the_installer`. Windows: ConPTY checked in the trial; an agent that fails the four checks (starts, `/login` works, resize redraws, Ctrl-C reaches it) is not offered there.
5. **Files:** owns `crates/ledger-gui/src/dock.rs`, `commands::books_home`; touches `owner.rs`, `app.rs`, `commands.rs`.
6. **Tests and proofs:** named above.
7. **Done when:** the three tests pass on Linux; the dock runs Claude Code in a books home on Linux and macOS; Windows recorded as offered or not.
8. **As built:** none yet.

#### P12: max testing, again (D-33)

1. **Status:** not started.
2. **Depends on:** everything else merged. Hardware: the owner's desktop, or the scheduled GitHub Actions workflow (`.github/workflows/scheduled.yml`), or both; the gate for Accept is the scheduled workflow's run (reproducible by anyone), with the desktop as the faster place to triage.
3. **Feature and schema:** none.
4. **Build:**
   1. Code first (2-3 days): the fuzz targets P2 promised and never added, `fuzz_http_mcp_request` and `fuzz_upload_put` (`fuzz/fuzz_targets/`, listed in scheduled.yml); `fuzz_op_sequence::Op` gains the document, settlement, write-off and draft-state ops; mutation recipes in the justfile for the crates that have none (`mutants-host`, `mutants-package`, `mutants-vault`, `mutants-cli`, each with `--test-package` set like `mutants-mcp`) and their shards in scheduled.yml; `docs/testing.md` consolidates the ~30 **For P12** paragraphs into one checklist per crate (U0c adds the three missing ones).
   2. The run: `cargo mutants` over the whole diff from `168494b` (main before P2 slice 3) to the tip, crate by crate, each crate's own tests first and the survivors again with the end-to-end `ledger-cli` tests (`--iterate`); every survivor killed by a test or documented as equivalent in `docs/testing.md` with its reason; the viewer has no mutation run (its two musts are `tests/read_only.rs` and `tests/view_model.rs`). Every fuzz target for 4 hours (or the owner's number), any crash a regression test. Property tests at their maxima: `STORE_CASES=5000`, `WB_ORDERS_DEPTH=6`, the proptest case counts documented in `docs/testing.md`. The GnuCash differential (`just oracle`). `just prove-all`. Load: `cargo mutants -j <half the cores>`, one mutation run at a time.
5. **Files:** owns the justfile mutation recipes, the two fuzz targets, scheduled.yml, `docs/testing.md`'s P12 section.
6. **Tests and proofs:** the run itself.
7. **Done when:** 0 unexplained mutation survivors workspace-wide (the viewer aside); every fuzz target clean for its time; `just prove-all` and `just check` green; `docs/testing.md` records each crate's counts and every equivalent mutant with its reason.
8. **As built:** none yet.

#### P13: bookspec, a formal accounting specification (D-34)

1. **Status:** not started as a unit. The seed exists in this repository: `bookspec/README.md` (the TOML schema: `id, title, rule, status, sources, tags, appendix?`; `[setup]`; `[[events]]` of kinds `lines | reverses | close | bill | invoice | run_depreciation | dispose_asset | run_schedules | register_schedule` with `refused`, `payee`, `parties`, `instrument`, `older`; `[[changes]]`; `[[expect]]` with its query table), 56 examples in `bookspec/examples/`, the loader `crates/ledger-core/tests/bookspec.rs` (`every_bookspec_example_holds`: applies each example to a real `Store` and verifies it) and `crates/ledger-package/tests/package.rs::the_bookspec_iif_example_matches_the_engine`. Every P3 regression test and every P4b example is written there as it is built. The spec text, the formal validation and the conformance harness are the unit.
2. **Depends on:** nothing for steps 1-4; the engine stable (P4b, P4 landed) for step 5. Runs beside everything.
3. **Feature and schema:** none.
4. **Build:**
   1. **The checker crate.** Move the loader's types out of the test into `bookspec/checker/` (its own Cargo workspace inside the repository for now: `bookspec/Cargo.toml`), exposing `Example`, `Setup`, `Event`, `Expect`, `Query`, `Refusal` and the TOML parser with `deny_unknown_fields`; `crates/ledger-core/tests/bookspec.rs` becomes the wanderbooks adapter depending on it by path. `just check` keeps running every example (the test is unchanged in what it proves).
   2. **The `Engine` trait** (`bookspec/checker/src/engine.rs`): `pub trait Engine { fn load(&mut self, setup: &Setup) -> Result<(), Refusal>; fn apply(&mut self, event: &Event) -> Result<(), Refusal>; fn query(&self, q: &Query) -> Result<serde_json::Value, Refusal>; fn claims(&self) -> Claims; }`, `Refusal { rule: RuleId, reason: String }`, `Claims` = the rule ids and choices the engine declares. The wanderbooks adapter implements it over `Store`; the runner (`bookspec/runner`) reports per rule: pass, fail, not claimed.
   3. **Rule identifiers.** `BS-<area>-<n>` (areas: `entry`, `ledger`, `statement`, `period`, `partner`, `asset`, `prepaid`, `arap`, `revenue`, `1099`, `format`); a table from B-1..B-45 to bookspec ids in `bookspec/spec/rules-index.md`; `docs/spec/rules.md` gains each rule's bookspec id; rules.md `DECISION` becomes bookspec `CHOICE` (the spec names the options; an engine declares its pick).
   4. **The specification text** (`bookspec/spec/*.md`): definitions and the model; every rule with its normative statement (MUST, MUST NOT, MAY), its source pinned exactly and its status (AUTHORITY, CONVENTION, CHOICE), its symbolic form, its concrete examples (the TOML files, by id), its applicability. First edition done line: every B-rule has an id, a statement, a source and at least one example; the five laws (debits equal credits; A = L + E; the trial balance balances; net income articulates to equity; the cash-flow sections sum to the change in cash) are Lean theorems in `bookspec/lean/` (ported from `verify/lean`); every citation resolves to a vendored source (`docs/spec/sources/`) or an exact external reference; US GAAP (the FASB Codification, cited never copied) and US federal tax (IRC, Treasury Regulations, IRS publications and form instructions, public domain) first; the Big Four guides never a source; a page budget of 60 pages for the first edition.
   5. **The spec's own checker:** every example's expected value derived from its rule's symbolic form by the checker (exact decimal), so an example whose expectation disagrees with its derivation fails the spec's build; the runner against wanderbooks for every claimed rule; the GnuCash oracle (`docs/oracle.md`) as the second engine, through an adapter over the oracle's files; hledger, Beancount and QuickBooks are candidates, not scope.
   6. Later, when step 5 is stable: its own public repository (CC BY 4.0 for the spec, MIT OR Apache-2.0 for the code), consumed by wanderbooks as a pinned dependency so `just check` keeps running the examples (D-34, Z-17).
5. **Files:** owns `bookspec/**`; touches `crates/ledger-core/tests/bookspec.rs`, `crates/ledger-package/tests/package.rs`, `docs/spec/rules.md` (ids), `Cargo.toml` (workspace members if the checker joins the workspace).
6. **Tests and proofs:** `every_bookspec_example_holds` unchanged; `bookspec/checker` tests (the schema, the derivations); `bookspec/lean` builds with standard axioms only.
7. **Done when:** (1) the checker crate exists and `just check` still checks all 56+ examples; (2) the `Engine` trait and the wanderbooks adapter run the runner green for every claimed rule; (3) every B-rule has an id, statement, source and example; (4) the five laws are theorems in `bookspec/lean`; (5) the GnuCash adapter runs and its differences are settled from the sources (recorded in the As-built block).
8. **As built:** none yet.

### 8.3 P4b reference design: receivables and payables, first class (D-45)

The design the S4 to S7 units build from. Corrected on 2026-10-26 against what S1 to S3 built (8.1) and the review's findings; superseded sentences are struck through with the decision that superseded them. In this section "document" means an invoice or a bill; "preview" means a tool called without `confirmed` (nothing is journaled); "draft" means a draft entry (P4).

#### 8.3.1 The owner's ask and answers (2026-10-23)

**The ask:** "we kinda need stronger invoice support. Like make it a first class citizen. It could even spit out PDF invoices, track when they're sent, money received against, etc.. and maybe there's some accounting term for the opposite, like if we receive an invoice (we have to pay someone else) we need to pay it on its specific listed payment terms, tracking all that."

**The answers, through the question tool.** Numbered A-1 to A-14 so the units can cite them; A-5 to A-14 were one answer ("the rest as recommended") and are split here one per line.
- **A-1.** Preview, then confirm, for invoices and bills, as for entries; an abandoned preview uses up no number.
- **A-2.** Numbers `INV-0001`, running: the owner's prefix and a running number, with a one-time starting number; never reused; a void keeps its number.
- **A-3.** Payment methods: any ("Full ACH, payment link, QR code, whatever whatever. Companies accept payments in SO MANY ways, we should support anything."). So `invoicing.payment_methods` is a list, each method `{ kind: ach | wire | link | check | p2p | crypto | other, label, lines, link?, qr? }`; the PDF prints each with its label and lines, and a QR code (an SVG from `labels::qr_svg`, placed as an image) for any method with `qr: true` and a link or text to encode. The bank-detail lines of `ach` and `wire` methods are exempt from the nine-digit TIN check (a routing number is nine digits), as `company.duns` is; nothing else in the list is.
- **A-4.** The books never send, receive or charge; they record ("Wanderbooks just does books. Nothing else. … it's more likely my agent (who talks to wanderbooks MCP) doing the sending and receiving and payment capture … We took the same approach on payroll."). So `books.mark_sent` records who sent it (the owner, or a named delegate such as an agent), the channel (email, mail, portal, hand, other), the recipients, when, and the channel's own reference (a message id); a payment records its processor and reference (a Stripe charge id, a bank trace) and, when the processor kept a fee, the fee: Accounts Receivable is credited the gross amount, the fee goes to its expense account, the deposit account gets the net (the gross presentation payouts already use). The `mailto` link stays as a convenience for a human sender.
- **A-5.** Bad debts by direct write-off (IRC §166; Treas. Reg. §§1.166-2, 1.166-3).
- **A-6.** Hand entries to 1200 and 2000 are refused (B-23; built in S1).
- **A-7.** The issued PDF is stored at issue.
- **A-8.** Discounts by the gross method.
- **A-9.** The bills posted to 6999 are fixed by reclassifying entries.
- **A-10.** Every bill line names its account.
- **A-11.** ~~Two upgrades (after S1, and after S7)~~ → the first (`document_standing`) was given 2026-10-24; D-52 makes the second the only remaining one: `documents_v2` covers S4 to S7 (and `drafts_v2` joins it), declared once after S7. Q-5 asks whether to declare it earlier.
- **A-12.** Overpayments refused; customer credits later (8.4).
- **A-13.** Each party gets an email and default terms, copied onto each document at issue.
- **A-14.** (Reserved: the owner's answer to Q-5.)

#### 8.3.2 In plain language

Before S1 an invoice kept its own copy of "how much was paid" and "is it paid", and that copy went out of step with the ledger: a reversed payment still counted, a reversed invoice still aged, a wrong payment reversed blocked the right one. S1 fixed that: a document stores no figure. Every ledger line on Accounts Receivable or Accounts Payable is tagged with its document, and a document's balance is the sum of its tagged lines dated on or before the day asked about. Reversals, voids, bounced payments and write-offs come out right without extra code, and the control accounts always equal their documents, by construction.

On top of that come (S4 to S7): invoice numbers, payment terms and due dates; a PDF kept exactly as it was sent; a record of when it was sent; bills coded to their real accounts with the vendor's invoice number; a "what to pay this week" list that follows each vendor's terms; payments that settle several documents at once, with early-payment discounts and the processor's fee; voids and bad-debt write-offs.

**Terms, each explained once:**
- **Accrual basis.** Income counts when it is earned and expenses when they are incurred, not when cash moves (IRS Pub. 538, "Accrual Method"; the all-events test in Treas. Reg. §1.451-1(a) for income and §1.461-1(a)(2) for expenses, with economic performance under §1.461-4). The company files on this basis (`books.accounting_basis`). Income is earned when the work is delivered (ASC 606-10-25-23), not when the invoice is written; an invoice normally carries the date the work was done. Billing ahead of the work goes to 2400 Deferred Revenue (a contract liability, ASC 606-10-45-2), and a B-16 schedule recognizes it as the work is done.
- **Invoice; accounts receivable (A/R, 1200).** The bill we send a customer; until paid, a receivable, an asset: "an entity's right to consideration that is unconditional" (ASC 606-10-45-4); presentation ASC 310-10; expected credit losses ASC 326-20.
- **Bill; accounts payable (A/P, 2000).** An invoice someone sends us; a liability that leaves the books only when paid or when the creditor legally releases us (ASC 405-20-40-1). A bill is never written off: paid, credited by the vendor, or voided as an error.
- **Control account; subledger.** 1200 and 2000 are control accounts: each balance must equal the total of the open documents behind it (B-23, B-38). The first thing a CPA checks is that the aging ties to the trial balance; here it ties by construction, and `books.control_check` shows it.
- **Aging.** Open balances by days past due: Current, 1-30, 31-60, 61-90, 90+ (B-18; ASC 326-20-55-37/38 illustrative). Due today is Current.
- **Payment terms.** "Due on receipt": due on the document date. "Net 30": 30 days after. "Net 15 EOM": 15 days after the end of the document's month. "2/10 net 30": 2% off if paid within 10 days, else the full amount in 30. For goods with no agreed terms, due when the buyer receives them (UCC §2-310(a)); for services the contract sets the terms. Sending an invoice late does not move its due date.
- **Gross method.** The document is booked at its full price and a discount only when taken. ASC 606-10-32-5 to 32-9 strictly treat an expected discount as variable consideration estimated when revenue is recognized; at this company's size the difference is immaterial: a stated simplification (DECISION, A-8), not ASC 606's own treatment.
- **Void.** The document should never have existed (a duplicate, the wrong customer): its entry is reversed (B-9); it keeps its number, marked VOID, so a gap in the series always has a reason. Built in S1 (`books.void_document`).
- **Credit memo; vendor credit.** Lowers a valid document with no cash moving. Not scheduled (8.4).
- **Bad debt (write-off).** An invoice that will not be collected. Tax: IRC §166(a); worthless on "all pertinent evidence" (Treas. Reg. §1.166-2(a)); a partial write-off deductible to the extent charged off on the books (§1.166-3(a)); allowed only because accrual already counted the income (§1.166-1(e)); a later recovery is income (IRC §111; §1.166-1(f)); Form 1065 line 12. GAAP prefers an allowance (ASC 326-20); for small receivables a CPA accepts the direct method and notes it.
- **"Paid in full" checks (UCC §3-311).** Cashing a check marked as full payment of a disputed amount can settle the whole claim; the payment tool warns before recording one.
- **Records.** Pub. 583 lists invoices among the documents to keep; Rev. Proc. 98-25 requires electronic records to keep their audit trail: why the PDF as issued is filed with its hash and voids keep their numbers.

**Rules this section fixes (so no one has to find them in the prose):** due today is Current; the sent date never moves the due date; `paid_in_full_marking` warns under UCC §3-311; the gross method is a DECISION (A-8); the CPA's list below goes into the handbook.

**What a CPA expects (goes into `invoices-and-bills.md`):** agings at month and year end that tie to 1200 and 2000; invoice numbers unique and sequential, voids visible, none deleted; every bill coded to its real account (the Form 1065 lines, B-32), never a catch-all, with the vendor's invoice attached, no bill paid twice; payments matched to documents, card payments identified (excluded from 1099-NEC: Treas. Reg. §1.6041-1(a)(1)(iv); D-40); write-offs with a reason and evidence of collection attempts; customer deposits as liabilities, never netted inside A/R; Schedule L lines 2a and 15 straight from the control accounts; approvals and three-way matching deliberately not adopted for a two-member LLC (light governance), and the handbook says so.

#### 8.3.3 The core mechanism (built in S1): documents read their figures from tagged ledger lines

- **Tags.** `Ledger.line_docs: HashMap<(EntryId, usize), DocTag { doc: DocRef, movement: Movement }>`, beside `cleared` and the class tags; the kernel's `JournalLine` and its hash untouched. Live, only the document commands write tags, in the same step that posts the entry. On replay, tags come from the document events already journaled, with no format change: `invoice.created`/`bill.created` tag the one control-role line of the entry just replayed (the role as it stood at that point of the journal); `invoice.payment_received`/`bill.paid` tag "the first untagged control line for the payment's amount" (rules.md B-38 item 1), so several payment events naming one entry tag its lines in order; S6 adds an explicit `line` index to new payment events. `reverse_entry_steps` copies each line's tag onto its mirror. An entry whose lines cannot be matched stays untagged and is listed by `control_check`; replay never refuses (B-28).
- **Standing.** `Ledger::stands_on(entry, as_of)`: posted, dated on or before `as_of`, and an even number of the reversals after it in its chain are dated by then, counted in chain order (a re-reversal dated before the reversal it undoes counts from that reversal's date). `in_effect(e) = stands_on(e, NaiveDate::MAX)`.
- **Derived figures.** `outstanding(d, D)` = the signed sum of the posted lines tagged `d` dated on or before `D` (a debit to A/R counts plus for an invoice; a credit to A/P plus for a bill), **whatever their entries' standing**: a reversal's mirror carries the same tag and cancels its original, so standing never enters the sum. ~~"over entries that stand on D"~~ → wrong (it double-counts a re-reversal: origin +100, reversal −100, re-reversal +100 would sum to +200); the code and rules.md B-38 item 2 have the date-sum, and the Lean theorem is stated over it. `paid`, `discounted`, `written_off` are the same sum over one movement each. Status as of D (`arap::doc_status`): **Void** if the origin entry does not stand on D; else **Unattached** if the origin's control line carries no tag; else **Overpaid** if outstanding < 0; else **Written off** if outstanding is 0 and written_off > 0; else **Paid** if 0; else **Partial** if < total; else **Open**. The breakdown is always shown. Overdue and sent are flags.
- **Stored fields.** ~~`Invoice.amount_paid`, `status`, `invoice_payments`, `bill_payments` stop being state~~ → as built, `amount_paid`, `owed` and `status` stay on `Invoice`/`Bill` as derived caches refreshed by `refresh_doc`, never journaled; every reader (the viewer, the MCP, `cashflow::bill_splits`, party totals, the 1099 worksheet, the projection, the IIF) reads the derived figures.
- **Bounds.** `check_item_bounds` (private; run inside the payment path and from `check_reversal`, only on the documents the step touches): at every date in date order, 0 ≤ outstanding ≤ total. So: no void while a payment stands; no overpayment; no reversal of an origin under a payment. Reversing a payment, a discount or a write-off is always allowed and reopens the document. A payment received before a bounce is known is recorded through 2300 (the handbook procedure in S6; D-49).
- **The control guard.** A line on an account holding the `receivables` or `payables` role (or holding a line of one of the role's documents) must carry a tag, or the posting is refused with `LedgerError::UntaggedControlLine { account, entry, line }` and the remedy names the document tools. It sits in `Ledger`, so MCP, GUI, recurring runs and templates cannot bypass it; `books.add_line`, `books.draft_entry` and `books.create_recurring` refuse earlier with the same remedy. The one exception is the mirror of an untagged legacy line in a reversal that takes the line off the books; reversing that cleanup (which would put it back) is refused (B-38 item 3, a deliberate B-10 exception). Replay never refuses. ~~"or a later control role"~~ → the receivables and payables roles only (payroll's payables are untagged by design).
- **The identity.** For each control account A and date D: `GL(A, D) = Σ_docs outstanding(d, D) + unattached(A, D)`, where `unattached` is the legacy untagged lines that still stand; no command adds to it, and it changes only when a legacy line is reversed. (A document's lines never straddle two accounts: a control role moves only off an account with no balance, B-23, so the identity is well defined per account. The per-account `owed_on` refinement is needed only if `move_balance` is ever built: 8.4.)
- **Dating rules.** A payment, discount or write-off is never dated before its document (`PaymentBeforeDocument`); a reversal of a document-owned entry never before the original; every date outside closed periods (B-12); a void of an invoice from a closed period is dated in an open one and listed in the CPA README.
- **Payment accounts.** The deposit or payer account is a postable asset or liability holding no role (a bank account, a card payable, the member payable under D-42, 2300 Customer Deposits): never a control, revenue or expense account (`check_payment_account`).

#### 8.3.4 Data model (`crates/ledger-core/src/arap.rs`)

Sketch, not compilable; the S4 and S5 units give the exact fields.

```rust
pub enum TermsKind { DueOnReceipt, Net(u16), NetEom(u16), Custom }
pub struct EarlyDiscount { pub percent_bp: u32 /* 1..=9999 basis points */, pub days: u16 /* < the net days */ }
pub struct Terms { pub kind: TermsKind, pub discount: Option<EarlyDiscount>, pub text: String /* Terms::describe() */ }
// Terms::due_date(date) -> Result<NaiveDate>; Terms::discount_by(date) -> Option<NaiveDate>;
// kernel::discount_max(total, percent_bp) bounds a discount; the caller gives the exact cents taken.

pub struct PartySnapshot { pub name: String, pub address: Option<String>, pub email: Option<String> }

pub struct DocumentLine {               // existing fields, plus:
    description, amount, class_id,      // one project per line (D-43)
    pub account: Option<AccountId>,     // None = the role account (legacy: what it posted to)
    pub quantity: Option<Decimal>, pub unit_price: Option<Decimal>, // both or neither; kernel::line_amount(q, u) == amount exactly
}

pub struct Invoice { id, customer_id, date, due_date, lines, entry_id,
    pub number: Option<String>,          // at issue; None = legacy, shown "Invoice {id}"
    pub terms: Option<Terms>, pub bill_to: Option<PartySnapshot>, pub seller: Option<PartySnapshot>,
    pub customer_ref: Option<String>,    // the customer's PO
    pub memo: Option<String>,
    pub sends: Vec<Sent> }               // S5
    // no issued_pdf field: the issued copy is the receipt.attached on the entry, linked Subject::Invoice (S4.3)

pub struct Bill { id, vendor_id, date, due_date, lines, entry_id,
    pub vendor_ref: Option<String>,      // the vendor's invoice number
    pub received_on: Option<NaiveDate>, pub terms: Option<Terms>, pub memo: Option<String> }

pub struct Sent { pub on: NaiveDate, pub by: String, pub via: SentVia /* Email | Mail | Portal | Hand | Other(String) */,
    pub to: Vec<String>, pub kind: SendKind /* Original | Reminder */, pub reference: Option<String>, pub note: Option<String> }
```

**Line accounts.** `check_document_line_account(kind, account)` refuses role accounts, placeholders and cash accounts. An invoice line takes revenue (default the `revenue` role) or a liability holding no role (2400 Deferred Revenue). A bill line takes expense, or an asset that is not cash and holds no role (1500 Equipment, which feeds cash-flow rule 12 and asset registration under B-15; 1300 Prepaid, which feeds a B-16 schedule). `document_parts` groups by `(account, class)`; a document whose lines name neither posts exactly as before. Under `documents_v2` a bill line without `account` is refused (A-10); an invoice line without one defaults to the revenue role; the struct field is `Option` for legacy replay.

**Numbers.** `prefix + the number zero-padded to four digits` (wider past 9999), the number = `invoicing.first_number` + the count of numbered invoices issued before; stored at issue, never changed, even if the prefix later is. `invoicing.first_number` is settable only before the first numbered invoice; legacy invoices have no number and are not counted. Numbers are never reused; voids keep theirs. The preview shows the number the confirm will take, and the P4 preview digest covers it, so a number taken by another session in between refuses the confirm with `stale_preview`.

**Duplicate bills.** `(vendor, normalized vendor_ref)` unique among bills whose entry stands (case, spaces and hyphens ignored); a duplicate is refused unless `duplicate_reason` is given, which is journaled.

**Party.** Gains `email`, `default_terms_as_customer` (the terms we grant), `default_terms_as_vendor` (the terms they give us). `has_full_tin` scans the new fields.

**Roles** (created on demand like the existing ones; the codes are free in the chart template; the owner's live chart is checked at the upgrade, not by code):

| Role | Default | Type | Tax category | Used for |
|---|---|---|---|---|
| `bad_debts` | 6990 Bad Debt Expense | Expense | `bad_debts` (1065 line 12) | write-offs (S7) |
| `purchase_discounts` | 6995 Purchase Discounts | Expense (a credit-balance expense: there is no contra type; the account's normal side is the expense's) | `other_deductions` | discounts taken on bills (S6) |
| `sales_allowances` | 4100 Refunds and Allowances (existing) | Revenue (debit balance) | `returns_allowances` (1065 line 1b) | discounts given (S6); credit memos later |
| `payment_processing_fees` | the account payouts' fee lines use | Expense | `other_deductions` | the processor's fee on a payment received (S6) |

**Settings** (typed registry, journaled, gated by `setting_feature` → `documents_v2`): `invoicing.number_prefix`, `invoicing.first_number`, `invoicing.default_terms` (Net 30), `invoicing.payment_methods` (A-3; the list, with the exemption for `ach`/`wire` lines only), `invoicing.footer`, `invoicing.unsent_days` (2), `invoicing.reminder_days` (14), `bills.remind_days` (7), `bills.discount_warn_days` (3), `company.email`, `company.website`. ~~`invoicing.payment_instructions`, "the exemption covers that key only"~~ → A-3's list. ~~`invoicing.paper`~~ → US Letter only until someone asks. The seller snapshot comes from `company.name`, `company.address`, `company.email`, `company.website`; the EIN is never printed.

#### 8.3.5 State machines

**Invoice.** `(preview) --confirm--> Open --payment < outstanding--> Partial --payment = outstanding--> Paid`; `Open/Partial --write_off ≤ outstanding--> Written off (when outstanding reaches 0)`; `Paid/Partial/Written off --reverse a payment, discount or write-off--> the state its new balance gives`; `Open --void (reverse the origin; only with nothing else standing)--> Void --reverse the void--> Open (B-10)`; **Overpaid** and **Unattached** are the two states only legacy books reach (listed by `control_check`). Flags: sent (one or more `Sent`), overdue (outstanding > 0 and as_of > due_date, today in the company's time zone, D-13).

**Bill.** The same states without write-off (a vendor's release is a later vendor credit, 8.4). Flags: `due_soon` (within `bills.remind_days`), `discount_open` (as_of ≤ discount_by), overdue.

**Payment.** Recorded, then voided by reversing its entry; every document it settled reopens together, because the reversal mirrors every tagged line.

**No stored preview.** Each create tool previews without `confirmed` (the posting, the number it will take, the due date, warnings, and a PDF stamped DRAFT handed as a file) and posts with `confirmed` (and, from P4 on, with `preview`). A-1.

#### 8.3.6 Postings for every transition ([d] = tagged with the document)

| Transition | Debit | Credit |
|---|---|---|
| Issue invoice | 1200 [d, Origin], total | each (account, project) part: the line account (default the `revenue` role; 2400 when billed ahead of the work) |
| Record bill | each (account, project) part: expense, equipment or prepaid, with the vendor tag | 2000 [d, Origin], total |
| Receive payment, one or several invoices (S6) | the deposit account, Σ cash − fee; `payment_processing_fees`, the fee; `sales_allowances`, Σ discounts given | 1200 [d_i, Payment], cash_i, one line per invoice (application order); 1200 [d_i, Discount], discount_i |
| Pay bill, one or several bills (S6) | 2000 [d_i, Payment], cash_i; 2000 [d_i, Discount], discount_i | the payer account (bank, card or member payable), Σ cash; `purchase_discounts`, Σ discounts |
| Void invoice or bill (S1) | the mirror of the origin entry (`reverse_entry`) | |
| Void (bounce) a payment | the mirror of the payment entry; a bank fee is its own entry | |
| Write off an invoice (S7) | `bad_debts` 6990, the amount (≤ outstanding; partial allowed, §1.166-3) | 1200 [d, WriteOff] |
| Recovery, same tax year | reverse the write-off (the document reopens), then receive the payment | |
| Recovery, a later year (D-49) | the deposit account | a "Bad debt recoveries" revenue account (`other_income`), memo naming the invoice; the document stays Written off (§111) |
| Record sent; change memo or references | no entry | |
| ~~Adopt a legacy line~~, ~~Move a control role~~ | → deferred, D-49 (8.4) | |

**Rules for every entry:** it carries its party tag and the payment `instrument` (D-40); only cash movements count toward a 1099, so discounts, write-offs and the fee never count as paid to the payee; `exact`: Σ cash + Σ discounts = Σ applications and deposit = Σ cash − fee (`kernel::settle`); a multi-document payment covers one party only; a discount is at most `kernel::discount_max(total, percent_bp)` and dated on or before `discount_by`, else `discount_reason` is required; a discount on a bill with an asset or prepaid line is refused with its remedy (D-49); the engine never rounds a discount: the caller gives the exact cents.

#### 8.3.7 Journal events and format features

The pattern is unchanged: the entry's own `entry.created`, `entry.line_added` and `entry.posted` lines come first, then the document event, then the attachment events. Replay derives the posting from the document event and checks it line by line against the entry just replayed (`ReplayMismatch`): on every open this proves the document owns its entry.

| Feature | Unit | Lines it marks | What an older version would misread |
|---|---|---|---|
| `document_standing` | S1 (declared on the live books 2026-10-24) | an `invoice.payment_received` or `bill.paid` accepted only because an earlier payment, or the document's own standing, changed through a reversal | it replays the payment on top of the reversed one: paid above total, silently |
| `documents_v2` | S4, S5, S6, S7 (D-52: one feature, one upgrade after S7) | `invoice.created` and `bill.created` carrying `number`, `terms`, `bill_to`, `seller`, `customer_ref`, `memo`, `vendor_ref`, `received_on`, `duplicate_reason`, and lines with `account`, `quantity` or `unit_price`; `party.updated` with `email` or the terms fields; `settings.set` of the `invoicing.*`, `bills.*`, `company.email`, `company.website` keys (S4); `invoice.sent` (S5); `invoice.payment_received`/`bill.paid` with `line`, `discount`, `discount_line`, `discount_reason`, `fee`, `fee_line` (S6); `invoice.written_off` (S7) | S4: it drops the line accounts, so the parts would post to 4000 and 6999; S6: it drops the discount and the fee, so the balance is wrong; S5, S7: it refuses the unknown type |
| ~~`invoice_sent`, `settlements`, `write_offs`, `control_lines`~~ | → folded into `documents_v2` (D-52); `control_lines` deferred with its tools (D-49) | | |

**What needs no feature:** a void (it is `entry.reversed` of the origin entry, the reason in the reversal's description); a document with none of the new fields (journaled byte for byte as before: the `document_line_projects` pattern); a legacy single-document payment event.

**Projection.** The SQLite schema gains columns for the new fields, a table for sends, and columns for discounts and fees; `projection_rows`, `compare_projections` and `Ledger::differs_from` cover them; every new event type gets a replay arm and a projection arm (`tests/format.rs` checks both). Numbers taken at merge (8.0.3).

**The upgrade.** Declared at the owner's go-ahead after a verified backup (D-39). Until then, on the live books a write that needs `documents_v2` is refused with the upgrade remedy; everything else in S4 to S7 works at once. One go-ahead for all four slices (and P4's `drafts_v2`); Q-5 asks the owner whether to give it earlier, after S4.

#### 8.3.8 MCP tools

Every description within 700 characters. Amounts are decimal strings. A document is referred to as `invoice:7` or `bill:12` (and, from S4, `INV-0007`). Mutating tools carry `destructiveHint`, preview without `confirmed`, and take `preview` from P4 on. The S4 to S7 units give each tool's exact arguments and outputs; the list here is the surface:

- **Changed:** `books.create_invoice`, `books.create_bill` (S4: `customer`/`vendor`, terms or due date, line accounts, quantity and unit price, PO, memo, bill-to, the bill's file and `vendor_ref`, preview and confirm, the issued PDF), `books.update_party` (S4: email, default terms per side), `books.receive_payment`, `books.pay_bill` (S6: `applications[]`, discounts, the fee, `paid_in_full_marking`), `books.ar_aging`, `books.ap_aging` (S6: `detail`, `control`), `books.export` (S6: `ar_aging`, `ap_aging`), `books.reverse_entry` (S1: the bounds check and the remedy "reverse payment entry N first").
- **New:** `books.mark_sent` (S5), `books.bills_due` (S6), `books.write_off_invoice` (S7). Built in S1 and S2: `books.void_document`, `books.control_check`, `books.documents`, `books.document`.
- ~~`books.adopt_control_line`, `books.assign_role { move_balance }`~~ → deferred (D-49, 8.4).

**Agent guidance.** `books.start`'s next steps and Needs attention list (after U0b, by one function): overdue invoices; invoices never sent (after `invoicing.unsent_days`); overdue invoices not reminded within `invoicing.reminder_days`; bills due within `bills.remind_days`; discounts ending within `bills.discount_warn_days`; bills missing `vendor_ref`; unattached control lines; bill lines on 6999; a write-off awaiting the CPA's note. The handbook's `invoices-and-bills` procedure (S4.5, extended by S5, S6, S7).

#### 8.3.9 Sent tracking

The books never send mail (A-4; D-17 deferred). `create_invoice` returns a download and a `mailto:` link (subject "Invoice INV-0007 from {company}", the body URL-encoded and truncated to 1,800 bytes at the last whole word); a mailto link cannot carry an attachment, so the agent says which file to attach; the human or a delegate sends and the agent calls `books.mark_sent` with who sent it, the channel, the recipients and the channel's reference; several sends are allowed (reminders); a send changes no figure; the sent date never moves the due date.

#### 8.3.10 Aging and the control check

`ar_aging` and `ap_aging` bucket `outstanding(d, D)` by the due date (`arap::aging_bucket`); documents dated after D and documents with outstanding 0 are left out; the buckets partition Σ outstanding exactly (B-18); a document owing less than nothing is left out of the aging and listed by `control_check` with the unattached lines. The CPA package gains `ar_aging.csv` and `ap_aging.csv`, each with a tie-out line, and its README states the B-23 agreement, the direct write-off method and the gross discount method (S6). `books.control_check` is also run by `books.verify` (which reports and never fails on it alone), the digest (`control_problems`) and Needs attention (S1).

#### 8.3.11 The PDF invoice

**Code (S3, extended in S4):** `crates/ledger-import/src/invoice_pdf.rs::invoice_pdf(&InvoiceDoc) -> Result<Vec<u8>, PdfError>` and `templates/invoice.typ`, the same `OnceLock` Typst engine, bundled fonts, data through `sys.inputs`; `crates/ledger-package/src/invoice.rs::invoice_document(ledger, company, id, as_of, today)` builds the data, so the server and the viewer give the same bytes.

**Contents:** the seller snapshot (name, address, and the email and website that are set); "INVOICE", the number, the issue date, the due date, the terms text, the early-discount line, the customer's PO; the bill-to snapshot; the lines (description, quantity × unit price when given, amount; never a project, account code or internal id); the total; in the current copy, each standing payment with its date, method and reference, discounts and written-off amounts, and the balance due; the payment methods with their QR codes; memo and footer; a stamp (DRAFT for a preview; PAID with the day, PARTIALLY PAID, PAST DUE or VOID on the current copy; none on the issued copy); no EIN or TIN.

**Issued copy (S4.3):** rendered before anything is journaled (a Typst failure leaves the books unchanged); filed in the same store call as the invoice as `receipt.attached` on the entry (`INV-0007.pdf`, `application/pdf`, "Issued invoice") plus `attachment.linked` to `Subject::Invoice`; its SHA-256 is the `blob_sha` in `receipt.attached`; found through the link, never by guessing. ~~The attachment id goes into `invoice.created`'s `issued_pdf`~~ → impossible in that order (the id is allocated after the entry exists; CD-A6) and unnecessary. A legacy invoice has no issued copy; its current copy says "Re-created on {as_of} from the books; not the copy sent".

**Invariants:** (1) deterministic: same data and same version, same bytes; no wall-clock timestamp anywhere (the "re-created on" date is the `as_of` argument); `typst`, `typst-pdf` and `typst-layout` pinned exactly; a golden-sha test; an upgrade may change new renders, never the filed issued copy; (2) every string is data, never markup, so `#import`, `]` and `$` print literally; (3) every amount reaches the page through `amount_parts` and Typst does no arithmetic on money: the printed total equals Σ lines equals the entry's control line.

#### 8.3.12 Viewer (read only, D-28)

Built in S2 and S3: Money in and Money out pages (open documents by due date, whole-row click, "12 days late", filter chips Open / Overdue / Paid / Void / All, bucket totals and the tie-out badge), the document pages (`Route::Invoice(id)`, `Route::Bill(id)`: header and status chip, lines with project links, the timeline with reversed items struck through, attachments with Open and Save, Save invoice PDF), the party card opening the document page, the overview tiles ("Owed to you: $X (Y overdue)", "You owe: $Z (W due this week)"), `books.show` targets `invoice` and `bill`. S4 adds the number, terms, bill-to and PO and the issued copy to Save; S5 the Sent column ("Oct 3 to ap@…" or "Not sent") and send history; S6 the discount-deadline column; S7 write-offs on the timeline. Every page change: render the screenshots (`crates/ledger-gui/tests/invoices.rs`, `shot(..)`, written under the cache folder the test names) and look at them before calling it ready; `view_model.rs` parity; no mutation runs on the viewer.

#### 8.3.13 Invariants (spec blocks; rules B-38 to B-42)

```text
GOAL: invoices and bills are the subledgers of the receivables and payables
      control accounts; every figure about a document is read from the
      ledger lines tagged with it.
SOURCE: rules.md B-8, B-9, B-10, B-18, B-23, B-28, B-38; ASC 310-10,
        ASC 405-20-40-1, ASC 606-10-45-4.
INVARIANTS:
  1. For every control account A and date D:
     GL(A, D) = Σ_docs outstanding(d, D) + unattached(A, D), where
     outstanding(d, D) is the signed sum of d's tagged lines dated ≤ D.
     No command adds an untagged control line; unattached changes only by
     reversing a legacy line.
  2. 0 ≤ outstanding(d, D) ≤ total(d) at every D for every document written
     under these rules, whatever sequence of creates, payments, discounts,
     reversals, voids and write-offs ran; replay accepts what was written.
  3. Reversing a tagged entry changes outstanding by exactly the opposite
     of its tagged lines, from the reversal's date; reversing that
     reversal restores it (except the B-38 item 3 cleanup, which is refused).
  4. DocState is a pure function of (journal, as_of): Store, Snapshot and
     the viewer give identical states, balances and aging.
  5. Aging as of D partitions Σ outstanding(·, D) over documents owing more
     than nothing, exactly.
  6. Every refusal leaves the books and the journal unchanged; the engine
     never panics.
SCENARIOS:
  1. Invoice 100, payment 100, payment reversed: Open, outstanding 100, A/R 100.
  2. Invoice reversed with nothing paid: Void, not in aging, number kept.
  3. Void with a payment standing: refused, naming the payment.
  4. As of a date between a payment and its reversal: counted as paid.
  5. Wrong payment reversed, right one recorded: accepted (document_standing).
  6. Hand Dr 1200 / Cr 4000: refused with the remedy tools named;
     reversing a legacy hand line on 1200: accepted, unattached shrinks.
  7. Half paid, rest written off: Written off, breakdown 50 paid / 50 written off.
  8. receive_payment deposit_to = 1200 or 4000: refused.
  9. Origin, reversal, re-reversal all before D: outstanding = total, Open.
```

```text
GOAL: terms give one due date and one discount date, exactly.          (B-39, S4)
SOURCE: CONVENTION: standard trade-terms notation; UCC §2-310(a) for goods;
        for services, the contract.
INVARIANTS:
  1. Net n = date + n; Net n EOM = last day of date's month + n; due on
     receipt = date; Custom = the given date. Always ≥ date, in the
     calendar (calendar::check); n in 0..=3650.
  2. discount_by = date + d, d < the net days (none for due on receipt or
     EOM); a taken discount ≤ kernel::discount_max(total, percent_bp), the
     caller's exact cents; after discount_by only with a reason.
SCENARIOS: 2026-01-31 net 30 → 2026-03-02; 2026-10-23 net 15 EOM →
  2026-11-15; leap day; the 9999-12 bound refused, not panicking.
```

```text
GOAL: an invoice number names one invoice forever; the copy sent is kept;
      a vendor invoice is entered once.                                  (B-40, S4)
SOURCE: Pub. 583; Rev. Proc. 98-25; CONVENTION: sequential numbers, voids
        kept; duplicate-payment control.
INVARIANTS:
  1. Numbers unique, in issue order, never reused; a void keeps its number.
  2. Number, snapshots, terms and lines never change after issue.
  3. The issued PDF is filed in the issuing call as receipt.attached on the
     entry and linked to the invoice; its sha256 is in that event.
  4. (vendor, normalized vendor_ref) unique among standing bills unless a
     duplicate_reason is journaled.
  5. Sends never change a figure.
```

```text
GOAL: write off what won't be collected, as the tax rules require.      (B-41, S7)
SOURCE: IRC §166(a), §111; Treas. Reg. §1.166-1(e),(f), -2(a), -3(a);
        ASC 326-20; DECISION: direct write-off (A-5).
INVARIANTS:
  1. amount ≤ outstanding; Dr bad_debts / Cr A/R [d, WriteOff]; evidence of
     worthlessness filed in the same call.
  2. Its reversal reinstates; bills are never written off (ASC 405-20-40-1).
  3. Refused on cash-basis books.
```

```text
GOAL: a settlement is exact and names its lines.                        (B-42, S6)
SOURCE: CONVENTION; ASC 606-10-32-5 to 32-9 noted; DECISION: the gross
        method (A-8), the fee's gross presentation (A-4).
INVARIANTS:
  1. Σ cash + Σ discounts = Σ applications; deposit = Σ cash − fee
     (kernel::settle); one party per settlement.
  2. Each application's cash + discount ≤ what the document owes on every
     day from the payment's date on.
  3. Each new payment event names the line it tags; replay tags that line;
     a legacy event without one takes the first untagged control line for
     its amount.
  4. A discount on a bill with an asset or prepaid line is refused.
```

Each rule keeps the CONVENTION label until its sources are vendored in `docs/spec/sources` with a SHA-256 (8.4).

#### 8.3.14 Migration of existing books

1. Before S1 was installed, the goldens were captured (`crates/ledger-store/tests/fixtures/p4-arap-before`, written by `0df7f52a`; adjudicated in `legacy_reversal_now_counts`). ~~`goldens/p4-arap-before/` through the MCP~~ → the fixture is the record; the owner's own goldens live privately in the books home (5.6) and are compared at each upgrade.
2. No journal line is rewritten (B-8). S1 derives tags on replay. Figures changed only for documents whose origin or payment entry has a reversal in its chain, plus `cashflow::bill_splits`; the balance sheet and P&L did not move.
3. Out-of-bounds legacy states replay as written (a voided invoice with a payment still standing shows in Needs attention with the remedy: reverse the payment).
4. Unattached hand lines on 1200 or 2000 are listed by `books.control_check`. For each, the owner chooses: (a) reverse it and re-enter it as a document while its period is open; (c) leave it, with a recorded decision and a note in the CPA README. ~~(b) adopt it (S7)~~ → deferred (D-49); built only if a line on the live books fits neither (a) nor (c).
5. Legacy documents keep "Invoice {id}": terms Custom with their due dates, no snapshot, line accounts None (the role accounts they posted to). New numbers start at `invoicing.first_number` (the owner's choice; the default 1), so the series is gap-free from the first numbered invoice.
6. Bills on 6999 are reclassified by ordinary entries (Dr the right account / Cr 6999), each citing the bill's receipt (A-9).
7. Older-rule fixtures (reversed payments, reversed invoices, a hand line on 1200, a multi-payment entry, a role reassigned mid-stream) are in `books_written_under_older_rules_still_open`; each unit adds its shape (8.0.6).
8. At each upgrade of the owner's books: `books.verify`, `books.control_check`, then the cent-exact comparison with the previous goldens in the books home's `goldens/` (the EM's or the owner's step; never an engineer's).

#### 8.3.15 Tests

Named in each unit (8.2). Shipped in S1-S3: `crates/ledger-core/tests/spec_documents.rs` (`reversing_a_payment_reopens_the_invoice`, `a_voided_invoice_never_ages`, `aging_as_of_ignores_a_later_reversal`, `a_corrected_payment_after_reversing_the_wrong_one`, `void_refused_while_paid`, `hand_line_on_control_refused`, `reversing_a_legacy_hand_line_is_allowed`, `deposit_account_must_be_cash_like`, `several_payment_events_on_one_entry_tag_its_lines_in_order`), `crates/ledger-store/tests/arap_standing.rs` (`legacy_documents_replay_with_tags`, `legacy_reversal_now_counts`, `role_reassigned_mid_stream_tags_the_role_then`, `document_posting_mismatch_is_replay_mismatch`, `repaying_after_a_reversal_waits_for_document_standing`, `a_payment_journals_its_method_and_reference`), `arap_props.rs` (`subledger_equals_control`, `status_is_a_function_of_journal_and_date`, `reversal_twice_restores_state`), `bounds.rs`, Kani `bounds_check_matches_definition`, Lean `control_agrees`, over the wire `documents_void_and_tie_out_over_the_wire`, viewer `the_list_shows_the_ledgers_figures`, `an_invoice_renders_deterministically`. Properties every later unit keeps green: `subledger_equals_control` after every op (the identity, the bounds, replay equals live, at every date), `status_is_a_function_of_journal_and_date`, `reversal_twice_restores_state`, `due_date_monotone` (S4), `document_parts_partition_the_total` (S4). `fuzz_op_sequence` gains the document ops in P12.

#### 8.3.16 Risks

- **Golden figures.** S4 to S7 move no figure; 8.0.6 makes a moved figure a stop. (S1's changes were adjudicated at its upgrade.)
- **The control guard refuses old habits.** Any recurring template, convention or agent routine writing to 1200 or 2000 by hand is refused (S1, live). The remedy names the document tools.
- **The guard must not block real document posting.** The tag is set inside the same atomic step; the reversal-mirror exemption must not widen into a bypass (the B-38 item 3 refusal of the cleanup's reversal is the test).
- **Multi-document payments.** New events name their lines (S6); every reader that assumed one document per payment entry is tested against double counting (`multi_document_payment_counts_once_for_1099_and_party_totals`).
- **Bounds at every date cost more.** The check runs only on the documents a step touches.
- **The upgrade.** One go-ahead for S4 to S7 and P4 (D-52); until then their writes refuse on the live books with the remedy, and the handbook says so. Too many prompts wear the owner down; one is the right number.
- **PDF bytes depend on pinned versions.** The filed issued copy is the record; strings are data.
- **The payment-methods exemption must not spread.** It is one `Kind` arm for `ach`/`wire` lines; the inbound file scan still runs on everything filed.
- **Revenue can be dated too early.** An invoice dated before the work overstates revenue (ASC 606-10-25-23); the agent asks when the work happens and uses 2400 for advance billing (the handbook says so).
- **Equipment on a bill must be registered.** A bill line coded to equipment raises "register the asset" (B-15) in Needs attention.
- **The tool budget.** Three new tools and several extended descriptions must fit 700 characters; P5 drives the pinned count of over-long descriptions to 0.
- **The mailto flow cannot attach the PDF.** Sent tracking and reminders mitigate a forgotten attachment.
- **Tax judgments stay with the CPA.** Write-offs, recoveries, deposit timing (§1.451-8) and vendor releases are flagged, never presented as settled.

### 8.4 Not scheduled

Recorded so no one builds them by accident and so the decision that set each aside is one click away. Each becomes a unit only when the EM writes one, with the owner's go-ahead where a decision is amended.

| Item | Why not now | Decision | Notes for when it is built |
|---|---|---|---|
| `books.adopt_control_line` | no legacy line on the live books is known to need it; 8.3.14's remedies (a) and (c) cost no code | D-49 | `as: invoice \| bill \| payment_of \| discount_of \| write_off_of`; an adopted origin gets one synthesized line (`account: None`); event `control_line.adopted`; `Ledger::differs_from` compares `line_docs` directly; Lean `Step.adopt` and `Reachable.unattached_zero` restated |
| `books.assign_role { move_balance }` | B-23 already keeps a control role with a balance where it is; nobody has asked to renumber 1200 or 2000 | D-49 | RoleMove lines per document through `Ledger::atomically`, then `assign_role`; the identity becomes per account (`owed_on(A, d, D)`); event `role.balance_moved` |
| `Movement::Recovery`, a `bad_debt_recoveries` role | a handbook procedure gives the same books (8.3.6) | D-49 | only if the CPA asks for recoveries on the document's timeline |
| `receive_payment.excess_to`; customer deposits and credits (2300), vendor credits (1250) | A-12: overpayments refused, credits later; the replacement-before-bounce case has a procedure | D-49 | a `customer_credit` document kind; ASC 606-10-45-2; a release of a valid debt is cancellation-of-debt income (IRC §61(a)(11)), exclusions at partner level (§108(d)(6)) |
| Apportioning a purchase discount across a bill's asset lines | refused with a remedy instead (8.3.6) | D-49 | `kernel::apportion` across line accounts; asset lines reduce cost (Pub. 946) |
| Credit memos (to 4100), customer statements (PDF), recurring invoices, a cash-basis P&L view, sales tax (Utah Code §59-12-103; ask the CPA first) | the owner's list of later items | the plan of 2026-10-23 (its former P4b.16, item 8) | each a small design of its own |
| Reversal as a draft with its own evidence gate | no decision asked for it; it breaks the built immediate void | D-47 | if ever wanted: a reversal draft is immutable, the whole reversal check re-runs at post time, Lean `Step.postReversal` |
| Vault v2: HPKE, keyslots, recovery key, `rekey`, `/h/tin`, `request_tin_entry`, `set_w9_facts`, the GUI passphrase unlock | D-28, D-29 keep TINs in the v1 vault with no extra handling; D-31 carries the key in every backup | D-48 | belongs to the hosted service (section 11, model B) |
| `books.job`, the 20 s rule, jobs as lease holders | no local call approaches a client limit over the bridge | D-53 | with P9b, if claude.ai's 240 s limit is measured |
| `ledger-view`, `ledger-extract` crates | `PackageBooks` is the shared view; extraction is in-process in `ledger-import` | D-53 | a wasm viewer (section 11) may want a view crate later: a mechanical move |
| `attach_email`, `attachment.rendered`, `fuzz_eml`, `.eml` uploads | email-in waits for the hosted service; the convention is "email as PDF" | D-17 | `mail-parser`, a deterministic Typst render of headers and body |
| `compare_reference`, `record_inventory`, `check_inventory`, their events; the exact-decimal XLSX reader for the Sheet | the Sheet and the books as of 2026-10-11 are frozen; the final `verify-sheet` run is a filed workpaper | 2026-10-26 review (Z-6) | a `migrate` prompt for a second company is speculative |
| `/h/inbox` drop page, `books.inbox`, `books.download_link`, `request_human_download`, the MCP App upload widget, `openai/fileParams` | D-28 (the agent does it all locally), D-46 (files only by handle); the hosted server adds a URL | D-28, D-46, D-51 | P9c |
| The remote viewer replica (`/replica/*`, `Snapshot::from_lines`, the blob LRU), `books.invite`, passkeys for remote people | not on the P10 path; a backup clone opened read-only serves the owner | D-51 | P9c |
| An in-house OAuth 2.1 authorization server | D-7 (b), an external identity provider, removes it and the external review | D-51, Q-6 | P9b |
| Attestation bar versioning, the Lean counting lemma | the bar is one setting evaluated when asked | 2026-10-26 review (IZ-3d) | |
| bookspec's own repository | the harness must be stable first; `just check` keeps running the examples | D-34, Z-17 | P13 step 6 |
| hledger, Beancount, QuickBooks adapters for bookspec | candidates, not scope | P13 | |
| **Sources to vendor before B-38 to B-45 become AUTHORITY** (`docs/spec/README.md` "Open"): ASC 606-10-25-23, 45-2, 45-4, 32-5 to 32-9; ASC 405-20-40-1; ASC 310-10; ASC 326-20 (incl. 55-37/38); IRC §§61(a)(11), 108(d)(6), 111, 166; Treas. Reg. §§1.166-1(e),(f), 1.166-2, 1.166-3, 1.451-1(a), 1.461-1(a), 1.461-4, 1.6041-1(a)(1)(iv); Rev. Proc. 98-25; UCC §§2-310, 3-311; Form 1065 instructions for lines 1b and 12, Schedule L lines 2a and 15; the P3 surface review's list (ASC 606-10-25-31, 25-32, 55-20, 55-39; 230-10-45-15(f); the ASC 250 paragraph B-15 rule 6 means; 105-10-05-1 to 05-3's current wording) | a documentation unit anyone can take, no code | | each read in the ASC basic view or vendored verbatim (public domain) with a SHA-256 |

---

## 9. Risks and mitigations

1. **Journal forward compatibility.** Old GUIs or CLIs cannot read newer events.
   - The P0 `NewerJournal` gate shipped first; every new feature follows recipe R-A and the 8.0.6 checklist. The local GUI reads the journal directly, so it goes through the same gate.
   - One executable (D-22): an update replaces it whole; the server's `format_upgrade_pending` gate (3.9) protects the books however it arrived.
   - ~~The replica (P9) carries a minimum version.~~ → D-51
   - Fixtures from every phase are kept.
   - Downgrade is impossible after the first newer event, so back up first (3.9 as built: the upgrade makes a verified backup before it writes).
2. **Retired (P1 landed 2026-10-17; the goldens were byte-identical).** ~~The settings migration shifts numbers.~~ Byte-identical goldens gate P1. `company.members` effective dating must reproduce the `close_to` history exactly.
3. **Retired (P3 landed 2026-10-22; every difference adjudicated in the private change log).** P3 fixes change the first company's reports and tax output. Every difference is hand-adjudicated in the change log, and the 1099 and 704(c) changes are shown to the CPA. Posted entries never change. The cumulative close rounding counts from inception (D-10), so the first new close may carry a one- or two-cent true-up; it is listed in the change log.
4. **Retired (the cutover happened 2026-10-20, P2 slice 9).** The P2 cutover is large and could disrupt daily use.
   - The scripted `adopt` checks the head hash.
   - If the owner agrees (Q-3), the old copies move into the data root's `legacy/` area. Nothing is deleted.
   - Rollback is documented.
   - The final runs of `verify-sheet` and `drive-manifest` are filed before cutover; Drive deletion waits for P6.
5. **Closed by decision (D-28, D-29, D-48):** TINs cross the MCP on `set_tin`/`set_w9` on their way into the vault and come out in the exports the owner chose, with no added prompt; this is the design, not an interim. ~~Mitigation: P7 is scheduled before mid-January … TIN-bearing outputs are human-only from P2.~~
6. **Desktop isolation is advisory.** A process running as the human, including an agent that deliberately hunts, can read the data root, the agent token and the wrapped keyslots.
   - ~~TIN plaintext still needs the passphrase.~~ → D-29
   - The path is never named, the root sits outside every project directory, and every place that reports isolation says "advisory".
   - ~~The server's own Landlock and seccomp are never presented as protecting the books from the agent.~~ → D-32: there is no confinement
   - Enforcement is the self-host deployment (P9), tested in CI from P2.
7. **The books are open only while something holds them** (D-21, D-23: the app, an agent session, a `serve` terminal; no background service).
   - An agent session opens them itself through its bridge, so it needs no reconnect. A client connected over HTTP instead of the bridge holds no lease and needs them already open.
   - The viewer reads the data root directly, so it works even if the server can't start; "Restore…" and "New books…" say why they can't run then.
   - Backups run only while the books are open. The books change only then, so nothing goes unbacked; the server pushes anything left over, and a drill that fell due, at the next open.
   - The host outlives the opener that started it, by design, until the last lease ends; it is never left running with no holder for more than 10 s. A crash or power loss is what the append-only journal already survives.
8. **~~The OAuth authorization server is security-critical code written in-house~~ → D-51, Q-6: the recommendation is an external identity provider (D-7 b), which removes this risk; the notes below apply only if the owner chooses D-7 a.** It is needed only on the self-host remote path (P9b).
   - Small surface: passkeys only, no password grant.
   - Built on audited crates (webauthn-rs, oauth2 types); fuzzed; externally reviewed before the remote path is opened publicly.
   - Alternatively an external IdP (D-7).
9. **Client variance.**
   - claude.ai: no elicitation, about 150K-character results, 240 s per call.
   - Cursor: legacy protocol versions only.
   - Codex: a 512-character budget.
   - The `resource_link` content type is undocumented, and the files extension is experimental.
   - Mitigations: links repeated in text, both protocol eras served, jobs for long work, the drop page, and an adapter for `io.modelcontextprotocol/files` once it stabilizes.
10. **Extraction quality.** There is no OCR in core (D-15), so image receipts rely on page images plus attestation. The extractor version is journaled, and re-extraction appends events and never rewrites. Extraction runs in the server's own process (D-32, D-53): a malformed PDF is caught by `catch_unwind` and size caps, not by a sandbox; the fuzz targets are the mitigation.
11. **Attestation `method` is self-reported.** The bar is a policy on claims by an authenticated client. Verified and asserted fields are separated, and the deterministic cross-checks are the hard floor.
12. **Performance.** One mutex per book serializes writers. Mitigations: reader snapshots, `spawn_blocking`. ~~jobs, SSE backpressure, the blob LRU~~ → D-53, D-51
13. **`server.rs` size (12,566 lines on 2026-10-26).** ~~It is split per tool group as each phase touches it~~ → U0a splits it in one go before the remaining units open, because every unit adds tools to it and parallel worktrees would conflict on every merge (8.0.3).
14. **Tool surface churn** (`preview`, renamed parameters). Aliases (`party_id`, `document_id`, `cash_account`) last until P5's `docs/mcp-interface.md` rewrite removes them; the agent-eval harness (P5) catches guidance regressions.
15. **Proof obligations** (cash-flow footing, cumulative drift, the document steps) may take time. A failing proof is a finding and blocks its phase by design.

Hosted-service risks (operator-served unwrap code, the legal exposure of hosting SSNs) are in section 11.

---

## 10. Decisions

### 10.1 Settled

The owner decisions of 2026-10-16 (at the top of this document) replace the former questions D-1 (local isolation default), D-6 (hosting timeline), D-8 (posting friction) and D-14 (viewer on the desktop). The engineer settled the following from the standards:

| # | Question | Decision |
|---|---|---|
| D-5 | Company EIN | (a) A journaled `company.ein` with sensitivity `identity`: printed on every 1099, masked in `books.start`, full in packages. |
| D-9 | Attestation bar default | (a) Receipts and month-end: reasoning effort ≥ high. Annual and CPA handoff: adversarial verification with ≥ 2 independent verifier agents. No model allow-list. The CPA package refuses without a counted full audit unless given a reason (`policy.cpa_package_requires_audit = refuse`). |
| D-10 | Odd-cent cumulative apportionment start | (a) From inception, so the next close carries any historical true-up of a cent or two; lifetime drift stays under one cent while members and shares are unchanged (a change of members or shares starts a new count, §706(d); P3 review R-8). |
| D-11 | Default proration for new prepaid schedules | (a) Daily, matching depreciation; existing schedules stay `monthly_even`. |
| D-12 | Legacy reversals already posted without their own evidence | (a) During the re-audit, link the original's receipts with `inherit_evidence`, so the audit is fully clean. |
| D-13 | The first company timezone | (a) `company.timezone = America/Denver`. |
| D-15 | OCR for image-only receipts | (a) None in core for now: page images plus agent attestation. |
| D-16 | CPA access to TINs | ~~(a) To start, the owner prepares and sends a single-use human link each time.~~ **Superseded by D-29** (2026-10-19): the CPA package with TINs is an ordinary download the owner sends on. |
| D-18 | `dev-stdio` | (a) Kept permanently for tests and demo. It refuses inside the data root and is never offered by `connect`. **Superseded by D-44** (2026-10-23): `dev-stdio` is gone; tests and demos use a books server on a data root of their own. |
| D-19 | One app for Windows, Mac and Linux (owner, 2026-10-17) | Planned: backup and restore in P2, the app itself in P11. One installer and auto-updater per system. The server, the viewer and the CLI ship and update together; since D-22 they are one executable that runs each as its own process: the agent needs the server while the viewer is closed, and the viewer stays read-only as a separate process. No separate launcher app, which would be a third program and a second writer (the helper of 3.9 is a mode of the one executable, not an app). The viewer is the app's face and gains menu items: "Export for CPA" (read-only; the viewer does it) and "Back up now" (handed to the archive process, 3.9), "Restore…" and "New books…" (they change books, so the viewer hands them to the server), and ~~the CPA package with TINs through the human path (P2 `/h/`, P7 vault)~~ → D-29, D-48: the CPA package with TINs from the agent or the viewer's menu. The books live in each system's usual app-data folder (the data root of 3.4): hidden, not locked; a little more protected on macOS. **Backups must restore**, into an existing install or a new one, proven before use and never destructive (3.7). |
| D-20 | Company records (owner, 2026-10-17: "It's just too damn convenient to keep it all together in one place.") | Every company document goes into the books, not only what the CPA needs: formation papers, registrations, plans, notices, drafts, history. Done for the first company 2026-10-17 with the existing categories; the filing scan passes the company's own EIN printed 2-7 (rules.md B-26). P6 adds what makes it pleasant (below). |
| D-21 | How the local server runs (owner, 2026-10-18: "Not sure I want a background service at all. Running in the foreground for now works for me.") | Started by hand in a terminal, not a background service: `wanderbooks serve` opens the books (the server and its archive process) and runs until stopped; agents connect over HTTP while it runs, several at once; Ctrl-C or closing the terminal closes the books cleanly. Nothing is installed to run in the background: no systemd unit file, no LaunchAgent, no socket activation, no login autostart. The viewer works with the server stopped (it reads the data root). **Amended by D-23** (same day): the books also open when the app or an agent starts, and close when the last of them ends. |
| D-22 | One app or several (owner, 2026-10-18: "My objective is simplifying installation and updates.") | **One app, one executable, separate processes.** `wanderbooks` is the only program installed; each role is a mode of it, run as its own process: the viewer (what opening the app shows), the books server and its host, the agent bridge (`wanderbooks mcp`), the archive process and the extractor. One thing to sign, notarize, ship and update. Mixed versions are still possible across a swap, so the host starts its children from its own executable and every handshake carries the version (3.9). The viewer's read-only rule never rested on a separate file: `ledger-gui` already links `ledger-store` and stays read-only through `Snapshot` and `tests/read_only.rs`. As its own process it keeps that, and on Linux it also confines itself read-only (Landlock). Each mode confines itself at start. The crates stay as they are; the executable is a thin dispatcher (P2). One exception: a server-only build (the `gui` feature off) for self-host (P9), so a headless machine carries no window libraries. |
| D-23 | When the books are open (owner, 2026-10-18: "Also when an agent starts") | Amends D-21. The books are open while anything that opened them runs: the app, an agent session (its bridge opens them, windowless, for the session), a terminal running `wanderbooks serve`, or a CLI command. Each holds a lease on the host's lease-only socket, so a crashed holder lets go by itself; local agents speak to the server over a peer-checked local socket, never TCP (3.9). The first opener starts the one host; later ones attach; when the last lease ends the books close 10 s later, cleanly. "Close the books" ends every lease at once. Still no background service: nothing starts at login and nothing runs after the last holder. An agent session started with the books closed needs no reconnect (P2). |
| D-24 | Installers and updates (owner, 2026-10-18: "A dmg for macOS, msi or whatever for windows, and packaged for Linux distributions. Win and Mac with updaters.") | macOS: a signed, notarized `.dmg`; the app updates itself. Windows: a signed per-user installer with no admin prompt (`.msi` only if a managed install needs one); the app updates itself. Linux: AUR, `.deb` with our apt repository, `.rpm` with our dnf repository (all updated by the package manager), and a self-updating AppImage. Every self-update is verified against our own release keys and the system's code signing; Linux packages are verified and updated by their package managers. An update restarts the books cleanly. A newer books format is the server's gate whatever the install: read-only until the owner consents, after a backup with the keys (3.9). "No admin password" holds for the dmg, the Windows installer and the AppImage; Linux packages use `sudo` as usual. Details and tooling in P11. |
| D-25 | Packaging tool (owner, 2026-10-18: "Cargo packager looks good. Would like to use that.") | **cargo-packager** and its updater (crabnebula-dev, MIT/Apache-2.0) for the `.app`/`.dmg`, the per-user NSIS installer, `.deb`, AppImage and the AUR's `PKGBUILD`; cargo-generate-rpm for `.rpm`, which it lacks. Our release manifest (two keys, rotation, a version that only increases, an expiry) wraps its single-key minisign check (P11). |
| D-26 | Where releases are hosted (owner, 2026-10-19: "I'm assuming the GH releases page will do the hosting?") | **GitHub Releases** on the public mirror (`swaits/wanderbooks`): each release carries the `.dmg`, the setup `.exe`, the AppImage, the `.deb` and `.rpm`, and the signed release manifest; the self-updaters read the manifest at the fixed `releases/latest/download/` address, so no server of ours is needed. CI builds and code-signs on release tags on GitHub's macOS, Windows and Linux runners and holds no release key (P11). The apt and dnf repositories' signed indexes are served from GitHub Pages on the same repository; the AUR package points at the Release assets. |
| D-27 | The agent dock and books homes (owner, 2026-10-19: bundle the agents "then it'd be a one stop shop … a chat window maybe full span across the bottom"; "Yeah I like the idea of the embedded alacritty launching claude/codex … maybe MVP defaults to claude"; "you'd want like a 'books home' dir somewhere that makes sense so it has a place to work"; "add it to the plan!") | The app docks a terminal (alacritty_terminal; ConPTY on Windows) along the bottom of its window and runs Claude Code (the default) or Codex in it, found or installed through the vendors' installers, never bundled; started by the viewer's helper, in the book's **books home**: a visible folder per book holding only the agent's pointer and configuration for that book, never the data root. Trust in that folder is the person's to give. **Never required** (owner, 2026-10-19: "we wouldn't want to prevent people from using their OWN AI agent. They should never HAVE to use our built in tty+claude session! Ideally they could even use the claude.ai browser version to connect to it. And of course, eventually we'd host the entire MCP (and gui!) remotely ... use any agent they prefer to remote connect into that.. and they'd view the GUI in a browser."): the books stay fully usable from any agent the person runs themselves, locally or (P9) remotely, claude.ai in a browser included; the dock's one extra is pasting the app's suggested instructions straight into its session. A hosted MCP and a browser viewer are the later service (section 11). P11 (the agent dock and books homes). |
| D-28 | Who does what, locally (owner, 2026-10-19: "Just have the agent ask for them and the user replies. The interface is through claude/codex and that's that. Don't fucking stray from it. TIN, W-9 is NOT critical or super secret PII." and "for downloading the CPA package or other assets, I don't mind if that can happen through the GUI app ... the GUI is for READING the data. The claude/codex is for MUTATAING (and reading) the data!!!") | **Two interfaces, no third.** Every change to the books goes through the agent (Claude Code or Codex) over the MCP; the agent asks the human for what it needs, TINs and W-9s included, and seals them in the vault (`books.set_tin`, `books.set_w9`): no human web pages, terminal commands or open ports for that locally. Reading and downloading (the CPA package and other exports, receipts, documents) happen in the viewer, which still never writes the books. This supersedes, for local books, the `/h/` pages of 3.3, 4.7 and P7's TIN entry: I-7's "the agent never holds a full TIN" becomes "a TIN passes through the agent only on its way into the vault; no result shows one". `/h/` pages remain for self-hosted books reached remotely (P9). |
| D-29 | The full-TIN outputs, from both doors (owner, 2026-10-19, asked whether the agent's CPA export should go away once the viewer has one: "Both WITH TINs. Stop acting like TINS MATTER SO MUCH THEY DONT") | **The agent and the viewer both make them, TINs included.** `books.export_cpa_package` (TINs by default), `books.vendor_1099` with `include_tins` and `books.get_w9` keep working for the agent; their files are made by the server and handed over by a command, `wanderbooks download`, like a receipt too large for a call (no path parameter, nothing in the agent's context). The viewer's menu makes the same files into Downloads. (Over stdio, the development transport, the file came back inline until D-44 removed it.) TINs get no handling beyond the vault: no passphrase, no extra prompts, no special cleanup. |
| D-30 | Q-1 answered: where the backups go (owner, 2026-10-19, asked "Where should the books' automatic backups go?" with a private git repo, a folder or other disk, or both: chose "Both") | **Two targets.** The archive pushes to a private git repository the owner names and also copies to a folder the owner names (an external disk or a synced folder); both hold the archive (not encrypted since D-31). The repository's address and the folder are asked for when backups are first set up (slice 7, or at the cutover for the first company), and kept as the book's backup settings. |
| D-31 | Backups are not encrypted (owner, 2026-10-19, asked where to keep the backup's recovery key: "Why encrypt them? How about we let the backup system handle the security???? You're over complicating everything.") | **A backup is the book as it is, in git.** No age encryption, no archive key, no recovery key: the archive is a plain git repository of the book's journal, receipts, vault (as the vault keeps it) and keys, one commit per backup, pushed to the owner's private git repository and to a git repository in the owner's folder (D-30). The destinations are trusted to keep it private. This replaces 3.7's and 3.4's age encryption on push and 3.5's archive key. |
| D-32 | No self-confinement (owner, 2026-10-20, asked how much of the planned Landlock/seccomp sandboxing of the server and the viewer, with the viewer's helper process, to build now that the server runs git for its backups: chose "None") | **No Landlock, no seccomp, no helper process for confinement.** The viewer stays read-only by design and by test (it never writes the books; its menu hands work to the server, through its admin socket as the command line does); agents still reach the books only through the MCP. Isolation is reported everywhere as `same_user (advisory)`. This replaces the self-confinement of P2's scope, 3.9's confined modes and their helper, and the archive-vs-Landlock decision row. |
| D-33 | Mutation testing waits for P12 (owner, 2026-10-20: "Let's just put a pause on all the mutation testing. Let's add a P12 that runs the whole \"max test\" workflow again. By then maybe I'll have my (much beefier) desktop PC setup and we can run all that shit over there. For now I want to focus on the building.") | **Each phase from P2 slice 3 on is built, reviewed until clean and passes `just check` and `just prove`, without its mutation run.** P12 runs the whole max-testing pass (mutation, fuzzing, full property runs, proofs) over everything since P1, on the desktop. Each phase records in `docs/testing.md` what P12 must cover. |
| D-34 | bookspec (owner, 2026-10-20: "Let's add a P13 which is a bookspec project. This is a FORMALLY written accounting spec … properly sourced with actual references, it's thorough, it might even have its OWN formal validation. It includes symbolic specs as well as concrete examples. It's useful for ANY accounting engine testing." Then: "bookspec is a SPEC first, and then it can also include formal methods/validation/proofs as well as a test harness that could be run against an accounting engine that conforms to the interface.") | **P13 writes bookspec**, its own public project: first the specification (every rule stated normatively, sourced, with a symbolic form and concrete examples), then its formal validation (Lean model, consistency, the laws as theorems, every example derived), then a conformance harness for any engine implementing its interface. wanderbooks is the first engine tested; `docs/spec/rules.md` and the Lean theorems seed it; P3's regression tests are its first examples. |
| D-35 | The cash flow statement (P3, defect 1; owner, 2026-10-20, asked three questions at once with the design's recommendation first, chose each recommendation) | **Expenses a member pays personally are cash the company borrowed and spent** (revised by D-42: first decided as noncash on a wrong reading of ASC 230-10-50-3). **The statement lays out by kind:** operating cash one row per kind ASC 230-10-45-25 requires, investing and financing as kinds too ("Contributions from members — " and the member's name), each transaction listed under its kind; every transaction stays in `books.cash_flow` and the exports. **Equipment paid for later is investing where the books can tell:** a bill payment carries its bill's split (the equipment part investing), a card purchase follows the card's classification, and a sale on an IOU is flagged with a fix. |
| D-36 | The QuickBooks file (IIF) in the CPA package (P3, defects 7 and 21; owner, 2026-10-20, after research: only QuickBooks Desktop reads IIF, QuickBooks Online imports CSV and Excel instead, and Desktop works in Windows code pages, its default IIF import turning anything past ASCII into "?") | **Kept, and written as plain ASCII:** a character it can't hold is transliterated ("René" → "Rene") and every changed name is listed in the export's warnings and the README; every other file in the package stays full Unicode. **CR LF line endings**, as Intuit's sample file. **The 1099 worksheet stops at the package's as-of date** like every other file in it. **Closing entries are left out of the IIF:** QuickBooks keeps no closing entries (it works out retained earnings itself; its "closing date" only locks a period), so imported closings would zero each closed period's profit; the package's K-1 worksheet and members' equity statement show the split by member. Whether the IIF holds every entry since the company began or only the fiscal year goes to the CPA before the 2027 package, with whether she uses QuickBooks Desktop at all. |
| D-37 | Schedules (P3, defect 9; owner, 2026-10-20) | **New schedules spread by day** (D-11 confirmed: a 12-month policy from March 15 posts 13 entries, a small first and last month; existing schedules keep equal months). **An equal-months schedule must start on the 1st of a month**, for prepaid expenses and deferred revenue alike (asked "what would a CPA/CFO say?": revenue is never recognized before it is earned, ASC 606-10-25-23, so equal-month revenue must start on the 1st; prepaid expenses could take a full first month as an immaterial convention, but one rule for both is simpler and never early). |
| D-38 | Odd cents everywhere alike (P3, defect 17; owner, 2026-10-20) | **The members' equity statement, the K-1 worksheet and the viewer split not-yet-closed earnings the same running way as the closes** (cumulative, largest remainder), so a member's capital on a given day never moves by a cent when its month is later closed. |
| D-39 | Pushing the format upgrade (owner, 2026-10-20: "at first open, but not limited to 'once', I think we should push users harder into the format upgrade") | **After an update that needs a newer books format, the app asks for the go-ahead (after a verified backup) at every open until it is given**, the viewer shows a standing notice, and `books.start` puts the upgrade first among its next steps. Declining never stops the books opening on the old format; nothing is upgraded without the owner's yes. |
| D-40 | 1099s (P3, defects 23, 12 and 18; owner, 2026-10-20) | **A bank payment with no method recorded counts toward 1099s** (presumed check, ACH or transfer; debit-card payments are tagged as card). **Anthropic, PBC is recorded "not reportable: a corporation (Delaware public benefit corporation, from the public record)"**; Namecheap, Apple and OpenRouter "a corporation by name" (Treas. Reg. §1.6041-3(p)(1)), USPS "the United States" (§1.6041-3(p)(3)); Google LLC stays undecided until a W-9 shows what it is. **Needs attention asks only about vendors paid in a way that could need a 1099.** **A vendor's 1099 status is dated** (owner: "I think legally this has to be 2 dated statuses, right? If someone incorporates on Dec 30 …"): each status carries the date it takes effect (the W-9's date, or a tax election's effective date), and the worksheet applies the status in force on each payment's date. A payee that incorporates is a new legal person (a new vendor record with its own W-9); an LLC electing corporate taxation keeps its EIN and changes status on the election's date: dated statuses cover both. A status changed for a closed year warns. |
| D-41 | The contributed devices' tax basis (P3, defect 13; owner, 2026-10-20) | **Depreciable basis is the lower of fair market value and the member's adjusted basis at conversion** (Treas. Reg. §1.168(i)-4(b); Pub. 946, property changed from personal use): for the four contributed devices, the CPA to confirm at year end. **§704(c) method: traditional**, recorded now and confirmed with the CPA before the first Form 1065 (for members filing jointly every method gives the household the same tax). **The four devices' values on 2026-10-15 are the books' amounts**, citing the entries that recorded them (figures in the company's private change log). **New P3 item: the K-1 worksheet reports tax-basis capital** (Form 1065 instructions, Item L, since 2020), the contributed property at its §723 carryover basis, beside the book figures. |
| D-42 | A member paying for the company, on the cash flow statement (P3, defect 1; owner, 2026-10-20, revising D-35's first point after the review found ASC 230 silent on a payment a third party makes on the entity's behalf) | **Constructive receipt and disbursement:** when a member (or a lender) pays a company expense or purchase directly, the statement shows a financing inflow (borrowed from the member) and the operating or investing outflow, on that date; paying the member back is a financing outflow. Source: ASC 230 does not address it; practice fills the gap by substance (Deloitte DART Roadmap 7.2 with AICPA TQA 1300.16 and the SEC staff's view; PwC, BDO and EY guides alike), the way a CPA would present it; KPMG accepts noncash only for equipment a lender pays for. Property a member contributes that it already owned stays noncash (ASC 230-10-50-4: no cash moved). Total cash is the same either way; the first company's figures are in its private P3 change log. |
| D-43 | Invoices and bills take a project, line by line (owner, 2026-10-22) | **Each line of an invoice or bill takes one project or none.** A document that spans N projects is N lines, each with its own amount; no percentage split, so every figure is exact. A cost truly shared among projects names none on the document and reaches projects only as allocation entries of their own under the company's monthly allocation convention, when the books have one (otherwise it stays untagged and the agent asks the human). Reasoning: job costing charges a job its traceable (direct) costs whole and reaches shared costs only by allocation on a stated basis. ASC 340-40-25-7 counts as directly related both the costs traceable to a contract (25-7(a), (b), (d), (e)) and allocations of costs that relate directly to it (25-7(c)); 25-8 expenses general and administrative costs unless chargeable to the customer. The books carry a traceable cost whole on the document's line and an allocated cost as entries under the allocation convention on its basis; neither is a percentage on the document. Behind its own journal feature, `document_line_projects`, so books that declared `projects` wait for the owner's next upgrade (rules.md B-37 item 8, B-27). |
| D-44 | One way to serve books: the stdio server is gone (owner, 2026-10-23: "We should probably just delete the stdio path altogether. THere's no point in that. And it sitting there unused will just lead to bitrot. Nuke it?") | **Books are served only by the books server** (`wanderbooks server`: every book of the data root on its local sockets, `/b/{book}/mcp`, uploads and downloads), reached by agents through the bridge (`wanderbooks mcp --book ID`: stdio to the agent, the server's socket upstream), which stays. Removed: `wanderbooks dev-stdio`, the `ledger-mcp` binary and `ledger_mcp::run`'s stdio and `--http` serving of a bare data dir, with everything only that mode had: files the books make coming back inline (`content_base64` on a handed file; every one is now a download), books with no access log and no backups (`books.access_log`, `books.backup` and `books.back_up_now` are always offered, by scope), a settings file read and reloaded (`legacy_files`; old books bring theirs in by `books adopt`), the vault key named by a settings file or `--vault-key-file`, and the key-inside-the-books and books-kept-in-git checks of the books dir (a book in the data root keeps its own key). `books.settings` loses `source`, `books_toml`, `books_toml_exists`, `config_json_present`, `reload_error` and `loads`, and `books.start` its `settings_source` (always the books now). **Owner decisions, 2026-10-23 (asked with the question tool):** (a) `books.import_settings` is removed entirely (the tool, its types, handbook, docs and tests; 118 tools): old books still come in whole through `wanderbooks books adopt`, which reads their settings file once, and ledger-config keeps its settings-file parsing only as far as adoption uses it (`load` and the path-reading `effective_receipt_policy` are gone); `books.chart_template` gives its settings as `books.set_setting` changes. (b) The viewer's loose-books-folder mode is removed (`wanderbooks gui --data-dir DIR --books-toml FILE --vault-key-file FILE`, and following a settings file and `config.json`): the viewer opens only books of the data root (`wanderbooks gui ID`), reads their settings from the journal and their vault from their own `vault/` and `keys/`; tests open throwaway books through the same `Source` and loader the live viewer uses. (c) `wanderbooks dev-stdio` is deleted with no stub: it is an unknown command like any other word. An unknown tool is now a protocol error, as mcp-tools.md asks. Inline file input (`content_base64` with its `sha256`, at most 256 KiB) is unchanged: a separate decision (since made: D-46 removed it). Every test that drove the removed mode drives a real books server on a data root of its own, reached as an agent reaches it (ledger-cli `tests/common`), and checks a handed file by its download. Supersedes D-18. |
| D-45 | Receivables and payables, first class (P4b; owner, 2026-10-23: "we kinda need stronger invoice support. Like make it a first class citizen") | **A document's figures come from its tagged ledger lines that still stand**: every line on 1200 or 2000 carries its document; paid, outstanding and status are derived, so a reversal, void or write-off is right by construction and the control accounts always equal their documents (B-23). **Hand entries to 1200 and 2000 are refused.** Invoices: numbered `INV-0001` running, previewed then confirmed, terms and due dates, the issued PDF filed at issue, any payment methods printed (ACH, wire, links, QR codes, check, P2P handles, crypto, other). Bills: the vendor's number, terms, every line on its own account, paid on terms with early-payment discounts by the gross method. **The books never send, receive or charge** (as for payroll and taxes): the owner or a delegate (an agent) does, and the books record the send (who, channel, recipients, reference) and the payment (processor, reference, fee: gross to the receivable, fee to expense, net deposited). Bad debts by direct write-off (IRC §166). The slices and the owner's answers are in P4b. |
| D-46 | Files only by handle: uploads in, downloads out (owner, 2026-10-23, via the question tool: "Uploads only"; and before it: "What we would DEFINITELY want to avoid is passing a bunch of BLOBs through AI context windows. That's just a pure waste.") | **Files never pass through an agent's context, in either direction.** In: every tool argument that carried a file's bytes or text inline is gone (`content_base64` and its `sha256` on `books.attach_receipt` and `books.file_document`, `w9_content_base64` and `w9_sha256` on `books.set_w9`, `csv_text` on `books.import_bank_csv` and `books.import_chart_csv`), with the 256 KiB inline limit; a file comes only by `books.begin_upload` and `wanderbooks upload`, named by its `upload_id` (`w9_upload_id`); the bytes stored already are still named by `existing_attachment_id`. Those five tools take no other argument, and any inline argument, on any tool, is refused before it runs as unknown, pointing to `books.begin_upload`. Out: every file a tool gave inline is handed over (D-29's `HandedFile`, a download command and its expiry): `books.export` in every format, `books.asset_labels`, and `books.get_receipt` view `bytes`; resource reads were and stay text (a receipt's resource is its meta). Small structured answers (reports as JSON data) are not files and stay. A token may keep 64 handed files at once (was 16), since every export is one now. Supersedes the plan's inline-bytes decision (section 2.1) and every planned inline path in 4.7. |
| D-47 | Reversals stay posted; no reversal drafts (the engineer's recommendation from the 2026-10-26 review; the owner may revisit) | **A reversal is previewed, then posted in one call, every reversal check at the moment it posts; its evidence is its original's** (top decision 5 and D-12 already say so for old reversals; the live handbook and `ReceiptPolicy` say reversals need none; S1 built `books.void_document` as an immediate reversal and the owner upgraded to it). Evidence a reversal genuinely needs of its own (a bank's returned-item notice for a bounce) is attached right after, as every at-once tool works. Nothing of "reversal as a draft" is built: no `entry.reversal_drafted`, no `Step.draftReversal`, no `evidence: link_original \| later`. I-4 and I-5 are amended. `inherit_evidence` stays as the re-audit's annotation (D-12). The 2026-10-16 audit's defect 2 is closed by this and D-12, not by code. |
| D-48 | Vault v2 and the TIN human pages are closed (2026-10-26 review; follows D-28, D-29, D-31) | **The vault stays v1** (`ledger-vault`: one key in `keys/legacy.key`, carried in every backup per D-31). Not built: HPKE, keyslots, a recovery key, `rekey`, `/h/tin`, `/h/w9`, `/h/unlock`, `request_tin_entry`, `set_w9_facts`, the GUI passphrase unlock, the removal of `set_tin`/`set_w9`/`get_w9`/`include_tins`. P7 is the TIN scanner's separator defect. Supersedes D-16 and the "P7 vault" clause of D-19; the hosted service (section 11, model B) owns client-side unwrap if it is ever built. |
| D-49 | P4b deferrals (2026-10-26 review; the engineer's recommendation, the owner may revisit) | Deferred, each with a no-code procedure instead (8.3.6, the handbook): `books.adopt_control_line` and `books.assign_role { move_balance }` (P4b.14's remedies (a) and (c) cover a legacy hand line); `Movement::Recovery` (same year: reverse the write-off, then receive the payment; a later year: a plain entry to a "Bad debt recoveries" revenue account, the document staying Written off, §111); `receive_payment.excess_to` and customer credits (A-12; a replacement payment received before a bounce is known is parked through 2300 Customer Deposits, no role, then applied after the bounce is recorded); apportioning a purchase discount across a bill's asset lines (refused with the remedy: record the bill net, or reclassify Dr 6995 / Cr the asset); the per-account identity `owed_on` (needed only with `move_balance`). S7 is `write_off_invoice` alone. 8.4 holds the notes for when any is built. |
| D-50 | The preview check is a digest in the result envelope, with no key (2026-10-26 review; amends I-6) | **Every tool called without `confirmed` returns `preview`, the SHA-256 of its canonical preview payload; every confirm takes it and refuses a stale one** (`stale_preview`). For a draft entry the payload is the draft's canonical state and the settings seq; for an operation without a draft (a close, a reconciliation, a disposal, a run, a document create, a settlement) it is the tool, its canonical arguments, the settings seq and the journal head seq, so a post by another session in between forces a re-preview (owner decision 3). No HMAC, no `keys/preview.key` (I-6 admitted a same-user key proves nothing), no "accepted without it for one release": required from P4 on. |
| D-51 | P9 split (2026-10-26 review; honours top decision 4 and D-27) | **P9a now:** the self-host recipe (a system unit, no seccomp or Landlock lines: D-32), the move by `books adopt` with the head check (D-31: a backup clone is the transport; no `fetch`, no `verify-archive`), a static bearer over a tunnel for Claude Code and Codex, the two-user CI job. **P9b planned, after Q-6:** claude.ai and ChatGPT through OAuth 2.1 with an external identity provider (D-7 b recommended: no authorization server in-house, no external security review), `books.job` if a call is measured over claude.ai's limit. **P9c not planned:** the remote viewer replica, `books.invite`, passkeys and `/h/` pages for remote people, the MCP App upload widget, `openai/fileParams`; the viewer on another machine opens a backup clone read-only instead. |
| D-52 | One remaining P4b upgrade (2026-10-26 review; applies A-11 after the `document_standing` go-ahead of 2026-10-24) | **`documents_v2` is the one journal feature for S4, S5, S6 and S7**, and P4's `drafts_v2` is declared in the same go-ahead, after S7 lands. Until then, on the owner's books, a write that carries a new field or event type is refused with the upgrade remedy and everything else works. ~~`invoice_sent`, `settlements`, `write_offs`, `control_lines`~~ are not separate features. Q-5 asks the owner whether to give the go-ahead earlier, after S4. |
| D-53 | No `books.job` locally; no `ledger-view` or `ledger-extract` crate (2026-10-26 review) | Jobs, the 20 s rule and jobs as lease holders are not built locally (no call over the bridge approaches a client limit; 3.9 as built lets a running call finish before a close); they come with P9b if measured necessary. Needs attention lives in `ledger-package` over `PackageBooks` (U0b), not a new crate; extraction is in-process in `ledger-import` (P6), not a sandboxed subprocess (D-32). |
| D-54 | P11 split (2026-10-26 review; D-24 to D-27 unchanged) | **P11a:** installers and updates. **P11b:** the agent dock and books homes. The dock starts the agent as an ordinary child process of the viewer's `Owner` (D-32 removed the helper and the confinement); the viewer stays read-only by design and by test. The two-key release manifest of D-25 is built unless the owner answers Q-4 "minisign only". |

### 10.2 Deferred

- **D-7. OAuth for remote access.** This applies only to the self-host remote path for claude.ai and ChatGPT (P9b). The options: (a) build a minimal authorization server into wanderbooks (passkeys, CIMD/DCR); or (b) delegate to an external identity provider (Authentik, Ory, Keycloak) and only validate tokens. The 2026-10-26 review recommends (b): no security-critical code in-house and no external review (D-51). Asked as Q-6 when P9b starts.
- **D-17. Remote file intake beyond the drop page** (an email-in address) is deferred with the hosted service (section 11). P9 ships the MCP App upload widget only.

### 10.3 Open questions for the owner, and the closed ones

**Open (2026-10-26).** Each is asked when its unit starts, one at a time, in plain English; none blocks the units before it.

**Q-4 (before P11a step 5).** Releases: should the app check updates against our own signed release list as well (two keys, one on your hardware token and one offline, a version that only goes up, an expiry), as decided on 2026-10-18 (D-25), or is the packaging tool's own single-key signature plus Apple's and Microsoft's code signing enough? The second is simpler and already refuses an older version; the first also stops a frozen or replayed update list. Recommended: the first, as decided, since the key work is small once the token exists.

**Q-5 (when S4 lands).** The receivables and payables work needs one more go-ahead to switch your books to the new format (numbers, terms, line accounts, sent records, discounts, write-offs). Should we ask once, after everything is built (write-offs and discounts included), or earlier, after invoice numbers and terms land, with a second ask later? Recommended: once, after everything, unless you need numbered invoices sooner.

**Q-6 (before P9b).** For claude.ai on the web to reach the books on your own server, it needs a sign-in service. Should we run an existing one (Authentik, Ory or Keycloak) beside the books, which we only trust tokens from, or write our own small one into the books? Recommended: an existing one (D-7 b): no security-critical code of ours, no outside review needed.

**Q-7 (when P11b starts).** When the app connects Codex to a company's books folder, may it mark that folder as trusted in Codex's own settings, or should Codex ask you itself the first time? Recommended: let Codex ask.

**Closed: the P2 cutover questions (Q-1 to Q-3).** Answered 2026-10-17 and 2026-10-20; the cutover happened 2026-10-20 (P2 slice 9). Kept for the record:

**Q-1. Where should the automatic backups go?** The books back themselves up after every change. Where should that copy live: in a private git repository you name (for example on your own git server), or on another disk? Either way, the backup includes the whole history and the sealed tax ID numbers, and it is locked with a key only you hold.

**Q-2. Should the papers in the `company/` folder be filed into the books?** These are the formation papers, notices and company history. Once they are in the books, every agent and the viewer can find them, and if you later run the books on your own server, claude.ai and your CPA can reach them too. If they stay as files, they live only on this computer. Recommended: file them.

*Answered 2026-10-17 (D-20, the owner): "I would like to have all the non books docs be able to be in there too." Every `company/` record was filed at the P1 cutover (attachments 73-82, linked to the company); only the local copies remain, until P6 moves the Drive manifest in. Don't ask Q-2 again.*

**Q-3. After the move, should the old copies of the books leave the project folder?** The project folder still holds the old journal and receipts (`books-data/`), the old sealed tax ID numbers (`vault/`), and the folder's git history, which contains the journal and receipts too. **Recommended: move them into the private folder's legacy area.** That is what "hidden away" means; otherwise an agent working in the project folder can still read the old copies. Nothing is deleted.

*Answered 2026-10-20 (the owner), one question at a time:* **Q-3: move them aside** (into the root's `legacy/`, nothing deleted). **Q-1's addresses:** the git remote is a new private repository on his server, `ssh://git@git.swaits.com:2222/swaits/fernhill-books-backup.git` (made private, with his go-ahead), and the folder is `~/Sync/wanderbooks-backups` (his Syncthing folder: a local disk, copied to his other devices; only this machine writes it).

**Crumbs to clear at the cutover** (owner, 2026-10-19: "migrate the first company books into the new scheme, data directory, clear out the crumbs"). After the adoption verifies, nothing of the old arrangement may be left where an agent or a later setup trips on it; every item is moved per Q-3 (into the root's `legacy/`, nothing deleted, by `wanderbooks books put-aside ID DATA-DIR [PATH...]`, added at the cutover: only once the adopted books hold everything the old copy holds, only while nothing holds it open, only on the root's own disk, and a move cut short puts back what it moved) or removed if it holds no books data:
- `books-data/` (journal, blobs, database, lock, `gui.log`) and `vault/`, and the project folder's git history holding them (Q-3);
- the v1 vault key outside the books, `~/.config/wanderbooks/vault.key` (adoption copies it to the book's `keys/legacy.key`; the old one goes to `legacy/` with the vault);
- `wanderbooks.toml` (its settings are in the books since P1; only `[viewer] open_at_start` remains, which the app's own preferences replace);
- the scaffold: `justfile`, `scripts/`, `.claude/settings.json`'s SessionStart hook, `.codex/hooks.json`; `AGENTS.md`/`CLAUDE.md` become the 3-line pointer;
- the agents' old registrations: Claude Code's stdio `wanderbooks` server from `just mcp-add` (replaced by `wanderbooks connect`), and Codex's;
- old binaries: `ledger-mcp`/`ledger-gui` from `just build` or `cargo install` (the one `wanderbooks` executable replaces them);
- the `live` jj workspace (`~/.cache/wanderbooks/live-ws`) that `just build` builds the released `main` in: until the cutover these books run only released code, since the work on top of `main` drops what their stdio server needs (slice 5: files over 256 KiB only by upload, which needs the data root);
- the viewer's old preferences file, eframe's `app.ron` in `$XDG_DATA_HOME/wanderbooks` (the data root's default place): the viewer moves it on its first run (ledger-gui prefs.rs), and `setup` must not refuse the default place for holding only that file (slice 8 handles it: moved to the viewer's own folder first);
- the agent's own notes that name the old paths (its memory), rewritten to the new arrangement.
The cutover's check adds: the project folder holds none of the above, and `wanderbooks doctor` is clean.

*For the engineer:* Q-1 was D-3, Q-2 was D-4 and Q-3 was D-2. Q-1's technical side is settled: `archive.git` is seeded from the existing git history, filtered to the journal and blob paths, includes the vault ciphertext, and is age-encrypted as a whole on push (3.4, 5.5). (Since D-31 not encrypted, and slice 7 begins each book's archive at its first backup: whether to seed it from the old history is the cutover's question.)


### 10.4 Decisions by unit

| Unit | Decisions it rests on |
|---|---|
| U0a, U0b, U0c | D-33 (no mutation runs), D-53 (no `ledger-view`) |
| P4b S4 | A-1, A-2, A-3, A-7, A-10, A-13; D-39, D-43, D-45, D-46, D-52 |
| P4b S5 | A-4; D-17, D-52 |
| P4b S6 | A-4, A-8, A-12; D-40, D-42, D-49, D-52 |
| P4b S7 | A-5; D-49, D-52 |
| P4 | top decision 3; D-12, D-33, D-39, D-47, D-50, D-52 |
| P5 | D-28, D-44, D-46, D-53; I-11, I-12 |
| P6 | D-15, D-17, D-20, D-32, D-46, D-53 |
| P7 | D-28, D-29, D-31, D-48 |
| P8 | D-9, D-12, D-39 |
| P9a, P9b, P9c | top decision 4; D-7, D-27, D-28, D-31, D-32, D-51 |
| P10 | D-9, D-12, D-29, D-31, D-47 |
| P11a, P11b | D-19, D-22, D-24, D-25, D-26, D-27, D-32, D-54; Q-4, Q-7 |
| P12 | D-33 |
| P13 | D-34 |

---

## 11. Later: hosted service (not built in this plan)

A paid, multi-tenant service in which an operator runs the books for customers. P0–P12 build none of this. It is recorded here so the design stays compatible with it.

**Shape.**
- Per-tenant pair. `front` (`ledger-mcp front --tenant <t>`, user ID `wb-f-<t>`) does OAuth 2.1 (3.3), passkeys, consent, the human pages and token validation; its Landlock denies `books/`, so it **never** reads tenant disk. `worker` (`ledger-mcp worker --tenant <t> --socket`, user ID `wb-t-<t>`, `DynamicUser=`) serves the MCP, uploads, downloads and the replica for its own tenant only, and learns identity only from `X-WB-Principal` on its 0660 socket. Caddy handles TLS and routing and holds no authority. No component holds authority across tenants. Both roles ship in one binary.
- Workers run from the systemd template `wanderbooks-worker@.service` with seccomp `@system-service` and Landlock; a container image is the alternative. The archive unit runs per tenant.
- Data root `/srv/wanderbooks/t/<tenant>`, with the 3.4 layout, on a LUKS volume per tenant. Books are at `https://host/t/<tenant>/b/<book>/mcp`. Each tenant's front is its own issuer.
- Tenant provisioning, quotas and invitations.

**Vault model B (required before any paid launch).** Unwrap happens on the client, in the signed native viewer or in reproducible WASM with a published SRI manifest. Keyslot material is released only inside an unlock exchange authenticated by a passkey assertion. The web viewer is egui→WASM on `ledger-core` + `ledger-view`; it needs a pure-Rust evidence projection (no SQLite), reproducible builds and a published SRI manifest. The signed native viewer is the reference implementation. Hosting is not marketed as "operator cannot read TINs" until model B ships, because model B depends on operator-served code.

**Tamper evidence.** Opt-in head delivery by email or webhook, and an optional transparency witness.

**File intake.** A per-book email-in inbox (D-17).

**Threats specific to hosting.**

| Adversary | Goal | Controls |
|---|---|---|
| Other tenants | cross-tenant access | per-tenant user IDs and processes; Landlock; per-tenant issuer; token audience is the book URI; capability handles keyed `<sub>:<book>:<256-bit>` |
| Hosting operator (assumed hostile) | read TINs; rewrite or fork history | **TINs:** HPKE to a journaled public key; customer keyslots; model B before any paid launch. **History:** head in every result, quiet-period handoffs, emailed heads (opt-in), `verify-archive --expect-head`, optional witness. **Ledger confidentiality is not provided** (the server is the engine); the terms say so; self-hosting is the zero-trust option |

**Customer flow.**
1. Sign up with a passkey and create books (tenant provisioned: front, worker and archive units, plus a volume).
2. Vault ceremony: passphrase, recovery key shown once and confirmed saved, optional passkey PRF.
3. "Connect" cards for claude.ai, Claude Code, Codex and ChatGPT. claude.ai: Settings → Connectors → Add custom connector → `https://books.example.com/t/acme/b/main/mcp`; OAuth: passkey, then consent with scope checkboxes.
4. Optional: archive to the customer's own git or S3; opt-in head emails.
5. Viewer: `wanderbooks gui https://books.example.com/t/acme/b/main` (OAuth `books:view`, in memory), or the web viewer.
6. Verify at any time: `wanderbooks fetch … && wanderbooks verify-archive <dir> --expect-head <hash>`.
7. Leave: download the archive bundle, `books adopt` it on a self-host, and get the same head.

**Before launch.**
- Tests: `tenant_isolation`; `front_cannot_open_books` (Landlock); `client_side_unwrap_server_never_sees_plaintext` (instrumented); `wasm_viewer_equals_native_view`; `witness_detects_fork`; cross-tenant → 401/403.
- An external security review of the authorization server.
- Load: 1,000 idle and 50 active tenants per node, p95 non-report latency < 300 ms.
- The directory review checklist passes.
- Compliance: state breach statutes and the FTC Safeguards Rule apply to stored SSNs, and data residency matters, so counsel is needed before launch. Retention is 7 years. Account deletion is crypto-shred plus purge.
