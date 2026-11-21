# P15: the app is the host, and web assistants reach it through a relay

Status: **design, not started** (2026-11-20). Nothing here is built except
the throwaway trial server (`~/Code/wb-connect-trial`, outside this repo).
The owner's decisions below were given on 2026-11-20 through the question
tool; the rest is the lead's proposal and is marked so.

This plan changes how the product runs. It reverses D-21 and D-23 (the
books are open while a lease lasts, with no window needed) and narrows
D-22 (separate processes per role) for the desktop app; it answers Q-6
and replaces D-7's preference for an external identity provider on the
desktop; D-44 (the bridge is the only local agent path) stands.

## 1. Why

Packaging for Windows and macOS is next, and the hard part of packaging
is the detached background host: it outlives the window, scrubs its
environment, restarts itself onto a replaced file, and needs separate
handling per system (the AppImage rule of 2026-11-10, macOS
translocation, no updater on Windows, D-62). A person who installs a
bookkeeping program expects one window that opens companies. And the
people this is for use chat products in a browser, not a terminal.

## 2. What the owner decided (2026-11-20)

| Id | Decision |
|---|---|
| D-77 | **The app is the host.** Starting Wanderbooks opens the books; agents can reach a company only while the app has it open. Bookkeeping stays MCP-only: the window reads, and its only writes are company-file actions (new, open, close, back up, restore, upgrade), which exist today as `wanderbooks books …`. |
| D-78 | **One top-level tab per open company.** A company is reachable exactly while it has a tab. A connection names its company and is never redirected to another. |
| D-79 | **Closing the window with assistants connected warns, then closes.** No tray icon. Minimising is how the app stays out of the way. |
| D-80 | **Web assistants reach the app through a relay we run on `wanderbooks.link`, encrypted end to end**: the app holds its own certificate and key; the relay routes by name and cannot read traffic. The relay is written in Rust, bounded in memory, and sized for hundreds of thousands of idle apps on one modest host. |
| D-81 | **First release promises Claude and ChatGPT on the web**, with tested steps. Gemini, Grok and muse.ai get a generic path on the same address, labelled untested. |
| D-82 | **Addresses are four words from a curated finance and accounting list** (`ledger-accrual-tally-mint`), never a word a customer would not want in an address. |

Also decided by the owner on 2026-11-20:

| Id | Decision |
|---|---|
| D-83 | **The app is its own sign-in server for web assistants** (OAuth 2.1), and a new assistant is approved in the app's window, by whoever is at the machine. No external identity provider, no passkeys, no account with us. (Q-6 closes for the desktop app; the hosted service of section 11 of `served.md` may still choose an external provider.) |
| D-86 | **Web assistants get read and draft by default**; posting is a permission the owner turns on per connection in the app. |
| D-87 | **Each app holds its own certificate** (kept after the owner weighed one wildcard at the relay; section 5 says why and what it does not promise). |

Proposed by the lead (engineering choices the owner has not objected to):

| Id | Proposal |
|---|---|
| D-84 | **The books server stays its own process, a child of the app.** The app takes the host's job (start the server from its own executable, close it by closing its input); it does not run the server on a thread. |
| D-85 | **The relay keeps one small durable table**, name to the app's public key, and nothing else durable. The routing table lives in memory and is rebuilt as apps reconnect. |

## 2a. Every write is one transaction, exactly once (D-88)

The owner, 2026-11-20, on a write whose answer is lost and which is then
retried: "Writes have to be TRANSACTIONAL ALWAYS! You can record a failed
transaction." Decided: **D-88, every write through the tools is one
transaction that happens exactly once.**

What the code does today (read 2026-11-20 by a worker; file and line in
`docs/delegation-log.md`'s entry of that day):

- One journal line is all or nothing (appended and fsynced, then memory,
  then SQLite). But a tool call may append several lines with nothing
  joining them: an invoice writes its entry's events, `invoice.created`,
  `receipt.attached` and `attachment.linked`; a settlement may add an
  account, the entry, then one event per document; a draft with a party
  is `entry.created` then `entry.party_set`. A crash between them leaves
  part of a call in the books, and it replays as if whole.
- Nothing identifies a call. The envelope is `{seq, ts, type, payload,
  prev}` and is frozen (WB-format-1). The access log's request id is per
  session, so a retry looks like a new call.
- About 25 to 30 write tools make something new on every call (drafts,
  lines, parties, payroll runs, payouts, statements, attestations), so a
  retry after a lost answer records twice. Fifteen confirmed tools refuse
  a repeat through the preview digest but cannot return what the first
  call answered. The bridge already tells an agent that a cut-off call
  "may have run", which is exactly this case.
- What a write returns is not a function of the books afterwards
  (balances before and after, today's outstanding, next steps), so the
  original answer must be kept, not recomputed.

Landed 2026-11-21: the law is bookspec law 47 (`lean/Bookspec/Calls.lean`),
pinned at bookspec rev 5127b5b9b110a8d459a5f33a78e6986344df651e.

The rule:

1. **A call's lines commit together or not at all.** A call's events are
   followed by one closing line naming the call, its first and last
   sequence numbers, and its outcome. On opening the books, lines after
   the last closing line that belong to no closed call are a call that
   never finished: they are set aside as a torn tail is today, recorded,
   and not applied. No state is ever built from half a call.
2. **Every write call carries a request key**, required by the tool's
   schema. A write without one is refused. The key and the answer that
   was given are in the closing line, so replay rebuilds the table of
   keys from the journal alone.
3. **A repeat of a key** (same book, same token) with the same arguments
   returns the original answer and writes nothing; with different
   arguments it is refused as a misuse of the key. The access log records
   it as a repeat.
4. **A refused or failed write is on record** (the access log, as now),
   and where a call got as far as writing lines before failing, its
   closing line says it failed and its lines are not applied.
5. Lines written before the books declared this feature have no closing
   lines and stand as they are: posted is forever.
6. **D-89 (owner, 2026-11-20): books that have not declared the feature
   take no writes.** They open and are read as ever; every write is
   refused with the upgrade remedy until the owner gives the format
   upgrade its go-ahead. So there is one way a write can happen, the one
   that is proved. This is stricter than WB-format-3's "everything that
   needs nothing new is written as ever" and replaces it for writes from
   the release that carries this feature.

It is a journal feature (a new line type and a new payload field), so it
follows `served.md` 8.0.5 R-A and R-B and 8.0.6: an old-books fixture,
the `is_its_own_feature` test, replay arms, the fuzz types, and a Lean
law in bookspec for "a replay of any prefix that ends inside a call
equals the replay of the prefix before that call". Unit **T1**, built
before the remote door (B1); it does not wait for the relay.

## 3. The shape

```
 Wanderbooks (one window, a tab per company)
   ├─ starts ─ books server (child process; every open company)
   │             ├─ run/mcp.sock        local door: the stdio bridge
   │             └─ 127.0.0.1 (off unless web is on): the remote door
   └─ relay client ── dials out ──▶ relay.wanderbooks.link:443
                                         ▲
   Claude Code / Codex ─ `wanderbooks mcp --book X` ─▶ run/mcp.sock
   claude.ai / ChatGPT ─ https://<secret>.<four-words>.wanderbooks.link
```

### 3.1 The app and the server (D-77, D-84)

What the code is today (read 2026-11-20, `ledger-host/src/host.rs`,
`start.rs`, `ledger-mcp/src/local.rs`, `ledger-gui/src/lib.rs`): four
processes when an agent is connected and the viewer is open; the viewer
reads the journal itself through a read-only snapshot and already links
the host and server crates; the server opens every book in the data root
at start and has no way to close one book; `local::run` is a process
entry point (argv, `server.lock`, signals, stdin), not a library call.

The change:

- The app starts the server as a child, from its own executable, and
  closes it by closing its input, exactly as the host does now
  (`host.rs` INVARIANTS 3 and 10). The server's closing rules stand:
  running calls finish, nothing is killed, the backup at close runs (up
  to 60 s). The close dialog shows that wait.
- Why a child and not a thread: a crash in the window or its graphics
  driver then closes the server's input and the books close cleanly;
  the server's code and its tests (`local_server.rs`, `startup.rs`,
  `http_concurrency.rs`) stay as they are.
- New in the server: **open and close one book** at run time (admin
  requests), so a tab opening or closing is a book opening or closing.
- New in the server: **who is connected** (an admin request listing
  sessions: client name and version, the token's name, in-flight calls,
  the last tool and when). The lease no longer carries this.
- The detached host, the lease protocol, the 10 s close delay, the
  environment scrubbing, the restart onto a replaced file and
  `update.lock` are not used by the desktop app. `wanderbooks serve`
  keeps a host for the headless build (P9a) until that is redesigned.
- `wanderbooks mcp --book X` stays (D-44) as a thin bridge to
  `run/mcp.sock`. With the app closed, or the company not open, it
  answers the agent in words: "Wanderbooks isn't running" or "Company X
  isn't open: open it in Wanderbooks".
- Update becomes an ordinary app update (an installer on Windows and
  macOS, replace and relaunch on Linux). The translocation refusal goes
  when the bridge no longer needs the app's path to start a host; it
  still needs a stable path to itself, to be settled in the packaging
  unit.

### 3.2 Tabs (D-78)

`App` holds 13 per-company fields of 17 (`app.rs:130-161`); they move
into a `CompanyView` (loader, books, nav, page state, receipts, export
menu, owner menu, dock, welcome). `owner::OWNER` is a process global and
becomes one per tab. Persistence gains per-company keys. 65 test sites
construct `App::new`; they move to `CompanyView` through `test_support`.
If this refactor runs long, the tab bar ships limited to one tab and the
limit is lifted after; nothing else in this plan changes.

The File menu: New company, Open (the data root's list and recent),
Close company, Back up now, Restore, and the format upgrade, each
calling what `wanderbooks books …` calls today.

### 3.3 What the person sees (the visual language)

The word "MCP" appears nowhere in the app. The word is **assistant**.

- **Every tab carries one of three states**, as a shape and a word, not
  colour alone: *No assistant connected*, *Connected* (with the name:
  "Claude Code", "Claude on the web"), *Working* (a call is running).
  Beside it, the last thing done: "posted entry 412, 10 seconds ago".
- **Where a person looks for a button that edits the books** ("New
  entry", "Add invoice") there is **Ask your assistant**, which copies a
  ready sentence to paste into the assistant. A banner saying
  "read-only" is not the design; it is ignored.
- **Connect** is a page per company:
  1. *On this computer*: a row per tool found (Claude Code, Codex,
     Gemini CLI, Grok Build, Muse Code, Cursor) with one button that
     registers the bridge itself (what `connect` does today for two of
     them); a row "Claude desktop app" that opens a one-click extension.
  2. *On the web*: a switch "Allow web assistants", then the address
     with Copy, and three numbered steps for Claude and for ChatGPT; an
     "Other assistant" entry with the same address and, for products
     that only take a key, a key made on the spot.
  3. *Connections*: every assistant that may connect, its state, when it
     last did anything, what it may do, and **Revoke**.
- **A new web assistant is approved in the window**: "Claude on the web
  wants to connect to Wanderland Labs. Allow?", with what it will be
  able to do. Nothing on the web page can approve it.

### 3.4 What an assistant is told

Today the server's opening instructions name the company and the role
("the double-entry books of X. You are their bookkeeping agent. Call
books.start first", `handbook.rs:246-300`), and `books.start` returns
the company's rules, conventions and open items. ChatGPT uses only the
first 512 characters of that text, and claude.ai is reported to drop it
(one issue, 2026-03, unverified since). So: the first 512 characters
stand alone, and the company's name and "call books.start first" also
go into the description of `books.start` and of the tools a model
reaches for first. A test pins both.

## 4. Sign-in for web assistants (D-83)

The server gains a remote door (the existing router and
`/b/{book}/mcp`, `local.rs:1734`) that is its own OAuth 2.1
authorization server. Rules, each from someone else's bug (sources in
section 8):

1. Protected resource metadata and authorization server metadata at
   every location the specification allows for a resource with a path;
   every address in them is the one public address, never built from the
   incoming request (Home Assistant core #153820 broke claude.ai for a
   year on this).
2. Three ways for a product to introduce itself: client ID metadata
   documents (advertised), dynamic registration, and a client id and
   secret shown in the app for products that make the person type them
   (Gemini's one reported success). Redirect addresses are checked
   against the products' known ones; a registration may carry six or
   more.
3. PKCE with S256 only; the `resource` parameter checked; tokens issued
   for this company's address only; codes single-use and short-lived;
   302 after consent.
4. Every unsigned request is refused with a 401 and the pointer to the
   metadata, the first `initialize` included (Grok otherwise shows
   "connected" and never signs in).
5. No per-session memory on the remote door, and plain JSON replies,
   never a stream (both cured failures behind tunnels). The local door
   keeps its sessions.
6. Every new sign-in is asked in the window (D-90): an approval belongs to
   one sign-in, never to a product, and is never reused. The app can tell
   which product asks (the redirect host is public), not whose account, so
   a stranger with the secret address who registers a client with
   claude.ai's callback must not be handed a code because Claude was
   approved before. Every Allow makes a new approval with exactly the
   permissions ticked; revoking ends its tokens at once. Each waiting
   request has a short code shown on the browser's page and on the card,
   to be matched, and the window says where the assistant is sent back to
   in neutral words, never that it is a known product. Access tokens are
   short; refresh tokens are long and sliding, and refresh within a
   sign-in is unchanged: a refresh after the app was closed for days still
   works. (Claude and Grok register anew on most sign-ins, so they are
   asked each time; that is the price of the secret address not being
   enough.)
7. Never ban an address for failed sign-ins: the vendors' addresses are
   shared. Rate-limit per approval instead.
8. The relay passes the real caller's address to the app, and the app
   never treats a relayed request as local (Home Assistant's advisory
   GHSA-wx3j-3v2j-rf45). The door gives a per-start secret to the app over
   the admin socket (`proxy_secret`, in the answer of `remote_start`), and
   refuses any connection that does not begin with it (the 64 characters,
   as given, before the PROXY header), so only the app's relay line can
   state a caller's address. Unit B3 writes exactly that first.
9. A key for products that take only a key: made in the app, named,
   revocable, shown once, with the same permissions model.
10. Read tools carry the read-only mark, so products ask before each
    write.

Permissions (D-86): the scopes exist (`books:read`, `write`,
`post`, `admin`, `auth.rs`). A web approval grants read and write
(drafts) and not post or admin; the owner can raise it per connection.
An assistant that also reads email can be steered by what it reads;
keeping posting off by default is the guard that does not depend on the
vendor.

## 5. The relay (D-80, D-82, D-85)

The pattern is Home Assistant Cloud's (SniTun), with Tailscale Funnel's
split of keys and Plex's certificate naming. We write it; we copy no
GPL or AGPL code.

**What it does.**
1. The app dials out to the relay on 443 and keeps one multiplexed line.
   It proves it holds the key that owns its name.
2. `*.wanderbooks.link` points at the relay. For each inbound
   connection the relay reads the name in the first, unencrypted
   message, finds that app's line, opens a stream on it, writes the
   caller's address, then pipes the encrypted bytes both ways. It holds
   no certificate for any app and cannot read or alter the traffic.
3. The app gets its own certificate from Let's Encrypt by the DNS
   method: it makes its key and the request; the relay's service
   publishes the one proof record for that app's name and nothing else
   (the acme-dns pattern). The relay never proves a name over the tunnel
   itself, which would let a dishonest relay obtain a certificate.
4. With the app closed the relay can only refuse the connection: it has
   no certificate with which to show an error page.

**Why a certificate per app, and what it does not promise.** The owner
asked (2026-11-20) whether one wildcard certificate at the relay plus
our own encryption to the app could replace a certificate per user. It
cannot be end to end: the far end is the vendor's server, which speaks
only HTTPS to a publicly trusted name, so a relay holding the
certificate reads everything whatever runs behind it. The owner kept
the certificate per app. Its limits, stated plainly:

- We control the domain's DNS, so a relay service that was taken over
  could obtain a certificate of its own for a user's name and intercept.
  That is visible in the public certificate logs: the app watches them
  for its own name and warns the owner of a certificate it did not ask
  for. Detection, not prevention; Home Assistant Cloud and Tailscale
  Funnel are the same.
- The AI vendor reads the books either way.
- A second free authority that speaks the same protocol is configured
  as the fallback, so one authority's outage or limit does not stop new
  users.

Where both ends are ours (the app's line to the relay) there is no
authority: the line is encrypted and authenticated with the app's own
key, pinned in the relay's table (D-85).

**The relay is also the domain's DNS server.** A small authoritative
server for `wanderbooks.link`, part of the relay service, answers two
questions: any name under the domain is at the relay's address, and the
proof code for a name is the one its app just handed in (held in memory
for minutes). Nothing is stored per user. A hosted DNS provider with an
API was set aside: one more outside dependency, thousands of stored
records, and a catch-all record stops covering names beneath any label
that has a record of its own (RFC 4592; to be shown by a test), so each
user would need a wildcard record as well. The cost: DNS must stay up,
on at least two name servers, and is a public service to protect
against abuse.

**Is a certificate per installation a permitted use?** Checked
2026-11-20 with quotes (a worker's reading; the lead has not re-read
the pages). Tailscale's documentation: each machine's certificate is
requested "using Let's Encrypt", Tailscale publishes the record for the
DNS challenge, and the keys are "generated and stored locally on your
machine". Home Assistant Cloud's source does the same per instance.
Plex has issued from Let's Encrypt since 2021. Let's Encrypt's
subscriber agreement asks only that the subscriber hold its own key,
which the app does. Its rate-limit page offers a higher limit to "a
large hosting provider or organization working on a Let's Encrypt
integration", taking "a few weeks"; a staff reply to a vendor with
50,000 devices called that case "a good candidate for a rate limit
override". One vendor's request was denied in 2023, so the request is
made for the registered domain, from one account, with the use
explained, and it is not guaranteed. Until it is granted: 50 new
installations a week. Fallbacks that speak the same protocol and are
free: Google Trust Services, ZeroSSL (limits not confirmed).

**Authorities are data, not code** (owner, 2026-11-20: Let's Encrypt
must be swappable for Google Trust Services or ZeroSSL). Every
authority we would use speaks the same protocol (ACME, RFC 8555). The
app's certificate code knows that protocol and nothing about any one
authority. At each issuance it asks the relay service for the list to
try, in order: for each authority its directory address, whether it
needs an account binding, and if so a binding credential the relay
service obtained for this app. Choosing among the authorities the app
knows, and their order, is a change to that list on our server, with no
new release of the app. **Adding a brand-new authority needs a release**
(B3 review, 2026-11-21): the app sends requests only to directory hosts
compiled into it (Let's Encrypt production and staging, Google Trust
Services, ZeroSSL, SSL.com: `DIRECTORY_HOSTS` in
`crates/ledger-link/src/acme.rs`, each host from the authority's own
documentation), because a relay that could name any host in the list
could make every app send requests to a server of its choice. For the
same reason the app believes a certificate only if it chains to the
public roots it ships, for its own wildcard, and lives no longer than
100 days; the relay's list can choose among authorities but cannot make
the app trust a new one. What
differs between authorities beyond the protocol (renewal information,
lifetimes, limits) sits behind one small trait. The tests issue from a
local test authority configured two ways, one with an account binding
and one without, and fail over from the first to the second, so the
swap is exercised and not only designed.

**Names.** Each app has a public name of four words (D-82) and asks for
a wildcard certificate under it; the endpoint is a long random label
beneath (`<secret>.<four-words>.wanderbooks.link`). The certificate logs
then show only the wildcard. The words are for recognition; the secret
label stops listing and guessing; the sign-in is the lock. One name per
installation; each company is a path under it.

**What the label does not hide, and who can read what** (B3 review,
2026-11-21). The secret label is in the caller's TLS hello (the server
name) and in the DNS queries that find the address, so the relay, the
vendor's resolvers and the network path all see it. It is not a secret
from them; it stops listing and guessing, and the sign-in is the lock.
The relay cannot read the traffic (its certificate is not there to read
it with). The assistant's own company (Anthropic, OpenAI and the like)
reads what it is sent, which is what the person lets that assistant read.
The Connect page says these two things and not "nobody can read what
passes through".

**State (D-85).** Durable: name, public key, created, blocked. In
memory: name to open line. Transient: proof records, minutes each.

**Bounds, as acceptance tests.** `#![forbid(unsafe_code)]`; a cap on
streams per app, on bytes buffered per stream, on handshake time and on
idle time; the parser of the first message fuzzed; a soak test with
100,000 idle lines and churn in which memory stays flat; the memory per
idle line measured before the transport is chosen (yamux over TLS, or
QUIC).

**Operating it.** Let's Encrypt issues 50 new certificates a week for a
domain: a cap on new users at launch. The request for a higher limit
takes weeks and goes in as soon as the relay runs. Certificate lives
fall to 64 days in February 2027 and 45 in 2028, so the app renews on
waking, with the renewal information the authority publishes. A listing
on the Public Suffix List is asked for on isolation grounds and may be
declined.

## 6. Units, in build order

| Unit | What | Size (guess) |
|---|---|---|
| A1 | Server: open and close one book at run time; the sessions request | medium |
| A2 | App starts and stops the server; the bridge talks to the running app and says so in words when it cannot | medium |
| A3 | Tabs: `CompanyView`, one owner per tab, per-company persistence; the File menu | large |
| A4 | The three connection states, "Ask your assistant", the Connect page (local rows, connections list, revoke); the close warning | medium |
| A5 | Retire the detached host on the desktop: move or retire `open_while_held.rs`, `update.rs`, the AppImage and translocation tests, each named with what replaces it | medium |
| B1 | The remote door: sign-in server, approvals in the window, keys, the remote permissions | large |
| B2 | The relay service: router, registry, the proof-record service, its DNS | large |
| B3 | The app's side: the line to the relay, certificates, the web half of the Connect page | medium |
| C1 | Packages for Windows, macOS and Linux on the new shape; the Claude desktop extension | medium |

A1 to A5 need nothing from B. B2 needs a server and the domain's DNS in
our hands. Every unit follows `AGENTS.md`: spec blocks cite a rule
(`WB-…` product rules for all of this), tests come with the code, the
gate runs once per batch.

The first release to test users (the owner, 2026-11-21): it waits for
the relay. Nothing is handed to a test user until web assistants work on
day one, so every unit in this table is built before the first installer
goes out. Order from here: A2 and T1 together, then A4, A5, B1, B2, B3,
C1.

Every new sign-in is asked in the window (the owner, 2026-11-21,
D-90). The app can tell which product asks, not whose account, so an
approval is never reused for a new sign-in: the secret address alone
never opens the books. This replaces rule 6 of section 4 where it says
an approval survives a product registering again; refresh within a
sign-in is unchanged.

## 6a. Follow-ups, written down so they are not lost

From the reviews of unit A1 (the server opens and closes one book),
left out of its last round on purpose:

- A crash between a restore's two renames leaves the book only in
  `legacy/`; the next start lists it as absent and neither repairs nor
  reports it (`ledger-store/src/root.rs`, the placement).
- After a normal git exit its process group is signalled by an id whose
  leader was already reaped (`ledger-store/src/backup.rs`); unreachable
  in practice on Linux, wrong in principle.
- The adopt's source-lock same-file check and the archive's lock removal
  lose their swap guard off Unix.
- `run/books.lock` (`ledger-store/src/root.rs`) is a flock with the same
  exposure the store's writer lock had: a child forked for another job
  holds a copy until it execs. The writer lock was fixed on 2026-11-20
  (`ledger-store/src/lock.rs`); this one was left.
- `books.import_bank_csv` journals its rows one by one and cannot be
  stopped by a close; D-88's unit makes an import one transaction.
- The scenario test for "closing A never delays B" shows B's jobs finish
  unstopped, not that they were undelayed.

From the review of unit B2 (the relay service):

- The relay never serves a shared account-binding secret: a binding
  comes only from a per-app source, which is not built. Where section 5
  above speaks of a binding in the authority list, this is what stands.
  An authority that needs one is served as not usable until then.
- An attacker with many networks can still spend the weekly certificate
  budget (charged at a name's first publish) or fill the hello pool. A
  raised Let's Encrypt limit and a second authority are the answer.
- yamux 0.14 fixes the stream window at 256 KiB, so a hostile app can
  make its line hold up to 3.5 MiB; the stream count was cut to 8
  callers per app to bound it. Measured idle cost: 15.4 KiB per line.
- The relay's own certificate is loaded from files, with no renewal.
- The 100,000-line soak has not been run; 20,000 is the largest.
- Unit B3's duties: ping at least every 60 s (a line silent for 180 s is
  dropped); a secret label of 25 or more random base36 characters; a
  yamux stream limit at least the relay's. Done in `crates/ledger-link`
  (ping every 30 s, a 32-character label, 64 streams).

From unit B3 (the app's side of the relay), left out on purpose:

- Certificate transparency watching is not built. Section 5 says the app
  watches the public certificate logs for its own name and warns of a
  certificate it did not ask for; it names no log service, API or timing,
  and the unit's brief left it for later if the plan left it open. It needs
  a log monitor choice (crt.sh and the like are third-party services the
  app would call with the app's name), so it waits for the owner's word.
  Until it is built, a relay that misuses its DNS control to get a
  certificate for a user's name would not be detected.
- The steps for ChatGPT, Gemini, Grok and muse.ai are not written: the plan
  has none it stands behind (section 7: OpenAI's two pages disagree), so
  the page says "we haven't tested this yet". The steps for Claude on the
  web are the ones in `plan/served.md` (Settings, Connectors, Add custom
  connector, paste, sign in); nobody has tried them on the live product.
- "Other assistant": a key made on the spot for products that take only a
  key (section 3.3 and `remote_make_key`) is not reached from the page yet.
- The line has no way to learn that the books server's door is gone while
  the window is still open (the server exited): callers are turned away,
  and the window's own notice says the server is down.
- ACME against a real authority, and a real relay, are untried: the tests
  run the real `instant-acme` client against an authority written in the
  test, and the link against the relay crate in-process.
- Several companies share one address; a new address ends the sign-ins of
  all of them (one issuer), and the approvals listed under "Who can reach
  these books" for the old address stay listed until revoked.
- A first run for the window with no server asks nothing and opens nothing.

From the review of unit A2 (the app starts and stops the server), left
for unit A5 or later:

- `commands::close_books` (the "Close the books" menu item) still asks a
  host lease socket, which the app does not have.
- `wanderbooks books …` commands still start a detached host when the
  app is closed; the `run/app-hosted` marker is permanent.
- A command arriving while the app's server is still opening large
  companies finds no admin socket yet and gets a connection error.
- On Windows the app's child has no job-object breakaway, so a launcher
  job with kill-on-close would kill the server mid-backup.
- A terminal Ctrl+C reaches the child directly on Unix; untested.

From the review of unit A3a (tabs): the identity used to tell two sets of
books apart is the hash of the journal's first line, which carries only a
currency and a time to the second; D-88's unit should give every set of
books a unique id at creation.

## 7. What is not known

- **muse.ai.** No report of a self-hosted server working with it;
  guides disagree on how a custom connector is added. Ten minutes with
  the owner's account against the trial server settles it.
- **Claude on the web asks some people to reconnect daily** (open
  reports, the newest 2026-11-16, unanswered). It may not be ours to
  fix.
- **ChatGPT Plus and write tools**: OpenAI's two pages disagree.
- **Memory per idle line**, so the size of the relay's host.
- The research behind sections 4 and 5 is the workers' reading of public
  pages on 2026-11-20; the lead has not re-read the sources.

## 8. Sources

Home Assistant: `NabuCasa/hass-nabucasa` (`remote.py`, `acme.py`),
`NabuCasa/snitun` README, core `auth/login_flow.py`, core issue #153820,
home-assistant.io issue #47091, advisory GHSA-wx3j-3v2j-rf45. Tailscale:
KB 1153 and 1223, the Funnel launch post. Plex: Filippo Valsorda's
write-up (third party). acme-dns: its README. Let's Encrypt: rate
limits, the 64-day and 45-day announcements. Public Suffix List: the
guidelines. MCP authorization specification, revision 2026-07-28.
Vendor pages: Claude custom connectors and connector authentication,
OpenAI developer mode and custom MCP server guides, Google's connected
apps OAuth guide, xAI's connector and tunnelling pages. Field reports:
anthropics/claude-ai-mcp #131, #228, #840, #1237; anthropics/claude-code
#48557, #100527; homeassistant-ai/ha-mcp #1787; the Grok and Gemini
issues named in the research of 2026-11-20.

## 9. Backups without git (D-91, D-92; the owner, 2026-11-21)

Decided: git leaves backups entirely (D-91). A backup is a plain folder
the person picks (another disk, or a folder Dropbox, iCloud, OneDrive or
Google Drive already syncs), plus one backup file on request. A backup
can be protected with a passphrase, offered at set-up and off by default
(D-92). Built before the first test users. The owner's words: "an
average user doesn't want to hear anything about git at all".

What stays exactly as it is: what a backup holds (the journal, the
stored files, the vault, the keys, the access log; never the SQLite
projection, which a restore rebuilds); when a backup runs (at open,
after writes go quiet, before close, on request); the proof after it
(the chain replays, every stored file re-hashes, the vault and its key
are there, the head is the one recorded); the weekly drill; a format
upgrade needing a proven local backup made under the store's lock; a
restore previewing first, restoring only the head it previewed, and
moving old books to `legacy/`, never deleting them; a destination more
than 24 hours behind being flagged.

The form (the lead's design; the unit may refine it and must say where):

- A destination holds one folder per set of books, named for the books,
  with a short plain-words read-me a person can open.
- Nothing in it is ever deleted or rewritten. The journal and the access
  log are kept as numbered segment files, one written per backup that
  has new lines: immutable once written, so a syncing service never sees
  a file change under it, and the same form serves sealed backups.
  Stored files are added once, under their hash. The vault and keys,
  which can change, are kept by content hash.
- A backup is complete when its manifest is in place: a small file,
  written last and atomically, naming the books' id, the sequence number
  and head it reaches, the segments, and the vault and key files by
  hash. A reader uses the newest manifest that proves whole; a run that
  died leaves pieces no manifest names, which are harmless.
- The journal is the history: an earlier state is a shorter prefix, and
  each manifest marks one. No separate history is kept.
- Before writing, the destination's newest manifest must be a point on
  the live journal (same head at that sequence number). If not, the
  destination holds other or diverged books: say so, write nothing.
- The local copy that proves a format upgrade is the same form, in the
  books' own folder, in place of the bare repository.
- "Save a backup…" writes the same contents as one file; "Restore…"
  takes a folder or that file.
- With a passphrase: every piece is sealed (XChaCha20-Poly1305, as the
  vault is; the key derived from the passphrase with a memory-hard
  function); file names give away no more than counts and sizes; the
  derived key is kept privately on this computer so backups stay
  automatic; a restore on another computer asks for the passphrase; a
  forgotten passphrase cannot be recovered, and the set-up says so.
- Old backups made with git are left exactly where they are and never
  touched. A person restoring from one clones it with git themselves and
  uses the existing adopt of a plain folder; the docs say how. A books'
  old `backup.git_remote` setting is reported as no longer used.
