# Kernel sweep audit, and hledger as a second oracle

Status: read-only audit, 2026-11-08. No code was changed. This file is the
deliverable; it inventories money arithmetic outside `kernel.rs`, lists the
panic-capable sites, groups the work into independently landable units, and
assesses hledger as a second cross-check oracle.

Method: `grep` over `crates/*/src` for `round_dp*`, `rescale`, `checked_*`,
`.sum()`, and raw `+ - * /` next to `.amount`/`Decimal`, followed by reading
every hit in context. Tests, benches, examples and `ledger-cli`/`ledger-config`
(no money arithmetic found) are out of scope. A **site** is one arithmetic
expression, or one tightly coupled chain such as
`a.checked_mul(b).and_then(|x| x.checked_div(c))`; date arithmetic and
`chrono` `checked_add_*` are excluded.

---

## 0. Summary

- **211 catalogued sites** outside `kernel.rs`.
  - **(a) must move to the kernel: 169.** Money that reaches the books or a
    report/filed figure (including MCP/package/store figures).
  - **(b) display-only: 35.** Formatting and viewer figures; they should share
    one rounding function, but they are not kernel laws.
  - **(c) not money: 7.** Member shares summing to 1, percentage templates,
    UI chart floats, demo data.
- **76 raw/panic-capable sites**: a raw `+ - * /` or `.sum()` on `Decimal`/
  `Money`, or a division that panics on zero. Most are only reachable with
  books far larger than memory (see §2); a handful are reachable from a public
  API with an edge-case value.
- **The largest gap is not a missing add.** `Amount::checked_add` and friends
  already exist. What is missing is the *shape* above them: rounding to a
  unit, proportional shares, collection totals, FX conversion, even monthly
  plans, ratios, and payroll sums. Six local re-implementations of
  "share = whole x weight / total, round, hand out the remainder" exist in
  `tax_capital.rs`, `k1.rs`, `conventions.rs`, `shares.rs`, `form1099.rs` and
  `cashflow.rs`; two more in `periods.rs` are frozen replay rules.
- **hledger verdict (three lines):** hledger can do every comparison the
  GnuCash harness does (balances at dates, monthly P&L, trial balance, and a
  derived balance sheet), because the harness feeds it base-currency lines
  only; pin the official static **1.52.4** binary by sha256 in a container
  (Debian trixie's 1.32.3 if an apt pin is preferred). It tracks exact
  decimals but **reports at commodity display precision**, so the harness must
  pass `-c '$1,000.0000000000'`; it has no `--precision` flag even in 1.52.4,
  and dates need an exclusive upper bound (`-e <as_of + 1 day>`). An hledger
  adapter in bookspec can run every example the GnuCash adapter claims, and
  more (multi-currency, no Python bindings).

---

## 1. Inventory

### 1.1 `ledger-core` (the engine and its reports)

Counts by file. "Kernel?" says whether the site already delegates to a kernel
function; a bare `Amount::checked_*` call delegates only to the trait
primitive, not to a law-bearing kernel function.

| File | (a) | (b) | (c) | Sites | Panic-capable | Main rules |
|---|---:|---:|---:|---:|---:|---|
| `money.rs` | 2 | 1 | 0 | 3 | 2 | B-3, B-21 |
| `ledger.rs` | 23 | 0 | 1 | 24 | 4 | B-3, B-13, B-28, B-38 |
| `reports.rs` | 24 | 0 | 1 | 25 | 17 | B-6, B-7, B-14, B-30, B-37 |
| `tax_capital.rs` | 23 | 0 | 0 | 23 | 8 | B-25, B-36 |
| `schedules.rs` | 5 | 0 | 0 | 5 | 0 | B-15, B-16 |
| `assets.rs` | 10 | 0 | 0 | 10 | 0 | B-15, B-24, B-25 |
| `form1099.rs` | 5 | 0 | 0 | 5 | 5 | B-19, B-35 |
| `parties.rs` | 4 | 0 | 0 | 4 | 0 | B-26, B-35 |
| `shares.rs` | 5 | 0 | 0 | 5 | 0 | B-35, B-36 |
| `arap.rs` | 3 | 0 | 0 | 3 | 0 | B-18, B-38 |
| `cashflow.rs` | 6 | 0 | 0 | 6 | 1 | B-14 |
| `payroll.rs` | 2 | 0 | 0 | 2 | 0 | B-19 (payroll) |
| `periods.rs` | 4 | 0 | 0 | 4 | 1 | B-11, B-13 |
| `k1.rs` | 7 | 0 | 0 | 7 | 5 | B-36 |
| `conventions.rs` | 3 | 0 | 0 | 3 | 0 | B-29 |
| `tax.rs` | 3 | 0 | 0 | 3 | 2 | B-21, B-32 |
| `evidence.rs` | 2 | 0 | 0 | 2 | 0 | B-20, B-43 |
| `audit.rs` | 2 | 0 | 0 | 2 | 2 | B-45 |
| `analysis.rs` | 1 | 0 | 0 | 1 | 0 | B-14 |
| `settings.rs` | 0 | 0 | 2 | 2 | 0 | B-13 (shares) |
| **Total** | **134** | **1** | **4** | **139** | **47** | |

#### `money.rs`

| Site | What it computes | Kernel? | Panic? | Rule | Class |
|---|---|---|---|---|---|
| `money.rs:133-142` (`checked_add`/`checked_sub`) | Adds/subtracts two `Money` after a currency check, with `self.amount + rhs.amount` | No; named checked but uses the raw operator | **Yes** (`Decimal::MAX + 1`) | B-3, B-21 | a |
| `money.rs:202-212` (`Steps::decimal`, `rescale`) | Pads a step count to the currency's minor places for display | No | No | B-3 | b |

**Finding:** `Money::checked_add`/`checked_sub` return `Result` but panic on
overflow. `Money::new(Decimal::MAX, USD).checked_add(Money::new(Decimal::ONE,
USD))` is a public-API panic, the exact class AGENTS.md names.

#### `ledger.rs`

| Site | What it computes | Kernel? | Panic? | Rule | Class |
|---|---|---|---|---|---|
| `1862-1882` (`add_line_fx`) | Foreign amount x rate, rounded once to the base currency's minor unit, half away from zero | No | No (`checked_mul`; `round_dp`) | B-3 | a |
| `3384` (`carried_between`) | Posted depreciation in a date span | `checked_add` | No | B-15 | a |
| `4047` (`validate_close_to`) | Σ member shares, checked against 1 | `.sum()` raw | Yes in theory | B-13 | c |
| `6254-6266` (`doc_state`) | Outstanding/paid/discount/write-off from document movements, with sign flips | `checked_add`, raw `-` | No | B-38 | a |
| `6322` (`payments_recorded`) | Σ payments on a document | `checked_add` | No | B-38 | a |
| `6364` (`doc_deltas`) | A document's movement per date | `checked_add` | No | B-38 | a |
| `6399` (`check_item_bounds`) | Owed through a date, for the refusal text | `.sum()` raw | Yes in theory | B-38 | a |
| `6440-6444` (`least_owed_from`) | Running document balance and its least | `checked_add` | No | B-38, B-41 | a |
| `6636-6674` (`settle_documents`) | Applications = cash + discount; Σ discounts | `checked_add` | No | B-42 | a |
| `7305-7390` (`control_checks`) | Control-account balance, document totals, unattached lines, difference | `checked_*` mostly, raw `-` | No | B-38 | a |
| `7418-7419` (`refresh_doc`) | Settled = paid + discounted + written off | `checked_add` | No | B-38 | a |
| `8077-8096` (`reconciliation`) | Book and cleared balances, signed on the normal side | Raw `+=` and raw `-` | **Yes** | B-6, B-14 | a |
| `8120` (`reconciliation`) | Cleared less statement balance | `checked_sub` | No | B-14 | a |
| `8337` (payroll party totals) | Σ net pay per party | `checked_add` | No | B-19 | a |
| `8418` (`remit_payroll_taxes`) | Σ payroll tax payables | `.sum()` raw | **Yes** | B-19 | a |
| `9107` (`approvals_missing`) | Entry debits for approval limits | `checked_add` | No | B-45 | a |

#### `reports.rs`

| Site | What it computes | Kernel? | Panic? | Rule | Class |
|---|---|---|---|---|---|
| `94` (`trial_balance_where`) | Signed movement into a per-account map | Raw `+=` | **Yes** | B-6, B-7 | a |
| `270-271` (`profit_and_loss_where`) | Revenue/expense totals, sign on the normal side | Raw `+=` | **Yes** | B-6, B-14 | a |
| `412` (`statement_lines`) | Signed movement per account for statements | Raw `+=` | **Yes** | B-6, B-7 | a |
| `517-518` (`profit_and_loss_statement`) | Total revenue / total expenses | `.sum()` raw | **Yes** | B-6, B-14 | a |
| `630-633` (`contribution_margin_pct`) | Contribution / revenue x 100, one place, half-to-even | No (checked + `round_dp`) | No | B-37 | a |
| `689-694` (`project_figures`) | Running months-to-date nets | `checked_*` | No | B-37 | a |
| `760-767` (`project_to_date`) | A project's net per entry | Raw `+=`/`-=` | **Yes** | B-37 | a |
| `819-825` (`balance_sheet_as_of`) | Current earnings and statement totals | Raw `+=`, `.sum()` | **Yes** | B-7 | a |
| `948` (`general_ledger`) | Opening balances per account | Raw `+=` | **Yes** | B-6, B-14 | a |
| `1183` (`members_equity`) | Σ member shares, checked against 1 | `.sum()` raw | Yes in theory | B-13 | c |
| `1316-1323` (`members_equity`) | Unclosed equity, explained change, and their sums | `-`, `.sum()` raw | **Yes** | B-13 | a |
| `1354` (`members_equity`) | Column totals | `.sum()` raw | **Yes** | B-13 | a |
| `1584-1588` (`register_row`) | Asset book value: cost − posted depreciation | `checked_sub` | No | B-15, B-24 | a |
| `1675-1690` (`fixed_asset_register`) | Register totals | `checked_add` | No | B-24, B-25 | a |
| `1718` (`fixed_asset_register`) | Total book value | `checked_sub` | No | B-24 | a |
| `1723-1724` (`fixed_asset_register`) | Ledger cost/accumulated from the trial balance | `.sum()` raw | **Yes** | B-24 | a |

#### `tax_capital.rs` (the K-1 / Form 1065 worksheet)

| Site | What it computes | Kernel? | Panic? | Rule | Class |
|---|---|---|---|---|---|
| `126-128` (`disposal_depreciation`) | Depreciation on a disposal, signed | `checked_add`, raw `-` | No | B-25 | a |
| `277-279` (`Reconciliation::total`) | Beginning + §704(c) + no-basis property − distributed | `checked_*` | No | B-25, B-36 | a |
| `438` (`weight_sum`) | Σ weights | `checked_add` | No | B-36 | a |
| `446-447` (`RunningSplit::share`) | whole x weight / Σ weights | `checked_mul`/`checked_div` | No | B-36 | a |
| `469-473` | Running exact total + whole | `checked_*` | No | B-36 | a |
| `485-502` (`RunningSplit::split`) | Round exact running totals to 10 places, then down to cents; sign handling | `round_dp*`, raw `* sign`, `*p *= sign` | **Yes** (`x * sign`, `x - p`) | B-36 | a |
| `503` | Add the part to the running total | `checked_add` | No | B-36 | a |
| `537-541` (`unsplit`) | Reverse a split | `checked_*` | No | B-36 | a |
| `594`, `618-622`, `739`, `756`, `827` | Per-member/per-kind sums from journal lines | `.sum()` raw in two, `checked_add` in three | **Yes** (two `.sum()`) | B-36 | a |
| `758`, `784` (`entry_property`) | Pool less prepaid; pool less linked | Raw `-` | **Yes** | B-25, B-36 | a |
| `769-788` | Linked/known/unknown property sums | `checked_add` | No | B-25 | a |
| `850-868` (`span`) | Per-member built-in gain, no-basis property, distributions | `checked_add` | No | B-36 | a |
| `885-889` | Book and contributed depreciation | `checked_add` | No | B-15, B-25 | a |
| `927-984` | Contributions/distributions rows and the closing reconciliation | `checked_*` | No | B-36 | a |

#### `schedules.rs`, `assets.rs`

| Site | What it computes | Kernel? | Panic? | Rule | Class |
|---|---|---|---|---|---|
| `schedules.rs:197-210` (`amortization_plan`) | Monthly even plan: total / months rounded to **2 dp**, last month absorbs | No (and currency-blind) | No | B-16 | a |
| `schedules.rs:478` (`remaining`) | Σ unposted months | `checked_add` | No | B-16 | a |
| `schedules.rs:616` (`validate_recurring`) | Recurring entry's debit/credit totals | `checked_add` | No | B-1 | a |
| `schedules.rs:692` | Σ plan amounts | `checked_add` | No | B-16 | a |
| `assets.rs:690` | Cost − salvage (depreciable basis) | `checked_sub` | No | B-15 | a |
| `assets.rs:727` | Cost − accumulated (book value) | `checked_sub` | No | B-15, B-24 | a |
| `assets.rs:736` | Cumulative day weight | kernel `part_weight` + `checked_add` | No | B-15 | a |
| `assets.rs:766` | Cost − scheduled accumulated | `checked_sub` | No | B-15 | a |
| `assets.rs:884` | Month charge = cumulative − charged (i128 steps) | Raw `-` | No (bounded) | B-15 | a |
| `assets.rs:933` | Carried − scheduled = over-depreciation | `checked_sub` | No | B-15 | a |
| `assets.rs:984` | Running carried amount | `checked_add` | No | B-15 | a |
| `assets.rs:1100` | Over-depreciation at catch-up | `checked_sub` | No | B-15 | a |
| `assets.rs:1160-1165` | Disposal book value and gain/loss | `checked_sub` | No | B-15, B-24 | a |

**Finding (currency):** `amortization_plan` rounds to 2 places regardless of
currency. `daily_plan` correctly prorates in the currency's minor units via
`Steps`. For JPY or KWD, a monthly-even schedule books fractions of a yen or
misses mils. It also reproduces the plan on replay (`Schedule::plan()`), so a
fix must be versioned or limited to new schedules.

#### `form1099.rs`, `parties.rs`, `shares.rs`

| Site | What it computes | Kernel? | Panic? | Rule | Class |
|---|---|---|---|---|---|
| `form1099.rs:707-708` (`sum3`) | Σ three payment buckets | `checked_add` | No | B-19 | a |
| `724-731` (`vendor_part`) | A vendor's share of a multi-party payment: payment x share / target, rounded to cents | No | No (checked) | B-19, B-35 | a |
| `753-779` (`split_share`) | Split a vendor part across buckets by exact proportion, truncate to cents, hand out remainder; a **hand-rolled correction loop** (`part -= cent`, `part += cent`) | kernel `apportion` only at the end | **Yes** (`-=`, `+=`, `num -`) | B-19 | a |
| `841` (`add`) | Tally addition | `checked_add` | No | B-19 | a |
| `1246` | Counted + unknown out-of-pocket | `checked_add` | No | B-19 | a |
| `parties.rs:921-924` | Cash/in-kind totals of a party | `checked_add` | No | B-35 | a |
| `1155-1156` (`entry_flow`) | Four signed sums (cash/liability, debit/credit) | checked closures | No | B-35 | a |
| `1400-1410` (`resolve`) | Paid/received plus in-kind | `checked_*` | No | B-35 | a |
| `shares.rs:280` (`checked_sum`) | Σ weights | `checked_add` | No | B-35 | a |
| `296-302` (`split_first`) | whole x weight / total, truncate, leftover to the **first** | No (a third leftover policy) | No | B-35 | a |
| `411` | Per-party total | `checked_add` | No | B-35 | a |
| `534` | Sign a line on the party's account | Raw `-` | No | B-35 | a |
| `579` (`share_by`) | Sizes less in-kind shares | `checked_sub` | No | B-35 | a |

#### `arap.rs`, `cashflow.rs`, `payroll.rs`

| Site | What it computes | Kernel? | Panic? | Rule | Class |
|---|---|---|---|---|---|
| `arap.rs:1069` (`document_total`) | Σ document lines | `checked_add` | No | B-38 | a |
| `arap.rs:1300` (`AgingRow::total`) | Σ aging buckets | `checked_add` | No | B-18 | a |
| `arap.rs:1307-1315` (`AgingRow::add`) | Add two aging rows bucket-wise | checked closure | No | B-18 | a |
| `cashflow.rs:408` (`foot`/building) | `k.amount += l.amount` for a flow bucket | Raw `+=` | **Yes** | B-14 | a |
| `cashflow.rs:647` (`foot`) | Sections sum − change in cash | `checked_sub` | No | B-14 | a |
| `cashflow.rs:668` (`add`) | Sign/sum helper used throughout | `checked_add` | No | B-14 | a |
| `cashflow.rs:1094` | Negate a party line for the government share | Raw `-` | No | B-14 r5 | a |
| `cashflow.rs:1721-1726` (`split_part`) | amount x ours / whole, rounded half-even to places; rest = amount − part | No | No (checked) | B-14 r5 | a |
| `cashflow.rs:2017` | Cash ending − beginning | `checked_sub` | No | B-14 | a |
| `payroll.rs:63` (`net_pay`) | Gross − all employee withholdings | `checked_sub` fold | No | B-19 (payroll) | a |
| `payroll.rs:178-186` (`payroll_entry_plan`) | Gross, employer FICA, each tax, net, per run | `checked_add` | No | B-19 | a |

#### `periods.rs`, `k1.rs`, `conventions.rs`, `tax.rs`, `evidence.rs`, `audit.rs`, `analysis.rs`, `settings.rs`

| Site | What it computes | Kernel? | Panic? | Rule | Class |
|---|---|---|---|---|---|
| `periods.rs:132` (`allocate`, legacy) | Cents split, truncate then largest remainder; frozen replay rule | kernel `apportion` | No | B-13 | a |
| `periods.rs:155` (`allocate_last_absorbs`, legacy) | amount x share, half away, last absorbs; frozen replay | kernel `plug_last` | **Yes** (`amount * s`) | B-13 | a |
| `periods.rs:232-239` (`allocate_cumulative`) | Exact share, truncate to places, remainder | kernel `apportion_cumulative` | No | B-13 | a |
| `k1.rs:184/203/237/264/569` | Per-member K-1 sums | `.sum()` raw | **Yes** | B-36 | a |
| `k1.rs:322-337` (`share_property`) | Property pool x member credit / total, truncate, largest remainder | kernel `apportion` at the end | No | B-36 | a |
| `conventions.rs:460-474` (`split`) | amount x share, optional places, truncate, largest remainder or plug-last | kernel `apportion`/`plug_last` at the end | No | B-29 | a |
| `tax.rs:522-524` (`tax_net`) | Income − COGS − deductions | Raw `-` | **Yes** | B-21, B-32 | a |
| `tax.rs:530` (`sum`) | Σ a tax column | `.sum()` raw | **Yes** | B-32 | a |
| `tax.rs:573` | Sign a contra income line | Raw `-` | No | B-32 | a |
| `evidence.rs:207` (`entry_debits`) | Σ debits | `checked_add` | No | B-20, B-43 | a |
| `evidence.rs:346` | Threshold total | `checked_add` | No | B-43 | a |
| `audit.rs:351` | Signed revenue/expense net | Raw `+=`, raw `-` | **Yes** | B-45 | a |
| `audit.rs:525` | Σ schedule months | `.sum()` raw | **Yes** | B-16, B-45 | a |
| `analysis.rs:90` | Signed posting for a series | Raw `-` | No | B-14 | a |
| `settings.rs:855` | Parse "50%" to 0.5 | `checked_div` | No | B-13 | c |
| `settings.rs:1409` | Σ member shares, checked against 1 | `checked_add` | No | B-13 | c |

### 1.2 Other crates

| Crate/file | (a) | (b) | (c) | Sites | Panic-capable | Main rules |
|---|---:|---:|---:|---:|---:|---|
| `ledger-import` (`report.rs`, `xlsx.rs`, `templates.rs`, `pdf.rs`, `iif.rs`) | 3 | 6 | 1 | 10 | 4 | B-30, B-33, B-34 |
| `ledger-gui` (`books.rs`, `pages/*.rs`, `demo.rs`, `charts.rs`) | 0 | 26 | 2 | 28 | 10 | viewer only |
| `ledger-package` (`attention.rs`, `invoice.rs`, `package.rs`, `vendor_1099.rs`, `text.rs`) | 11 | 2 | 0 | 13 | 4 | B-19, B-33, B-45 |
| `ledger-mcp` (`server.rs`, `tools/*.rs`) | 9 | 0 | 0 | 9 | 5 | B-28, B-38 |
| `ledger-store` (`store.rs`, `audit.rs`) | 12 | 0 | 0 | 12 | 6 | B-27, B-28, B-38 |
| **Total** | **35** | **34** | **3** | **72** | **29** | |

Selected sites (the rest are the same patterns: `.sum()`, sign flips, and
checked add/sub on figures already computed by the core):

| Site | What it computes | Kernel? | Panic? | Rule | Class |
|---|---|---|---|---|---|
| `ledger-import/report.rs:278` | Retained-earnings total in a report | `.sum()` raw | Yes | B-30 | a |
| `ledger-import/report.rs:522-523` | Book and tax totals in the tax worksheet | `.sum()` raw | Yes | B-32 | a |
| `ledger-import/report.rs:635/667` | `format_amount`/`amount_parts`: round to 2 dp half-away for display | No | No | B-33 | b |
| `ledger-import/xlsx.rs:445` | `exact_f64`: round to cents before the Excel cell | No | No | B-33 | b |
| `ledger-import/templates.rs:118-121` | Equal owner percentages to 2 dp, last absorbs | No (`/`, `-`, `.sum`) | Yes if `n = 0` | B-33 | c |
| `ledger-import/pdf.rs:86` | Percent column rounded to 1 dp | No | No | B-33 | b |
| `ledger-import/iif.rs:955/963` | Debit-positive sign and 2-dp text | No | No | B-34 | b |
| `ledger-gui/books.rs:720-721` | Cash effect of an entry | `checked_*` | No | viewer | b |
| `ledger-gui/books.rs:963/1932/2155/2232` | Receivable, due, depreciation, and other totals | `.sum()` raw | Yes | viewer | b |
| `ledger-gui/books.rs:998` | Average monthly loss, cents | `round_dp` | No | viewer | b |
| `ledger-gui/books.rs:1811-1853` | Scale a party's flow by its resolved share | `checked_*`, `round_dp` | No | B-35 | b |
| `ledger-gui/books.rs:2083-2091` | Split a flow by a share, leftover to the first | `checked_*`, `round_dp` | No | B-35 | b |
| `ledger-gui/pages/cash.rs:183-186` | Average monthly net: `(i - o)` summed then / months | Raw `-`, `/`, `round_dp` | Yes (overflow; months guarded) | viewer | b |
| `ledger-gui/pages/cash.rs:220` | Runway months | `checked_div`, `round_dp` | No | viewer | b |
| `ledger-gui/pages/overview.rs:68/137` | `cash / burn` months | Raw `/` | If `burn = 0` (guarded) | viewer | b |
| `ledger-gui/pages/reports.rs:247/279` | Change % and % of income columns | Raw `/`, `*`, `round_dp` | If denominator 0 (guarded) | viewer | b |
| `ledger-gui/pages/profit.rs:22-27` | `pct(part, whole)`: /, x 100, 1 dp | `checked_*`, `round_dp` | No | viewer | b |
| `ledger-gui/demo.rs:791` | Demo fee = 10% + 0.30 | Raw `*`, `+` | Demo only | none | c |
| `ledger-gui/charts.rs:175` | Chart x position from a fraction (f32) | Float, UI layout | No | none | c |
| `ledger-package/attention.rs:347/431/707/716/873` | Attention totals (receipts, schedules, overdue, write-offs) | `checked_*` | No | B-43, B-45 | a |
| `ledger-package/package.rs:295-297/538` | K-1 totals, member share sum, index totals | `.sum()` raw | Yes | B-36 | a |
| `ledger-package/vendor_1099.rs:196/271` | Σ 1099 payments; negate for display | `.sum()` raw, raw `-` | Yes | B-19 | a |
| `ledger-package/invoice.rs:272` | Payment amount sign for the invoice copy | Raw `-` | No | B-40 | a |
| `ledger-package/text.rs:27/39` | Pad money/share to two places | `rescale` | No | B-33 | b |
| `ledger-mcp/tools/reports.rs:41-43` | Account movement for a balance | `.sum()` raw | Yes | B-6 | a |
| `ledger-mcp/tools/reports.rs:1059` | Sign a line on the normal side | Raw `-` | No | B-6 | a |
| `ledger-mcp/tools/assets.rs:842` | Caught-up depreciation = amount − scheduled | Raw `-` | Yes | B-15 | a |
| `ledger-mcp/tools/assets.rs:920` | Disposal preview debits: accumulated + proceeds + catch-up (+ loss) | Raw `+` chain | Yes | B-15, B-24 | a |
| `ledger-mcp/tools/documents.rs:938` | Σ outstanding on documents | `checked_add` | No | B-38 | a |
| `ledger-mcp/tools/helpers.rs:465/551` | Checked sum helper; preview balance check | `checked_add` | No | B-28 | a |
| `ledger-mcp/tools/payroll.rs:122` | Payroll tax payables total | `.sum()` raw | Yes | B-19 | a |
| `ledger-mcp/server.rs:1350` | Preview balance after a line: `before + delta` | Raw `+` | **Yes** | B-28 | a |
| `ledger-store/store.rs:2573-2574` | Depreciation runs at/through a month | `.sum()` raw | Yes | B-15 | a |
| `ledger-store/store.rs:2688-2717` | Disposal preview figures | `checked_*` | No | B-15, B-24 | a |
| `ledger-store/store.rs:3449/3744` | Invoice/bill total from document lines | `.sum()` raw | **Yes** (replayed lines are any size, B-28) | B-38 | a |
| `ledger-store/store.rs:4452/4503` | Payroll remittance total | `.sum()` raw | Yes | B-19 | a |
| `ledger-store/audit.rs:649` | Entry debits for the audit | `checked_add` | No | B-45 | a |

### 1.3 What the counts mean

- Class **(a)** is not "all of this must be deleted". Many sites are a single
  `checked_add`/`checked_sub`, which is already the kernel's `Amount`
  primitive; they can stay as call sites once the *shape* around them is a
  kernel function (e.g. `total`, `split_by_weights`).
- Class **(b)** should still converge on one shared rounding function
  (`ledger-import::report::format_amount` is already the best candidate for
  money text; `ledger-gui::fmt` should call it), so a display change never
  changes a booked figure and a booked figure never changes because of a
  display change.
- Class **(c)** is correctly outside the kernel: member shares are weights,
  not money; percentage templates and chart floats are UI.

---

## 2. Panics

A raw `Decimal` operator panics on overflow; `checked_*` returns `None`.
`rust_decimal`'s `+`, `-`, `*`, `/` and `+=` are the raw operators. The
engine's line cap (`kernel::MAX_LINE_AMOUNT` = 10^15, `MAX_LINE_SCALE` = 10,
rules.md B-28) applies to **new** lines; **replayed** journal lines are
accepted "of any size" (B-28), which is why the durable store must never use
raw arithmetic on restored figures.

### 2.1 Reachable from a public API with an edge-case value

| Site | Input path | Why it panics |
|---|---|---|
| `money.rs:135`, `money.rs:141` | `Money::new(amount, currency).checked_add(other)` — `Money` is public and exported; no range check in the constructor | Raw `Decimal +` / `-` at `Decimal::MAX` |
| `ledger-store/store.rs:3449`, `3744` | `Store::create_invoice`/`create_bill` with caller-supplied `DocumentLine`s, or a restored journal carrying oversized lines; raw `.sum()` before `arap::document_total` (which is checked) | Two restored lines near `Decimal::MAX` overflow the sum |
| `ledger-mcp/server.rs:1350` | `books.preview_entry`-style line preview: `before + delta` where `before` is the account balance and `delta` a line amount | A large existing balance plus a line near the cap |
| `ledger-mcp/tools/assets.rs:920` | `books.preview_disposal`: `accumulated + proceeds + catch_up + loss` | Each term is a stored/replayed figure; the sum is raw |
| `ledger-mcp/tools/assets.rs:842` | `r.amount - r.scheduled` (raw) on a run preview | Either side can be a replayed figure |
| `ledger-gui/pages/cash.rs:183-186` | Book with two months' `(i - o)` values near the cap | Raw subtraction inside the map |
| `ledger-gui/books.rs:2083-2091` | Party flow split `v * share / net` (checked) then `parts[0].1 += left` (raw) | Raw `+=` on the leftover correction |
| `periods.rs:155` (`allocate_last_absorbs`) | Replay of a legacy close with a huge journaled amount | Raw `amount * s` |

### 2.2 Raw sites that are practically unreachable but violate the bar

These are sums of line-scoped amounts. With each line ≤ 10^15 and `Decimal`
holding ~7.9 x 10^28, overflow needs ~7.9 x 10^13 lines: far beyond memory
and the journal format, but the code is still one mutation away from the bug
class. They should go through `kernel::total` anyway.

- `ledger.rs:6399`, `8418`
- `reports.rs:94`, `270-271`, `412`, `517-518`, `760-767`, `819-825`, `948`,
  `1316-1323`, `1354`, `1723-1724`
- `tax_capital.rs:594`, `827`, `758`, `784`, `485-502`, `597`
- `form1099.rs:765`, `768`, `770`, `774`, `779`
- `k1.rs:184`, `203`, `237`, `264`, `569`
- `tax.rs:522-524`, `530`
- `audit.rs:351`, `525`
- `cashflow.rs:408`
- `ledger-store/store.rs:2573-2574`, `4452`, `4503`
- `ledger-package/package.rs:295-297`, `538`; `vendor_1099.rs:196`
- `ledger-mcp/tools/reports.rs:43`; `tools/payroll.rs:122`
- `ledger-import/report.rs:278`, `522-523`
- `ledger-gui/books.rs:963`, `1932`, `2155`, `2232`

### 2.3 Divisions that panic on zero

| Site | Guard today |
|---|---|
| `reports.rs:630` (`contribution_margin_pct`) | Explicit `revenue <= 0` → `None` |
| `cashflow.rs:1721` (`split_part`) | `whole = ours + theirs`; caller ensures a party split, but `whole = 0` would make `checked_div` return `None` (safe) |
| `ledger-gui/pages/overview.rs:68/137` | `burn > 0` guard in the caller |
| `ledger-gui/pages/reports.rs:247/279` | `!b.is_zero()` / `income.is_zero()` guards |
| `ledger-gui/pages/cash.rs:183-186` | `months` non-empty guard |
| `ledger-import/templates.rs:118` | `equal_shares(n)` has no `n = 0` guard; check the caller before moving it |

---

## 3. Sweep units

Each unit adds kernel functions (generic over `Amount` or `Units`), a Kani
harness, a bridge property and a Lean law. Existing kernel functions are
named so a unit never adds a second copy.

### U0 (serial, first): module scaffolding so units do not collide

Nothing in `kernel.rs` is a module today; `proofs.rs` is one 2,089-line file
and the `_kani` recipe **greps that exact path**:

```sh
awk '/#\[kani::proof\]/ ...' crates/ledger-core/src/proofs.rs
...
args+=(--harness "proofs::$h")
```

A `kernel/` module does **not** disturb Kani: Kani compiles the crate as
usual, and `crate::kernel::foo` still resolves if `kernel/mod.rs` re-exports
each submodule. A `proofs/` module **does** disturb the recipe twice (the
grep path and the `proofs::$h` prefix), so U0 must also update `_kani`:

- `kernel.rs` → `kernel/mod.rs`, with `mod fx; mod round; mod allocate; mod
  total; mod schedule; mod ratio; mod payroll; mod control;` and
  `pub use ...::*;` for each. Existing 2,485 lines move unchanged into
  `kernel/entries.rs` (or stay in `mod.rs`; either keeps every path).
- `proofs.rs` → `proofs/mod.rs`; new harnesses live in `proofs/<unit>.rs`.
  `_kani`: scan `crates/ledger-core/src/proofs/mod.rs` plus
  `crates/ledger-core/src/proofs/*.rs`, and pass `--harness "$h"` (bare
  names, unique; verified against `kani_slow`, which stays a list of names).
- `tests/kernel_bridge.rs` → `tests/kernel_bridge/main.rs` plus
  `tests/kernel_bridge/<unit>.rs`; Cargo still builds one test target named
  `kernel_bridge`.
- `docs/verification.md`: add a "Kernel sweep" heading and a one-line policy
  that each unit's proof note is folded in by the integrator, not edited by
  every worker (see §3.10).

U0 is the only unit allowed to touch `kernel/mod.rs`, `proofs/mod.rs`,
`tests/kernel_bridge/main.rs` and `justfile`. It must run `just prove` once to
show the move changed no proof. It adds no harness and no theorem.

### U1: FX conversion (B-3) — **highest value: it books money**

```rust
pub enum Rounding { HalfAwayFromZero, HalfToEven, TowardZero, TowardNegativeInfinity }

/// foreign x rate / 10^quotes, rounded once to `step` (the base currency's
/// minor unit), exactly; None on overflow or a non-positive input.
pub fn fx_booked<T: Amount>(foreign: T, rate: T, quotes: u32, step: T) -> Option<T>
```

`ledger.rs::add_line_fx` calls it; `ledger.rs:1862-1882` disappears. Rule:
B-3 ("one money; foreign amounts at the transaction-date rate"; the DECISION
of 2026-10-13 that the booked base is a whole minor unit).

- Kani: `fx_booked_is_whole_unit`, `fx_booked_within_half_unit` (with
  `kani::cover!` for a nonzero remainder), `fx_booked_positive_for_positive`
  (bounded `i64` with a fixed quotes scale).
- Bridge: `kernel_bridge/fx.rs` — random `Decimal` foreign/rate, compare
  against a rational reference; edges `Decimal::MAX`, scale 28, half-unit
  ties, negative zero.
- Lean: new `Fx.lean`, `fx_booked_whole` and `fx_booked_near`; the rounding
  half reuses `Deductible.roundHA_*`.
- Files: `kernel/fx.rs`, `proofs/fx.rs`, `tests/kernel_bridge/fx.rs`,
  `ledger.rs`, `docs/verification.md` note.

### U2: one rounding primitive (B-3, B-21, B-32) — **the enabler**

```rust
/// `amount` to a whole multiple of `step`, in `mode`; None on overflow.
pub fn round_step<T: Amount>(amount: T, step: T, mode: Rounding) -> Option<T>
```

Every `round_dp*`/`rescale` on money becomes `round_step`. The engine-level
wrapper `round_minor(amount: Decimal, currency: Currency)` lives in
`money.rs` or `kernel/round.rs` and picks `10^-currency.minor_units()`.

- Kani: `round_step_is_nearest`, `round_step_is_idempotent`,
  `round_step_ties_away`, `round_step_never_negative_zero`.
- Bridge: exact table at .5, -0.5, 0.004, 1.005, scale 28, `Decimal::MAX`.
- Lean: `round_step_near` (`|result - amount| <= step/2`) and
  `round_step_scale`; existing `Deductible.roundHA_*` and
  `Prorate.roundUnit_*` carry the arithmetic.
- Files: `kernel/round.rs`, `proofs/round.rs`, bridge, plus the call-site
  replacements in `periods.rs`, `schedules.rs`, `form1099.rs`, `shares.rs`,
  `k1.rs`, `conventions.rs`, `tax_capital.rs`, `cashflow.rs`, `reports.rs`,
  and the display crates.

**Do U2 before U1/U5/U6**: they call it.

### U3: proportional shares and weighted splits (B-13, B-35, B-36, B-39)

```rust
/// The exact parts of `whole` by non-negative `weights`; None if a product
/// or the sum is not representable (never a rounded share).
pub fn exact_shares<T: Amount>(whole: T, weights: &[T]) -> Option<Vec<T>>

/// Split `whole` by `weights` to `step`, each part truncated (or rounded
/// per `mode`), the leftover units by largest remainder ([`apportion`]).
pub fn split_by_weights<T: Amount>(
    whole: T, weights: &[T], step: T, mode: Rounding,
) -> Option<Vec<T>>

/// One member's exact share `whole x weight / total` (unrounded).
pub fn share_of<T: Amount>(whole: T, weight: T, total: T) -> Option<T>
```

Replaces the six copies listed in §0. Leftover policies become explicit:
largest remainder (`apportion`), last absorbs (`plug_last`), first absorbs
(`shares::split_first`, a legacy policy that stays a separate named function
if replay needs it), and the cumulative one (`apportion_cumulative`).

- Kani: `exact_shares_sum`, `split_by_weights_sums`,
  `split_by_weights_within_one_step`, `split_by_weights_no_part_exceeds`.
- Bridge: `kernel_bridge/allocate.rs` over `Decimal`, with weights summing to
  1 and to arbitrary totals.
- Lean: `Apportion.apportion_sum`, `apportion_drift`, `allocW_*` already
  prove the core; add `exact_shares_sum` and `shares_of_near`.
- Files: `kernel/allocate.rs`, `proofs/allocate.rs`, bridge, `periods.rs`,
  `k1.rs`, `conventions.rs`, `shares.rs`, `form1099.rs`, `tax_capital.rs`,
  `parties.rs`, `cashflow.rs`.
- Caveat: `periods::allocate` and `allocate_last_absorbs` are **frozen replay
  rules** (their doc comments say so). U3 does not change their arithmetic;
  it names them and routes new work through the kernel. Any change to their
  cent policy needs a format/version decision, not a refactor.

### U4: collection totals and signed sums (B-21, B-28)

```rust
pub fn total<T: Amount>(values: impl IntoIterator<Item = T>) -> Option<T>
pub fn total_of<T, I>(values: impl IntoIterator<Item = I>, f: impl FnMut(I) -> T) -> Option<T>
pub fn signed_total<T: Amount>(lines: impl IntoIterator<Item = (Side, T)>) -> Option<T>
```

`kernel::totals`/`net` already exist for `(Side, T)`; `total`/`total_of` are
the missing shapes. This removes every `.sum()` and `x += signed` in §2.2
and gives the store's `document_total` one implementation.

- Kani: `sums_are_exact` exists; add `total_empty_is_zero`,
  `total_append`, `total_of_is_map_sum`.
- Bridge: `kernel_bridge/total.rs` over random `Vec<Decimal>`.
- Lean: `Basic.sumOver_add`, `net_append`; no new law needed.
- Files: `kernel/total.rs`, `proofs/total.rs`, bridge, and the call sites in
  `ledger.rs`, `reports.rs`, `store.rs`, `package.rs`, `mcp`, `import`.

### U5: schedule monthly-even plans (B-15, B-16)

```rust
/// `months` parts summing to `total`, all but the last equal to
/// `round(total / months)` to `step`; None for months = 0 or overflow.
pub fn even_parts<T: Amount>(total: T, months: u32, step: T) -> Option<Vec<T>>
```

`amortization_plan` calls it with the currency's minor unit. This is also the
fix for the currency-blind `round_dp(2)`. Because the plan is reproduced on
replay, the unit must either (i) keep the old cent rule for schedules whose
`created_seq` predates the change, or (ii) prove that the new rule gives the
same plan for every posted schedule (it does for USD; not for JPY/KWD). That
is a **decision for the owner**, flagged in the unit's PR, not silently
taken.

- Kani: `even_parts_sum`, `even_parts_all_but_last_equal`,
  `even_parts_last_absorbs`, `even_parts_within_half_step`.
- Bridge: `kernel_bridge/schedule.rs`, USD/JPY/KWD steps.
- Lean: `plugLast_total` + `Prorate.roundUnit_near`; add `even_parts_sum`.
- Files: `kernel/schedule.rs`, `proofs/schedule.rs`, bridge, `schedules.rs`.

### U6: percentages and ratios (B-14 r5, B-37)

```rust
/// part / whole x 100 to `step` in `mode`; None for whole <= 0.
pub fn percent_of<T: Amount>(part: T, whole: T, step: T, mode: Rounding) -> Option<T>
/// Split `amount` as a:b, one side rounded and the other the rest.
pub fn split_ratio<T: Amount>(amount: T, a: T, b: T, step: T, mode: Rounding) -> Option<(T, T)>
```

Serves the contribution margin (B-37), the cash flow government split
(B-14 rule 5), and the viewer's percentage columns (the latter still class
(b), but can share the function).

- Kani: `percent_of_within_half_step`, `percent_of_zero_whole_is_none`,
  `split_ratio_sums`.
- Bridge: `kernel_bridge/ratio.rs`.
- Lean: `Deductible.roundHA_*`; add `percent_near`.
- Files: `kernel/ratio.rs`, `proofs/ratio.rs`, bridge, `reports.rs`,
  `cashflow.rs`, `ledger-gui/pages/*` (display use only).

### U7: payroll (B-19 payroll section; out of proof scope today)

```rust
pub fn net_pay<T: Amount>(gross: T, deductions: &[T]) -> Option<T>
```

`payroll_entry_plan` keeps its account mapping but uses `net_pay` + `total`;
the plan is then checked by the existing `check_lines` before posting.

- Kani: `net_pay_is_gross_less_deductions`, `net_pay_nonnegative_when`
  (deductions ≤ gross), `payroll_plan_balances` (through `check_lines`).
- Bridge: `kernel_bridge/payroll.rs`.
- Lean: `Basic.net_*`; new `payroll_net_nonneg` and
  `payroll_plan_balances`.
- Files: `kernel/payroll.rs`, `proofs/payroll.rs`, bridge, `payroll.rs`.

### U8: control, aging and reconciliation totals (B-18, B-25, B-38)

```rust
pub fn aging_total<T: Amount>(current: T, b30: T, b60: T, b90: T, b90p: T) -> Option<T>
pub fn control_difference<T: Amount>(balance: T, documents: T, unattached: T) -> Option<T>
pub fn reconciliation_total<T: Amount>(
    beginning: T, built_in_gain: T, without_basis: T, distributed: T,
) -> Option<T>
```

Mostly uses U4; the value is naming the B-38/B-25 identities so they are
proved, not just summed.

- Kani: `aging_total_is_sum`, `control_difference_identity`,
  `reconciliation_total_identity`.
- Bridge: `kernel_bridge/control.rs`.
- Lean: `Documents.control_agrees`; add `reconciliation_total`.
- Files: `kernel/control.rs`, `proofs/control.rs`, bridge, `arap.rs`,
  `tax_capital.rs`, `ledger.rs`.

### U9: display rounding (class b; no Kani, no Lean)

One rounding function for presentation: `ledger-import::report::format_amount`
already has the exact two-place, no-negative-zero rule (B-33). Move the
rounding half into a small `ledger-import::format::round_display(amount,
places)` and make `ledger-gui::fmt` call it (or call `ledger-import`'s public
helper); route `pages/*.rs`'s `round_dp(1|2)` through it. This is not a
kernel law (display never reaches the books), so it is last.

- Tests: goldens and the viewer's `tests/view_model.rs`; no proofs.
- Files: `ledger-import/report.rs`, `ledger-import/pdf.rs`,
  `ledger-gui/fmt.rs`, `ledger-gui/pages/*.rs`, `ledger-package/text.rs`.

### 3.10 Order, value and conflict plan

**Value order** (money that reaches filed figures and the books first):

1. **U1 FX** — it writes the base amount to the books; the first formal pass
   already found a conversion crash.
2. **U3 proportional shares** — K-1/1065, 1099 and member closes; six
   independent copies today.
3. **U2 rounding** — the enabler for U1/U5/U6; the currency bug lives here.
   Build it first in time, but its standalone value is lower.
4. **U5 schedule plans** — books every month; fixes the JPY/KWD rounding.
5. **U4 totals** — books and every report figure; removes the raw-panic
   class.
6. **U6 ratios**, 7. **U7 payroll**, 8. **U8 control/aging**, 9. **U9
   display**.

**Conflicts.** All units touch `kernel.rs`, `proofs.rs`,
`tests/kernel_bridge.rs` and `docs/verification.md` today. U0 removes the
first three conflicts by giving each unit its own file and re-export. What
remains:

- `kernel/mod.rs` / `proofs/mod.rs` / `tests/kernel_bridge/main.rs`: only U0
  edits them (one `mod` line per unit, added by U0 before branches open).
- `justfile`: only U0 edits it.
- Call-site files shared by two units (`periods.rs`, `form1099.rs`,
  `tax_capital.rs`, `reports.rs`): different hunks; merge in value order and
  rebase the later unit. The EM serializes merges (plan D-60: one full gate
  per batch, not per merge).
- `docs/verification.md`: each worker writes `docs/proofs/<unit>.md` with its
  harness names, laws, bounds and measured time. The integrator folds those
  into the table and the "What is and isn't proved" section in one edit
  after the batch. Workers do not edit `verification.md` or `Audit.lean`;
  the integrator adds the `#print axioms` lines in one place.
- `docs/parity.md`: this sweep changes no feature status (it is a refactor),
  so no unit edits it. If U5's currency fix changes behavior, that is a
  separate `feat`/`fix` with its own parity and golden update.

**Recipe discipline.** Inside a unit: `cargo test -p ledger-core` (plus the
touched crate) and `cargo kani --harness <the unit's harnesses>`.
`just check` / `just prove` run once at the end of the batch, per D-60. U0
runs `just prove` alone because it changes the proof harness layout.

---

## 4. hledger as a second oracle

### 4.1 What the GnuCash harness compares today

`crates/ledger-core/tests/oracle_gnucash.rs`:

- Generates 200 books (seed `0x005e_ed0f_b00c`), 6 months of entries,
  foreign lines (`add_line_fx`), reversals one time in three, and period
  closes on every other book.
- `book_json` sends **base-currency amounts only** (`ln.amount.amount`), one
  account per line, debit/credit as a sign, plus the as-of dates.
- Compares every account's signed balance (`debit - credit`) at six month
  ends plus three random dates, and each month's P&L on books **without
  closes** (15,800 comparisons).
- `oracle/gnucash/books.py` builds an XML book through the GnuCash Python
  bindings and returns `{account: exact decimal string}` per date.
- `just oracle` builds `wanderbooks-gnucash-oracle` and runs the ignored
  test.

### 4.2 Capability matrix

| Comparison | hledger | How |
|---|---|---|
| Account balance at a date | Yes | `hledger -f book.journal balance -O csv --layout=bare --no-total date:..<as_of + 1 day>` (signed, debit-positive) |
| Trial balance | Yes | same, one row per account; filter zero rows to match `trial_balance_as_of` |
| Monthly P&L | Yes | `hledger incomestatement -p YYYY-MM`; or `balance --change -p` on income/expense accounts. **Only books without closes**, as with GnuCash: a close is an ordinary entry and zeroes income/expense activity |
| Balance sheet | Partly / derived | `hledger balancesheet` lists assets and liabilities and a "Net:" line but **omits equity accounts** (verified on 1.52.4 and 1.32.3); compare the underlying account balances from `balance` and derive the statement, as the GnuCash harness already does. Do not diff hledger's presentation |
| Foreign-currency lines | Not as booked base | The harness sends only base amounts, so hledger sees one commodity. If FX itself is to be compared, emit `100.00 EUR @@ $108.35` (total cost = wanderbooks' booked base) rather than `@ rate`, because `@ rate` keeps the unrounded exact product and `-B` would differ by the B-3 rounding. Then hledger's cost equals the booked amount by construction |
| Reversals | Yes | Ordinary opposite postings; nothing special |
| Closes | Yes as entries; no for monthly P&L | Same restriction as GnuCash |
| Zero/omitted accounts | Filter | hledger prints `0` rows; wanderbooks omits zero statement lines |

### 4.3 Journal mapping for what wanderbooks books contain

hledger's journal is line-oriented:

```text
commodity $1,000.0000000000

2026-01-15 FX purchase
    assets:cash             $-108.35
    assets:euro              $108.35
```

Mapping rules:

- **Account names:** use the code as the hledger account (`1000`, `1500`,
  ...), or a `code name` with a single space is ambiguous because hledger
  splits the amount at two or more spaces. Codes alone are safest; a
  `account 1000` directive plus a comment keeps it readable. Colons are
  hierarchy separators; a wanderbooks account name containing `:` must not
  be used raw.
- **Amounts:** exact decimal strings. A `commodity $1,000.0000000000`
  directive sets the display precision; it does not change stored values.
- **Signs:** debit positive, credit negative (hledger has no debit/credit
  side). `balance` then equals wanderbooks' `debit - credit`.
- **Foreign original:** either omit it (base-only, matching the GnuCash
  harness) or record it as `@@ total cost`; never `@ rate` if the harness is
  asserting the booked base.
- **Descriptions/dates:** `YYYY-MM-DD description`; no escaping beyond not
  starting a line with a digit that is not a date.
- **Reversals and closes:** ordinary entries, exactly as the store books
  them.
- **Balance assertions:** optional (`= $x`) and not needed; the harness
  compares balances itself, and an assertion failure would make hledger exit
  nonzero.

### 4.4 Version and pinning

- Latest release on 2026-11-08: **hledger 1.52.4** (release binaries
  `hledger-linux-x64.tar.gz`, static). Verified sha256:
  `f76a2e4088e162b7658e43077502569d5a6fdacd219b19bfb9aab6588b69643a`.
- Debian trixie ships **1.32.3-1+b2**; an apt pin mirrors the GnuCash
  Dockerfile's style but is two years behind and lacks newer flags.
- Recommended, mirroring `oracle/gnucash/`:
  - `FROM debian@sha256:<same trixie digest as oracle/gnucash>`
  - `ARG HLEDGER_VERSION=1.52.4`,
    `ARG HLEDGER_SHA256=f76a...`
  - download the official static tarball, `sha256sum -c`, extract
    `hledger` to `/usr/local/bin/hledger`, no apt package of hledger.
  - `COPY books.py /oracle/books.py`, `ENTRYPOINT ["python3",
    "/oracle/books.py"]` (same JSON-in/JSON-out protocol as GnuCash's
    `books.py`, so the Rust test can share `book_json`/`ours`).
- Do not install hledger on the host; the container is the only runtime, as
  with GnuCash.

### 4.5 Rounding and display precision (measured)

Probes in a throwaway `debian:trixie-slim` container, hledger 1.32.3 and
1.52.4:

- hledger stores exact decimals; `print` reproduces `$0.004` unchanged.
- Reports render at the commodity's **display precision**, inferred from the
  amounts or set by a `commodity` directive. With `$1,000.00` inferred, a
  balance of `891.656` prints `$891.66` and a `$0.004` expense prints `0`.
- `-c '$1,000.0000000000'` (`--commodity-style`) prints `891.6560000000`
  and `0.0040000000`; CSV output obeys the same style.
- **There is no `--precision` flag** in 1.32.3 or 1.52.4 (`Unknown flag`).
- `-B/--cost` values foreign amounts at their recorded cost; `--value=end`
  needs `P` market prices and otherwise leaves the commodity unconverted.
- Dates: `-e DATE` is **exclusive**; `date:..DATE` also excluded the end
  date in the 1.52.4 probe. For an inclusive `as_of`, use
  `-e <as_of + 1 day>` (or `date:..<as_of + 1 day>`).
- Parsing refuses an unbalanced transaction with a nonzero exit; wanderbooks
  entries always balance, but the harness should treat a parse refusal as a
  test failure with hledger's message, not as an oracle answer.

So the cross-check is exact **provided the harness sets the display style**;
otherwise a genuine sub-cent difference can hide behind `0`.

### 4.6 An hledger adapter in bookspec

The checkout at
`~/.cargo/git/checkouts/bookspec-fb51a087b3434606/74f2853/gnucash` is a
`bookspec-gnucash` crate implementing `bookspec_checker::Engine`:

- `load` builds accounts; `apply` appends an entry or a reversal (opposite
  postings); `query` supports only `trial_balance` and `balances`; `change`
  is unsupported; `claims()` lists Entry 1-3, Ledger 1 and 3, Statement 1 and
  2 (seven rules whose examples ask only for balances).
- It shells out to `docker run --rm -i wanderbooks-gnucash-oracle` and reads
  JSON.

An `hledger` adapter is a smaller version of the same thing:

- Keep an in-memory journal text; on `query`, run
  `hledger -f - balance -O csv --layout=bare --no-total -c
  '$1,000.0000000000' -e <as_of + 1 day>` with the journal on stdin (or a
  temp file).
- Map hledger's CSV `account,commodity,balance` back to codes; for
  `balances`, flip non-normal-debit accounts exactly as the GnuCash adapter
  does; pad to two places as it does.
- It can claim **every rule the GnuCash adapter claims**, because those
  examples are balanced single-currency entries and reversals. It could also
  claim multi-currency examples (the GnuCash adapter refuses a non-USD
  `setup.currency`; hledger does not), if bookspec has any.
- It cannot support `ChangeSpec` any more than the GnuCash adapter does
  (hledger has no rename/rewrite command in this flow), and it refuses
  unbalanced entries where GnuCash books an `Imbalance` line; bookspec's
  claimed rules do not exercise that difference. Record it as a known
  limitation, as the GnuCash adapter records GnuCash's imbalance behavior.
- Clean-room: hledger is GPL-3; use it as an external process only, never
  copy its code. Same boundary as GnuCash (AGENTS.md non-negotiable 5).

### 4.7 Suggested integration (not built here)

1. Add `oracle/hledger/Dockerfile` and `oracle/hledger/books.py` (JSON in,
   `{"balances": {date: {account: "..."}}}` out, same schema as GnuCash).
2. Add `just oracle-hledger` mirroring `just oracle`; keep `just oracle` as
   the GnuCash recipe or make it run both (a new name avoids a breaking
   change for anyone using the old one).
3. Add `crates/ledger-core/tests/oracle_hledger.rs` with the same generator
   (`mod` the shared generator out of `oracle_gnucash.rs`, or copy the ~60
   lines) and the same comparisons; assert `diffs.is_empty()` with the same
   "adjudicate from rules.md" message.
4. Update `docs/oracle.md` Layer 5 from "(live: GnuCash)" to two oracles, and
   `docs/parity.md` only if the harness changes a feature status (it should
   not: it adds evidence).
5. First run: expect zero differences if the display style is set; if a
   difference appears, adjudicate it from B-3/B-21 (rounding) or B-1/B-5
   (sign) before touching either side.

---

## 5. What was not done

- No code, no kernel changes, no golden changes.
- No `just check`, `just prove`, mutation, fuzzing, or full test run (the
  brief forbids them).
- hledger was run only in throwaway containers; nothing was installed on the
  host.
- The inventory is grep-assisted and read in context, but a line number can
  drift if this branch is rebased; the file/function names are the durable
  references.

---

## 6. Questions for the owner (decisions the units must not take silently)

1. **U5 replay compatibility.** `amortization_plan` re-derives posted months
   on replay. Fixing its 2-decimal rounding to the currency's minor unit
   changes JPY/KWD schedules already in a journal. Options:
   (a) **Recommended:** version the rule (new schedules use the currency's
   minor unit; schedules created before a `schedules_v2` feature replay the
   old cent rule), or
   (b) keep cents for every schedule and document the JPY/KWD limitation, or
   (c) refuse non-2-decimal base currencies for monthly-even schedules.
2. **U3 legacy policies.** `periods::allocate` (largest remainder) and
   `allocate_last_absorbs` (round each, last absorbs) are both frozen replay
   rules. Confirm that U3 may only *name* them (no arithmetic change) and
   that new closes keep using `allocate_cumulative`.
3. **hledger pin.** Official static 1.52.4 (**recommended**) vs Debian
   trixie's 1.32.3 apt package. The former needs a downloaded binary and a
   pinned sha256; the latter matches the GnuCash Dockerfile's apt style but
   is older and its display/flags were the ones probed for the caveats above.
