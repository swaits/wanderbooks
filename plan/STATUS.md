# STATUS — 2026-10-22: "Served" P0 to P3 live, with Projects

## Now: Served (plan/served.md), P0 to P3 live; P4 next

The 2026-10-16 audit of the first company's books (62 verified findings,
20 engine defects) and the design discussion that followed produced
`plan/served.md`: the books become a server that owns its data, agents
reach them only through the MCP from anywhere, settings and conventions
move into the journal, no scaffold. Phases P0-P13; the owner's decisions
of 2026-10-16 are at the top of the plan and bind every phase. P0 turned
up defects 21 (the CPA package's IIF ignores `as_of`) and 22
(depreciation's final month plugs mid-month); both are for P3.

The owner's bar (2026-10-17): this is bookkeeping software and it has to
be perfect. Full verification every phase (review rounds until clean,
`just check`, `just prove`, goldens); economize only on waste. Mutation
testing waits for P12 from P2 slice 3 on (D-33, owner 2026-10-20: focus
on building; P12 reruns the whole max-testing pass, on the owner's desktop
or CI).

**P0 (guard rails): done.** On `main` at `391634a`, pushed (origin +
GitHub): pins, transports doc, tools-only template; the store's format
gate (newer is not corrupt; the envelope and its hash frozen across
formats; no repair of newer books; command vs replay validators, B-28);
the short connect text and `books.handbook`; tool annotations; the import
roots' lockdown (Downloads only by default); fail closed; no file with a
full TIN filed into the books. Four review rounds, the last clean.
Cutover on the first company's books, 2026-10-17: `books.verify` ok
(366 events, head unchanged), and the CPA package as of 2026-10-11 and
2026-10-17 canonically byte-identical before and after (record in that
books dir, `goldens/p0-cutover-2026-10-17/`). Connect text with their
settings: 1,396 characters.

**P1 (journal-native settings and conventions): landed.** Settings,
conventions (posting templates, approval rules, policies, close steps,
calendar, mappings, notes), decisions, open items and approvals are
journal events (format 2); books that keep their settings never read
`wanderbooks.toml` or `config.json`, and adoption moves a file in
exactly once (`books.import_settings` did too, until D-44). Two review rounds (all findings fixed), `just
check`, `just prove` (Kani 20/20, Lean 31 theorems, standard axioms
only), and mutation on the whole diff (956 mutants; every survivor
killed by a test but two equivalent: `docs/testing.md`). **Live on the
first company's books 2026-10-17:** settings imported (20; the time zone
set after), its conventions, decisions 1-13 and open items 1-7 recorded,
and every company document filed (D-20). The cutover found two defects,
both fixed in `94338b7`: the K-1 worksheet printed a share read from the
journal at the scale it was written ("0.5", not "0.50"), and the file
guard refused documents printing the company's own EIN. On the rebuilt
server `books.verify` ok (434 events) and the CPA package as of
2026-10-11 and 2026-10-17 canonically byte-identical to before, and to
the P0 goldens (record in that books dir, `goldens/p1-cutover-2026-10-17/`).

**Before-snapshots for later phases:** the first company's reports as of
2026-10-11 and 2026-10-16 (journal head `d1ccd2fd…`, 355 events) are
saved in that books dir under `goldens/p0-2026-10-16/` (exported through
the MCP, no TINs), privately, by decision, never in this public repo
(plan 5.6). P1+ must reproduce them except where the P3 change log
adjudicates a difference (cash flow, defect 1).

**P2 (one executable, open while held, the cutover): built, slices 1-8.**
Slices 1-2 on `main` earlier. Slices 3-8 (2026-10-19 to 2026-10-20),
each reviewed until a round came back clean: 3, opening and closing the
books (the server on local sockets, the host and its leases, the agents'
bridge; no background service, D-21/D-23); 4, access (tokens in the
keyring, the door, scopes, the access log, the owner passphrase); 5, files
in and out through the server (uploads, exports); 6, the viewer on the data
root and one CPA-package builder (`ledger-package`); 7, backups and restore
(D-30, D-31: plain git to a remote and a folder, drills,
`wanderbooks books restore`); 8, the viewer's Books menu, `connect`,
`books back-up`, no confinement (D-32), the scaffold deleted. Then the
tests mutation had found before the pause (D-33), with three real bugs
fixed: the admin answer's four-MiB line, and the CPA package's file
naming (a twin's number in another folder; a name of dots climbing out of
its folder). `docs/testing.md` lists what P12 must cover.

**P2 live on the first company's books, 2026-10-20 (slice 9).** Asked one
at a time: the old copies move aside (Q-3), backups go to a new private
repository on the owner's git server and to his Syncthing folder (Q-1,
D-30). Adopted as "fernhill" at event 466, 66 stored files and the TIN
vault checked; `books.verify` ok on the new server with the same head; the
CPA package as of 2026-10-11, 2026-10-17 and 2026-10-20 canonically
byte-identical before and after (record in that books dir,
`goldens/p2-cutover-2026-10-20/`); both backups whole by a restore
preview. The cutover added `wanderbooks books put-aside` (three review
rounds of data safety), which moved the old copies whole into the private
folder's `legacy/`. Agents connect with `wanderbooks connect`, the viewer is
`wanderbooks gui fernhill`, and the books dir is a books home holding no
books data. The before-check found defect 21 again (the IIF ignores
`as_of`).

**P3 (engine correctness): live since 2026-10-22.** Every defect group of plan P3 is on `main` (`2c5b5a7`), each
built from its brief, reviewed until a round came back clean, and with
its regression tests written as bookspec examples (51 in
`bookspec/examples/`, every one checked by `cargo test`): the odd cent
(defect 17, D-10, D-38), the IIF in plain ASCII and the CPA package as of
one day (defects 7, 21, D-36), the cash flow statement by ASC 230's
direct method (defect 1, D-35, D-42), rename, one set of period defaults
and tax categories that warn or refuse (defects 15, 16, 8), party totals
by what moved the money (defects 14, 11, 24), dated 1099 decisions,
payment methods and the card exception (defects 23, 12, 18, D-40),
depreciation straight-line by days through the final month and daily
schedules (defects 22, 9, D-37), the format upgrade only with the owner's
go-ahead (D-39), and assets' posted and scheduled depreciation, their
bases and the K-1 on the tax basis (defect 13, D-41); then three review
commits: the seams between the pieces (one fictional year checked
across engine, viewer, tools and package; three defects found and
fixed), asset basis (schema 14, shipped migrations pinned by hash; the
tax K-1's net income always open), and data safety (three new journal
features, replay with P2's TIN detector, the P2 served-books fixture). Mutation waits for
P12 (D-33): each piece's targets are listed in `docs/testing.md`.

**The P3 surface review (2026-10-21)** checked the agent's surface, the
docs and the whole proof run on `main` with all of P3: `just check`
clean; the tools' docs (`docs/mcp-interface.md`) corrected tool by tool
against the code (about 60 fixes: wrong parameter names, missing outputs
and refusals, the idempotent list, the handbook's section list,
`books.start`'s next steps, the CPA package's files) and a new test that
every tool is documented and every documented tool exists
(`every_tool_is_documented_and_every_documented_tool_exists`; 115 tools,
`books.access_log` was undocumented); B-rule cross-references fixed in
code and tests (eight wrong); bookspec's statuses made consistent (an
example whose figures follow an owner decision is CHOICE and cites it:
thirteen statuses changed, five more examples given the decision they
rest on), its README now saying exactly what the loader takes,
and the loader refusing a closed-list value spelled any other way. **`just
prove` could not finish in its 30-minute cap at four jobs** (a CI
runner's cores) on a heavily loaded laptop: 52 of 53 harnesses in, the
reversal mirror at three lines still running. It is split, nothing
dropped: that harness is proved in two halves (`kernel::entry_flows` is
now `entry_movements` then `flows_of`) plus end to end at two lines, and
`just prove` runs 47 harnesses and Lean (308 s at four jobs on a quiet
machine; 83 headline theorems, standard axioms only) while `just
prove-slow` runs the eight slowest, each under its own cap (467 s), and
CI runs both (`docs/verification.md`, "Running it"). The laws are numbered
1 to 31 with no gaps, every harness listed with its runtime. The viewer's
P3 pages were rendered on the demo books and looked at
(`~/.cache/wanderbooks/shots-p3-surface/`): the figures tie across
pages (the register, the asset, the balance sheet, the tax summary, the
1099 worksheet and the party agree to the cent); two faults fixed: a
report's notes were cut to one line (the fixed asset register's lost its
explanation of the tax columns; they wrap now), and the Accounts page
listed accounts in the order they were added, not by code.

**The P3 cutover (2026-10-22).** The first company's books were reinstalled
from `main` (`3eda95e`: P3, its four reviews, Projects with its snapshot,
and the keyring fix). Before the owner's go-ahead, every report was compared
with the P2 goldens at four dates: the statements, ledger, journal,
members' equity, K-1 worksheet and tax summary identical; the cash flow,
asset register, IIF, 1099 worksheet, README and the new tax-basis K-1 file
different, each difference adjudicated twice in the private change log
with every changed figure recomputed, none unexplained. The owner ran
`wanderbooks books upgrade` (a verified backup first); after it the
packages were byte-identical to before it, `books.verify` ok, both backups
whole by a restore preview. Then the P3 facts were recorded: the devices'
values and §723 bases with the traditional method (D-41), every vendor's
1099 decision (D-40), the payment methods with every member- and
card-paid entry tagged, the creditors of 2050 and 2100, and
`policy.require_tax_category = refuse`.

**Projects (2026-10-22, live with P3).** A project is a class with facts
(B-37), four tools, and a project page in the viewer opening on a
snapshot: revenue, direct costs, contribution before overhead and its
margin, in the black or red, the on-spec cost unrecovered, by month with
the net to date, and costs by category. The sidebar measures its footer.

**The keyring (2026-10-22).** One keyring item per token, read back when
made; the bridge says a locked keyring instead of "make a token"; `token
check` tells a kept token live, revoked, missing or duplicated.

**Invoices and bills take a project (2026-10-22).** Each line of
`books.create_invoice` and `books.create_bill` takes an optional `class`
(a project, by id or name): the entry's revenue or expense side is one
line per project, the untagged lines' as one, the receivable or payable
line and the payments never tagged (rules.md B-37 item 8), so a client
project's invoiced revenue and a contractor's billed cost count in its
figures, the P&L by class, and its snapshot; the P&L, the 1099 worksheet
and party totals don't move. Journaled as the line's `class_id` behind a
new feature, `document_line_projects` (not `projects`, which the live
books already declared); a document naming none is journaled byte for
byte as before. Late-tag notes as `books.add_line`'s; `books.get_entry`
shows a line's project; the viewer's invoice and bill pages list the
document's lines, each project a link; the demo's "Client work" carries
its invoices. The handbook's caveat is gone for books with the feature.

**One way to serve books (2026-10-23, D-44).** The owner: "We should
probably just delete the stdio path altogether. THere's no point in that.
And it sitting there unused will just lead to bitrot. Nuke it?" Gone:
`wanderbooks dev-stdio`, the `ledger-mcp` binary and `ledger_mcp::run`
(stdio and `--http` over a bare data dir), with what only it had: files
the books make coming back inline (every handed file is a download now),
books with no access log or backups, the settings file read and reloaded
(`legacy_files`), the vault key named by a file or `--vault-key-file`.
`BooksServer` is built one way, from a book as the books server serves it
(`Served`); an unknown tool is a protocol error. Every suite that drove
the stdio server moved to `crates/ledger-cli/tests/` and drives a real
`wanderbooks server` on a data root of its own (`tests/common`), files by
their download. Inline file input was unchanged then (D-46 removed it,
below). The owner then decided
(2026-10-23, the question tool): `books.import_settings` goes (118
tools; old books come in whole by `books adopt`; `books.chart_template`
gives its settings as `books.set_setting` changes); the viewer's
loose-books-folder mode goes (`--data-dir`, `--books-toml`,
`--vault-key-file`, the settings file followed): it opens only books of
the data root; and `wanderbooks dev-stdio` goes with no stub, an unknown
command like any other.

**Files only by handle (2026-10-23, D-46).** The owner, via the question
tool: "Uploads only"; and before it: "What we would DEFINITELY want to
avoid is passing a bunch of BLOBs through AI context windows. That's just
a pure waste." No file passes through an agent's context, either way. In:
`content_base64` and its `sha256` (`books.attach_receipt`,
`books.file_document`), `w9_content_base64` and `w9_sha256`
(`books.set_w9`) and `csv_text` (the two CSV imports) are gone with the
256 KiB inline limit; files come only by `books.begin_upload` and
`wanderbooks upload`; those tools take no other argument, and an inline
argument to any tool is refused before it runs as unknown, pointing to
`books.begin_upload`. Out: `books.export` (every format),
`books.asset_labels` and `books.get_receipt` view `bytes` hand the file
over as a download (`HandedFile`), as the CPA package and the W-9 did; a
token keeps up to 64 handed files (was 16). The tests file by upload
(`tests/common`: `Wire::upload`, `upload_on`) and read every file by its
download (`docs/testing.md`, "D-46").

**Still open after P3 (each for the phase named, or the lead):**
- Invoices and bills take a project now (above), behind the new journal
  feature `document_line_projects`: the first company's books declared
  `projects` but not it, so until the owner's next format upgrade
  (`wanderbooks books upgrade`, D-39) their invoices and bills still take
  none (the handbook says so).
- Raised by the cutover review (private change log, "Open points"):
  its three points are done. The third (2026-10-22, B-36 with D-38): the
  tax-basis K-1's odd cents of property basis no longer all go to the
  first member; they run cumulatively, D-38's largest remainder, one
  count per set of members with a part in an entry, in date order
  whatever the span, a reversal taking its original back out
  (`tax_capital::RunningSplit`, `entry_parts`; tests
  `tied_odd_cents_of_property_basis_run_cumulatively`,
  `a_member_outside_an_entry_never_takes_its_cents`,
  `an_entrys_odd_cent_is_the_same_in_every_span`,
  `the_split_runs_in_date_order`, the law
  `a_running_split_never_drifts_a_cent`; bookspec
  `partnership-k1-tax-basis-odd-cents`). Left as it was, and said in
  B-36: the book property each member contributed in an entry is still
  split entry by entry, ties to the first member (each member's property
  must stay within their own credit in the entry); its odd cents arise
  only in an entry part cash and part property whose credits don't
  divide the property evenly. Whether the first company's three-cent
  gap between its members came wholly from the basis split is to be
  checked on its books when the K-1 is next run, at the year-end close.
  Its second point is done (2026-10-22, B-14 rule 5): a payment to a
  government (ASC 230-10-45-17(c), "Cash payments to governments for
  taxes, duties, fines, and other fees or penalties") is other operating
  cash payments, (g), never cash paid to suppliers, decided by its payee
  (a party with the government or tax agency role) on any account, as
  well as by a `taxes_licenses` account as before; a payment shared with
  a supplier is split by the parties' shares (45-22); payroll
  remittances from the payroll payables stay (d) as before; a government
  selling goods or services is a supplier (add it as a vendor). The
  first company's state filing fee, booked to its organization-costs
  account, moves from "Cash paid to suppliers and employees" to "Other
  operating cash payments" at every date (no total changes), provided
  (1) its entry names the state's agency with the `government_agency`
  role (check with `books.party`, give the role with
  `books.set_party_roles` if not), and (2) the fee's own entry is the
  one whose cash the statement lists: paid from the bank; charged to a
  card that is a lender's credit (the `financing_accounts` setting, D-42,
  borrowed and spent); or paid by a member personally through the
  members' account (Due to Shareholders) while that account is in
  `financing_accounts` (B-14 rule 11: borrowed from the member and spent
  on the fee's date, the operating part classed (g) with its
  `on_behalf`, and split the same way when shared:
  `a_shared_government_payment_a_member_paid_splits_both_ways`). The
  first company's fee was paid the last way (the books home's goldens
  change log, entry #1), so check its members' account is a financing
  account. Charged to a card, or paid through a members' account, that
  is an operating payable, its cash leaves in the later payment (to the
  bank, or the member's reimbursement), whose payee is not the state,
  and it stays with suppliers: check how it was paid (`books.get_entry`).
  Expected golden change: once this ships, the cash-flow snapshots in the
  books home's `goldens/` (each cutover's) differ from a fresh
  run on two operating lines, "Cash paid to suppliers and employees"
  smaller and "Other operating cash payments" larger by the fee, every
  section total, net change and cash balance the same. That is this
  rule, not a regression: diff the class lines against it at the next
  cutover and refresh the snapshot then.
  Its first point is done (2026-10-22, B-34 rule 8): the IIF writes each
  line's project in CLASS, the projects declared first in a `!CLASS`
  list, their names held to QuickBooks' 31 characters, no two alike, and
  listed in the README when written differently (tests
  `iif_writes_each_lines_class_and_declares_the_classes_first`,
  `iif_class_names_fit_quickbooks_and_every_change_is_listed`,
  `iif_class_names_written_as_the_books_have_them_are_kept`,
  `iif_refuses_an_undeclared_or_unfit_class`, the property
  `iif_class_names_never_clash`, `the_package_iif_carries_each_lines_project`;
  bookspec `iif-quickbooks-journal-file`). The first company's
  package changes at its next build: its project-tagged lines carry
  the class, and the class list is new.
- The viewer recomputes a long-life asset's whole schedule every frame
  (`ledger-gui` `pages/assets.rs` `schedule`, called from the asset
  page's draw; defect 22's note: slow, never wrong).
- A schedule month a run leaves for a human (dated in a closed period, or
  reversed after it ended) has nothing to mark it booked by hand, so its
  schedule never shows complete; a schedule or an asset registered by
  mistake has no way to stop. Both want a design.
- A reversal dated before what it reverses is refused for depreciation
  runs and schedule postings only (B-9); every other entry still takes
  one: a general rule to settle (P4's reversal as a draft is the natural
  place).
- Sources not yet read or vendored (`docs/spec/README.md`, "Open"): ASC
  606-10-25-23 (DART only), 25-31, 25-32, 55-20 (B-16); 230-10-45-15(f)
  (B-14 rule 11); the ASC 250 paragraph B-15 rule 6 means; 606-10-55-39
  (B-17); 105-10-05-1 to 05-3's current wording. Public domain but not
  vendored: IRC §701 and §6225 (B-14 rule 5), Treas. Reg.
  §1.704-1(b)(2)(iv)(d)(1) and (g)(3) (B-25, cited only). Several cited
  pages in `sources/intuit-iif.md` carry no SHA-256, and
  `irs-entity-classification.md` and `file-names.md` no Source, Retrieved
  or SHA-256 lines; Unicode UAX #14 (B-34) has no source file; Rev. Proc.
  98-25's excerpt (`irs-records.md`) carries no "§5.01" label.
- Eight tool descriptions are over 700 characters (listed in
  `tool_descriptions_fit_700_characters`): shortened in P5.
- The every-order asset search runs three commands deep in `just check`
  (four and five clean by hand, `WB_ORDERS_DEPTH`); six is P12's. The
  `ledger-store` test
  `ops::tests::the_server_makes_the_folder_and_the_viewer_only_reads_it`
  failed once in a whole-workspace run (likely a forked git child holding
  the viewer lock's descriptor); it passed in this review's runs.

**Next:** the P3 cutover, then P4-P13 as planned. P11: one app for
Windows, Mac and Linux (D-24). P12: max testing again (D-33). P13:
bookspec (D-34), a formal, sourced accounting specification first, then
its own formal validation and a conformance harness for any engine; P3's
regression tests are its first examples.

---

# Earlier status — 2026-10-11 (all 10 natural next steps done; repo now public)

## Where we are

All ten natural next steps are done, tested, and pushed. The repo is now
public. Parity is 12 of 12 against the spreadsheet workflow.

**Done:**
- Repo `swaits/wanderbooks` on git.swaits.com (public as of 2026-10-11).
- `ledger-core` 0.1.0: `Money` (exact Decimal + ISO currency), five account
  types with normal sides, chart of accounts (codes, hierarchy,
  placeholders), journal entries with full validation, `Ledger` command
  surface (create/add-line/post/reverse), append-only posting log.
- Reports: trial balance, P&L, balance sheet (accounting equation holds;
  current earnings folded into equity pre-close), canonical JSON `snapshot()`
  for oracle diffing.
- 14 tests green: validation rejections, posted-entry immutability,
  reversal semantics, the accounting laws on a fixture company, a 300-entry
  randomized invariant test, and the first golden file.
- `ledger-store` 0.1.0: the source of truth is an append-only, hash-chained
  JSONL journal (fsync per append; tamper-evident); content-addressed
  SHA-256 blob store (immutable, deduplicated, hash-verified on read);
  SQLite as a *disposable* projection (TEXT amounts, WAL, `synchronous=FULL`,
  `integrity_check` on open). Single write path: validate → journal →
  mutate → apply. `verify` replays the journal and diffs reports three ways;
  `rebuild` drops SQLite and replays; a missing database heals on open.
  7 tests green: round-trip, tamper detection, torn-tail truncation,
  rebuild, blob dedup/corruption detection, unbalanced-post rejection.
- `ledger-mcp` 0.1.0: stdio server on official rmcp 3.5 (23 narrow typed
  tools, amounts as decimal strings, `initialize.instructions` carrying the
  agent operating contract). Draft → add lines → preview → confirmed post;
  server-side receipt policy (strictly greater than $25, per-account
  overrides, typed `receipt_required` rejection); digest; search; CSV/JSON
  export; period P&L and as-of balance sheet; fixed-asset tools (register,
  schedule, list, run-depreciation, dispose with preview). End-to-end stdio
  protocol tests green (initialize → tools → draft → preview →
  receipt_required → attach → post → reports → digest → reverse; and a full
  asset flow: register → schedule → idempotent runs → disposal preview →
  receipt_required → bill-of-sale → confirmed disposal → gain posted).
- **Fixed assets** (`ledger-core` + `ledger-store` + `ledger-mcp`):
  fixed-asset register (description, purchase date, cost, salvage, useful
  life, linked cost/accumulated-depreciation/expense accounts), straight-line
  book depreciation with day-prorated partial months and a final-month
  true-up so the lifetime total is exactly (cost − salvage), idempotent
  monthly runs, disposal with auto-posted final depreciation and gain/loss.
  Tax depreciation stays with the CPA. Tests: 10 asset unit tests
  (ledger-core), 6 store tests, 7 property tests, MCP e2e.
- **Testing initiative** (in progress): `proptest` property suite on
  `ledger-core` (trial balance always balances, A = L + E, reversals restore
  zero, posted-entry immutability, JSON round-trip, depreciation exactness —
  the last caught a real month-length-drift bug, fixed with the final-month
  true-up); `cargo-mutants` run on `ledger-core` (228 mutants); `cargo-fuzz`
  targets (journal-line parsing, entry validation, op sequences) scaffolded.
- 120+ tests green workspace-wide; `cargo fmt` clean; `cargo clippy
  --workspace --all-targets` clean, zero warnings (a hard rule: the owner,
  2026-10-11).
- Docs: AGENTS.md (operating manual), requirements, architecture, oracle
  design, parity catalog (floor: 10 done), glossary, mcp-interface (incl. the
  5 asset tools), receipt-store (journal design), capability-parity
  (10 of 12 rows covered). Plan: this file + roadmap.
- **Formal verification** (2026-10-13, `docs/verification.md`): the
  money arithmetic moved into one generic kernel (`kernel.rs`) that the
  engine calls. 18 Kani harnesses prove it exhaustively within bounds,
  with covers against vacuity and a canary for a Kani soundness bug. 24
  Lean 4 theorems prove the accounting laws for books of any size, after
  any sequence of commands. A property test bridges `i64` and `Decimal`.
  `just prove`. Found and fixed: negative line amounts accepted by the
  engine, an overflow crash on huge amounts, and an overflow crash in FX
  conversion.

**In progress:** nothing — the next arcs are queued below, unstarted.

## Next arcs (in order)

The project went AI-first on 2026-10-10 (the owner's call): the primary
interface is MCP, not a cloned GUI. The first storage + MCP slice is done
(arcs 1–3 below). What remains:

1. ~~**`ledger-store`**~~ done — journal + blob store + SQLite projection.
2. ~~**Blob store**~~ done — folded into `ledger-store`.
3. ~~**`ledger-mcp`**~~ done — 18 tools, receipt policy, digest, export.
   Remaining from the spec: MCP *resources* (rmcp 3.x has no resource
   macros; hand-roll `list_resources`/`read_resource` next), Streamable HTTP
   transport, XLSX/IIF export formats.
4. **Capability parity** (`docs/capability-parity.md`): 11 of 12 rows
   covered; full CPA export (xlsx/iif) remains. Then the month-long parallel
   run against the GSheet; cutover per the cutover rule.
5. Then: oracle harness (cross-check diffs, GnuCash first), read-only GUI on the event log,
   workflow parity.

## Decisions log

- 2026-10-23: D-46, files only by handle: no inline file bytes in or out;
  uploads in, downloads out (plan/served.md D-46).
- 2026-10-23: D-44, one way to serve books: the stdio server, its inline
  file outputs and its settings-file mode are gone; tests drive a real
  books server (plan/served.md D-44).
- 2026-10-13: Formal methods are part of the bar (the owner: "This shit is
  fucking GOLD"). Money arithmetic lives in `kernel.rs` with Kani
  harnesses; accounting laws get Lean theorems; `just prove` stays green.
  The engine now rejects negative and out-of-range line amounts (max one
  quadrillion, ten decimal places). Every dev tool is pinned in
  `mise.toml`. See AGENTS.md "Formal methods".

- 2026-10-10: Single currency per ledger for day one (R-5). Multi-currency
  is designed-for, not built.
- 2026-10-10: `Money` arithmetic is checked and returns `Result`; no
  panicking ops, no float paths anywhere.
- 2026-10-10: Negative zero is normalized away at `account_balance` — money
  never prints "-0". (Caught by the golden test on day one; the oracle works.)
- 2026-10-10: License MIT OR Apache-2.0, matching the storytold projects.
  GnuCash clean-room boundary documented in `docs/oracle.md`.
- 2026-10-10: AI-first pivot (the owner). Primary interface is MCP
  (`docs/mcp-interface.md`), not a cloned GUI. Receipts/documents become a
  content-addressed blob store (`docs/receipt-store.md`). Parity target for
  the phase is the GSheet+GDrive system (`docs/capability-parity.md`), with
  a parallel-run cutover rule. GUI drops to read-only, event-log-driven,
  phase 4.
- 2026-10-10: Import/export targets QuickBooks formats (CSV, IIF, XLSX) —
  "could not give a single shit about GnuCash XML." GnuCash stays a
  *behavioral* cross-check only. (2026-10-13, the owner: it is "an" oracle,
  not "the" oracle; never trust it to be correct.) Receipt enforcement is server-side policy in the
  MCP layer (threshold + per-account overrides, typed `receipt_required`
  error, checklist in every preview); the engine stays policy-free.
- 2026-10-10: Storage durability (the owner). SQLite is the query engine, not
  the source of truth: the source of truth is an append-only, hash-chained
  JSONL journal (fsync per append; tamper-evident; git-backup-friendly),
  SQLite is a disposable projection rebuilt from it, blobs stay
  content-addressed. Single write path: journal → fsync → apply to SQLite.
- 2026-10-10: Draft durability (house decision). Drafts are **durable**, not
  ephemeral: `entry.created` + `entry.line_added` journal lines record the
  draft as the agent builds it, so drafts and their attached receipts survive
  restarts. Drafts never touch the SQLite projection until posted. Rationale:
  the journal is the source of truth (phantom `entry.created` lines without
  draft lines were the worst of both worlds), and the draft → attach receipts
  → post workflow spans multiple MCP calls.
- 2026-10-10: Corrupt SQLite heals itself. If the projection fails
  `integrity_check` (or isn't a database at all), `Store::open` quarantines
  it to `wanderbooks.corrupt.<timestamp>` and rebuilds from the journal,
  instead of failing open. WAL/SHM are dropped as derived state.
- 2026-10-10: Cash flow statement (R-FUTURE-3) done. Engine
  `reports::cash_flow` (indirect method) + `books.cash_flow` MCP tool (24
  tools total). Disposals identified structurally; gains/losses move from
  operating to investing; the three sections always reconcile to the net
  change in the designated cash accounts. Two invariant tests, including a
  full purchase → depreciate → dispose cycle.
- 2026-10-10: MCP resources done (hand-rolled `list_resources` /
  `read_resource`: chart, trial-balance, balance-sheet, profit-and-loss,
  digest, receipts). `#[tool_handler]` must sit on the `impl ServerHandler`
  block, not the inherent impl — misplacing it silently drops all tools.
- 2026-10-10: QuickBooks interop (R-FUTURE-4) done. New `ledger-import`
  crate: QBO bank CSV parse (3-col and 4-col layouts), chart CSV parse,
  IIF journal generation, XLSX report generation (rust_xlsxwriter, MIT).
  MCP: `books.import_bank_csv` (drafts + categorization rules),
  `books.import_chart_csv` (all-or-nothing), `books.export` gains `xlsx`
  (base64) and `iif` formats plus `cash_flow` report. 26 tools total.
  Capability parity 12 of 12 rows covered.
- 2026-10-10: Streamable HTTP transport done. `ledger-mcp --http ADDR`
  serves MCP at `/mcp` via rmcp's tower StreamableHttpService (axum).
  27 tools total. Verified with curl: initialize returns SSE stream.
- 2026-10-10: Drive backfill path done. `books.import_local_file` reads a
  file from the server's filesystem into the blob store (receipt/document).
  The agent downloads from Drive via the CLI, then calls this — Drive
  credentials never enter the server.
- 2026-10-11: `wanderbooks init` done. New `ledger-cli` crate (binary
  `wanderbooks`, std-only): `init [DIR]` scaffolds a books dir — `mise.toml`
  (pinned Rust toolchain; tasks: build, mcp-add, backup), `AGENT.md` (agent
  operating contract: bootstrap, MCP contract, first-run onboarding incl.
  migration via conversation, ongoing rules), `wanderbooks.toml` (company
  profile, filled during onboarding), `books-data/` + `.gitignore` (SQLite
  projection ignored; journal is the record). Fresh dirs get `git init` and
  a scaffold commit so the journal archive starts on day one. One generic
  init by design — no per-situation flags; the agent is the import layer
  (QBO/CSV/email/sheets, conversational). 5 integration tests green; clippy
  clean. Bootstrap: `mise trust && mise install`, `mise run build`,
  `mise run mcp-add`, then `claude` in the dir.
- 2026-10-11: Receipts are mutable through events, never edits (the owner:
  "the journal is just a history of every single thing that happens").
  `books.unlink_receipt` and `books.withdraw_attachment` (both require a
  reason) journal `receipt.unlinked` / `attachment.withdrawn`; nothing is
  erased from the journal or blob store. Unlinking can't strand an entry the
  receipt policy requires evidence for (attach the replacement first).
  `link_receipt` now refuses an already-linked or withdrawn attachment, which
  closes a silent path for moving a receipt off a posted entry; disposals
  check their receipt ids before posting. Schema v5. First real use: swapping
  the migrated the first company `.eml` receipts for their PDF renders.
- 2026-10-11: The cash flow statement is a first-class output everywhere the
  other two statements are: `cash_flow_pdf` in ledger-import, a `pdf`
  format on `books.export` (P&L, balance sheet, cash flow), and
  `cash-flow.pdf` in the CPA package, which now requires `cash_accounts`
  (the statement reconciles to them). Known gaps, next: the P&L and balance
  sheet PDF/XLSX ignore start/end/as_of (all-time balances under the
  requested label; core needs period-scoped statements); P&L and balance
  sheet XLSX are totals-only; PDF amounts don't align (padding in a
  proportional font), no company name, negatives as "-x".
- 2026-10-11: Every statement export (csv, json, xlsx, pdf, CPA package)
  now renders core's period statements: the date range and as-of date are
  real, with a line per account. One layout (`ledger-import::report`:
  sections, lines, subtotals, totals; exact "(1,234.56)" formatting) feeds
  every renderer. The server reads `wanderbooks.toml`: company name for
  report headers, `fiscal_year_start` for fiscal YTD (end derived), and
  `receipt_threshold` (it was silently ignored before). Reports carry
  "Prepared from journal through seq N".
- 2026-10-11: PDFs are typeset with Typst (typst 0.15 via typst-as-lib,
  fonts bundled by typst-assets; printpdf removed). One template,
  `ledger-import/templates/statement.typ`, takes the laid-out statement as
  JSON data (labels are never markup). US accounting presentation in PDF
  and XLSX alike: tabular figures, "$" on each section's first amount and
  on totals, negatives in parentheses with the closing paren hanging so
  decimal points align, zero as a dash, single rule over sums, double rule
  under totals, amounts never bold (bold figures are wider). Pages flow
  with a repeated header and "Page n of m". Core's cash flow now orders
  lines by account code (it followed HashMap order, so line order could
  change between runs).
- 2026-10-11: The canonical report set. Core gained the general ledger by
  account (running balances; QuickBooks' "split" = the opposite side of the
  entry, "-Split-" when several accounts), the trial balance as of a date,
  and the statement of changes in members' equity (capital roll-forward per
  member, allocation by share to the cent, ties to balance-sheet equity).
  The entry-ordered listing is now correctly called the general journal.
  Reports render QuickBooks-style (the owner's reference): Inter (bundled,
  OFL) instead of the typst-assets serif fonts, centered title block, a
  column-header row, multi-column statements (trial balance Debit/Credit,
  one column per member), "$" on sums, rules above sums, a double rule
  under totals, zero as 0.00, entity terms (Members' equity, Net income
  (loss), standard cash-flow wording) and an "Accrual basis · Prepared …"
  footer. "Today" is local time, not UTC (evening exports were dated
  tomorrow).
- 2026-10-11: Period close fixed and made partnership-aware. The close swept
  every revenue/expense posting ever made, so a period closed after later
  entries existed swallowed them (closing January in March closed February
  and March too); it now zeroes balances as of the period end, refuses to
  close out of order, and its preview is the real close run on a copy of
  the ledger. A multi-member LLC closes into each member's capital account
  by share (CloseTo::Members), the partnership convention, so capital
  accounts carry allocated earnings; the members'-equity statement treats
  those closes as income, not contributions, and the balance sheet shows
  only unallocated earnings. Replay now checks closing lines, not just ids.
- 2026-10-11: Tax categories (the owner: wanderbooks is a general MCP-first
  accounting application, so features ship for every small business, not
  when the first company needs them). Accounts map to stable categories rather than
  form line numbers; each category knows its returns and default
  deductible percent; the tax summary shows book vs. tax by category and
  every uncategorized account. tax_form comes from config or the tax
  classification. Schema v6.
- 2026-10-11: Owner equity for every entity type. Chart templates for sole
  proprietorships, partnerships/multi-member LLCs, S corps, and C corps
  (equity and owner-pay accounts, tax categories, matching config). Closes
  sweep draws, distributions, and dividends into their target; partners'
  distribution accounts count as capital on the members' equity statement
  and close into it. CloseTo became RetainedEarnings { sweep } / Account
  { into, sweep } / Members([MemberClose]); journals without a target, or
  with the old plain "retained_earnings", still replay.
- 2026-10-11: Schedules. Prepaid expenses and deferred revenue amortize
  monthly on month ends, to the cent; runs are previewed, idempotent, and
  exempt from the receipt policy (the schedule is the evidence). Recurring
  entries generate drafts, never posts. Journal events schedule.registered,
  schedule.posted (with the full entry, checked on replay),
  recurring.registered, recurring.generated. No schema change. 59 tools.
- 2026-10-11: Payouts. books.record_payout grosses a platform deposit up
  into revenue (by class), fees, and adjustments, refusing unless it
  reconciles to the cent; it creates a draft so the payout statement is
  attached as the receipt. Pure core logic (payouts.rs), no new journal
  events (it journals as a draft and lines). 60 tools. Statement parsers
  (Apple, Google, Stripe) wait for sample files.
- 2026-10-11: 1099 tracking. Vendors get form_1099 (NEC/MISC) and
  tin_on_file (never the TIN); entries get an optional vendor tag (bills,
  bill payments, and reversals inherit it). books.vendor_1099 counts cash
  paid per vendor per calendar year against $600 (≤2025) / $2,000 (2026+,
  OBBBA) or an override, and lists reportable vendors missing a W-9.
  Journal events vendor.1099_set and entry.vendor_set. Schema v7. 64 tools.
  Also: requirements and parity no longer list finished R-FUTURE work as
  future (only budgets remain, declined).
- 2026-10-12: ledger-gui, the read-only desktop view (egui 0.36, wgpu).
  Overview (the answer in a sentence, KPI tiles with honest comparisons,
  cash chart, needs-attention with copyable agent prompts, recent
  activity), Cash (accounts, in/out, burn and runway, coming up), Profit
  (by month with margin, spending vs. its recent average, by customer and
  class), Transactions (search, filters, cash direction, detail with
  lines, the rendered receipt via hayro, and journal history), Reports
  (P&L, balance sheet, cash flow, owners' equity, trial balance, GL, tax
  summary, 1099, agings; date presets, month/quarter/year columns, prior
  period/year comparison, % of income, drill-down to the account register
  for exactly those dates, copy as TSV), Accounts and registers,
  Documents, Company, Journal (every event, hash, payload), Ctrl+K
  palette, back/forward, light/dark. Live: notify-driven, ~30 ms from
  write to screen. Engine additions: ledger-store Snapshot (read-only,
  incremental, torn-tail tolerant), ledger-core analysis (intervals,
  balance series and history, P&L by interval, flows), ledger-config
  (extracted from ledger-mcp; full profile, cash_accounts). Fonts: Inter
  with tnum frozen in and private-use glyphs removed (Phosphor icons).
- 2026-10-12: One-command onboarding. `wanderbooks new DIR` installs the
  server and GUI if missing, writes AGENTS.md (+ CLAUDE.md importing it),
  wanderbooks.toml, a justfile (agent, gui, backup, update), .gitignore,
  .mcp.json (the server registered for that dir only) and
  .claude/settings.json (approved, no prompt), commits, and starts claude
  with the onboarding prompt. `install.sh` does Rust + cargo install +
  `wanderbooks new` in one line. mise is no longer part of the scaffold.
- 2026-10-12: Integrity, end to end. Store::verify now also compares the
  whole SQLite projection (every table, row for row) with a fresh
  projection of the journal; verify_and_heal rebuilds a drifted database
  (keeping the old file) and runs at every server start; books.verify
  (65 tools) adds a re-hash of every blob. The GUI compares the server's
  database (read-only) with its own projection and re-hashes each file
  once, showing "Database matches journal · N files verified" and raising
  problems in Needs attention. Also: Windows-safe blob writes (no
  directory fsync off Unix), .exe-aware binary lookup, installer OS and
  C-compiler checks with WANDERBOOKS_REPO, the website (site/, `just
  site`, `just publish`), and a repo justfile (check, build, install, demo).
- 2026-10-12: The CPA story. The CPA package now also carries every
  receipt behind the period's entries (receipts/, with index.csv tying each
  to its entry, date, amount, and SHA-256), the company's documents, the
  journal as IIF (QuickBooks Desktop), the 1099 worksheet, and a README
  with the verified journal size; include_receipts=false for a light one.
  The scaffolded AGENTS.md teaches the bookkeeper to find receipts in
  email, migrate from QuickBooks/Xero/spreadsheets (match every statement
  to the cent; keep the old system read-only for a month), and work with
  the CPA (package, read-only clone + GUI). The site leads with owning
  your books and an AI bookkeeper: example prompts, a QuickBooks
  comparison, an honest "what it doesn't do yet", the CPA section, FAQ.
- 2026-10-12: The agent can drive the viewer. books.show (66 tools) asks
  the viewer to open an entry, account register, report (with dates),
  document, search, or page. The request lives outside the books (a JSON
  file in the runtime dir keyed by the data dir, written atomically); the
  viewer watches it beside the journal and applies books before
  navigation. ~30 ms end to end.
