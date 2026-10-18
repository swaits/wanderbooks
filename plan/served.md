# wanderbooks Served: the plan

## Decisions of 2026-10-16

Tobin Harlow made these decisions on 2026-10-16. Where anything below conflicts with them, they win, and the plan has been revised to match.

1. **The books live hidden away, as you.** Your computer gets no separate system account, no sudo and no containers. The books server runs as you, as a background service that starts when it is needed. It keeps the books in a private folder that only your account can open, outside every project folder. Nothing the agent is told or shown ever names that folder. This means an agent cannot stumble into the books. An agent that went looking on purpose could still find them, and the books say so plainly ("same user, advisory"). Anyone who wants a hard wall runs the same server on another machine they control, such as a home server, a NAS or a rented server.
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

## 1. Summary

**Today.** Your books are a folder inside your project directory. That folder holds the journal, the receipts, the settings, a list of helper commands and the agent's instructions. The agent opens the books by starting a helper program itself, so it can also read every file in the folder directly. That is how an agent ended up reading the journal file instead of asking the books.

**After this plan.** The books move into a private folder that belongs to a small books server. The server runs quietly in the background as you. Agents are never told where the folder is, and they only talk to the server. In practice:
- You install once, with at most three commands and no administrator password. After that you just type `claude` in any folder. There is no project folder, there are no helper commands, and there is nothing to do at the start of a session.
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
| I-1 | **The MCP is the only agent interface to the books.** Locally this is **advisory** (owner decision 2026-10-16): the server runs as the human, and the books are hidden, not locked. Mechanical enforcement is the self-host deployment on another machine, where agents have no access to the data root. | A data root outside every project directory (3.4), mode 0700, created with an explicit `set_permissions` call (not the umask). No tool, instruction, handbook text or result names a filesystem path (I-2, I-12). The stdio data-dir transport is removed from every `connect` path (`dev-stdio` refuses any data dir inside the data root). `books.start`, `wanderbooks doctor` and the docs report `isolation: "same_user (advisory)"`. Landlock and seccomp confine the server itself where available; they do not restrict the agent and are never described as if they did. Test `agent_uid_cannot_read_tenant`, run in CI against the two-user self-host deployment, not the desktop default. |
| I-2 | **No tool takes or returns a server filesystem path.** | Schema lint `no_path_params`: the build fails on any parameter matching `*path*`, `save_to` or `*_file`. Landlock harness `no_tool_touches_outside_tenant`, run with fuzzed arguments. Static check: no `std::fs` in `ledger-mcp/src/tools/**` outside the ledger-store APIs. |
| I-3 | **The journal holds all books state.** Settings, conventions, decisions, draft states, evidence links, annotations, extraction records and attestations are journaled. Reports are pure functions of the journal. `wanderbooks.toml` is import/export text only. The server never reads a toml file. | `ledger-core::settings` replay. Property test `settings_replay_is_pure` (Store, Snapshot and the view are identical). Test `toml_round_trip_zero_events`. The daemon binary contains no toml file-read code path. |
| I-4 | **Money is exact Decimal, every entry balances, and posted entries are permanent.** Corrections are reversal drafts that need evidence. | Existing kernel, Kani and Lean. New `Step.draftReversal` and `Step.discard`, with preservation proofs. |
| I-5 | **Evidence gate.** One function, `ledger_core::evidence::requirement`, decides whether an entry may post. Store, MCP and GUI all call it. Reversals are not exempt. Depreciation, amortization and closes are exempt by origin, and the exemption is sourced. | The gate lives in `Store::post_entry`. Test `reversal_needs_evidence`. Test `gui_and_server_gate_agree`. |
| I-6 | **A confirmed commit is bound to the server's most recent preview of that exact draft** (stale or altered drafts are refused). It does not prove that a human saw the preview, and locally the key is readable by any process running as the human (advisory). | `preview_token` = HMAC-SHA256 over (entry id, canonical draft state, settings seq, evidence-link set, policy result), keyed with a server secret held in `keys/`. A post with a stale token is refused with guard `stale_preview`. |
| I-7 | **The agent never holds a full TIN or vault key material.** | Vault records are sealed with HPKE to a public key whose sha256 is journaled. The private key exists only wrapped in keyslots held by the customer. Unwrap code is reachable only from human-page handlers and the GUI unlock path (module-privacy test). A pattern-based TIN scanner fails closed on every tool result. A canary TIN/key fuzz test covers responses, errors, logs and the access log. |
| I-8 | **The GUI never mutates and never uses the MCP.** Locally it reads the data root directly, read-only, as today. It finds the root from the platform's well-known location or from `wanderbooks gui <book>`, never from a project directory or `wanderbooks.toml`. `books.show` reaches it through `ops/show.jsonl` in the book's own data root, never through the MCP. Remote books (self-host) are viewed through a read-only replica stream (P9). | `ledger-gui/tests/read_only.rs`. Tests `gui_works_under_read_only_landlock` and `gui_finds_root_without_project_dir`. Test `replica_snapshot_equals_store_snapshot` (P9). The GUI writes nothing under the data root and leaves no files under `$HOME` except eframe preferences. |
| I-9 | **Every access through the server is logged, and logging fails closed.** | A hash-chained `access.jsonl` (own genesis). Property: exactly one record per tool call, upload, download, human action, remote replica connection and backup, refusals included. The local viewer's direct reads are not logged, because the GUI never writes; the docs say so. |
| I-10 | **Tamper evidence.** Every mutating result carries `head {seq, hash}`. The head is handed off when a session goes quiet. The customer can clone the archive and run `wanderbooks verify-archive --expect-head`. Backup and access records stay out of the journal, so they never advance the head they report. | Test `backup_does_not_advance_head`. Test `clone_verify_expect_head`. |
| I-11 | **No loss of ease of use.** Local setup is at most 3 commands on a clean machine, with no sudo, no system account and no container. There is nothing to do per session, it works from any directory, the daily flow adds no client permission prompts (no `requiresUserInteraction`), and the viewer updates within 1 s of a post. | Acceptance criteria in P2 and P5. Tests `service_install_needs_no_root` and `annotations_complete`. The agent-eval harness. |
| I-12 | **Guidance survives client truncation, and never names a path.** Connect text is at most 1,400 characters, and its first 512 stand alone and name `books.start`. Tool descriptions are at most 700 characters. No tool text mentions `just`, `wanderbooks.toml`, `~/`, git, GitHub PRs or any filesystem path, the data root included, and no result names one. | Tests `instructions_budget`, `tool_text_lint` and `no_result_names_a_path`. |
| I-13 | **Formal-methods bar.** Kani harnesses cover kernel arithmetic. Every new ledger command is a Lean `Step` with a preservation proof. A failing proof is a finding. All tools are pinned in `mise.toml`. Clippy reports zero warnings. Every mutation survivor is killed or documented as equivalent. | `just check` and `just prove` pass in every phase. |

### 2.1 Conflict resolutions (explicit)

| Question | Proposals | Resolution |
|---|---|---|
| Process shape | A: single process. B: gateway plus per-book worker. C: daemon, with a gateway when hosted. | **One `ledger-mcp serve` process** (A), both locally and on a self-host machine. Locally it runs as the human under a user-level service (a systemd `--user` unit on Linux, a LaunchAgent on macOS). It has no OAuth authorization server and only a minimal human-page surface. The multi-tenant front/worker split (B) belongs to the hosted service, which this plan does not build (section 11). |
| Isolation, and when it lands | A: P5. B: P6. C: P3. All three assumed a service user. | **Owner decision 2026-10-16: "you, hidden away."** The desktop gets no service user, sudo, system account or container. P2 moves the books into a server-owned data root (0700, outside every project directory) whose path no text ever names. This isolation is advisory and is labelled that way. Mechanical enforcement is the self-host deployment on another machine (P9). From P2 on, CI proves it with `agent_uid_cannot_read_tenant` against a two-user deployment. P2 needs journaled settings first (P1), because the server never reads toml. |
| Settings import | A: server reads legacy toml (`from_legacy:true`). B/C: agent passes text. | **`import_settings {toml_text, config_json_text?}` only.** A legacy fallback (today's `settings::Live`) survives only in `dev-stdio`, until a book is adopted. `books adopt` refuses a journal with no `settings.set` unless `--toml FILE` is given. The human's CLI reads that file and passes the text. |
| Before import | C: writes blocked by a `settings_not_imported` guard. | **Never block.** Legacy books keep `legacy_files` mode in `dev-stdio` (P1). Daemon books always have journaled settings (enforced by `adopt` and `create`). |
| Preview/post | A: `confirmed`. C: split tools plus `preview_token`. B: seven read-only twins. | **Keep `confirmed`, add `preview_token`.** Add one read-only twin, `books.preview_entry`, for the high-frequency post path, so claude.ai doesn't prompt on every preview. Close, reconcile, dispose, `run_*` and purge keep `confirmed` plus the token. `confirmed` without a token is accepted for one release with a deprecation warning. This limits churn in `mcp_stdio.rs`, `oracle_mcp.rs` and `spec_mcp.rs`. |
| `requiresUserInteraction` | B: on every post. | **Not used anywhere in this plan** (owner decision 2026-10-16: no added friction). Post, reverse, close, reconcile, dispose, `run_*`, `purge_tin`, `consolidate_duplicates` and `import_settings` carry `destructiveHint` plus `preview_token`, and the client's normal permission rules apply. Full TINs leave the agent's path through human pages: TIN-bearing outputs from P2, and all TIN handling in P7. |
| Harness deny rules | C: P0 `.claude/settings.json` deny rules. | **Rejected.** That is plan A, which is dropped. P0 corrects only the sentence in the server text and the generated template that caused the `cat` (`/home/swaits/Documents/the first company-Books/AGENTS.md:68-69`, "the journal is the evidence — read it"). This is a content fix, not a guard. |
| Defect 12 rule | A: everything through `financing_accounts` is excluded. | **Classified by instrument (C).** Payment-card and third-party-network payments are excluded (1099-K) and listed. A member's personal cash or check through 2100 stays reportable. An unknown instrument is `undetermined`. |
| Odd cent (17) | A: fair ties. C: rotating tie-break. | **Cumulative apportionment (B)**, extending `kernel::apportion` (kernel.rs:309), its existing Kani proofs, and `Apportion.lean`. |
| Cash flow (1) | A/C: per-entry heuristics. | **Structural partition plus direct-method lines (B).** Footing becomes an identity derived from trial-balance balance. A nonzero difference is an engine error. |
| Vault crypto | A: bespoke X25519/HKDF/XChaCha. | **HPKE, RFC 9180** (X25519-HKDF-SHA256, ChaCha20-Poly1305) (B). The public key's sha256 is journaled. W-9 files are sealed into the vault and never become blobs. |
| Vault unwrap | A: model A first. C: client-side default. | **Model A, locally and on self-host.** The server unwraps inside one human request, or the local GUI unwraps in its own process after the human types the passphrase. Client-side unwrap (model B) is a requirement of the hosted service (section 11). |
| Human-session bootstrap | A: `wanderbooks open-link`, which a same-user-ID agent can mint. | **Rejected.** Every `/h/` page needs a factor that requires human presence: a passkey with user verification, or the owner passphrase set at install. That passphrase becomes vault keyslot 1 in P7. |
| Replica keyring route | A: `/replica/keyring` serves wrapped keyslots to `books:view`. | **Removed.** No route ever serves wrapped keyslots. Local GUI TIN click: the GUI reads the wrapped keyslot from the data root (read-only), unwraps in its own process after the human types the passphrase, copies the value to the clipboard only, and zeroizes. A remote replica (P9) carries no keyslot material; remote TINs are seen only on `/h/` pages. |
| TIN output scanner | A: match decrypted vault records. | **Pattern-based** (extend `has_full_tin`/`redact_full_tins` behind S:870 to every result, with the vault's last-four set as a hint). Fails closed. Documented as a pattern check. |
| Extraction | In-process (A/B). | **Sandboxed subprocess** `ledger-extract` (C): rlimits, seccomp, no network, Landlock read-only on the input blob. |
| Archive vs Landlock | A: server shells out to git, which contradicts its own Landlock rules. | **Separate user-level unit `wanderbooks-archive@<book>`.** It has read-only Landlock on `books/<id>/{journal.jsonl,blobs}`. Locally, Landlock ABI ≥ 4 limits its TCP connect to the target's port (443 or 22); it cannot filter by host, and `IPAddressAllow`/`IPAddressDeny` are not available to unprivileged `systemd --user` units, so host-level egress filtering exists only under a system unit on self-host (`IPAddressAllow`). The server: Landlock ABI ≥ 4 denies all TCP bind and connect (the listening socket arrives through socket activation on 127.0.0.1), so the local server has no egress. On self-host, a system unit adds `IPAddressDeny=any` (loopback allowed); if the built-in authorization server fetches CIMD documents (D-7 a), connect is allowed to port 443 only and `IPAddressDeny` narrows to private and link-local ranges (loopback still allowed), matching the CIMD SSRF guard. This confines the server only; it does not restrict the agent. |
| Attention engine location | A: `ledger-store/attention.rs`. B: wasm-clean `ledger-view`. | **New crate `ledger-view`** (depends only on `ledger-core`), holding `attention::find`, `next_steps`, the evidence gate view, and a `BooksView` trait implemented by Store and Snapshot. A pure-Rust evidence projection (dropping SQLite from the viewer) is deferred to the hosted service's web viewer (section 11). |
| Inline bytes | A: 256 KiB. B: 1 MiB without sha. C: 256 KiB plus sha. | **256 KiB with a mandatory matching sha256.** A mismatch is refused with guard `base64_corrupt`, whose remedy is `books.begin_upload`. `/mcp` body cap is 1 MiB. |
| Same-user mode | C: an announced tier. B: refuse to start without an acknowledgement flag. | **The local default, with no gate flag** (owner decision 2026-10-16). `books.start`, Needs attention, `wanderbooks doctor` and the docs report `isolation: "same_user (advisory)"`. |
| Viewer channel | A: replica over a peer-credential socket. | **Direct read-only disk access locally.** This is the owner's rule: "the GUI has r/o access to ALL state on disk; it never mutates; it doesn't go through the MCP." `books.show` requests reach the GUI through `ops/show.jsonl` in the book's own data root, which the server writes and the GUI watches. Presence is a shared `flock` that the GUI holds on a read-only descriptor of `ops/viewer.lock`. Locally there is no socket, no peer-credential check and no executable check. The replica protocol exists only for viewing remote books (P9). |
| Local token | A: literal header in the Claude config. | **OS keyring plus a helper.** Claude Code `headersHelper` and Codex `http_headers_helper` run `wanderbooks token print --book X` (C). |

---

## 3. Architecture

### 3.1 Processes

```
                  agents (any directory, any client)                               human
   Claude Code (any dir)   Codex    claude.ai / ChatGPT    stdio-only       browser         ledger-gui
          |                  |      (P9: self-host or     (ledger-mcp          |            (as the human)
          | Bearer (keyring  | Bearer  tunnel)            proxy URL)           | owner           |
          |  helper)         |         | OAuth or token        |               | passphrase      |
          v                  v         v                       v               v                 |
   +---------------------------------------------------------------------------------+          |
   | ledger-mcp serve    local: user-level service, runs as the human                |          |
   |                     self-host (P9): same binary on another machine              |          |
   |   confines itself where available (Landlock, seccomp, no outbound TCP)          |          |
   |   /b/{book}/mcp   /b/{book}/u/{id} PUT   /b/{book}/d/{id} GET                   |          |
   |   /b/{book}/h/*  (human pages)    /b/{book}/replica/*  (P9, remote books only)  |          |
   +---------------------------------------------------------------------------------+          |
          | sole writer (0700; the path is never named)    ^ read-only Landlock                 | read-only,
          v                                                |                                    | direct
   <data root>/books/<id>/ journal.jsonl blobs/ ops/show.jsonl ... <-----------------------------+
          ^                                                |
          | sandboxed subprocess (rlimits, seccomp, no net) +-- wanderbooks-archive@<id> --> git / s3 / dir / pull
   ledger-extract (pdf / xlsx / eml / page raster)
```

The data root is `$XDG_DATA_HOME/wanderbooks` on Linux (default `~/.local/share/wanderbooks`) and `~/Library/Application Support/wanderbooks` on macOS. It sits outside every project directory, and no tool, instruction, handbook section or result names it. Because the server runs as the human, this isolation is **advisory**: an agent cannot stumble into the books, but an agent that deliberately hunts for them could read them. The server's own Landlock and seccomp confinement limits what a compromised server can do; it protects nothing from the agent and is never described as if it did. Mechanical enforcement is the self-host deployment (3.8, P9).

| Process | Binary | Runs as | Reads tenant disk |
|---|---|---|---|
| Books server (local) | `ledger-mcp serve --root <root>`, started by a systemd `--user` socket unit on 127.0.0.1:7411 (Linux) or a LaunchAgent (macOS) | the human | yes, sole writer, holds the flock (`ledger-store/src/store.rs:74-89`) |
| Books server (self-host, P9) | the same binary under a system unit on another machine | a dedicated account on that machine | yes, sole writer |
| Archive pusher | `ledger-mcp archive --book <id>` | the server's account, separate unit | read-only journal and blobs |
| Extractor | `ledger-extract` (subprocess) | inherits, sandboxed | the single input blob, via fd |
| Viewer | `wanderbooks gui <book>` runs `ledger-gui --root <root> --book <id>` (local, direct, the default); `ledger-gui --books <url>` (remote replica, P9) | the human | local: yes, read-only, never writes. Remote: none |
| CLI | `wanderbooks` (ledger-cli) | the human | **by design never**. Talks to `run/admin.sock` (peer-credential check: same user ID as the server). The admin socket cannot read books data; it can create or adopt books and mint agent tokens, admin-scope ones included. The peer-credential check keeps other local accounts out; any process running as the human, agents included, can use it (advisory, like the rest of the desktop deployment). |
| stdio shim | `ledger-mcp proxy <url>` | the agent's user | never; it has no data-dir flag |
| Test/demo | `ledger-mcp dev-stdio --data-dir` | any | refuses inside the data root; `wanderbooks connect` never configures it (D-18) |

### 3.2 Transport (one code path, local and remote)

Replaces `StreamableHttpServerConfig::default()` in `serve_http` (`ledger-mcp/src/main.rs:154-180`) with an explicit configuration:
- `allowed_hosts` taken from `server.toml` (deployment config only).
- `enforce_origin_validation()` with an explicit list: loopback by default, plus `https://claude.ai` once OAuth exists in P9. Today's empty list turns the Origin check off.
- `json_response=true`.
- `legacy_session_mode=true` while Cursor and claude.ai stay on ≤ 2025-11-25.
- `max_request_body_bytes = 1 MiB`.

Both protocol eras are served: `initialize` and `server/discover` are both built from `get_info`, which calls `refresh` first, with `cacheScope: private` and `ttlMs: 60000`. rmcp features gain `elicitation` and `request-state` (`ledger-mcp/Cargo.toml:22`). Tools read the principal through `Extension<http::request::Parts>`. Long calls (CPA package, audit, extraction backfill, consolidation) run under `spawn_blocking` and return `{job_id}` when they exceed 20 s; `books.job` polls. The 240 s claude.ai limit and Codex's 60 s are therefore never hit.

**Concurrency.** One `Store` per book behind `tokio::sync::Mutex` (replacing the std mutex at S:553). Tool bodies use `blocking_lock` inside `spawn_blocking`. Readers use a `Snapshot` refreshed after each write, so reports never wait on the writer.

### 3.3 Auth and scopes

| Scope | Allows | Default holders |
|---|---|---|
| `books:read` | reads, reports, `start`, handbook, `get_receipt` text and images | all |
| `books:write` | drafts, lines, attach, annotate, upload, attest, settings preview | agent |
| `books:post` | confirmed post, close, reconcile, dispose, `run_*`, settings commit | agent (local default) |
| `books:admin` | tokens, access log, backup run, consolidate, invite | owner's agent (local default), owner |
| `books:view` | remote replica only (P9) | viewer client only, never agents |

No scope unwraps the vault. Unwrap happens only inside an `/h/` request that has just presented a human factor, or inside the local GUI process after the human types the passphrase (3.6).

- **Local:** a static bearer `wb_` + 32 random bytes, stored only as a sha256, compared in constant time, kept in the OS keyring, minted by `wanderbooks connect`, and revocable with `wanderbooks token revoke`. Agent and server run as the same user, so the token identifies the client and keeps other local users and web pages out. It is not a wall against processes running as the human.
  - Claude Code: `wanderbooks connect claude-code --book <book>` runs `claude mcp add --scope user --transport http wanderbooks-<book> http://127.0.0.1:7411/b/<book>/mcp`, sets `headersHelper` to `wanderbooks token print --book <book>`, and offers a `permissions.allow mcp__wanderbooks-<book>` line.
  - Codex: `wanderbooks connect codex --book <book>` writes:

    ```toml
    [mcp_servers.wanderbooks-acme]
    url = "http://127.0.0.1:7411/b/acme/mcp"
    http_headers_helper = ["wanderbooks", "token", "print", "--book", "acme"]
    ```

    No `default_tools_approval_mode` is written. Codex documents `writes` as prompting for every tool not marked read-only, which would prompt on every post, close and reconcile (against owner decision 3); leaving the key unset keeps Codex's normal rules.
- **Self-host, remote (P9):** the same static bearer over TLS for clients that send headers (Claude Code, Codex). claude.ai and ChatGPT need OAuth, so the self-hosted server offers OAuth 2.1 on this remote path only (D-7):
  - RFC 9728 PRM per book; 401 with `resource_metadata`.
  - 403 `insufficient_scope` for step-up.
  - RFC 8707 audience equal to the book URI.
  - PKCE S256, RFC 9207 `iss`, CIMD preferred (SSRF-guarded), DCR fallback.
  - Exact redirect match (`https://claude.ai/api/mcp/auth_callback`, loopback on any port).
  - `offline_access` only in the authorization server metadata.
  - 1 h access tokens with rotating refresh tokens.
  - Signing keys held through systemd-creds.
  - The self-hosted server is its own issuer (D-7 a), or it validates an external IdP's tokens (D-7 b). Codex uses `codex mcp login`.

**Human factor for `/h/`:** passkey with user verification (webauthn-rs; localhost is a secure context), or the owner passphrase (Argon2id, RFC 9106, m=64 MiB, t=3, p=1). Locally the passphrase is the default. Cookies are `__Host-` and SameSite=Strict, with CSRF tokens and CSP `default-src 'self'`. URL-mode elicitation is used where the client declares it, and the server requires that the session's `sub` equal the token's `sub`.

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
    journal.jsonl  blobs/aa/bb/<sha>  wanderbooks.db(+wal/shm)  wanderbooks.lock
    vault/{tin,w9}-<party>.bin  HPKE-sealed (v2); moved from <books>/vault (S:511)
    keys/keyring.json           {pubkey, slots:[passphrase|recovery|passkey_prf|member_passkey]}, all wrapped
    keys/preview.key            HMAC key for preview_token (0600)
    keys/legacy.key             v1 raw key, P2..P7 only, deleted at rekey
    access.jsonl                hash-chained access log, own genesis "wanderbooks-access-v1"
    ops/backup.jsonl ops/heads.jsonl   not journaled
    ops/show.jsonl              books.show requests: the server appends (bounded, rotated), the GUI watches; not journaled
    ops/viewer.lock             created by the server; the GUI holds a shared flock on a read-only descriptor (presence)
    uploads/<id>.part           staging, swept after 15 min
    derived/<sha>/page-N.png    disposable page-raster cache
    archive.git/                bare; journal.jsonl + blobs/ + vault ciphertext; age-encrypted on push; destination Q-1
```

The server, the CLI and the GUI resolve the data root with one shared function (`ledger-store::paths::data_root()`: the platform data directory, overridable with `--root`), never from the working directory, a project directory or a toml file. The root is created with an explicit `set_permissions(0o700)`, not the umask. Extracted text is stored as a content-addressed blob, recorded in the journal by `attachment.text_extracted`, and archived with the books. Page rasters are disposable. Deleted assumptions: `books_dir = data_dir.parent()` (S:1155, `ledger-gui/src/books.rs:1474`, `ledger-mcp/src/main.rs:102`), the `.gitignore` check (S:531-543), and the `$XDG_RUNTIME_DIR` viewer file (`ledger-store/src/viewer.rs:98-130`), which `ops/show.jsonl` replaces.

### 3.5 Keys

| Key | Location | Usable by |
|---|---|---|
| Vault public key | `keyring.json`; sha256 journaled (`vault.created`/`vault.rotated`) | server, to seal new TINs. A swap fails `books.verify` and `verify-archive`. |
| Vault private key | wrapped per keyslot; plaintext only inside one `/h/` request (server) or one GUI unlock (in the GUI process, local), then zeroized | the human holding the passphrase, passkey PRF, recovery key, or an optional CPA keyslot |
| `preview.key` | `keys/` | server |
| Agent tokens | sha256 in `auth/`; plaintext in the user's keyring, which any process running as that user can read (advisory locally) | the agent's user |
| Archive credentials | OS keyring entry read by the archive unit (local); systemd-creds for the archive unit (self-host) | archive pusher |
| At-rest encryption | full-disk encryption (desktop); encrypted volume (self-host recipe) | protects against stolen disks |

### 3.6 GUI

- **Local: direct and read-only** (desktop default). This is the owner's rule: "the GUI has r/o access to ALL state on disk; it never mutates; it doesn't go through the MCP."
  - `wanderbooks gui <book>` runs `ledger-gui --root <root> --book <id>`. Without `--root`, the GUI resolves the platform data root itself (3.4). It never looks in a project directory or `wanderbooks.toml`.
  - It reads `journal.jsonl` and blobs into a `Snapshot` as today, tailing the journal so that a post appears within 1 s. Every open is read-only, and the GUI runs correctly with the data root under read-only Landlock (test).
  - **`books.show`:** the server appends `{seq, ts, target}` to `ops/show.jsonl` (bounded, rotated, not journaled). The GUI watches it with the same watcher it uses for the journal. This replaces `viewer::request` and the `$XDG_RUNTIME_DIR` viewer file (`ledger-store/src/viewer.rs`). A show request only navigates, so a stray writer can do no more than move the view.
  - **Presence:** the GUI holds a shared `flock` on `ops/viewer.lock`, opened read-only. The server probes the lock to report `viewer.open` and whether a show request was delivered.
  - Locally there is no socket, no peer-credential check and no executable check.
- **Remote: replica** (P9, self-host books only), `ledger-gui --books <url>` with the static token or OAuth, and `books:view`:
  - It pulls `/replica/journal?after_seq=N` (SSE of raw canonical lines) into `Snapshot::from_lines(LineSource, BlobSource)`. This reuses the parse, chain-check and replay loop of `snapshot.rs:137-183`.
  - Blobs are fetched on demand, hash-verified, and kept in a 512 MiB in-memory LRU, never on disk. "Not fetched" is a separate state from "damaged" (fixes `books.rs:456-478`, `attention.rs:551-568`).
  - `/replica/events` carries `books.show` targets per principal, and presence.
  - The replica carries no keyslot material.
- **Settings come from the journal.** Deleted: `data.rs:76-87,157-162,188-195`, `books.rs:536-542`, `main.rs:47-53`, `--books-toml`.
- **Needs attention** comes from `ledger_view::attention::find`, so it is the same list as `books.start`.
- **Save a copy / Open with system app** on any document (`Snapshot::attachment_bytes`, snapshot.rs:328). Fixes the viewer half of defect 3.
- **TIN click:** locally, the GUI prompts for the passphrase, reads the wrapped keyslot and the sealed record from the data root, unwraps in its own process, and returns the value to the clipboard-only path (`tests/tin_copy.rs` behaviour), then zeroizes. This is not access-logged, because the GUI never writes; the docs say so. Remotely, TINs appear only on `/h/` pages.
- `[viewer] open_at_start` becomes a per-human eframe preference (`app.rs:833`), not journaled.

### 3.7 Backup and tamper evidence

The `wanderbooks-archive@<book>` unit (a user-level unit locally, a system unit on self-host) is triggered by a head-advance signal (debounced 30 s), by `period.closed`, and by a timer. Each run:
1. Verifies the chain.
2. Commits the new journal lines and blobs to `archive.git` (one commit per push; the message is the seq range plus the head).
3. Pushes to the targets: git remote, S3, dir, or `pull` (the customer runs `wanderbooks fetch`).

It writes `ops/backup.jsonl`. `books.backup {status|run}` reports it, and Needs attention flags a target more than 24 h behind. Head handoff:
- every mutating result carries the head;
- `ops/heads.jsonl` records each 10-minute quiet period;
- the customer clones the archive and runs `verify-archive --expect-head`.

Email or webhook head delivery and a witness log belong to the hosted service (section 11).

**Restore (owner, 2026-10-17: "these backups need to be restorable, both in existing and new installations").** A backup is complete: the journal, every blob, the vault records (sealed), and the head `{seq, hash}` at backup time. `wanderbooks books restore <backup> [--id X]` (and the viewer's "Restore…", which hands it to the server) works the same on a clean install and on an existing one:
1. **Proven before use.** The server replays the whole chain, re-hashes every blob and checks that the head equals the one the backup records. Any mismatch refuses the restore with a plain reason; nothing is half-restored.
2. **Never destructive.** On an existing install a restore arrives as its own book beside the current ones. Replacing a book moves the old one into `legacy/<id>-<date>/`, kept, never deleted, and only on the owner's explicit choice.
3. **The vault comes back only with its key.** The backup carries the vault sealed; the passphrase or the printed recovery key (P7) unlocks it on any machine. Until P7 the v1 key is a separate file: a backup counts as complete only when that key is saved too, and Needs attention says so when it isn't.
4. **Restore drills.** On a schedule (default weekly) and after each close, the archive unit restores the latest backup into a scratch folder, verifies it as in step 1, compares its reports with the live book's as of the backup head, and throws it away. A failed drill is a Needs-attention item; `books.backup status` reports the last drill.

### 3.8 Local vs self-host

| | Local (T1, the default) | Self-host (T2, P9) |
|---|---|---|
| Where | the human's computer | another machine the owner controls: home server, NAS or VPS |
| Processes | one `serve` under a user-level service, plus a user-level archive unit | the same binaries under system units, plus TLS (Caddy) and, optionally, `wanderbooks tunnel` |
| Runs as | the human | a dedicated account; agents have no account on that machine |
| Isolation | `same_user (advisory)`: hidden, not locked | `service_user (enforced)`: machine and user-ID boundary |
| Auth | static bearer in the keyring; `/h/` uses the owner passphrase (or a passkey) | static bearer over TLS for header-capable clients; OAuth 2.1 for claude.ai and ChatGPT (D-7); `/h/` uses a passkey or the passphrase |
| Vault unwrap | model A (server, inside the human request), plus the in-process GUI unlock | model A |
| Viewer | reads the data root directly, read-only | replica over `books:view` |
| Residual risk | a process running as the human (for example an agent that deliberately goes hunting) can read the data root, the agent token and the wrapped keyslots; TIN plaintext still needs the passphrase. The server's Landlock and seccomp confine only the server | whoever administers that machine |

A multi-tenant hosted service is out of scope for this plan; see section 11.

---

## 4. The MCP surface after the change

### 4.1 Connect instructions (composed in `get_info`; replaces `INSTRUCTIONS` S:57 and the string replace at S:8179)

Hard cap 1,400 characters. The first 512 contain `books.start`. Composed from journaled state:

```
wanderbooks: the double-entry books of {company.name|"a new company"}. Call books.start first in every session: it returns the rules in force, this company's conventions, what needs attention, and your next steps. These tools are the only way to the books; never look for them in local files. Amounts are decimal strings ("123.45"). Entries: books.draft_entry, books.add_line, books.preview_entry, show the human the preview, then books.post_entry with its preview_token. Posted entries never change; books.reverse_entry makes a reversal draft.
{evidence_rule}. No receipt yet? books.set_draft_state awaiting_evidence keeps the draft visible until it arrives. Files go in with books.begin_upload (inline base64 only under 256 KiB, with its sha256) and come out of books.get_receipt as text, page images or download links. Every change returns next_steps, guards and the journal head: follow them and show the human the affected balances. Never ask for, accept or repeat a full TIN: books.request_tin_entry gives the human a secure link. Procedures are prompts (onboard, month_end_close, cpa_handoff, audit); the guide is books.handbook or books://handbook.
```

`{evidence_rule}` comes from `evidence::rule(&settings)`, which replaces `ReceiptPolicy::rule()` (`ledger-config/src/receipt_policy.rs:147`) and now names the required and exempt accounts. The first company example: "Every entry needs a receipt attached or linked before posting; depreciation, amortization and closes are exempt; a reversal needs its own receipt or the original's linked".

### 4.2 `books.start` (read-only, `_meta["anthropic/alwaysLoad"]=true`, ≤ 8 KB)

The v1 ships in P1 with the fields marked †. The full version ships in P5.

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
  "drafts": {"building":n, "awaiting_evidence":[{"entry_id","date","amount","note"}], "reversal_drafts":[...]},
  "reconciliation": {"tracked":[...], "never_reconciled":[...], "behind":[...]},
  "attestation": {"bar":{...}, "due":[...], "attested_through"},
  "viewer": {"open":bool, "remote":[principal...], "how_to_open":"wanderbooks gui <book>"},         // remote: P9 replicas
  "backup": {"last_at","through_seq","stale":bool},
  "handbook_sections": [{"id","uri","chars"}], "prompts": [...],
  "next_steps": [{"tool"|"prompt","why","args","required"}] }
```

When `state=needs_onboarding`, `next_steps = [{prompt:"onboard"}]`. Conventions are presented as company policy data, not as directives, which keeps the server eligible for directory review.

### 4.3 Handbook

`books.handbook {section?}` (read-only) and `books://handbook`, `books://handbook/{section}`. The source is `ledger-mcp/handbook/*.md`, compiled in with `include_str!`. It is generic and carries no the first company leaks ("FL-0001" and "Due to Shareholders" become neutral examples).

Sections: `contract`, `entries`, `drafts-and-reversals`, `evidence-and-receipts`, `attachments`, `files-in-out`, `reconciliation`, `fixed-assets`, `prepaid-and-recurring`, `revenue-and-payouts`, `parties-and-tins`, `1099`, `closing`, `attestations-and-audit`, `cpa-handoff`, `settings-and-conventions`, `viewer`, `rules-that-change`, `control-roles`, `tracing-figures`.

`tracing-figures` replaces "the journal is the evidence — read it" with "trace through `books.get_entry`, `books.search_entries`, `books.journal_history`". Each section ends with this company's conventions that apply to it, rendered from journaled `convention.set` events. The long tool descriptions move here: `export_cpa_package` (1,677 characters), `set_entry_party` (1,453), `set_w9` (1,378), `vendor_1099` (1,055).

### 4.4 Prompts (`#[prompt_router]`; the prompts capability is enabled at S:8172-8176; arguments are single tokens)

| Prompt | Args | Drives |
|---|---|---|
| `onboard` | none | company facts → `set_setting` previews; chart template by entity type; members; timezone; evidence policy; attestation bar; opening entry; trial-balance check; offer `migrate`; offer to open the viewer |
| `migrate` | `source=qbo\|csv\|sheet\|other` | import → `compare_reference` to the cent → `record_decision` cutover |
| `process_inbox` | none | each upload left on the human drop page → find or create the entry → attach → post when ready, or leave a draft awaiting evidence |
| `receipts_from_email` | none | `attach_email` flow |
| `reconcile` | `account=CODE month=YYYY-MM` | statement → `record_statement` → `mark_cleared` → `reconcile` |
| `month_end_close` | `month=YYYY-MM` | data-driven: tracked accounts, financing-account review, schedules, depreciation, recurring, `close_step` conventions, `books.audit scope=month`, `close_period`, month-end `attest_period` |
| `cpa_handoff` | `year=YYYY` | full audit at the bar → attestations → `export_cpa_package` (no TINs) → `request_human_download` |
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
| `books://attachments/{id}/file` | `BlobResourceContents` up to 1 MiB; larger files return an error naming `get_receipt view=link` |
| `books://journal/{seq}` | — |
| `books://workpapers/{id}` | — |

`books://receipts/{id}` (S:8142, base64) remains for one release as an alias of `/attachments/{id}` with text.

### 4.6 Result envelope (single choke point in `call_tool`, S:8160)

```json
{ ...payload,
  "next_steps": [{"tool","args","why","required":bool}],
  "guards":     [{"code","message","remedy_tool","remedy_args"}],
  "warnings":   [...],
  "head":       {"seq","hash"} }           // on every mutating result
```

`next_steps::decorate(tool, &result, &view)` takes items from `ledger_view::attention` filtered to the touched subject, plus tool-specific steps. Examples:
- After a post: attach or attest evidence if it is missing, `books.show entry`, the affected balances.
- After a reversal draft: add evidence.
- After a close: `attest_period`.
- After `export_cpa_package`: the human download link.

Report results also carry `as_of_status {drafts_open_on_or_before, unreconciled, attested_through, settings_problems}`. Properties: every mutating result has a `head` and a non-empty `next_steps`; every guard names a remedy tool that exists.

### 4.7 Files in and out

**In:**
- `books.begin_upload {sha256, size, mime, file_name, purpose: receipt|document|import}` returns `{upload_id, put_url, curl, human_url, expires_at, max_bytes}`. If the bytes are already stored it returns `{existing_attachment_id}`.
- `PUT /b/{book}/u/{upload_id}` is single-use, valid 15 minutes, bound to the principal, sha and size, capped at 25 MiB by default, and re-hashed before acceptance.
- `human_url` is a drop zone on `/h/inbox` for clients with no shell (claude.ai).
- `attach_receipt`, `file_document`, `import_bank_csv`, `import_chart_csv` and `attach_email` take `upload_id | inbox_id | sha256 | content_base64 (≤256 KiB + required sha256)`.
- `books.inbox` / `books.discard_inbox_item` manage the drop zone.
- `openai/fileParams` is declared on the attach tools (P9). An MCP App upload widget comes in P9.

**Out:**
- `books.get_receipt {id, view: meta+text (default) | text | page_image{page, max_px:1568} | link | bytes (≤256 KiB, with sha)}`.
- `books.download_link {attachment_id|export_id}` returns `{url, expires_at, sha256, size}`. Links are single-use, valid 10 minutes, access-logged, and served with `Content-Disposition: attachment` and `nosniff`.
- `export`, `asset_labels` and `export_cpa_package` return a link, plus inline JSON/CSV when ≤ 50K characters.
- TIN-bearing outputs come only from `books.request_human_download`, which returns an `/h/` link.

**Typical receipt flow (Claude Code, the daily case in 7.2):** `sha256sum` → `books.begin_upload` → `curl -T` the file to `put_url` → `attach_receipt {upload_id}` → `get_receipt` (text) → `draft_from_template` or `draft_entry` → `preview_entry`, shown to the human → on "yes", `post_entry` with the `preview_token` → `next_steps` and the head come back, and `books.show` moves the viewer to the entry. No step raises a client permission prompt beyond the client's normal rules. "Save it as a draft" is `set_draft_state awaiting_evidence`. On claude.ai, files ≤ 256 KiB go inline with their sha256; larger ones go through `human_url` and the `process_inbox` prompt.

### 4.8 Drafts waiting on evidence

- Draft states are `building | awaiting_evidence{note}`, plus reversal drafts.
- Open drafts appear in `books.start.drafts`, the digest, Needs attention ("Draft #52 awaiting receipt: Figma annual plan, noted 2026-10-16") and every report's `as_of_status`.
- A close is refused while a draft in the period is open (existing Lean `hdrafts`). `discard_draft` is the way out.
- `export_cpa_package` refuses while a draft dated on or before `as_of` is open, unless given `allow_open_drafts {reason}`.

### 4.9 Tool changes

**New:**

| Area | Tools |
|---|---|
| session | `books.start`, `books.handbook`, `books.job`, `books.journal_history {after_seq?, types?, subject?, limit}` (the only way to "read the journal"), `books.viewer_status` |
| settings / conventions | `books.settings` (from the journal, with provenance and schema), `books.set_setting`, `books.import_settings {toml_text, config_json_text?}`, `books.export_settings`, `books.conventions`, `books.set_convention`, `books.retire_convention`, `books.record_decision`, `books.decisions`, `books.open_items`, `books.raise_item`, `books.resolve_item`, `books.draft_from_template {template_id, ...}` |
| drafts | `books.preview_entry` (read-only), `books.list_drafts {as_of?, state?}`, `books.set_draft_state`, `books.discard_draft {reason}`, `books.inherit_evidence {entry_id, from_entry, reason}` |
| chart | `books.rename_account` |
| evidence | `books.begin_upload`, `books.inbox`, `books.discard_inbox_item`, `books.attach_email`, `books.find_attachment {sha256\|md5\|receipt_number\|text}`, `books.annotate_attachment`, `books.consolidate_duplicates`, `books.add_instrument`, `books.instruments`, `books.set_entry_instrument`, `books.download_link` |
| assurance | `books.audit {through, scope}` (job, deterministic workpaper), `books.workpaper`, `books.attest_receipt`, `books.review_entry`, `books.attest_period`, `books.attestations` |
| ops | `books.backup {status\|run}`, `books.access_log` (admin), `books.invite {kind: cpa\|member\|viewer, email}` (admin; remote people need the self-host server, P9) |
| migration | `books.compare_reference {reference_attachment, as_of, mapping_convention}`, `books.record_inventory {source, items\|upload_id}`, `books.check_inventory {listing}` |
| reports | `export` kinds `members_equity`, `general_ledger`, `general_journal` (JSON with `entry_id`) |
| vault (human links) | `books.request_tin_entry {party, kind: ssn\|ein\|itin\|w9}`, `books.request_human_download {what: cpa_package_with_tins\|vendor_1099_with_tins\|w9, party?, year?}`, `books.set_w9_facts {party, name, classification, address, signed_on, exempt_payee?}` |

**Changed:**
- `post_entry` requires `preview_token`; the gate moves into Store; the result carries `posted_seq` and `posted_at`.
- `reverse_entry` creates a reversal draft, with `evidence: "link_original" | {attachment_ids} | "later"`.
- `attach_receipt` and `file_document` deduplicate by sha: identical bytes link the existing record, so "deduplicated" becomes true. Text is extracted at attach.
- `get_receipt`: views as in 4.7.
- `digest` (defect 6).
- Reports: one default range, `as_of_status`, and `strict`.
- `cash_flow`: `reconciles`, `noncash_disclosures`, `operating_reconciliation`.
- `vendor_1099`: instrument-aware; `include_tins` and `save_csv_to` dropped.
- `set_vendor_1099` takes an explicit `not_reportable` plus a reason.
- `register_asset`: `in_service_date` + `acquired_date`, with `purchase_date` kept as an alias for one release; basis fields.
- `list_assets`: `accumulated_posted` and `accumulated_scheduled_through`.
- `list_schedules`: start, end, convention, next, remaining.
- `parties`: last four from `party.tin_set`.
- `add_account`: tax-category warning, or refusal under the policy.
- `show`: refuses `start > end`; appends the target to `ops/show.jsonl` in the book's data root (locally) or to the replica event channel (P9, remote viewers), and reports whether a viewer is open and was reached. The result never names a path.
- `verify`: also checks `access.jsonl`, the vault public-key sha and extraction blobs.
- `export_cpa_package`: no-TIN package returned as a link; becomes a job.

**Annotations (P0):** every tool gets a `title` and `readOnlyHint` or `destructiveHint`, plus `idempotentHint` where true. Corrections: `export_cpa_package` is no longer marked read-only; `verify`, `list_periods`, `ar_aging` and `ap_aging` are annotated. No tool gets `requiresUserInteraction` (2.1; owner decision 2026-10-16).

**Removed:**
- `import_local_file` (S:4511): restricted in P0, deleted in P2.
- `get_w9`.
- `set_tin`'s `tin` and `set_w9`'s `tin`/`w9_file_path`: guarded in P0, deleted in P7.
- Every `save_to`/`save_csv_to` (`types.rs:368, 2563, 2686, 2758-2764`): deleted in P2.
- `include_tins`.
- The `INSTRUCTIONS` const.
- `viewer::request` (replaced by `ops/show.jsonl`).
- `--books-toml`/`--config` in serve mode.
- Every mention of `just` or `wanderbooks.toml` in tool text: S:340, 390, 525, 546, 637, 640, 1902, 3061, 3251, 3374, 4060, 4315, 4568, 5194, 5829.
- `server.rs` (8,529 lines) is split into `tools/<group>.rs` as each phase touches a group.

The tool count goes from 93 to about 125. Only `books.start` and `books.handbook` load upfront under tool search.

---

## 5. State migration

### 5.1 Format gate (P0, before any new event)

- An unknown type is no longer `JournalCorrupt` (`store.rs:4419-4423`, `:4640`, Snapshot, GUI). It becomes `StoreError::NewerJournal {seq, type, need}`, shown as "update wanderbooks".
- A one-time `journal.format_set {format: 2, features: [...]}` is written before the first v2 event.
- Fixture journals from every phase are added to `books_written_under_older_rules_still_open`.
- `wanderbooks update` upgrades all binaries together. The local GUI reads the journal directly, so it hits the same `NewerJournal` gate; the replica protocol (P9) carries a minimum version header.

### 5.2 New events (all additive; legacy events replay unchanged)

Every event may carry an additive `actor {principal, client_id, kind}`, which replay ignores.

| Event | Phase | Replay target |
|---|---|---|
| `settings.set {key, value, effective_from?, source{decided_by, decided_on, citation?, note?}}` / `settings.cleared` | P1 | `ledger_core::Settings` (Lean `Step.setSetting`: balances and drafts unchanged) |
| `convention.set {id, kind: posting_template\|approval_rule\|policy\|close_step\|calendar\|mapping\|note, title, body_md, structured?, source, supersedes?}` / `convention.retired` | P1 | `Conventions` |
| `decision.recorded {id, subject, decision_md, decided_by[party], decided_on, evidence_attachment?, supersedes?}` | P1 | `Conventions` |
| `open_item.raised` / `open_item.resolved` | P1 | `Conventions` |
| `account.renamed {code, name, previous, reason}` | P3 | chart |
| `instrument.added {id, kind: bank\|business_card\|member_card\|member_cash_or_check\|platform, label, last4?, owner_party?, account?}` / `entry.instrument_set` | P3 | `Ledger` (1099) |
| `asset.basis_set {asset, rule: purchase\|carryover_723\|personal_use_conversion, fmv_at_contribution?, contributor_adjusted_basis?, tax_basis_723, depreciable_basis, built_in_gain_704c, sec704c_method?}` (legacy `asset.tax_basis_set` still replays) | P3 | assets |
| `schedule.registered` + `convention` (absent = `monthly_even`) | P3 | schedules |
| `vendor.1099_set` value `not_reportable{reason}`; legacy `null` replays as `undetermined` | P3 | vendors |
| `period.closed` with `close_to: MembersCumulative{since_seq}` (old variants replay as before) | P3 | periods |
| `entry.draft_state_set {id, state, note}` / `entry.draft_discarded {id, reason}` | P4 | `Ledger` drafts (`Step.discard`; `draftsOpen` at `Ledger.lean:133`) |
| `entry.reversal_drafted {id, reverses, date, description}` (lines derived from the original) | P4 | `Step.draftReversal`; posted later by an ordinary `entry.posted` |
| `entry.evidence_inherited {entry_id, from_entry, attachment_ids, reason}` | P4 | evidence |
| `entry.posted` + `actor`, `preview_digest` | P4 | ignored by replay |
| `attachment.text_extracted {id, text_sha256, extractor, version, pages, status: ok\|none\|failed}` | P6 | evidence |
| `attachment.rendered {id, from: eml, pdf_sha256, headers{message_id, from, to, date, subject}, source_sha256}` | P6 | evidence |
| `attachment.annotated {id, fields{receipt_number?, payee?, amount?, date?, instrument?, category?, source?, reimbursable_to?}, note}` | P6 | evidence |
| `entry.settles {entry_id, settles[]}` | P6 | derived `reimbursed` |
| `inventory.recorded`, `reference.compared {reference_sha256, as_of, mapping, workpaper, differences}` | P6 | migration |
| `vault.created` / `vault.rotated {pubkey_sha256, scheme:"hpke-x25519-chacha20poly1305-v2"}`, `vault.keyslot_added` / `vault.keyslot_removed {slot_id, kind, label}`, `vault.resealed {party, kind, old_hash, new_hash}` | P7 | evidence; `vault_tin` (S:1326-1378) accepts the newest hash |
| `audit.run {id, through, scope, program_version, through_seq, tb_hash, findings, workpaper}` | P8 | assurance |
| `receipt.attested`, `entry.reviewed`, `period.attested {..., method{model, reasoning_effort, adversarial, verifier_agents}, verified{principal, client_id}}`. `counts` is **derived** against the bar in force at that seq; it is never stored. | P8 | assurance |

**Not journaled:** the access log, backup runs, head handoffs, viewer preferences, page rasters, and any key material or key path.

**Projection (schema v12, `schema.rs:8`):**
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
| policy | `policy.evidence{threshold, required_accounts, exempt_accounts}` (replaces config.json `receipt_policy`), `policy.parties`, `policy.capitalization{de_minimis, twelve_month_rule, sources}`, `policy.require_tax_category`, `policy.attestation_bar` (default per D-9), `policy.cpa_package_requires_audit: refuse\|warn` (default `refuse`, overridable per export with a reason; D-9), `policy.audit_cadence`, `policy.approval_rules: off\|enforce` (default `off`: `approval_rule` conventions are recorded but never block a post) |

`base_currency` stays in `store.created`. `[books] data_dir`, `[vault] key_file` and `[viewer]` are dropped on import with a note. Unknown keys are reported as problems instead of being silently ignored.

### 5.4 toml import/export

`ledger-config` shrinks to a codec:
- `toml_to_events(&str, &Settings) -> Result<Vec<Event>, Problems>` emits only the diff.
- `settings_to_toml(&Settings, &Conventions)` writes provenance as comments.

The `WANDERBOOKS_TOML` template (`ledger-cli/src/main.rs:237-347`) becomes the export format. `ledger-cli/tests/init.rs:305,354` become round-trip properties: export then import yields zero events, and every example imports. The `config_parse` fuzz target is kept, and `fuzz_settings_replay` is added.

### 5.5 Migrating existing books (generic)

- `wanderbooks books adopt <data-dir> --id X [--toml FILE] [--config-json FILE]` runs as the human, with no sudo, through the admin socket. The server, which also runs as the human, does the copy:
  1. Takes the source flock, so it refuses while a stdio server holds it.
  2. Verifies the chain and records head H.
  3. Copies `journal.jsonl` and `blobs/`, re-hashes every blob, rebuilds the projection, and checks that the head equals H.
  4. Imports settings from the given texts (preview printed, confirmation at the TTY) unless the journal already has `settings.set` events.
  5. Moves `vault/*.bin` into `books/X/vault/`, and the v1 key into `keys/legacy.key`.
  6. Seeds `archive.git` from the source's git history, filtered to the journal and blob paths, and records the old repository's last commit hash. The push destination is the owner's answer to Q-1.
  7. If the owner agrees (Q-3), moves the source data dir, its vault and the project folder's git history into `<root>/legacy/X-<date>/`. Nothing is deleted. If the owner says no, the source stays where it is, read-only, and Needs attention lists it as an exposed old copy.
- `ledger-cli` `init`/`new` stop scaffolding and call `books create`.
- `dev-stdio` keeps `legacy_files` mode for unadopted data dirs for one minor release; `books.start` returns `next_steps: adopt`.
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
| P1 | The agent reads `/home/swaits/Documents/the first company-Books/wanderbooks.toml` (and `books-data/config.json` only by asking the owner to state its values, since it sits under `books-data/`) and calls `import_settings`. Expected diff:<br>• the 15 `[company]` keys; EIN as identity<br>• members 50/50 → 3000/3001 with `files_jointly_with`<br>• `accounting_basis=accrual`, `policy.evidence.threshold=0.00` (the owner, 2026-10-12)<br>• `cash_accounts=["1000"]`, `financing_accounts=["2100"]`; the citation is corrected to ASC 230-10-45-14(b)<br>• `parties=require` (2026-10-15), `timezone=America/Denver` (D-13, decided)<br>• `policy.approval_rules` stays `off`<br><br>Conventions are proposed in one batch preview from `AGENTS.md:74-227`:<br>• 9 posting templates: joint contribution as two entries; expense from joint personal funds → 2100; expense from Bluevine; household reimbursement (`Dr 2100 / Cr 1000`) with a `note` that the owner gives the go-ahead before money moves, recorded with its source and not an enforced gate; 50/50 distribution; revenue 4000; card 2050 expense and payment; cash entries dated by bank posting date<br>• policies: numbering, one file per receipt (PDF preferred), email as PDF, capitalization (`policy.capitalization` plus decisions 2026-10-11 and 2026-10-16)<br>• `close_step`: "ask the owner whether a member paid anything personally"<br>• `calendar`: Utah annual report around Oct 4, renewal fee to taxes & licenses<br>• memory-only rules become conventions: post when ready; decisions to the owner go one question at a time; TINs only in the vault<br><br>Decisions: one file per receipt (10-11), threshold (10-12), cutover with Sheet frozen as of 10-11 and reference attachment 37 (10-14), renumber 6100→6050 and card at 2050 (10-14), TINs in vault (10-15); light governance, meaning no consent paperwork, spending limits, signed memos or amendments for the two-member LLC, with decisions recorded with their source (10-16); local isolation "you, hidden away", the viewer reading the disk directly, no added permission prompts, and self-host now with hosting later (10-16).<br><br>Open items:<br>• IRS name control "WALN" discrepancy; letter = attachment 70, mailing pending<br>• deferred revenue to be raised with the CPA before the first subscription<br><br>Per-entry notes: entries 4 and 5 dated Oct 7 to match the Sheet. | Every golden report and export is byte-identical before and after the import. GUI shows the first company correctly with `wanderbooks.toml` renamed away. No `approval_rule` convention is created, and every daily-flow post still succeeds without an approval |
| P2 (pre-cutover) | Run `verify-sheet` and `drive-manifest verify` a final time and file their outputs as `workpaper` documents. Recapture goldens. | Both pass |
| P2 cutover (about 10 min) | 1. Ask the owner the three cutover questions (section 10.3: Q-1 backups, Q-2 `company/` records, Q-3 old copies), one at a time, in plain English.<br>2. Quit the agent sessions and the GUI.<br>3. `wanderbooks service install`, with no sudo: it writes the systemd `--user` socket and service units, creates the data root (0700), and asks for the owner passphrase.<br>4. `wanderbooks books adopt /home/swaits/Documents/the first company-Books/books-data --id fernhill` (archive target from Q-1; old copies moved per Q-3).<br>5. `wanderbooks connect claude-code --book fernhill` (removes the old stdio registration made by `just mcp-add`, adds the HTTP registration with `headersHelper`).<br>6. `wanderbooks connect codex --book fernhill`.<br>7. `wanderbooks gui fernhill` (offers login autostart).<br>8. Remove `justfile`, `.claude/settings.json` hook, `.codex/hooks.json`, `scripts/` (archived under the data root's `legacy/`); `AGENTS.md`/`CLAUDE.md` become a 3-line pointer ("connect the wanderbooks MCP; call books.start").<br>9. Per Q-2, file the `company/` records as documents, or leave them as local files. | `books.verify` over MCP equals H. Goldens byte-equal. `books.start` works from Claude Code started in `~` and in `/tmp/empty`, and reports `isolation: "same_user (advisory)"`. No connect text, tool text, handbook section or result in a recorded cutover session names the data root. No step asked for sudo. If Q-3 is "move": the project folder holds no journal, blobs, vault or git history containing them. Viewer updates within 1 s. **Rollback (nothing posted on the server):** stop the user service, move `legacy/fernhill-<date>/` back to the project folder (or use the untouched source if Q-3 was "no"), and re-register the P1 binary's stdio server as before cutover. **Rollback (entries posted):** `wanderbooks books export-legacy fernhill` writes a data dir with the same head check, then the same re-registration |
| P3 | Defect fixes change reports (cash flow, 1099 sections, asset fields, digest). Each difference is hand-adjudicated in the P3 change log. 1099 classification and 704(c) fields are shown to the CPA. | Change log signed off |
| P4 | `list_drafts`. Leftover drafts are discarded or set to `awaiting_evidence`. | Zero unexplained drafts |
| P6 | • `consolidate_duplicates` resolves the 11 redundant records (preview first).<br>• Extraction is backfilled for all 66 attachments.<br>• Receipt numbers 1000–1029 are annotated from the "Receipt ID" in each description. Anomalies become annotations: 1009 unissued; 1011/1020 byte-identical pairs (merged); 43/45/47 have no ID. The next number is derived as 1030.<br>• Instruments: Bluevine, joint Amex, member cards, Chase Ink 2050.<br>• `verify-sheet.py`'s ALIASES/RECLASSIFIED/RENUMBERED tables (`scripts/verify-sheet.py:52, 69-108`) become a `mapping` convention, then `compare_reference` runs against attachment 37 as of 2026-10-11.<br>• `drive-manifest.json` text goes to `record_inventory`; the Drive connector listing goes to `check_inventory`. | `compare_reference` gives 0 differences. Inventory has 0 gaps. Drive deletion becomes possible (still the owner's call) |
| P7 | `wanderbooks vault init --book fernhill` (on the human page, behind the owner passphrase): generates the HPKE keypair; keyslots for passphrase, recovery key (printed) and optional passkey PRF; reseals every record; journals `vault.created`, `vault.keyslot_added`×N, `vault.resealed`. A TIN is checked in the viewer, then `keys/legacy.key` and `~/.config/wanderbooks/vault.key` are shredded (the owner confirms). | Last-four and kind match the journal. No vault key material remains on disk except the wrapped keyslots in the data root (no v1 key, no `~/.config/wanderbooks/vault.key`) |
| P10 | Re-audit (section 8). | — |

---

## 6. Security and threat model

This section covers the local (T1) and self-host (T2) deployments. Threats specific to a multi-tenant hosted service (other tenants, a hostile operator, SSN compliance) are in section 11.

**Adversaries and controls**

| Adversary | Goal | Controls |
|---|---|---|
| The agent (careless or prompt-injected; locally it runs as the human's user ID, the same as the server) | read or alter books outside the MCP; see TINs | I-2, I-7, I-12: no text ever names the data root, which sits outside every project directory. Every call is access-logged. Destructive hints are set. `preview_token`. **Locally this is advisory:** an agent that deliberately hunts can read the data root, just as the human can, and `books.start`, `wanderbooks doctor` and the docs say so. TIN plaintext stays sealed even then, because unwrap needs the passphrase. Enforcement is the self-host deployment, proven by `agent_uid_cannot_read_tenant`. Where a company turns on approval-rule conventions (off by default), they are asserted by the agent (`approval{by_party}`): a policy, not proof, which the access log records |
| Network attacker or browser | DNS rebinding, CSRF against localhost | Host allowlist; Origin enforced (403); bearer tokens only on `/mcp`; `/h/` cookies `__Host-` and SameSite=Strict, with CSRF tokens; no tokens in query strings |
| Compromised OAuth client (self-host remote path, P9) | over-broad access | PKCE, exact redirect match, `iss`, CIMD SSRF guard (public IPs, 5 s, 64 KiB), consent per client per book, minimal base scopes with step-up, 1 h tokens with rotating refresh |
| History rewrite or fork (any deployment) | alter or fork the books unnoticed | head in every result, quiet-period handoffs, `verify-archive --expect-head` against an independent clone |
| Stolen disk or backup | read books | full-disk encryption on the desktop, an encrypted volume in the self-host recipe; archive encrypted with age to a customer recipient; the vault is always ciphertext |
| Untrusted files (PDF/XLSX/EML) | parser exploit inside the data-owning process | the `ledger-extract` subprocess sandbox; fuzz targets |
| A compromised server process | reach beyond the data root into the human's account | the server confines itself where available: Landlock (filesystem limited to the data root; on ABI ≥ 4, all TCP bind and connect denied, since the listening socket arrives through socket activation), seccomp, rlimits; extraction runs in a further sandbox. This limits the server; it does not restrict the agent |
| Other processes running as the human, on the desktop | read the books; drive the viewer | no mechanical control locally (same user). A same-user process can alter anything the GUI reads (journal, blobs, keyslots). Journal tampering is detected by the hash chain and head handoff, not prevented; a vault public-key swap fails `books.verify`; a forged show request can only navigate. The GUI never writes. Stated as the advisory residual in `books.start`, `doctor` and the docs; the self-host deployment is the enforced option |

**Immediate hole (P0).** `import_local_file` (S:4511) does an unrestricted `std::fs::read`, so it can file `~/.config/wanderbooks/vault.key` (`ledger-vault/src/lib.rs:216-222`), and `get_receipt` then returns the key as base64. P0 changes this:
- An explicit `--import-root` allowlist (default: `$XDG_DOWNLOAD_DIR` only).
- Canonicalize, then refuse anything under the data dir, the books dir, the vault dir or the key directory, symlink escapes, and non-regular files.
- Test `import_local_file_refuses_key_and_journal`, plus a property over generated paths.
- No `requiresUserInteraction` is added (owner decision 2026-10-16). The interim TIN-bearing tools (`get_w9`, `set_tin`, `set_w9` with `tin`, `vendor_1099 include_tins`, `export_cpa_package` with TINs) keep today's behaviour (S:1198-1266, S:6111-6117).
- P2 deletes the path parameters and makes TIN-bearing outputs human-only. P7 deletes full TINs from the MCP entirely.

**Access log record:**

```
{seq, ts, prev, principal{id, kind: agent|human|viewer|cpa|system, label},
 client{client_id (verified), name, version (asserted)}, model? (asserted), request_id,
 action: tool|upload|download|replica.connect|human.tin_entered|human.tin_viewed|
         human.vault_unlocked|human.cpa_package|token.created|backup.run|keyslot.changed,
 subject, args_digest (sha256 of canonical args), result: ok|refused(code), head_after?}
```

Each record is fsynced, and a failed write fails the request. Readable through `books.access_log` (admin) and the viewer's Access page. The CPA package carries a summary. Its head is part of the head handoff. `replica.connect` occurs only for remote viewers (P9). The local viewer's direct reads, including its in-process TIN unlock, are not logged, because the GUI never writes.

**Security test suite** (`ledger-mcp/tests/security_*.rs`):
- auth on every route; wrong audience → 401; bad Origin → 403; bad Host → 400; body over limit → 413
- expired or replayed grant → 410; upload sha mismatch refused
- `scope_step_up_403`; `handles_bound_to_sub`; `tin_entry_requires_same_sub`
- `unwrap_only_in_human_handler` (module privacy plus grep gate; the GUI unlock path is the one other caller)
- `vault_pubkey_swap_detected_by_verify`
- `no_mcp_output_contains_full_tin`: canary TIN and key; session fuzz across responses, errors, server logs and access-log records
- `no_result_names_a_path`: the same session fuzz with the data root as the canary, across connect text, tool text, handbook, prompts, results and errors
- `access_log_one_record_per_call`, refusals included
- `isolation_label_is_advisory`: a user-level install reports `same_user (advisory)` in `books.start` and `doctor`
- `agent_uid_cannot_read_tenant`, run against the two-user self-host deployment in CI (the desktop default is advisory and is not claimed to pass it); `no_tool_touches_outside_tenant` (Landlock harness)

New fuzz targets: `fuzz_http_mcp_request`, `fuzz_upload_put`, `fuzz_settings_replay`, `fuzz_extract_pdf`, `fuzz_extract_xlsx`, `fuzz_eml`, and `fuzz_replica_stream` (P9). New mutation recipes and scheduled CI for `ledger-vault`, `ledger-cli`, `ledger-view`, and `ledger-mcp/http`; none of these have a recipe today.

**Retention:** 7 years. A local or self-hosted book keeps SSNs, sealed, on a machine the owner controls.

---

## 7. User flows

What you will see and do. The technical details are in sections 3 and 4.

### 7.1 Setting up on your computer

1. You run one install command. It installs the books programs (no developer tools needed), and then it:
   - sets up the books server to run in the background as you, starting by itself whenever an agent connects. It does not ask for an administrator password;
   - asks you to choose a passphrase. You need it to type or see someone's Social Security or tax ID number, to download anything that contains them (such as the CPA package with them), and whenever the books open a private page for you, such as the page where you drop a big file;
   - creates your company's books;
   - connects Claude Code to the books, and Codex too if you have it;
   - asks once whether the viewer should open when you log in.
2. You type `claude` in any folder and say "set up my books". The agent sees that the books are new and walks you through setup: company facts, accounts, members, opening balances, and a check that everything balances. Every answer is recorded in the books.

### 7.2 Day to day

- You say: "The receipt is in Downloads, inv-883.pdf, the Figma annual plan." The agent sends the file to the books, reads it, drafts the entry, and shows you exactly what it will post. You say yes, it posts, and the viewer jumps to the new entry. Posting brings up no extra permission pop-up.
- You say: "Save it as a draft, I'll find the receipt." The draft stays on the to-do list until the receipt arrives. You see it in the viewer's Needs attention, and the agent sees it at the start of every session.
- You say: "Show me Q3 against last year." The viewer opens that report.
- Backups happen on their own. Every change the agent makes comes back with a fingerprint of the books, so any tampering would show.

**Compared with today:** no helper commands, no project folder, it works from any folder, you answer the same number of questions, and installing needs no administrator password.

### 7.3 How private your books are

- The books live in a private folder that only your account can open, outside every project folder. Nothing the agent is told or shown names that folder, so an agent cannot wander in by accident.
- It is a curtain, not a lock. The server runs as you, so an agent that deliberately went looking could find the folder. The books say so whenever they report on privacy ("same user, advisory"), and `wanderbooks doctor` shows it too.
- Social Security and tax ID numbers stay sealed even then. Opening one needs your passphrase.
- For a real lock, run the books on another machine you own (7.4). The agent then has no way to reach the files at all.

### 7.4 Your own server, and claude.ai on the web

- You install the same programs on a home server, a network drive (NAS) or a rented server by following the recipe. Two commands move your books there, one on each machine, and the books' fingerprint must match before and after the move.
- One command gives that server a web address claude.ai can reach. Anyone can find the address, but only you can sign in. In claude.ai you go to Settings → Connectors → Add custom connector, paste the address and sign in.
- Then you chat as usual, for example "what needs attention?"
- Small receipt files go straight through the chat. For bigger ones, you drop them on a private upload page and say "process my inbox".
- Downloads come as one-time links.
- When someone's tax ID number is needed, the agent gives you a link to a private page and you type the number there. The chat only ever sees "last four 1234 recorded".
- The viewer on your computer can show the books on that server too. It streams a copy that it cannot change.

### 7.5 Codex

The installer offers to connect Codex, or you can run `wanderbooks connect codex --book <name>` later. After that you run `codex` from any folder. Codex gets the same rules and procedures as Claude Code.

### 7.6 Your CPA

1. On your computer, you send the CPA the package the agent prepares. With your own server (7.4), you can also invite the CPA to look at the books directly. The CPA signs in with their phone or computer's built-in sign-in (a passkey), with no password, and sees the reports, documents, journal and audit papers, all read-only.
2. The CPA package without tax ID numbers is an ordinary download.
3. The version with tax ID numbers comes only through a one-time link. You open it, unlock it with your passphrase, and send it on. Every download is recorded.
4. If the CPA wants, they can connect their own agent with read-only access.

### 7.7 What stays easy (acceptance)

- At most three commands from a new computer to the first posted entry, with no administrator password.
- Nothing to do at the start of a session.
- Everything works from any folder.
- The viewer catches up within a second.
- No new permission pop-ups in daily work.
- `wanderbooks doctor` shows on one screen: the server, your books, the connected apps, the viewer, the last backup, the books' fingerprint, and how private the books are.
- A test proves that an agent that knows nothing about the books can finish the everyday jobs just by following what the books tell it to do next.

---

## 8. Phased milestones

Every phase is one merge train. Each one:
- passes `just check` (fmt, clippy `-D warnings`, rustdoc `-D warnings`, all tests) and `just prove` (Kani with no vacuous covers; Lean with standard axioms only, no `sorry`);
- updates `docs/parity.md`, `docs/spec/rules.md` (SOURCE / CONVENTION / DECISION lines), `docs/mcp-interface.md`, `docs/testing.md`, `docs/verification.md` and `docs/architecture.md`;
- runs mutation testing on the crates it touches, with every survivor killed or documented as equivalent;
- builds features whole: core, store, MCP, viewer, exports, docs, tests.

The first company keeps working between phases.

### P0: Guard rails (days)

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

### P1: Journal-native settings and conventions (B, part 1)

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
- `dev-stdio` keeps `legacy_files` mode.
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
- `ledger-view` (`attention::find` moved out of the viewer) moves to P5, where its first user (`books.start`'s needs_attention, `next_steps`) lands; P1 has no server-side consumer, and moving 700 lines of the viewer for nothing would be risk without use. `attention_server_equals_gui` goes with it.
- The "due to" heuristic keeps reading account names: `financing_accounts` also holds bank loans, which aren't "paid personally".
- Approval rules are checked by the engine (`Ledger::approvals_missing`) and enforced by the store: `Store::post_entry` refuses a covered draft until approved, and the operations that post at once (invoices, bills, payments, payroll, remittances, disposals) are refused when a rule covers the entry they would post (`approval_gate`: draft it instead). Reversals, closes, depreciation and amortization are exempt. An approval records the draft's fingerprint (date, description, lines, classes); any change voids it. A rule names accounts that take postings, and an account a rule names can't be renumbered until the rule is revised.
- `books.import_settings` takes only the file the server reads, whole (compared key by key with the server's own reading), never turns approval rules on, and on books that keep their own settings changes nothing (review round 1: an agent-transcribed text could drop the EIN).
- `books.start` v1 leaves `book.id`, `book.url`, `head.last_handoff`, the `agent` input and `alwaysLoad` for P5 (they need the served book identity and tool search).
- Posting templates take slots (`{expense}`): an account each draft names, so "an expense a member paid: Dr <expense> / Cr 2100" is one template.
- A vault key file is never a books setting: `--vault-key-file` on the server and the viewer, and an import is refused unless the key in force stays the same.
- The connect text's lead names `books.start` first, then the handbook.

### P2: Serve locally, hidden away, and the the first company cutover

**Scope:**
- `ledger-mcp serve --root` with multiple books at `/b/{book}/mcp`; explicit transport config (3.2); bearer tokens and scopes; rate limits (tower-governor).
- **User-level service, no sudo.** `wanderbooks service install` writes a systemd `--user` socket and service unit on 127.0.0.1:7411 (Linux) or a LaunchAgent (macOS). It creates the data root (3.4) with an explicit `set_permissions(0o700)` and sets the owner passphrase. There is no system account, no container and no root step.
- **Honest isolation label.** `books.start`, Needs attention, `wanderbooks doctor` and the docs report `isolation: "same_user (advisory)"`. There is no `--same-user` flag and no acknowledgement gate.
- **Self-confinement.** Where available: on the server, Landlock (filesystem limited to the data root; on ABI ≥ 4, all TCP bind and connect denied, since the listening socket arrives through socket activation on 127.0.0.1) and seccomp; on the archive unit, read-only Landlock on the journal and blobs, TCP connect limited to the target's port (443 or 22) on ABI ≥ 4, and seccomp. A user-level unit cannot filter egress by host. If they cannot be applied, `doctor` reports it; it is not fatal. The docs describe them as protecting the human's account from a compromised server, not the books from the agent.
- Access log plus `books.access_log`.
- Owner passphrase for `/h/` (Argon2id).
- **Files transport:** `begin_upload`, the PUT route, the `/h/inbox` drop page plus `books.inbox` and the `process_inbox` prompt, `download_link`.
- **Every server-path parameter deleted:** `import_local_file`, `save_to`, `save_csv_to`, `w9_file_path`, get_w9 `save_to`. W-9 upload moves to `/h/w9/{party}` (sealed with the v1 key until P7). TIN-bearing outputs come only through `request_human_download` → `/h/`, behind the owner passphrase. This closes defect 19.
- `get_receipt view=link|bytes(≤256 KiB+sha)` (defect 3, transport part).
- **Viewer, direct and read-only (3.6):** data-root discovery through `ledger-store::paths::data_root()` or `--root` from `wanderbooks gui <book>`, never a project directory or `wanderbooks.toml`; `books.show` through `ops/show.jsonl` (delete `ledger-store/src/viewer.rs`); presence through the `ops/viewer.lock` shared flock; Save-a-copy and Open-with (defect 3, viewer part). Locally there is no replica, no socket and no peer-credential check.
- Archive unit (user-level) and module; `books.backup`; `ops/heads.jsonl`; `verify-archive`; `fetch`.
- **Restore (3.7, D-19):** `books restore`, proven before use, never destructive, onto a new or an existing install; restore drills; the vault key's backup reported until P7.
- **Viewer menu (D-19):** "Back up now" and "Export for CPA" (read-only, done by the viewer); "Restore…" and "New books…" handed to the server; nothing in the viewer writes a book.
- `ledger-mcp proxy`.
- CLI subcommands: `service`, `books create|adopt|list|export-legacy`, `connect`, `token`, `gui`, `doctor`, `verify-archive`, `fetch`, `update`. `books export-legacy <id>` (rollback only) writes a P1-format data dir outside the data root, verifies its head equals the server's, and stops the server's writes to that book.
- **Scaffold deleted:** `JUSTFILE`, `AGENTS_MD`, `CLAUDE_SETTINGS`, `CODEX_HOOKS`, `GITIGNORE` (`ledger-cli/src/main.rs:43-521`); `install.sh` rewritten, with no sudo.
- The test harness drives HTTP (`mcp_stdio.rs` keeps the proxy).
- **Two-user CI deployment:** a container that runs the server as a dedicated user and the agent as another, in the configuration P9 turns into the self-host recipe. It is the target of `agent_uid_cannot_read_tenant`.
- The first company cutover, preceded by the three cutover questions (10.3).

**Defects:** 19 (closed), 3 (partial).

**Tests:**
- `agent_uid_cannot_read_tenant` (the two-user CI deployment; not the desktop default).
- `service_install_needs_no_root` (install, adopt, connect and gui as an unprivileged user in a clean container).
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

### P3: Engine correctness (defects 1, 7, 21, 8, 9, 11, 12, 13, 14, 15, 16, 17, 18, 22)

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
- **The first company.** FY2026 must foot. It was −2,087.64 vs +612.36; the golden is recomputed by hand.

**Defect 7, IIF** (`ledger-import/src/iif.rs:169,243`): DOCNUM = entry_id. Output encoded in Windows-1252; characters outside it are transliterated (deunicode) and listed in `warnings`.

**Defect 21, CPA package IIF and as_of** (found in P0, 2026-10-16): the CPA package's `general-journal.iif` ignores `as_of` (an as-of package includes entries dated after it); its xlsx and csv files respect it. The IIF export takes the same `as_of`, and a test pins every file in an as-of package to it.

**Defect 22, depreciation through an asset's final month** (found in P0 review round 4, 2026-10-17): the depreciation schedule plugs to the full depreciable amount whenever the date it is computed through falls in the asset's final month, even mid-month. A disposed asset's schedule therefore shows full depreciation in its disposal month, and mid-final-month figures are overstated. Resolve from the standard (straight-line, partial months prorated by days, as the rest of the schedule already does), then adjudicate the changed figures in the P3 change log. Related, found in the same round and fixed in P0: the final-month true-up compared the month with chrono's `%Y`, so assets whose life ran past 9999 (or before year 0) never trued up. Assets whose life runs past 9999 also can't post months after 9999-12 (periods are bounded to years 1-9999); decide in P3 whether to refuse such lives at registration. Also noted: the viewer recomputes a very long-life asset's whole schedule every frame (slow; pre-existing).

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
- `built_in_gain()` (`assets.rs:204`) is redefined to match.

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

**Tests and accept:**
- A named regression test per defect, built from its 2026-10-16 reproduction (anonymized fixture).
- Goldens recomputed by hand.
- GnuCash differential re-adjudicated.
- 0 unexplained mutation survivors in `ledger-core` and `ledger-import`.
- The the first company change log is adjudicated.

### P4: Drafts, reversal as draft, the evidence gate, digest (defects 2, 5, 6)

**Scope:**
- **Core:** draft states; `entry.draft_state_set`, `draft_discarded`, `reversal_drafted`, `evidence_inherited`. `reverse_entry_steps` (`ledger.rs:1355-1380`) stops posting inline. `check_reversal` (`ledger.rs:1319`) also refuses a second open reversal draft. `ledger_core::evidence::requirement` is the single gate, used by `Store::post_entry` (every path), MCP and GUI.
- **Exemptions removed:** S:561-568, `receipt_policy.rs:6`, S:2540, GUI `books.rs:648`. Digest, settings and docs now agree.
- **Defect 5:** schema v12 columns; `list_drafts`; `discard_draft`; `set_draft_state`; `as_of_status` plus `strict` on reports; the CPA package refuses an open draft unless given a reason.
- **Defect 6, digest:** `since`/`since_seq` by `posted_seq`; `as_of`; zero-net touched accounts; `reconciliation_tracked`; `never_reconciled`; drafts by state.
- **Preview:** `preview_entry` plus `preview_token` on all confirm tools.
- **Needs attention:** "draft awaiting evidence" and "reversal draft needs evidence", distinct from "draft not posted".

**Proofs:**
- Lean `Step.draftReversal`, `Step.discard`, each with preservation.
- `Reachable.reversal_reverses_posted`: every posted reversal reverses a posted original, at most once.
- `draftsOpen` accounts for discard.
- `tests/model.rs` extended.

**Tests:**
- `reversal_needs_evidence`, `inherited_evidence_posts`, `legacy_reversed_replays_identically`, `discard_unblocks_close`, `report_warns_on_draft_before_as_of`, `cpa_package_refuses_open_draft`.
- `digest_since_is_posting_time` (an entry backdated but posted today appears).
- `stale_preview_refused`; `gui_and_server_gate_agree`.
- `fuzz_op_sequence` gains the new ops; store refusal law at `STORE_CASES=5000`.

**Accept:** legacy fixtures open; the first company has 0 unexplained drafts.

### P5: Agent surface complete (B, guidance)

**Scope:**
- Final connect text.
- `books.start` v2 (4.2).
- Full prompt set; resource templates.
- Result envelope (`next_steps`, `guards`, `warnings`, `head`) in `call_tool`.
- `books.journal_history`; `books.job`.
- **Correcting a record's source.** A convention's citation (or who decided it, or when) can't be corrected today unless its words change too: `set_convention` with only a new source is refused as "already says exactly that" (found at the the first company P1 cutover, 2026-10-17, correcting an unchecked citation). A source-only revision, journaled with its reason; the same for decisions and open items.
- Tool descriptions ≤ 700 characters; handbook complete; URL-mode elicitation where the client declares it.
- `docs/mcp-interface.md` rewrite (24-95, 580-648, 723-747, 909-931).
- **Agent-eval harness** `ledger-mcp/tests/agent_evals/`: scripted, with no LLM in CI. A zero-context transcript that follows only `books.start` and `next_steps` must complete onboarding, a receipt-backed post, a draft awaiting a receipt, a reversal draft with inherited evidence, a reconciliation and a month-end close.

**Tests:**
- Every mutating result has a head and a non-empty `next_steps`.
- Every guard's remedy tool exists.
- `instructions_budget` (first 512 characters contain `books.start`).
- `tool_text_lint`.
- Golden render per prompt.
- `books.start` ≤ 8 KB on the the first company fixture.

**Accept:**
- A fresh Claude Code session in `/tmp/empty` and a Codex session each follow journaled conventions to post an entry. (claude.ai needs the tunnel and OAuth, so its leg is in P9's acceptance.)

### P6: Evidence pipeline and scripts retired (defects 3, 4, 10, 20)

**Scope:**
- **Extraction.** Crate `ledger-extract`, run as a sandboxed subprocess:
  - PDF text layer (lopdf);
  - XLSX cells as exact decimal strings from the sheet XML (zip + quick-xml; never floats, fixing `verify-sheet.py:118-119`);
  - CSV and text;
  - EML via mail-parser;
  - page raster with hayro;
  - no OCR in core for now (D-15, decided): image-only receipts rely on page images plus attestation.
- `attachment.text_extracted` at attach time, plus a backfill job.
- `get_receipt` default meta + text, and `page_image`. Text and page resources. Defect 3 complete.
- **Dedup (defect 4):** identical bytes return the existing record and add a link; `find_attachment`; `consolidate_duplicates` (links move to the first record, duplicates withdrawn with "duplicate of #N").
- **Annotations (defect 10):** `annotate_attachment`; engine receipt numbers (next = max + 1); `reimbursed` derived (`entry.settles` when given, otherwise FIFO per party on the financing account, labelled `derived: fifo`).
- **Email.** `attach_email`:
  - an embedded PDF is preferred;
  - otherwise a deterministic Typst render of headers and body, with fixed PDF metadata and inline images kept;
  - `attachment.rendered` is journaled;
  - the `.eml` itself is never filed.
- **Company records (D-20).** Categories for notices and correspondence, plans, and history beside formation, legal and the rest; the viewer's Documents page grouped by category and subject; the books dir's `company/` folder retired (the first company: its files are filed, attachments 73–80, two more after the EIN fix). With extraction, the TIN scan reads PDFs' text too: the own-EIN exemption applies there as well.
- **Migration tools (defect 20):** `compare_reference` and `record_inventory`/`check_inventory` replace `scripts/verify-sheet.py` and `scripts/drive-manifest.py`. `eml2pdf.py` is retired.

**Tests:**
- `same_bytes_one_record` (property: N attaches, 1 record, N links).
- `get_receipt_default_has_no_bytes`, `page_image_is_png_of_page`, `inline_base64_requires_sha`.
- `extraction_deterministic`, `eml_render_deterministic`.
- `xlsx_cells_are_decimal_strings` (property).
- `extractor_sandboxed` (network and filesystem escape attempts fail).
- `compare_reference_detects_one_cent`.
- `fuzz_extract_pdf`, `fuzz_extract_xlsx`, `fuzz_eml`.

**Accept:**
- `get_receipt` default output is < 8 KB for every the first company attachment (the earlier audit reached 415K tokens).
- The first company 5.6 P6 checks: 0 differences, 0 inventory gaps.

### P7: Vault v2 and the human channel

**Scope:**
- **`ledger-vault` v2:**
  - HPKE sealing;
  - `keyring.json` with keyslots: passphrase (the owner passphrase from P2), recovery key, passkey PRF, member_passkey;
  - `rekey`;
  - W-9 files sealed.
- **Events:** `vault.created`, `vault.rotated`, `vault.keyslot_*`, `vault.resealed`. Verify checks the public-key sha.
- **Human pages:** `/h/tin/{party}`, `/h/w9/{party}`, `/h/unlock`, `/h/tin-view`, `/h/download` (CPA package and 1099 worksheet with TINs, W-9), keyslot management.
- **Tools:** `request_tin_entry`, `set_w9_facts`.
- **Removed from the MCP:** `set_tin`, `set_w9 tin|w9_file_path`, `include_tins`, all of them.
- **Scanner:** pattern-based TIN scanner on every result.
- **GUI:** in-process passphrase unlock (3.6): it reads the wrapped keyslot and the sealed record from the data root read-only, unwraps, copies to the clipboard only, and zeroizes. No socket.
- `wanderbooks vault init|rekey`.

**Tests:**
- `no_mcp_output_contains_full_tin` (session canary fuzz).
- `unwrap_only_in_human_handler`; `vault_pubkey_swap_detected_by_verify`; `reseal_preserves_last4_and_kind`; `cpa_package_with_tins_human_only`.
- Lost passphrase is recoverable with the recovery key.
- GUI `tin_copy`; `gui_unlock_zeroizes` (no plaintext TIN or private key left in the GUI's memory after the copy).
- Mutation testing on `ledger-vault`.

**Accept:**
- The the first company K-1 member worksheet and 1099 worksheet with TINs, produced through the human link, match the P0-era output byte for byte apart from timestamps.
- No vault key material remains on disk except the wrapped keyslots in the data root.

### P8: Audit, attestations, access-log tooling

**`books.audit`** (`ledger-core/src/audit.rs` plus the store's evidence view; deterministic; no LLM; run as a job):
- **Integrity:** chain, projection and blobs; access-log chain; vault public-key sha.
- **Statements:** TB balances; A = L + E; P&L = current earnings; cash flow `reconciles`; cash equals the last statement for each tracked account; reconciliation coverage.
- **Evidence:** the gate holds over every posted entry; `legacy_reversal_exempt` is informational; no drafts dated on or before `through`; duplicates.
- **Closes and schedules:** closes in order, with member deltas to the cent; schedules and depreciation posted through.
- **Assets:** register = cost accounts; tags and basis present.
- **Tax and parties:** tax categories; 1099 payees with W-9 or TIN and their kind; party policy.
- **Settings and freshness:** settings problems; 2100 aging; backup freshness.
- **Attestations:** coverage against the bar.

Output: a workpaper (JSON plus Typst PDF) filed as a `workpaper` document, and `audit.run` journaled. It is idempotent on `(through, through_seq, program_version)`.

**Attestations:** `attest_receipt`, `review_entry` and `attest_period`.
- `verified` (principal, client_id) is kept separate from `asserted` (model, effort, adversarial, agents).
- The engine cross-checks extracted amount and date against the entry; a mismatch never counts.
- A period attestation requires an `audit.run` at the current head.
- `counts` is derived against `policy.attestation_bar` in force at that seq, so a bar change applies only going forward.

**Needs attention items:** "month-end attestation due" after each close; "full audit due before CPA handoff" (`export_cpa_package` refuses without a counted full audit unless given a reason, per `policy.cpa_package_requires_audit = refuse`; D-9, decided); "annual audit due". Each states the bar.

**Tests:**
- `audit_is_deterministic` (same head gives the same sha).
- `audit_detects_each_seeded_defect` (one fixture per check).
- `below_bar_not_counted`, `bar_change_is_prospective`, `amount_mismatch_never_counts`, `stale_head_period_attestation_refused`.
- Lean: counting lemma (counted ⇒ meets the bar at seq) as a decidable predicate.

**Accept:** an injected fixture with an unfooted cash flow, a missing receipt, an open draft and a duplicate blob produces exactly those findings.

### P9: Self-host on another machine, and remote access

This is the documented path for anyone who wants mechanical enforcement, and it is how claude.ai on the web reaches the books. Multi-tenant hosting is not built here (section 11).

**Scope:**
- **Self-host recipe:** the same binaries on a machine the owner controls (home server, NAS, VPS), under system units and a dedicated account. `wanderbooks-server.service` with `IPAddressDeny=any` (loopback allowed; when the built-in authorization server fetches CIMD documents, D-7 a, connect to port 443 only, with private and link-local ranges denied), seccomp `@system-service` and Landlock; the archive unit with `IPAddressAllow` limited to its configured targets; an encrypted volume; Caddy for TLS. A container image is an alternative on that machine. `books.start` reports `isolation: "service_user (enforced)"`.
- **Moving books there:** `wanderbooks fetch` from the desktop server, then `books adopt` on the self-host machine, with the head check. Adopting an archive clone works the same way.
- **Remote access:** `wanderbooks tunnel --book <book>` (a Tailscale Funnel or cloudflared recipe). The static bearer over TLS for header-capable clients (Claude Code, Codex). OAuth 2.1 (3.3) for claude.ai and ChatGPT, built in (D-7 a) or delegated (D-7 b). OAuth is forced on whenever the tunnel is public. The Origin allowlist gains `https://claude.ai`.
- **Remote viewer:** replica routes, `books:view`, `Snapshot::from_lines`, GUI `--books <url>`, and `/replica/events` for `books.show` and presence (3.6). The replica carries a minimum-version header and no keyslot material.
- **Remote people:** `books.invite {kind: cpa|member|viewer, email}` sends a passkey enrolment link; the `cpa` role is read, view and archive. `/h/` pages are served over TLS behind a passkey or the passphrase.
- **File inputs:** MCP App upload widget (an app-only tool); `openai/fileParams`.

**Tests:**
- `agent_uid_cannot_read_tenant`, rerun against the recipe's deployment.
- `self_host_adopt_same_head`.
- OAuth: PRM and 401, audience, PKCE, exact redirect, `iss`, CIMD SSRF guard, step-up; an external Origin, Host and CSRF scan.
- `replica_snapshot_equals_store_snapshot`; `replica_carries_no_keyslot`; `fuzz_replica_stream`.
- An external security review of the built-in authorization server before the remote path is opened publicly (not needed under D-7 b).

**Accept:**
- claude.ai custom connector via CIMD → `books.start` → upload through the drop page → post → visible in a replica GUI on another machine.
- `codex mcp login` works, and the static token works over TLS.
- The audit and the CPA package each finish in < 60 s on the first company, or return a job handle.

### P10: Acceptance, the full the first company re-audit through the MCP alone

Run from clients that hold only a URL and a credential: Claude Code started in `/tmp/empty`, Codex, and claude.ai through the tunnel if a self-host machine (T2) is running by then; otherwise the re-audit runs from Claude Code and Codex only, and the claude.ai leg is recorded as not applicable. The agents use the tools alone. On the desktop (T1) this separation is advisory, because the agents run as the same user as the server: the evidence is the access log plus session transcripts that never read the data root or name its path. If the owner runs a self-host machine (T2) by then, the re-audit runs against it, and the separation is enforced.

1. `books.audit scope=full through=<month-end>` and `scope=cpa_handoff`. Every finding is fixed through journaled corrections: reversal drafts with evidence, annotations, decisions. Legacy reversals posted without their own evidence inherit the original's with `inherit_evidence` (D-12, decided).
2. Every receipt is attested from `get_receipt` text plus page image by a pair at the bar: high reasoning effort, adversarial, at least 2 independent verifier agents (D-9, decided). No base64 enters context.
3. `review_entry` for every posted entry; `attest_period` monthly since formation and `cpa_handoff` annually.
4. Each of the 22 defect reproductions (20 from the 2026-10-16 audit, and defects 21 and 22 from P0) is re-run, and each shows fixed.
5. `compare_reference` against the Sheet as of 2026-10-11 shows 0 differences. `check_inventory` against the Drive listing shows 0 gaps.
6. CPA package: no TINs through the agent; with TINs through the human link.
7. The head is handed to the owner and verified against a fresh `fetch` with `verify-archive --expect-head`.

**Exit criteria:**
- Zero failing workpaper checks; cash flow difference 0.00.
- 100% of receipts and posted entries carry a counted attestation, and 0 disagree.
- The access log shows every action under the right principal. No transcript reads the data root or names its path; the only other reader is the read-only viewer.
- No result carried a full TIN or more than 256 KiB of inline bytes.
- Agent context on receipts never exceeded 100K tokens.
- The owner's sign-off is journaled as `decision.recorded`.

### P11: One app for Windows, Mac and Linux (D-19)

**Scope:**
- One installer per system: the server, the viewer and the CLI, versioned and updated together (the format gate of 5.1 then never meets mismatched parts). The server stays a separate background program: a LaunchAgent on macOS, a systemd `--user` unit on Linux (both from P2), and the per-user equivalent on Windows (a logon task, no admin rights). No step needs admin rights on any system.
- Opening the app opens the viewer and starts the server if it isn't running.
- Auto-update per system: download, verify the signature, replace all parts together, restart the server, reopen the viewer. A failed update rolls back to the previous version.
- Data root in each system's usual app-data folder (Application Support, AppData, `~/.local/share`), as in 3.4.
- Signing: Apple Developer ID signing and notarization (the company's membership); a Windows code-signing certificate; signed Linux packages.
- Release builds and the full test suite on all three systems in CI.
- The viewer menu of P2 (back up, export, restore, new books) is the app's whole surface beyond the viewer; there is no separate launcher.

**Tests:** `install_needs_no_admin` (each system, clean machine); `update_replaces_all_parts_together`; `failed_update_rolls_back`; `unsigned_update_refused`; `server_runs_with_viewer_closed`; P2's restore tests on all three systems.

**Accept:**
- On a clean Windows, Mac and Linux machine: download, install, open, onboard and post a first entry with no terminal and no admin password.
- An update leaves every book's head and goldens unchanged.
- A backup made on one system restores on each of the others.

---

## 9. Risks and mitigations

1. **Journal forward compatibility.** Old GUIs or CLIs cannot read v2 events.
   - The P0 `NewerJournal` gate ships first. The local GUI reads the journal directly, so it goes through the same gate.
   - `wanderbooks update` upgrades every binary together.
   - The replica (P9) carries a minimum version.
   - Fixtures from every phase are kept.
   - Downgrade is impossible after the first v2 event, so back up first (P1 says so).
2. **The settings migration shifts numbers.** Byte-identical goldens gate P1. `company.members` effective dating must reproduce the `close_to` history exactly.
3. **P3 fixes change the first company's reports and tax output.** Every difference is hand-adjudicated in the change log, and the 1099 and 704(c) changes are shown to the CPA. Posted entries never change. The cumulative close rounding counts from inception (D-10), so the first new close may carry a one- or two-cent true-up; it is listed in the change log.
4. **The P2 cutover is large and could disrupt daily use.**
   - The scripted `adopt` checks the head hash.
   - If the owner agrees (Q-3), the old copies move into the data root's `legacy/` area. Nothing is deleted.
   - Rollback is documented.
   - The final runs of `verify-sheet` and `drive-manifest` are filed before cutover; Drive deletion waits for P6.
5. **The P2–P7 interim:** TINs still cross the MCP on `set_tin`/`set_w9`, with no added prompt (owner decision: no `requiresUserInteraction`).
   - Mitigation: P7 is scheduled before mid-January (1099s are due Jan 31, K-1s by Mar 15). TIN-bearing outputs are human-only from P2.
6. **Desktop isolation is advisory.** A process running as the human, including an agent that deliberately hunts, can read the data root, the agent token and the wrapped keyslots.
   - TIN plaintext still needs the passphrase.
   - The path is never named, the root sits outside every project directory, and every place that reports isolation says "advisory".
   - The server's own Landlock and seccomp are never presented as protecting the books from the agent.
   - Enforcement is the self-host deployment (P9), tested in CI from P2.
7. **User-level services stop at logout.** A systemd `--user` service stops at logout unless lingering is on, and a LaunchAgent runs per login session.
   - The server is socket-activated, so it starts on the first connection, and agents and the viewer only run while the human is logged in anyway.
   - The backup timer is persistent, so a run missed while logged out happens at the next login.
   - `doctor` offers lingering but never requires it.
8. **The OAuth authorization server is security-critical code written in-house** (rmcp has no server-side auth). It is needed only on the self-host remote path.
   - Small surface: passkeys only, no password grant.
   - Built on audited crates (webauthn-rs, oauth2 types); fuzzed; externally reviewed before the remote path is opened publicly.
   - Alternatively an external IdP (D-7).
9. **Client variance.**
   - claude.ai: no elicitation, about 150K-character results, 240 s per call.
   - Cursor: legacy protocol versions only.
   - Codex: a 512-character budget.
   - The `resource_link` content type is undocumented, and the files extension is experimental.
   - Mitigations: links repeated in text, both protocol eras served, jobs for long work, the drop page, and an adapter for `io.modelcontextprotocol/files` once it stabilizes.
10. **Extraction quality.** There is no OCR in core (D-15), so image receipts rely on page images plus attestation. The extractor version is journaled, and re-extraction appends events and never rewrites.
11. **Attestation `method` is self-reported.** The bar is a policy on claims by an authenticated client. Verified and asserted fields are separated, and the deterministic cross-checks are the hard floor.
12. **Performance.** One mutex per book serializes writers. Mitigations: reader snapshots, `spawn_blocking`, jobs, SSE backpressure, the blob LRU.
13. **`server.rs` size (8,529 lines).** It is split per tool group as each phase touches it, to keep review cost bounded.
14. **Tool surface churn** (`preview_token`, removed parameters). The `confirmed` alias and the removed-tool errors that name their replacement last one release, and the agent-eval harness catches guidance regressions.
15. **Proof obligations** (cash-flow footing, cumulative drift, reversal-draft transitions) may take time. A failing proof is a finding and blocks its phase by design.

Hosted-service risks (operator-served unwrap code, the legal exposure of hosting SSNs) are in section 11.

---

## 10. Decisions

### 10.1 Settled

The owner decisions of 2026-10-16 (at the top of this document) replace the former questions D-1 (local isolation default), D-6 (hosting timeline), D-8 (posting friction) and D-14 (viewer on the desktop). The engineer settled the following from the standards:

| # | Question | Decision |
|---|---|---|
| D-5 | Company EIN | (a) A journaled `company.ein` with sensitivity `identity`: printed on every 1099, masked in `books.start`, full in packages. |
| D-9 | Attestation bar default | (a) Receipts and month-end: reasoning effort ≥ high. Annual and CPA handoff: adversarial verification with ≥ 2 independent verifier agents. No model allow-list. The CPA package refuses without a counted full audit unless given a reason (`policy.cpa_package_requires_audit = refuse`). |
| D-10 | Odd-cent cumulative apportionment start | (a) From inception, so the next close carries any historical true-up of a cent or two; lifetime drift stays under one cent. |
| D-11 | Default proration for new prepaid schedules | (a) Daily, matching depreciation; existing schedules stay `monthly_even`. |
| D-12 | Legacy reversals already posted without their own evidence | (a) During the re-audit, link the original's receipts with `inherit_evidence`, so the audit is fully clean. |
| D-13 | The first company timezone | (a) `company.timezone = America/Denver`. |
| D-15 | OCR for image-only receipts | (a) None in core for now: page images plus agent attestation. |
| D-16 | CPA access to TINs | (a) To start, the owner prepares and sends a single-use human link each time. |
| D-18 | `dev-stdio` | (a) Kept permanently for tests and demo. It refuses inside the data root and is never offered by `connect`. |
| D-19 | One app for Windows, Mac and Linux (owner, 2026-10-17) | Planned: backup and restore in P2, the app itself in P11. One installer and auto-updater per system. The server, the viewer and the CLI ship and update together but stay separate programs inside the app: the agent needs the server while the viewer is closed, and the viewer stays read-only by being a separate program. No separate launcher, which would be a third program and a second writer. The viewer is the app's face and gains menu items: "Back up now" and "Export for CPA" (read-only; the viewer does them), "Restore…" and "New books…" (they change books, so the viewer hands them to the server), and the CPA package with TINs through the human path (P2 `/h/`, P7 vault). The books live in each system's usual app-data folder (the data root of 3.4): hidden, not locked; a little more protected on macOS. **Backups must restore**, into an existing install or a new one, proven before use and never destructive (3.7). |
| D-20 | Company records (owner, 2026-10-17: "It's just too damn convenient to keep it all together in one place.") | Every company document goes into the books, not only what the CPA needs: formation papers, registrations, plans, notices, drafts, history. Done for the first company 2026-10-17 with the existing categories; the filing scan passes the company's own EIN printed 2-7 (rules.md B-26). P6 adds what makes it pleasant (below). |

### 10.2 Deferred

- **D-7. OAuth for remote access.** This now applies only to the self-host remote path (P9). The options: (a) build a minimal authorization server into wanderbooks (passkeys, CIMD/DCR), self-contained and portable for self-hosters, which is recommended; or (b) delegate to an external IdP (Authentik, Ory, Keycloak) and only validate tokens. Settled when P9 starts.
- **D-17. Remote file intake beyond the drop page** (an email-in address) is deferred with the hosted service (section 11). P9 ships the MCP App upload widget only.

### 10.3 To ask the owner at the P2 cutover

These are the only open owner questions. Ask them just before the move, one at a time, in plain English.

**Q-1. Where should the automatic backups go?** The books back themselves up after every change. Where should that copy live: in a private git repository you name (for example on your own git server), or on another disk? Either way, the backup includes the whole history and the sealed tax ID numbers, and it is locked with a key only you hold.

**Q-2. Should the papers in the `company/` folder be filed into the books?** These are the formation papers, notices and company history. Once they are in the books, every agent and the viewer can find them, and if you later run the books on your own server, claude.ai and your CPA can reach them too. If they stay as files, they live only on this computer. Recommended: file them.

**Q-3. After the move, should the old copies of the books leave the project folder?** The project folder still holds the old journal and receipts (`books-data/`), the old sealed tax ID numbers (`vault/`), and the folder's git history, which contains the journal and receipts too. **Recommended: move them into the private folder's legacy area.** That is what "hidden away" means; otherwise an agent working in the project folder can still read the old copies. Nothing is deleted.

*For the engineer:* Q-1 was D-3, Q-2 was D-4 and Q-3 was D-2. Q-1's technical side is settled: `archive.git` is seeded from the existing git history, filtered to the journal and blob paths, includes the vault ciphertext, and is age-encrypted as a whole on push (3.4, 5.5).

---

## 11. Later: hosted service (not built in this plan)

A paid, multi-tenant service in which an operator runs the books for customers. P0–P11 build none of this. It is recorded here so the design stays compatible with it.

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
