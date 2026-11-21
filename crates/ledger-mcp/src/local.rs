//! `wanderbooks server`: the books of the data root, served on local
//! sockets (plan 3.1, 3.9), every one open at the start unless told
//! otherwise, and any one opened or closed while it runs (plan/p15-app-and-relay.md 3.1).
//! The host starts it; nothing else should.
//!
//! ```text
//! GOAL: one process, the books' sole writer, serves every book in the
//!       data root to this account alone, over sockets in the root's
//!       private `run/` folder, and lets go of every book cleanly.
//! SOURCE: plan/served.md 3.1 (the server's sockets; the admin socket
//!         creates and adopts books, mints tokens, and can't read books),
//!         3.3 (bearer tokens, scopes), 3.4 (data
//!         layout, run/books.lock), 3.9 (local channels, peer checks,
//!         closing: "no new requests, the one in progress finishes, every
//!         writer lock is released"), D-21 and D-23 (no service: the host
//!         starts and stops this process); plan/p15-app-and-relay.md 3.1
//!         (open and close one book while the server runs; who is
//!         connected), D-77 (the app is the host: a company is reachable
//!         exactly while the app has it open), D-78 (one tab per company).
//! INVARIANTS:
//!   1. One server per root: it holds `run/server.lock` from before it
//!      opens anything until it exits; a second server waits for the lock
//!      (an old server still finishing after its host died), touching
//!      nothing, and exits if told to close first. The sockets are
//!      replaced only under that lock.
//!   2. Only this account: a connection to either socket whose peer is
//!      another user ID is dropped before a byte is read.
//!   3. Every book chosen to be open (`--open`: all unless said) is opened
//!      and verified (rebuilding a drifted database) before the server
//!      says it is ready; one that won't open is reported with why, on its
//!      route and in the list, and the others are served. The books not
//!      chosen are closed (listed so), and open when asked (INVARIANT 10),
//!      by the same checks.
//!   4. Each book has one store, shared by every session: one writer.
//!   5. The ready line (stdout, once) comes only after both sockets are
//!      bound and every book has been tried.
//!   6. Closing (stdin reaches its end, or SIGTERM, SIGINT or SIGHUP): new
//!      requests are refused, every request in progress finishes (admin
//!      ones included), then the sessions end, the sockets are removed,
//!      and the process exits 0, releasing every lock.
//!   7. Nothing this process sends a client names the data root.
//!   8. An MCP request reaches a book only with a bearer token that is
//!      live for that book (auth/principals.jsonl; refused 401 without
//!      one, 403 with another book's), and a session only with the token
//!      that opened it (403). A request that has entered an open book's
//!      door and is then refused (no token, another book's, a revoked
//!      one, the server closing), and a request cut by a close (INVARIANT
//!      15), are recorded in that book's access log. A request for a book
//!      that is listed closed or failed, or that doesn't exist, and one
//!      that loses the race at the door (the door shut by a close of the
//!      book, a restore over it, or the server's close, between the lookup
//!      and the entry), is refused before it is dispatched and is NOT
//!      recorded: a closed book's log isn't written (it isn't open), and a
//!      shut door belongs to a close that is waiting for the log. None of
//!      these ran, and an unrecorded refusal can never be a call that
//!      ran: the only way to a book's tools is through its door; a request
//!      is counted inside it before anything of the book is touched, and
//!      a call that ran is recorded before its answer goes (INVARIANT 14;
//!      if it can't be, the server stops taking requests); a request
//!      refused outside the door never reached the tools. The token rides
//!      with the request to the book's tools, which check its scopes
//!      (access.rs). A session is
//!      bound to its token when opened, and forgotten when it ends, is
//!      found gone, or its token is revoked or replaced; past MAX_SESSIONS
//!      bindings a new session is refused (an initialize), never a binding
//!      dropped. Tokens are made, listed, checked (by a
//!      SHA-256, never a text) and revoked only through the admin socket.
//!   9. A file's bytes come in only by `PUT /b/{id}/u/{upload}` and go
//!      out only by `GET /b/{id}/a/{attachment}` (a stored file) and `GET
//!      /b/{id}/x/{export}` (a file the books made, D-29) (plan 4.7), each
//!      through the same door as 8 and counted for the close like any
//!      request: an upload only from the token that began it, a download
//!      only with books:read, an export only to the token it was made for
//!      (whose tool passed its own scope check; no other is asked). Each,
//!      once inside the door, ran or was refused there, is recorded in the
//!      book's access log as `upload`, `download` or `export` with its id
//!      (one refused outside the door is not recorded, and did not run:
//!      INVARIANT 8), and the
//!      bytes (kept, or read) and their record are one task that runs to
//!      its end if the client hangs up (a process killed in that task
//!      excepted); bytes whose record can't be written aren't kept, and
//!      the books stop. Lapsed uploads and exports are swept every
//!      SWEEP_EVERY. Nothing comes in or goes out once the books stopped.
//!  10. One book opens or closes while the server runs, by admin request,
//!      one change to a book at a time (create, adopt and restore among
//!      them): each book has its own turn (`Books::lives`), so a change to
//!      one book waits for no other book's, and a close of one book is
//!      never held up by, nor holds up, an open, a restore, an adoption or a
//!      create of another. Open:
//!      a closed (or failed) book of the root is opened and verified as at
//!      the start. Close: from the moment it begins, a call on that book is
//!      refused at its door, unrun, and not in its journal; a request not
//!      yet whole when the grace runs out is cut (INVARIANT 15); a call
//!      already running on it, on any route, either finishes (an ordinary
//!      call: nothing is killed, nothing is half-done) or, if it is one of
//!      the stoppable calls below, is stopped at its next look; the answers
//!      on their way out are sent, for the answer grace at most; its MCP
//!      sessions end; its last backup is made, for BACKUP_AT_CLOSE at most;
//!      and the book is let go (INVARIANT 13), its writer lock released, so
//!      another process could open it; and only then is a close answered.
//!      It is answered `closed`, or `closed_backup_incomplete` with why when
//!      its last backup wasn't whole (a target that failed, git that
//!      failed, a backup stopped after its time): never a plain `closed`
//!      for a backup that wasn't made. After that answer no call on that
//!      book runs until it is opened again. Every other book keeps serving,
//!      and a call on it is never held up by this one's close. A restore
//!      over an open book lets go of it the same way (but for its last
//!      backup), its door shut and the book listed closed from the start: a
//!      call that comes while the restore runs is told the books aren't
//!      open (503, INVARIANT 11), never that there are no such books.
//!
//!      What can run long, and what stops it. A close calls `hurry` at its
//!      start, before it waits for anything. The calls and jobs that can do
//!      long I/O or loop over a book's whole journal or files are exactly
//!      these, and each is stoppable:
//!        - on the book: `books.back_up_now`, and a backup a timer asked for
//!          (git is sent SIGTERM and killed only after TERM_GRACE: ledger_
//!          store backup INVARIANT 5; between targets the flag is looked at
//!          too); `books.verify` (once per journal line replayed and per
//!          stored file hashed); `books.export_cpa_package` (before it
//!          builds, and once per stored file it reads, unless it has
//!          journaled a decision: below); a drill; and a
//!          format upgrade not yet journaled. A call stopped this way
//!          answers its caller that it was stopped because the books are
//!          closing, its access record says `refused: stopped_by_close`, and
//!          nothing it was doing is recorded as done: a stopped backup
//!          records `stopped` and no push or commit outcome for the step it
//!          didn't finish (a commit made before it stands in the archive, and
//!          is recorded as made); a stopped verification changed nothing.
//!          Two points of no return: `books.verify` heals a drifted database
//!          by rebuilding it from the journal, begun only if no stop was
//!          asked before it (the look comes before the database is copied
//!          aside, so a stopped heal leaves no `.mismatch` copy and no
//!          rebuild), and then run to its end (stopped half way it would
//!          leave the live ledger part replayed; it is the fourth named
//!          exception below, and only after a drift); and a format upgrade: a
//!          stop is honoured up to its final look (made once the archive is
//!          proven, just before the `journal.format_set` line is appended);
//!          one that arrives after that look still upgrades and records
//!          success (safe: the books are in this version's format, proven
//!          backed up first), and finishes (a line and a database row).
//!          And a third: `books.export_cpa_package` that is exporting with
//!          a reason (without an audit, with open drafts) journals that
//!          decision before it builds, because the package counts it (its
//!          README and every footer carry the journal's event count at the
//!          build). The close is looked for once, immediately before the first
//!          decision is journaled (a stop then journals nothing); from there
//!          to the hand-over nothing looks at it, so a decision on record is
//!          a package handed over (tools/reports.rs INVARIANT 4). Its build
//!          then runs to its end: it is one of the named exceptions below.
//!        - on the admin socket: restore and its preview, adopt and its
//!          preview, create, and open. A stop is scoped to what it concerns.
//!          A job that changes books holds the turn of the id it works on
//!          (an open, a close, a create, an adoption, a restore: one at a
//!          time per id), and listens to the stop of THAT id, which a close
//!          of the id raises before it waits for the turn, and to the
//!          server's (the id's stop is raised by the server's close too):
//!          so a close of X stops an open of X, an adoption or a create
//!          making X, and a restore of X (over it, or making it), before
//!          their point of no return, and never waits behind one. A
//!          PREVIEW (of a restore, of an adoption) changes nothing, and
//!          holds no book's turn at all: a close never waits for one,
//!          however long its clone or its queue for `books.lock`; only the
//!          server's close stops it. No close of any other book stops, fails,
//!          refuses or delays an open, a close or a job of X, nor a job
//!          queued for its turn at making books (`books.lock` in the root,
//!          one at a time across processes: a deliberate rule, ledger_store::
//!          root INVARIANT 5, that a queued job leaves at once when it is
//!          asked to stop). Only the server's own close stops every job
//!          (`Books::stop_every_job`). A job for a book whose close is under
//!          way waits for that book's turn, so for the close to end, and is
//!          then asked as any other. Each is stopped, before its
//!          point of no return, at its next look (a phase, a stored file, a
//!          journal line, a git command): what it wrote is removed (a clone,
//!          a staging folder, and the lock file an adoption made in the
//!          source folder), and the root is byte for byte as it was. The
//!          point of no return of restore and adopt is the rename that puts
//!          the books in place; after it they finish, which is a rename or
//!          two and the sync of two folders (a restore over books moves them
//!          to `legacy/` and renames the copy in, both within the root); then
//!          the book is opened, which is itself stopped by the server's close
//!          and by a close of its id (the same stop the job held), left closed
//!          (its answer says so). Create's is the making of its folder, after which
//!          it makes one empty book. Open stops at every look of its
//!          verification (the book is left closed, intact), but for one
//!          place: once it has begun to heal a drifted database it finishes
//!          the heal (the fourth named exception below); a stop asked before
//!          the heal begins leaves no copy of the database either.
//!      Every other call does work bounded by the size of one request: its
//!      arguments, one entry, one stored file (at most MAX_UPLOAD) read,
//!      hashed or extracted (local CPU, no wait on a client, a disk of
//!      another's or the network). Three calls, and one step inside two
//!      stoppable ones, are neither stoppable nor bounded by one request's
//!      size, and they are the named exceptions to the close's bound below,
//!      each with its worst case, which a close that meets it waits for in
//!      full (nothing enforces a bound on them: that is a defect to be
//!      closed, not a promise kept):
//!        - `books.import_bank_csv`: a file of up to MAX_UPLOAD is rehearsed
//!          whole, then written row by row, each row journaled, and a close
//!          doesn't stop it. Worst case: a full import of the largest file
//!          accepted.
//!        - a report or export over the whole journal (`books.verify` apart,
//!          which is stoppable): CPU on the ledger already in memory, with no
//!          look at a stop. Worst case: one report over the whole book.
//!        - `books.export_cpa_package` once it has journaled a decision (above).
//!          Worst case: one build of the package over the whole book (every
//!          stored file it holds read once).
//!        - the heal of a drifted database, once begun, in `books.verify` and
//!          in an open (ledger_store `verify_and_heal`, cancel INVARIANT 4):
//!          the rebuild and its verification are shielded from the stop, and
//!          met only after a drift. Worst case: the copy of the database,
//!          and about six passes over the journal (three by the rebuild:
//!          read and chain-checked, applied to the database, replayed;
//!          three by the verification: read and chain-checked, replayed,
//!          applied to a fresh projection), and each stored file read and
//!          hashed once.
//!      The close is bounded, from its start, by the sum of these named
//!      terms, each enforced by code, and nothing else:
//!        a. T_calls: the longest ordinary call already running with its
//!           whole body (bounded by one request, above; none is cut or
//!           killed; nothing a client does lengthens it), or, for a
//!           stoppable call, T_stop, or, for a call or step of the four named
//!           exceptions above, its worst case (the bound is these terms,
//!           with that term unbounded by code while one of them runs);
//!        b. ANSWER_GRACE for the answers on their way out, counted from
//!           when the door is empty (the cut of a body not yet whole is at
//!           ANSWER_GRACE from the START of the close, inside (a): a
//!           stalled client adds nothing);
//!        c. T_stop: the longest between two looks of what is stopped, one
//!           of: a git command's stop (SIGTERM and up to TERM_GRACE, 5 s;
//!           then a kill and GROUP_GONE_WAIT, 2 s, and its pipes, 10 s at
//!           most), one journal line, one stored file, one 64 KiB chunk, one
//!           phase of a job; and the removal of what a stopped job or drill
//!           wrote (its scratch clone or staging folder: a local deletion of
//!           the job's own output, as long as that is large, waiting on no
//!           one);
//!        d. BACKUP_AT_CLOSE for the last backup;
//!        e. T_stop again for what is left of that backup at its bound
//!           (`Backups::shut`: git stopped, every step awaited);
//!        f. closing the logs and letting go of the store (instant).
//!      A drill (or a format upgrade) that took the backups' turn never
//!      costs the last backup its time.
//!  11. A call for a book that exists but is closed is told apart from one
//!      that does not exist, on the MCP, upload, download and export
//!      routes: 503, `x-wanderbooks-refused: closed`, and the words `The
//!      books "ID" aren't open: open them in Wanderbooks.` (the access log
//!      of a closed book is not written: it is not open).
//!  12. A book's live MCP sessions are known: for each, its token's NAME
//!      (never the token), the client's name and version, the agent's
//!      self-report, when it connected, its calls running now, and its last
//!      call (sessions.rs); a session that ended is not listed, and a
//!      book's sessions end with the book.
//!  13. After a close of a book is answered, this process holds nothing
//!      that can write to that book's folder (journal, access log, ops
//!      logs, stored files, upload parts, the backup's git repository) or
//!      read it on a caller's behalf, until the book is opened again; and
//!      a handle from before a close can never write after a reopen. Every
//!      path that touches a book is either counted by its gate (a request
//!      carries its place inside the door until its last write is done,
//!      even if its client hangs up and its future is dropped; a timer's
//!      sweep takes the place too), or checks, under the same lock as its
//!      write, that the book is still open, and refuses if not: the
//!      access log, the uploads, the exports and the backups (record,
//!      git, drill, upgrade) are each closed under the lock their own
//!      writes go in under, by the close, once the gate is empty; the
//!      store is let go under its lock. A close waits for the backup's
//!      work to END, not only for its future: a run past its bound is
//!      stopped (git killed with all it started) and waited for, so the
//!      close is never answered while a backup runs. The timers (the
//!      sweep, the look for a backup due, the look for a drill due) work
//!      on handles that are closed with the book, and so can't act on a
//!      book that closed, or was opened again, since they looked.
//!  14. A call whose answer isn't read within the answer grace is the same
//!      case as a connection that dropped at any other moment: the close
//!      ends the session after the grace, and a call that ran stays run.
//!      Its access record, written before its answer goes, says it ran
//!      (outcome and the journal head after it), so a client that
//!      reconnects can find out whether to ask again. Nothing in the
//!      journaled calls makes asking again safe (they carry no client key,
//!      and none checks for a duplicate): a draft asked for twice is two
//!      drafts. That is an open design point (plan/p15-app-and-relay.md),
//!      not something the close hides.
//!  15. A request that has begun and never finishes arriving can't hold a
//!      close: when the answer grace (counted from the start of the close,
//!      or of the server's) has run out, every request inside the door
//!      whose body has not yet been read to its end is cut. The line is
//!      the body's end, and no later: a body read to its end is whole,
//!      whatever the request does after (a call running, its answer being
//!      sent), and is never cut; a body not yet read to its end, however
//!      slowly or quickly it is coming, is cut at that moment. A cut
//!      request's connection is closed (`Connection: close`, 503,
//!      `x-wanderbooks-refused: cut_by_close`), what it had written is
//!      removed (an upload's part file, by `receive` on its read error),
//!      and its access record, written before its answer, says
//!      `cut_by_close`. Nothing of an MCP call can have been journaled:
//!      the service reads the whole body before it gives any message to a
//!      session or a tool (rmcp `expect_json`, which answers a body that
//!      fails with an error before anything else), and the handler checks
//!      for the cut again before it binds a session. This holds for a
//!      book's close, for a restore over an open book (both are
//!      `Books::release`) and for the server's close, which is the same
//!      path: at the start of the server's close (not after its admin
//!      jobs, store closes or backups) every open book's door is shut and
//!      its cut set for one grace from that moment, then every book still
//!      open is released as a close of one book releases it, together
//!      (so one book's slow backup holds no other's, each book's bound
//!      being INVARIANT 10's), its gate, sessions, logs, uploads, exports,
//!      backups and store all closed; only the HTTP side's end and the
//!      linger (a request that set out before its client heard of the
//!      close still meets a refusal, LINGER, 3 s at most) come after. A
//!      request that arrives once the server is closing and finds a book's
//!      door shut is refused as closing (503, `closing`), not recorded:
//!      the door is shut (INVARIANT 11), as in a close of one book.
//! SCENARIOS:
//!   1. Two books open; one is closed while agents call both: the other
//!      answers throughout, the closed one's calls are refused unrun
//!      (never half-done), its journal holds exactly the calls that were
//!      acknowledged; it is opened again and serves again.
//!   2. The server starts with `--open none`: every book is listed closed,
//!      a call to one says it isn't open, a call to a name that isn't a
//!      book says there is no such book; one is opened by request.
//!   3. A book is closed twice: the second answer is "was not open". A
//!      name that is no book: "no such book".
//!   4. A book is closed, then another process opens the store and writes
//!      to it: the lock was released; the server, asked to open it, says
//!      another program has it open, and opens once that one lets go.
//!   5. A client asks for a large answer and never reads it, and a close
//!      begins: the close waits the answer grace, ends the session and
//!      answers closed; nothing is written to the book afterwards, and when
//!      it is opened again its access log continues one whole chain.
//!   6. A book with a slow backup is closed: not answered while the backup
//!      runs; at its bound git is stopped and waited for, and the answer
//!      says the last backup wasn't whole; nothing runs after.
//!   7. A restore replaces an open book while an upload is half received:
//!      the restore is not answered until the upload is, and no part file
//!      is left in `legacy/`.
//!   8. The timers run every few milliseconds on a book that is closed and
//!      opened over and over: while it is closed not a file of it moves.
//!   9. An upload stops sending half way, and another call's body stops
//!      half way, and the book is closed: the close waits the grace, cuts
//!      both (no part file, nothing journaled, the records say cut), and
//!      answers closed. An upload still coming within the grace, and a
//!      call running past it, are not cut; an upload that keeps coming,
//!      steadily, past the grace is cut.
//!  10. A drill is slow (a large book): the book is closed, the drill is
//!      stopped at its next look and its clone removed, the last backup is
//!      made, and the close is answered within INVARIANT 10's bound. A
//!      format upgrade not yet journaled is stopped the same way, the books
//!      unchanged.
//!  11. A backup's git is stopped inside a commit or a push: it is sent
//!      SIGTERM and removes its own locks (nothing is left, nothing is
//!      recorded), the book opened again backs up at once. A git that can't
//!      end on SIGTERM and has to be killed after TERM_GRACE leaves locks
//!      that this process removes from the book's own archive only, and
//!      leaves in the owner's backup folder, recorded with their whole path
//!      and said in the close's answer and in the backup's status, until the
//!      owner removes them.
//!  12. The server is closed (standard input ends) with an upload stalled
//!      on one book and a call's body stalled on another: both are cut one
//!      grace after the close began, the server exits 0 within its bound.
//!  13. A backup asked for (`books.back_up_now`) is running, git stalled,
//!      and the book is closed: the call is answered, git still stalled,
//!      that it was stopped because the books are closing; its record says
//!      `refused: stopped_by_close`; the close is answered `closed`.
//!  14. A restore of a large backup over `beta` is running (an admin job)
//!      when `beta` is closed: the restore is stopped, answers that it was
//!      stopped because the books are closing, and the root is byte for byte
//!      as it was; asked again it goes through. The same restore, with
//!      another book (`acme`) closed instead, is not stopped.
//!  15. A restore over `beta`, a create of `epsilon` and an adoption of
//!      `delta` are in flight (held inside a phase), and `acme`, whose close
//!      is made slow, is closed and opened again over and over: all three
//!      finish correctly, none is stopped, none is told of an error, an open
//!      and a close of `cedar` during a close of `acme` are answered at once,
//!      and the close of `acme` is not answered until its own backup is done.
//!  16. The server is closed with a restore, an adoption, a create and an
//!      open in flight: each is stopped, and the root is as it was.
//!  17. An adoption of a folder that has no lock file is stopped, fails, or
//!      panics: the folder is, name for name and byte for byte, as it was.
//!  18. A package exported with a reason is held as it begins and the book is
//!      closed: stopped, no decision journaled. Held right after its decision
//!      is journaled and the book closed: the package is made.
//! ```

use crate::admin::{self, Request, Response};
use crate::ipc::{self, Peer};
use crate::server::{BooksServer, Isolation, Served, SharedStore};
use ledger_store::Store;
use ledger_store::access::{Access, AccessLog, Outcome, Principal};
use ledger_store::auth::{Principals, Scope, Token};
use ledger_store::root::{BookId, Root, RootError};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// What a restore or adoption that a close stopped after its point of no
/// return says of the books it put in place (INVARIANT 10).
const STOPPED_AFTER_PLACED: &str = "they are in place, but were not opened: the books server is closing. Open them when it runs again";
/// Why a book's opening was stopped (by a close, INVARIANT 10): the book is
/// intact and stays closed.
const OPEN_STOPPED: &str = "stopped because the books are closing: they were not opened";
/// The server's MCP socket, in the root's `run/`.
pub const MCP_SOCKET: &str = "mcp.sock";
/// The server's admin socket, in the root's `run/`.
pub const ADMIN_SOCKET: &str = "admin.sock";
/// Held by the one server for a root (INVARIANT 1).
pub const SERVER_LOCK: &str = "server.lock";
/// What the server prints on stdout when it is ready (INVARIANT 5).
pub const READY: &str = "ready";

/// How large an MCP request may be (plan 3.2): files come only through
/// uploads (`/b/{book}/u/{upload}`), never in a request (D-46).
pub const MAX_BODY: usize = 1024 * 1024;

/// How long the admin socket waits for a request to arrive.
const ADMIN_READ_TIMEOUT: Duration = Duration::from_secs(30);

use ledger_store::root::THE_ROOT;

/// The header on a request refused before it could run (books that
/// didn't open, books that don't exist): a client knows it didn't run.
pub const REFUSED_HEADER: &str = "x-wanderbooks-refused";

/// How often each book's lapsed uploads are swept.
const SWEEP_EVERY: Duration = Duration::from_secs(10);
/// How long the books stay unwritten before they are backed up.
const BACKUP_QUIET: Duration = Duration::from_secs(30);
/// How often the books are looked at for a backup due.
const BACKUP_LOOK: Duration = Duration::from_secs(2);
/// How often each book is looked at for a drill due.
const DRILL_LOOK: Duration = Duration::from_secs(3600);
/// How long after books are first seen open their first drill is looked
/// for (a drill that fell due while they were closed): long enough that a
/// backup on opening comes first.
const DRILL_FIRST: Duration = Duration::from_secs(60);
/// The most the last backup at a close may take.
const BACKUP_AT_CLOSE: Duration = Duration::from_secs(60);

/// The periods and bounds the server keeps, which tests shorten
/// (`--timers`, `--close-backup`, `--answer-grace`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Timers {
    /// How often each book's lapsed uploads and exports are swept.
    sweep_every: Duration,
    /// How often the books are looked at for a backup due.
    backup_look: Duration,
    /// How often each book is looked at for a drill due.
    drill_look: Duration,
    /// The most the last backup at a book's close may take.
    backup_at_close: Duration,
    /// How long a close waits for answers on their way out.
    answer_grace: Duration,
    /// A test knob (`--slow-steps`): each phase of a drill or format
    /// upgrade waits this long, so these can be long (backup.rs).
    slow_steps: Duration,
    /// A test knob (`--timer-hold`): a timer that has taken a book's
    /// handles waits, before it acts on them, while the file `timer-hold`
    /// in the root's `run/` names it (`sweep`, `backup`, `drill`), saying
    /// which timer it is in `run/timer-held` as it comes to wait (see
    /// [`Books::timer_hold`]).
    timer_hold: bool,
}

impl Default for Timers {
    fn default() -> Self {
        Self {
            sweep_every: SWEEP_EVERY,
            backup_look: BACKUP_LOOK,
            drill_look: DRILL_LOOK,
            backup_at_close: BACKUP_AT_CLOSE,
            answer_grace: ANSWER_GRACE,
            slow_steps: Duration::ZERO,
            timer_hold: false,
        }
    }
}

type Service = StreamableHttpService<BooksServer, LocalSessionManager>;

/// What a call is told for a book that exists but is closed (INVARIANT
/// 11): words an agent can relay to the person.
fn closed_words(book: &str) -> String {
    format!("The books \"{book}\" aren't open: open them in Wanderbooks.")
}

/// A book the server holds.
#[derive(Clone)]
enum Book {
    /// Open and served.
    Open {
        /// Its MCP service (boxed: the other variant is a line of text).
        service: Box<Service>,
        /// Its store, closed when the server closes (INVARIANT 6).
        store: Arc<SharedStore>,
        /// Its access log, where requests refused at the door are
        /// recorded too (access.rs).
        access: Arc<AccessLog>,
        /// Its files on their way in (`/b/{id}/u/{upload}`).
        uploads: ledger_store::uploads::Uploads,
        /// The files it made for agents (`/b/{id}/x/{export}`).
        exports: ledger_store::exports::Exports,
        /// Its backups (backup.rs).
        backups: Arc<crate::backup::Backups>,
        /// The format upgrade it waits for, if any (plan 3.9, D-39): read
        /// from its journal as it opened, and cleared by the upgrade (only
        /// the upgrade moves an open book's format).
        waiting: Arc<Mutex<Option<ledger_store::FormatUpgrade>>>,
        /// Its projection database's file, and other schemas' beside it,
        /// by name, as it opened (`wanderbooks doctor` says them).
        database: (String, Vec<String>),
        /// Its door: every request on it enters here, and a close shuts it
        /// and waits for those inside (INVARIANT 10).
        gate: Arc<Gate>,
        /// Ends its MCP sessions (a child of the server's own).
        ended: CancellationToken,
        /// Who is connected to it (sessions.rs, INVARIANT 12).
        board: Arc<crate::sessions::Board>,
    },
    /// Not opened, with why (INVARIANT 3).
    Failed(String),
    /// Not open, by choice: left closed at the start (`--open`), or closed
    /// since (INVARIANT 10). Not a problem.
    Closed,
}

impl Book {
    /// Why it isn't open, if it isn't.
    fn problem(&self) -> Option<String> {
        match self {
            Book::Open { .. } | Book::Closed => None,
            Book::Failed(why) => Some(why.clone()),
        }
    }

    /// What lets go of it, if it is open.
    fn parts(&self) -> Option<Parts> {
        match self {
            Book::Open {
                gate,
                store,
                access,
                uploads,
                exports,
                backups,
                ended,
                ..
            } => Some(Parts {
                gate: gate.clone(),
                store: store.clone(),
                access: access.clone(),
                uploads: uploads.clone(),
                exports: exports.clone(),
                backups: backups.clone(),
                ended: ended.clone(),
            }),
            Book::Failed(_) | Book::Closed => None,
        }
    }
}

/// Everything of an open book that can reach its folder, held so it can
/// be let go of together ([`Books::release`]).
struct Parts {
    gate: Arc<Gate>,
    store: Arc<SharedStore>,
    access: Arc<AccessLog>,
    uploads: ledger_store::uploads::Uploads,
    exports: ledger_store::exports::Exports,
    backups: Arc<crate::backup::Backups>,
    ended: CancellationToken,
}

/// `wait`, or None once `hurry` (a second signal to close: stop waiting)
/// is cancelled; without a `hurry`, `wait` alone.
async fn unless_hurried<T>(
    hurry: Option<&CancellationToken>,
    wait: impl std::future::Future<Output = T>,
) -> Option<T> {
    match hurry {
        Some(hurry) => tokio::select! {
            out = wait => Some(out),
            () = hurry.cancelled() => None,
        },
        None => Some(wait.await),
    }
}

/// One book's door (INVARIANT 10): requests are counted as they enter, and
/// once shut none enters. A close shuts it, then waits for the count to
/// reach zero: every request that got in has finished, its record written.
///
/// Entering adds to the count before it looks at `shut`, and shutting sets
/// `shut` before it looks at the count (both sequentially consistent), so
/// a request either sees the door shut, or is counted where the close will
/// wait for it: never neither.
///
/// An MCP request's answer may still be on its way after its handler is
/// done (the tool runs in the session's own task, its answer streamed back):
/// the handler takes an [`Answering`] before it leaves, which rides in the
/// answer's body until the body ends, so a close can wait for the answers
/// (as the server's own close does) before it ends the sessions.
#[derive(Default)]
pub(crate) struct Gate {
    shut: std::sync::atomic::AtomicBool,
    inside: AtomicUsize,
    answering: AtomicUsize,
    left: tokio::sync::Notify,
    /// Cancelled when the close's grace has run out: a request inside
    /// whose body hasn't finished arriving is cut (see [`Arrival`]).
    cut: CancellationToken,
}

/// Where a request's body is: still coming, whole, or cut by a close.
/// The body of a request inside a book's door is wrapped by
/// [`Inside::arriving`], which changes this state in one place only (its
/// own poll), so a body is never both whole and cut.
pub(crate) struct Arrival(std::sync::atomic::AtomicU8);

const ARRIVING: u8 = 0;
const ARRIVED: u8 = 1;
const CUT: u8 = 2;

impl Arrival {
    /// Whether a close cut this request's body before it was whole.
    fn was_cut(&self) -> bool {
        self.0.load(Ordering::SeqCst) == CUT
    }
}

/// A request inside a book's door, counted until dropped.
struct Inside(Arc<Gate>);

impl Drop for Inside {
    fn drop(&mut self) {
        self.0.inside.fetch_sub(1, Ordering::SeqCst);
        self.0.left.notify_waiters();
    }
}

impl Inside {
    /// An answer on its way out, for a request still inside: counted until
    /// the answer is sent.
    fn answering(&self) -> Answering {
        self.0.answering.fetch_add(1, Ordering::SeqCst);
        Answering(self.0.clone())
    }

    /// The request, its body watched: it is whole once the body's end has
    /// been read (a request with no body is whole at once), and until
    /// then a close whose grace ran out cuts it: the body fails, as a
    /// connection that dropped would, and the returned [`Arrival`] says
    /// so. A body already whole is never cut, whatever runs after it.
    fn arriving(&self, request: axum::extract::Request) -> (axum::extract::Request, Arc<Arrival>) {
        use axum::body::HttpBody as _;
        use futures_util::StreamExt;
        use std::task::Poll;
        let arrival = Arc::new(Arrival(std::sync::atomic::AtomicU8::new(ARRIVING)));
        let (parts, body) = request.into_parts();
        if body.is_end_stream() {
            arrival.0.store(ARRIVED, Ordering::SeqCst);
            return (axum::extract::Request::from_parts(parts, body), arrival);
        }
        let mut data = body.into_data_stream();
        let mut cut = Box::pin(self.0.cut.clone().cancelled_owned());
        let seen = arrival.clone();
        let stream = futures_util::stream::poll_fn(move |cx| {
            match seen.0.load(Ordering::SeqCst) {
                ARRIVED => return Poll::Ready(None),
                CUT => return Poll::Ready(Some(Err(cut_error()))),
                _ => {}
            }
            // Checked before the data: a body that keeps coming past the
            // grace is cut too, not only one that stalled.
            if cut.as_mut().poll(cx).is_ready() {
                seen.0.store(CUT, Ordering::SeqCst);
                return Poll::Ready(Some(Err(cut_error())));
            }
            match data.poll_next_unpin(cx) {
                Poll::Ready(None) => {
                    seen.0.store(ARRIVED, Ordering::SeqCst);
                    Poll::Ready(None)
                }
                Poll::Ready(Some(r)) => Poll::Ready(Some(r.map_err(std::io::Error::other))),
                Poll::Pending => Poll::Pending,
            }
        });
        (
            axum::extract::Request::from_parts(parts, axum::body::Body::from_stream(stream)),
            arrival,
        )
    }
}

fn cut_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::ConnectionAborted,
        "the request wasn't finished when the books closed",
    )
}

/// An answer on its way out of a book's door (see [`Gate`]).
struct Answering(Arc<Gate>);

impl Drop for Answering {
    fn drop(&mut self) {
        self.0.answering.fetch_sub(1, Ordering::SeqCst);
        self.0.left.notify_waiters();
    }
}

impl Gate {
    /// Let a request in, if the door is open.
    fn enter(self: &Arc<Self>) -> Option<Inside> {
        self.inside.fetch_add(1, Ordering::SeqCst);
        let inside = Inside(self.clone());
        (!self.shut.load(Ordering::SeqCst)).then_some(inside)
    }

    /// Cut every request inside whose body isn't whole (see
    /// [`Inside::arriving`]); the ones whose body is whole are left to
    /// finish.
    fn cut_unfinished(&self) {
        self.cut.cancel();
    }

    /// Let no request in from now on.
    fn shut(&self) {
        self.shut.store(true, Ordering::SeqCst);
    }

    /// Wait until every request inside has left.
    async fn emptied(&self) {
        loop {
            let left = self.left.notified();
            if self.inside.load(Ordering::SeqCst) == 0 {
                return;
            }
            left.await;
        }
    }

    /// Wait until every answer on its way out has been sent. Call after
    /// [`Self::emptied`]: no request is inside to start another.
    async fn answered(&self) {
        loop {
            let left = self.left.notified();
            if self.answering.load(Ordering::SeqCst) == 0 {
                return;
            }
            left.await;
        }
    }
}

/// A session bound to the token that opened it (INVARIANT 8).
struct Bound {
    /// The token's id.
    token: String,
    /// The token's name (listed by `sessions`: never the token).
    name: String,
    /// The book it is a session of.
    book: String,
    /// When it connected (seconds since the epoch, UTC).
    at: i64,
}

/// Which books are open at the server's start (`--open`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenAtStart {
    /// Every book of the root (the default).
    All,
    /// Only these (none: an empty list).
    Only(Vec<String>),
}

impl OpenAtStart {
    /// `--open`'s value: `all`, `none`, or books' ids separated by commas.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim() {
            "all" => Ok(Self::All),
            "none" => Ok(Self::Only(Vec::new())),
            list => list
                .split(',')
                .map(|id| {
                    BookId::parse(id.trim())
                        .map(|id| id.to_string())
                        .map_err(|e| format!("--open takes all, none, or books' ids: {e}"))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Self::Only),
        }
    }

    fn wants(&self, id: &BookId) -> bool {
        match self {
            Self::All => true,
            Self::Only(ids) => ids.iter().any(|i| i == id.as_str()),
        }
    }
}

/// Every book the server holds, and what serving them needs.
struct Books {
    root: Root,
    held: RwLock<BTreeMap<String, Book>>,
    /// Ends every MCP session (INVARIANT 6, last).
    sessions: CancellationToken,
    /// Set when the server starts closing: new requests are refused.
    closing: CancellationToken,
    /// MCP requests in progress (not counting event streams).
    in_flight: AtomicUsize,
    /// When the last MCP request arrived (milliseconds since the server
    /// started): closing lingers until none has for a while.
    last_arrival: std::sync::atomic::AtomicU64,
    /// When the server started, for `last_arrival`.
    started: std::time::Instant,
    /// Woken when one finishes.
    finished: tokio::sync::Notify,
    /// The agents' tokens (auth/principals.jsonl), or why they can't be
    /// read: then every MCP request is refused (INVARIANT 8).
    principals: RwLock<Result<Principals, String>>,
    /// Which token opened each MCP session: a session is used only with
    /// the token that opened it (INVARIANT 8).
    session_tokens: Mutex<BTreeMap<String, Bound>>,
    /// How long the books stay unwritten before a backup is made
    /// (`--backup-quiet`, for tests; else BACKUP_QUIET).
    backup_quiet: Duration,
    /// How long after books are first seen open a drill is first looked
    /// for (`--drill-first`, for tests; else DRILL_FIRST).
    drill_first: Duration,
    /// What each book's changes share: the lock they are made under, and
    /// the stop a close of that book raises (INVARIANT 10). One book's
    /// changes wait for one book's; no change to one book waits for, stops
    /// or fails a change to another.
    lives: Mutex<BTreeMap<String, Arc<Life>>>,
    /// Set for good when the server closes: the jobs that make a NEW book
    /// (a create, an adoption, a restore that replaces nothing) look at it.
    /// A close of one book never raises it: those jobs aren't work on that
    /// book (INVARIANT 10).
    server_stop: Arc<std::sync::atomic::AtomicBool>,
    /// How these books are kept from agents (plan 3.8, P9a): what
    /// `books.start` reports (`--isolation`).
    isolation: Isolation,
    /// The periods and bounds (tests shorten them).
    timers: Timers,
}

/// What the changes to ONE book share (`Books::lives`): the lock they are
/// made under, one at a time, and the stop a close of that book raises for
/// the jobs that work on it (INVARIANT 10).
struct Life {
    /// Held through every change to this book (open, close, a restore that
    /// replaces it; a create or an adoption making it).
    lock: Mutex<()>,
    /// Set while a close of this book is under way, and for good once the
    /// server closes: the admin jobs working on this book (an open, a
    /// restore over it) look at it and stop, before their point of no
    /// return, leaving the books as they were. `closers` counts the closes
    /// under way; the flag is clear when none is (and the server isn't
    /// closing). Set and cleared under `closers`.
    stop: Arc<std::sync::atomic::AtomicBool>,
    closers: Mutex<usize>,
}

/// A close of a book under way (`Books::closing_one`).
struct CloseGuard {
    life: Arc<Life>,
    closing: CancellationToken,
}

impl Drop for CloseGuard {
    fn drop(&mut self) {
        let mut closers = self.life.closers.lock().unwrap_or_else(|e| e.into_inner());
        *closers -= 1;
        if *closers == 0 && !self.closing.is_cancelled() {
            self.life
                .stop
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

impl Books {
    /// A server's books for `root`, none held yet.
    fn new(
        root: Root,
        backup_quiet: Duration,
        drill_first: Duration,
        isolation: Isolation,
    ) -> Self {
        Self {
            root,
            held: RwLock::new(BTreeMap::new()),
            sessions: CancellationToken::new(),
            closing: CancellationToken::new(),
            in_flight: AtomicUsize::new(0),
            finished: tokio::sync::Notify::new(),
            last_arrival: std::sync::atomic::AtomicU64::new(0),
            started: Instant::now(),
            principals: RwLock::new(Err(String::new())),
            session_tokens: Mutex::new(BTreeMap::new()),
            backup_quiet,
            drill_first,
            lives: Mutex::new(BTreeMap::new()),
            server_stop: Arc::default(),
            isolation,
            timers: Timers::default(),
        }
    }

    /// These books, with these periods and bounds.
    fn with_timers(mut self, timers: Timers) -> Self {
        self.timers = timers;
        self
    }

    /// What the changes to the book `id` share: made once, kept while
    /// anything holds it. A book first met after the server began closing
    /// is born stopped.
    fn life(&self, id: &BookId) -> Arc<Life> {
        let mut lives = self.lives.lock().unwrap_or_else(|e| e.into_inner());
        // Forget the books nothing is changing or closing.
        lives.retain(|_, life| Arc::strong_count(life) > 1);
        lives
            .entry(id.as_str().to_string())
            .or_insert_with(|| {
                Arc::new(Life {
                    lock: Mutex::new(()),
                    stop: Arc::new(std::sync::atomic::AtomicBool::new(
                        self.closing.is_cancelled(),
                    )),
                    closers: Mutex::new(0),
                })
            })
            .clone()
    }

    /// A close of the book `life` belongs to begins: the admin jobs working
    /// on THAT book that can run long stop (INVARIANT 10) until the returned
    /// guard is dropped. Nothing else is stopped.
    fn closing_one(&self, life: &Arc<Life>) -> CloseGuard {
        let mut closers = life.closers.lock().unwrap_or_else(|e| e.into_inner());
        *closers += 1;
        life.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        drop(closers);
        CloseGuard {
            life: life.clone(),
            closing: self.closing.clone(),
        }
    }

    /// The server closes: every stop is raised, for good (INVARIANT 10).
    /// `closing` is cancelled first, so a book first met from now is born
    /// stopped, and a close of one book ending now clears nothing.
    fn stop_every_job(&self) {
        self.server_stop
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let lives = self.lives.lock().unwrap_or_else(|e| e.into_inner());
        for life in lives.values() {
            // Under the lock a close's end clears the flag under.
            let _closers = life.closers.lock().unwrap_or_else(|e| e.into_inner());
            life.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// What an admin job that can run long is asked as it goes whether to
    /// stop (ledger_store::root `JobStop`): `flag` is the stop of the book
    /// it works on, or the server's (`server_stop`) for a job that makes a
    /// new book. With the test knob `--timer-hold` a job is held inside a
    /// phase named `job:phase` (in `run/timer-hold`) until the name is gone
    /// or a stop is asked.
    fn job(
        &self,
        job: &'static str,
        flag: &Arc<std::sync::atomic::AtomicBool>,
    ) -> ledger_store::root::JobStop {
        let stop = ledger_store::root::JobStop::new(flag.clone());
        if !self.timers.timer_hold {
            return stop;
        }
        let Ok(run) = self.root.run_dir() else {
            return stop;
        };
        let flag = flag.clone();
        stop.with_phase_watch(Arc::new(move |phase: &str| {
            let name = format!("{job}:{phase}");
            {
                use std::io::Write;
                if let Ok(mut said) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(run.join("timer-held"))
                {
                    let _ = writeln!(said, "{name}");
                }
            }
            let held = || {
                std::fs::read_to_string(run.join("timer-hold"))
                    .is_ok_and(|names| names.split(',').any(|n| n.trim() == name))
            };
            while held() && !flag.load(std::sync::atomic::Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }))
    }

    /// The test knob `--timer-hold`: `timer` has taken the handles of a
    /// book and is about to act on them. It says so (a line in
    /// `run/timer-held`) and waits until `run/timer-hold` is gone, so a
    /// test can close the book in between. Nothing without the knob.
    async fn timer_hold(&self, timer: &str) {
        if !self.timers.timer_hold {
            return;
        }
        let Ok(run) = self.root.run_dir() else {
            return;
        };
        {
            use std::io::Write;
            if let Ok(mut said) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(run.join("timer-held"))
            {
                let _ = writeln!(said, "{timer}");
            }
        }
        // Held while the file names this timer (a list, comma-separated).
        let held = || {
            std::fs::read_to_string(run.join("timer-hold"))
                .is_ok_and(|names| names.split(',').any(|n| n.trim() == timer))
        };
        while held() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Take the root's path out of a message (INVARIANT 7).
    fn hide(&self, text: impl Into<String>) -> String {
        let mut text = text.into();
        let path = self.root.path();
        let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        for p in [path, real.as_path()] {
            text = text.replace(&p.display().to_string(), THE_ROOT);
        }
        text
    }

    /// Change or read the principals; a log that failed to write is read
    /// again before anything else is done with it (chain.rs INVARIANT 2).
    fn with_principals(
        &self,
        f: impl FnOnce(&mut Principals) -> Result<Response, ledger_store::auth::AuthError>,
    ) -> Response {
        let mut guard = self.principals.write().unwrap_or_else(|e| e.into_inner());
        let p = match guard.as_mut() {
            Ok(p) => p,
            Err(why) => return Response::Refused(why.clone()),
        };
        match f(p) {
            Ok(r) => r,
            Err(e) => {
                if matches!(e, ledger_store::auth::AuthError::Log(_)) {
                    *guard = self.load_principals();
                }
                Response::Refused(self.hide(e.to_string()))
            }
        }
    }

    /// Forget the sessions a token opened (it was revoked or replaced:
    /// they can't be used again, and won't be ended by it).
    fn forget_sessions_of(&self, token_id: &str) {
        self.session_tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, bound| bound.token != token_id);
    }

    /// Read the principals from the root's `auth/`.
    fn load_principals(&self) -> Result<Principals, String> {
        let dir = self.root.auth_dir().map_err(|e| e.to_string())?;
        Principals::open(&dir).map_err(|e| self.hide(e.to_string()))
    }

    /// The live token whose text a request carries as its bearer, or why
    /// it is refused (INVARIANT 8): the code for the access log and the
    /// words for the client.
    fn bearer(&self, headers: &axum::http::HeaderMap) -> Result<Token, (&'static str, String)> {
        // The scheme's name in any case (RFC 9110 11.1).
        let presented = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim());
        let Some(presented) = presented else {
            return Err((
                crate::access::UNAUTHORIZED,
                "these books need a token, and this request carried none: the human makes one with `wanderbooks token create` (if one was made, `wanderbooks token check` says whether its token file still gives it)"
                    .into(),
            ));
        };
        let guard = self.principals.read().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Err(why) => Err((crate::access::UNAUTHORIZED, why.clone())),
            Ok(p) => p.check(presented).cloned().ok_or((
                crate::access::UNAUTHORIZED,
                "that token isn't one these books know, or it was revoked".into(),
            )),
        }
    }

    /// [`Self::open`] for an admin job that can be stopped: the store's
    /// replay and verification look at the stop, and a book whose opening
    /// was stopped is closed, not failed (it is intact: it opens when asked).
    fn open_stoppable(&self, id: &BookId, stop: &ledger_store::root::JobStop) -> Book {
        let _scope = ledger_store::cancel::scope(stop.flag());
        let book = self.open(id);
        match &book {
            Book::Failed(why) if why == OPEN_STOPPED => Book::Closed,
            _ => book,
        }
    }

    /// Open the book `id` from disk: served, or failed with why.
    fn open(&self, id: &BookId) -> Book {
        let opened = match Store::open(&self.root.book_dir(id)) {
            Ok(store) => self.serve(store),
            Err(ledger_store::StoreError::Locked(_)) => {
                Err("another program has these books open".to_string())
            }
            Err(ledger_store::StoreError::Cancelled) => Err(OPEN_STOPPED.to_string()),
            Err(e) => Err(self.hide(e.to_string())),
        };
        opened.unwrap_or_else(|why| {
            eprintln!("wanderbooks server: the books {id} were not opened: {why}");
            Book::Failed(why)
        })
    }

    /// Verify an open store and build its service (INVARIANTS 3, 4).
    fn serve(&self, mut store: Store) -> Result<Book, String> {
        let report = store.verify_and_heal().map_err(|e| match e {
            ledger_store::StoreError::Cancelled => OPEN_STOPPED.to_string(),
            e => format!("they don't verify: {}", self.hide(e.to_string())),
        })?;
        if let Some(why) = &report.healed {
            eprintln!(
                "wanderbooks server: rebuilt a database from its journal ({})",
                self.hide(why.clone())
            );
        }
        let waiting = Arc::new(Mutex::new(store.journal_format().upgrade()));
        let database = (store.database_file(), store.other_databases());
        let served = Served::open_with(store, self.isolation).map_err(|e| self.hide(e))?;
        let ended = self.sessions.child_token();
        let config = StreamableHttpServerConfig::default()
            .with_json_response(true)
            .with_legacy_session_mode(true)
            .with_max_request_body_bytes(MAX_BODY)
            .enforce_origin_validation()
            .with_cancellation_token(ended.clone());
        let parts = served.clone();
        // Every session's server is made the one way (server.rs Served).
        let service = StreamableHttpService::new(
            move || Ok(BooksServer::new(&parts)),
            Arc::new(sessions()),
            config,
        );
        let Served {
            store: shared,
            access,
            uploads,
            exports,
            backups,
            board,
            ..
        } = served;
        backups.slow_steps(self.timers.slow_steps);
        // The test knob `--timer-hold`: a call a close can stop is held as
        // it begins, named `call:<tool>` in `run/timer-hold`, until the
        // name is gone or the close begins.
        if self.timers.timer_hold
            && let Ok(run) = self.root.run_dir()
        {
            let flag = shared.hurry_flag();
            backups.set_hold(Arc::new(move |call: &str| {
                let name = format!("call:{call}");
                {
                    use std::io::Write;
                    if let Ok(mut said) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(run.join("timer-held"))
                    {
                        let _ = writeln!(said, "{name}");
                    }
                }
                let held = || {
                    std::fs::read_to_string(run.join("timer-hold"))
                        .is_ok_and(|names| names.split(',').any(|n| n.trim() == name))
                };
                while held() && !flag.load(std::sync::atomic::Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }));
        }
        Ok(Book::Open {
            gate: Arc::default(),
            ended,
            board,
            service: Box::new(service),
            store: shared,
            access,
            uploads,
            exports,
            backups,
            waiting,
            database,
        })
    }

    fn held(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<String, Book>> {
        self.held.read().unwrap_or_else(|e| e.into_inner())
    }

    fn hold(&self, id: &BookId, book: Book) {
        self.held
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string(), book);
    }

    /// Answer an admin request (blocking: it runs on its own thread).
    fn admin(&self, request: Request) -> Response {
        let refused = |e: RootError| Response::Refused(self.hide(e.to_string()));
        match request {
            Request::List => Response::Books(
                self.held()
                    .iter()
                    .map(|(id, book)| admin::BookStatus {
                        id: id.clone(),
                        open: matches!(book, Book::Open { .. }),
                        closed: matches!(book, Book::Closed),
                        problem: book.problem(),
                        format_upgrade: match book {
                            Book::Open { waiting, .. } => waiting
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .as_ref()
                                .map(admin::FormatWaiting::from),
                            Book::Failed(_) | Book::Closed => None,
                        },
                        database: match book {
                            Book::Open { database, .. } => Some(database.0.clone()),
                            Book::Failed(_) | Book::Closed => None,
                        },
                        other_databases: match book {
                            Book::Open { database, .. } => database.1.clone(),
                            Book::Failed(_) | Book::Closed => Vec::new(),
                        },
                    })
                    .collect(),
            ),
            Request::TokenCreate {
                book,
                name,
                scopes,
                replace,
            } => {
                let book = match BookId::parse(&book) {
                    Ok(id) => id,
                    Err(e) => return refused(e),
                };
                if !self.held().contains_key(book.as_str()) {
                    return Response::Refused(format!(
                        "there are no books called {book}: `wanderbooks books list` lists them"
                    ));
                }
                let mut wanted = std::collections::BTreeSet::new();
                for s in &scopes {
                    match Scope::parse(s) {
                        Some(scope) => {
                            wanted.insert(scope);
                        }
                        None => {
                            return Response::Refused(format!(
                                "{:?} isn't a scope: use books:read, books:write, books:post or books:admin",
                                s.chars().take(64).collect::<String>()
                            ));
                        }
                    }
                }
                let mut replaced: Option<String> = None;
                let answer = self.with_principals(|p| {
                    replaced = p
                        .tokens()
                        .iter()
                        .find(|t| t.book == book.as_str() && t.name == name)
                        .map(|t| t.id.clone())
                        .filter(|_| replace);
                    p.create(&book, &name, wanted, replace)
                        .map(|(token, text)| Response::TokenCreated {
                            token: admin::TokenInfo::from(&token),
                            text: admin::TokenText(text.expose().to_string()),
                        })
                });
                if let (Response::TokenCreated { .. }, Some(old)) = (&answer, &replaced) {
                    self.forget_sessions_of(old);
                }
                answer
            }
            Request::TokenList => self.with_principals(|p| {
                Ok(Response::Tokens(
                    p.tokens().iter().map(admin::TokenInfo::from).collect(),
                ))
            }),
            Request::TokenCheck { book, name, sha256 } => self.with_principals(|p| {
                let live = p.tokens().iter().find(|t| t.book == book && t.name == name);
                // A hash, not a secret: compared as it is.
                let matches = live.is_some_and(|t| t.sha256.eq_ignore_ascii_case(&sha256));
                Ok(Response::TokenChecked {
                    live: live.map(admin::TokenInfo::from),
                    matches,
                })
            }),
            Request::TokenRevoke { book, name, id } => {
                let book = match BookId::parse(&book) {
                    Ok(id) => id,
                    Err(e) => return refused(e),
                };
                let answer = self.with_principals(|p| {
                    p.revoke_if(&book, &name, id.as_deref())
                        .map(|t| Response::TokenRevoked(admin::TokenInfo::from(&t)))
                });
                if let Response::TokenRevoked(t) = &answer {
                    self.forget_sessions_of(&t.id);
                }
                answer
            }
            Request::Create { id, currency } => {
                let id = match BookId::parse(&id) {
                    Ok(id) => id,
                    Err(e) => return refused(e),
                };
                let Ok(currency) = ledger_core::Currency::new(currency.trim()) else {
                    return Response::Refused(
                        "that isn't a currency: give its three-letter ISO code, such as USD".into(),
                    );
                };
                let life = self.life(&id);
                let _one = life.lock.lock().unwrap_or_else(|e| e.into_inner());
                if self.held().contains_key(id.as_str()) {
                    return refused(RootError::Exists(id.to_string()));
                }
                // Stopped by the server's close, or by a close of this id
                // (which waits for the turn held here), before its folder is
                // made, it makes nothing; after that it makes one empty book.
                // A close of another book doesn't stop it (INVARIANT 10).
                match self.root.create_book_stoppable(
                    &id,
                    currency,
                    &self.job("create", &life.stop),
                ) {
                    Ok(store) => {
                        // Made: kept whether or not it opens (INVARIANT 3).
                        let book = self.serve(store).unwrap_or_else(Book::Failed);
                        let problem = book.problem();
                        self.hold(&id, book);
                        Response::Created {
                            id: id.to_string(),
                            problem,
                        }
                    }
                    Err(e) => refused(e),
                }
            }
            Request::Restore {
                id,
                source,
                replace,
                confirmed,
                expect_head,
            } => self.restore(id, source, replace, confirmed, expect_head),
            Request::UpgradeFormat { id, passphrase } => self.upgrade_format(id, &passphrase),
            Request::OpenBook { id } => self.open_one(id),
            Request::CloseBook { id } => self.close_one(id),
            Request::Sessions => self.sessions_listed(),
            Request::Extract { id, attachment_id } => {
                let id = match BookId::parse(&id) {
                    Ok(id) => id,
                    Err(e) => return refused(e),
                };
                let store = match self.held().get(id.as_str()) {
                    Some(Book::Open { store, .. }) => store.clone(),
                    Some(Book::Failed(why)) => {
                        return Response::Refused(format!("those books didn't open: {why}"));
                    }
                    Some(Book::Closed) => return Response::Refused(self.not_open(&id)),
                    None => {
                        return Response::Refused(format!(
                            "there are no books called {:?}",
                            id.as_str()
                        ));
                    }
                };
                match store.with_store_mut(|s| s.extract_attachment(attachment_id)) {
                    Ok(Ok(x)) => Response::Extracted {
                        attachment_id: x.attachment_id,
                        status: x.status.key().to_string(),
                        pages: x.pages,
                        text_sha256: x.text_sha256,
                    },
                    Ok(Err(e)) => Response::Refused(self.hide(e.to_string())),
                    Err(why) => Response::Refused(self.hide(why)),
                }
            }
            Request::BackUpNow { id, new_disk } => {
                let id = match BookId::parse(&id) {
                    Ok(id) => id,
                    Err(e) => return refused(e),
                };
                let backups = match self.held().get(id.as_str()) {
                    Some(Book::Open { backups, .. }) => backups.clone(),
                    Some(Book::Failed(why)) => {
                        return Response::Refused(format!("those books didn't open: {why}"));
                    }
                    Some(Book::Closed) => return Response::Refused(self.not_open(&id)),
                    None => {
                        return Response::Refused(format!(
                            "there are no books called {:?}",
                            id.as_str()
                        ));
                    }
                };
                let rt = tokio::runtime::Handle::current();
                match rt.block_on(backups.run_with("asked", new_disk)) {
                    Ok(status) => Response::BackedUp {
                        head_seq: status.head_seq,
                        targets: status
                            .targets
                            .into_iter()
                            .map(|t| admin::BackedUpTarget {
                                target: t.target,
                                at: t.at,
                                through_seq: t.through_seq,
                                problem: t.last_error,
                            })
                            .collect(),
                    },
                    Err(why) => Response::Refused(self.hide(why)),
                }
            }
            Request::Adopt {
                id,
                source,
                settings_toml,
                config_json,
                vault_dir,
                vault_key,
                confirmed,
                expect_head,
                skip_unknown,
            } => {
                let id = match BookId::parse(&id) {
                    Ok(id) => id,
                    Err(e) => return refused(e),
                };
                let life = self.life(&id);
                // A preview changes nothing, so it takes no book's turn: a
                // close of this id never waits behind it (INVARIANT 10). An
                // adoption takes the turn, and with it the stop of THIS id
                // (and the server's): a close of the id, which waits for the
                // turn, stops the adoption before its point of no return, and
                // stops the opening that follows its placing, as it stops an
                // open. A close of another book stops nothing of it.
                let _one = confirmed.then(|| life.lock.lock().unwrap_or_else(|e| e.into_inner()));
                if self.held().contains_key(id.as_str()) {
                    return refused(RootError::Exists(id.to_string()));
                }
                let stop = self.job(
                    "adopt",
                    if confirmed {
                        &life.stop
                    } else {
                        &self.server_stop
                    },
                );
                let found = match self.root.adopt_preview_stoppable(&source, &stop) {
                    Ok(found) => found,
                    Err(e) => return refused(e),
                };
                if vault_dir.is_some() != vault_key.is_some() {
                    return Response::Refused(
                        "a vault comes with its key: give both the vault folder and its key file, or neither"
                            .into(),
                    );
                }
                if confirmed && expect_head.is_none() {
                    // admin INVARIANT 4: a confirm names what it confirms.
                    return Response::Refused(
                        "an adoption is confirmed against the preview's head: preview first".into(),
                    );
                }
                let import = match (&settings_toml, found.keeps_settings) {
                    (Some(toml), false) => {
                        // The key the file names matters only with a vault
                        // to open.
                        let key = vault_dir.as_ref().and(vault_key.as_deref());
                        match settings_to_move(
                            toml,
                            config_json.as_deref(),
                            found.currency.code(),
                            vault_dir.is_some(),
                            key,
                        ) {
                            Ok(import) => Some(import),
                            Err(why) => return Response::Refused(why),
                        }
                    }
                    (None, false) => {
                        return Response::Refused(
                            "these books keep no settings of their own yet: give their settings file (--toml), and its settings are moved in as they are adopted"
                                .into(),
                        );
                    }
                    _ => None,
                };
                let vault_records = match (&vault_dir, &vault_key) {
                    (Some(dir), Some(key)) => match vault_opens(dir, key) {
                        Ok(n) => n,
                        Err(why) => return Response::Refused(why),
                    },
                    _ => 0,
                };
                if !confirmed {
                    let i = import.unwrap_or_default();
                    return Response::Preview(admin::AdoptPreview {
                        source_head: found.head,
                        keeps_settings: found.keeps_settings,
                        settings: i
                            .changes
                            .iter()
                            .map(|c| admin::SettingChange {
                                key: c.key.clone(),
                                value: c.value.clone(),
                            })
                            .collect(),
                        notes: i.notes,
                        unknown: i.unknown,
                        comments: i.comments,
                        vault_records,
                    });
                }
                // admin INVARIANT 5: nothing dropped unseen.
                if let Some(i) = &import
                    && !i.unknown.is_empty()
                    && !skip_unknown
                {
                    return Response::Refused(format!(
                        "the settings file has entries the books don't know, which wouldn't move in: {}. Take them out, or adopt with --skip-unknown to leave them behind",
                        i.unknown.join(", ")
                    ));
                }
                let options = ledger_store::root::AdoptOptions {
                    settings: match import {
                        Some(i) => {
                            let source = moved_in_at_adoption(&i.changes);
                            Some((i.changes, source))
                        }
                        // Given for books that keep their own: passed on so
                        // the report says they weren't used.
                        None if settings_toml.is_some() => {
                            Some((Vec::new(), moved_in_at_adoption(&[])))
                        }
                        None => None,
                    },
                    vault_dir,
                    vault_key,
                    expect_head,
                };
                match self.root.adopt_stoppable(&source, &id, &options, &stop) {
                    Ok(r) => {
                        // Adopted: kept whether or not it opens (not opened,
                        // if a close began: it is in place all the same).
                        let book = self.open_stoppable(&id, &stop);
                        let problem = book.problem().or_else(|| {
                            matches!(book, Book::Closed).then(|| STOPPED_AFTER_PLACED.to_string())
                        });
                        self.hold(&id, book);
                        Response::Adopted(admin::Adopted {
                            id: r.id.to_string(),
                            source_head: r.source_head,
                            head: r.head,
                            blob_files: r.blob_files,
                            blobs_verified: r.blobs_verified,
                            unnamed_blobs: r.unnamed_blobs,
                            skipped_files: r.skipped_files,
                            vault_files: r.vault_files,
                            key_copied: r.key_copied,
                            settings_imported: r.settings_imported,
                            settings_ignored: r.settings_ignored,
                            source_lock_created: r.source_lock_created,
                            format_upgraded: r.format_upgraded,
                            problem,
                        })
                    }
                    Err(e) => refused(e),
                }
            }
        }
    }
}

impl Books {
    /// The owner's go-ahead for a format upgrade (plan 3.9, D-39; admin
    /// INVARIANT 7): the owner passphrase checked (Argon2id, as the human
    /// pages check it) before anything is done; then a verified backup
    /// with the keys and the upgrade, under the books' lock
    /// (backup::Backups::upgrade_format). Books already in this version's
    /// format: nothing is done, and the answer says so.
    fn upgrade_format(&self, id: String, passphrase: &admin::Passphrase) -> Response {
        let id = match BookId::parse(&id) {
            Ok(id) => id,
            Err(e) => return Response::Refused(self.hide(e.to_string())),
        };
        let (backups, waiting) = match self.held().get(id.as_str()) {
            Some(Book::Open {
                backups, waiting, ..
            }) => (backups.clone(), waiting.clone()),
            Some(Book::Failed(why)) => {
                return Response::Refused(format!("those books didn't open: {why}"));
            }
            Some(Book::Closed) => return Response::Refused(self.not_open(&id)),
            None => {
                return Response::Refused(format!(
                    "there are no books called {:?}: `wanderbooks books list` lists them",
                    id.as_str()
                ));
            }
        };
        let auth = match self.root.auth_dir() {
            Ok(dir) => dir,
            Err(e) => return Response::Refused(self.hide(e.to_string())),
        };
        match ledger_store::passphrase::verify(&auth, &passphrase.0) {
            Ok(true) => {}
            Ok(false) => {
                return Response::Refused(
                    "that isn't the owner passphrase: nothing was changed".into(),
                );
            }
            Err(e) => {
                return Response::Refused(format!(
                    "{}: nothing was changed",
                    self.hide(e.to_string())
                ));
            }
        }
        let rt = tokio::runtime::Handle::current();
        match rt.block_on(backups.upgrade_format()) {
            Ok(done) => {
                // The books are in this version's format now (or were).
                *waiting.lock().unwrap_or_else(|e| e.into_inner()) = None;
                if let Some(up) = &done.upgrade {
                    eprintln!(
                        "wanderbooks server: the books {id} were upgraded to this version's journal format ({}), after a verified backup",
                        up.adds().join(", ")
                    );
                }
                Response::FormatUpgraded(admin::FormatUpgraded {
                    id: id.to_string(),
                    added: done.upgrade.map(|u| u.adds()).unwrap_or_default(),
                    backup: done.backup.map(|b| admin::UpgradeBackup {
                        commit: b.commit,
                        head: b.head,
                    }),
                    head: done.head,
                })
            }
            Err(why) => Response::Refused(self.hide(why)),
        }
    }

    /// Open the one book `id` (INVARIANT 10): a closed or failed book of
    /// the root is opened and verified as at the start, and says so.
    fn open_one(&self, id: String) -> Response {
        let id = match BookId::parse(&id) {
            Ok(id) => id,
            Err(e) => return Response::Refused(self.hide(e.to_string())),
        };
        let said = |outcome| Response::OpenBook {
            id: id.to_string(),
            outcome,
        };
        // This book's turn only: an open of this book waits for a close or a
        // restore of THIS book, never for any other book's (INVARIANT 10).
        let life = self.life(&id);
        let _one = life.lock.lock().unwrap_or_else(|e| e.into_inner());
        if self.closing.is_cancelled() {
            return Response::Refused("the books are closing: nothing was done".into());
        }
        let is_open = self
            .held()
            .get(id.as_str())
            .map(|b| matches!(b, Book::Open { .. }));
        match is_open {
            Some(true) => return said(admin::OpenOutcome::AlreadyOpen),
            Some(false) => {}
            // Not held: a book of the root this server didn't take up, or
            // none at all.
            None => match self.root.list_books() {
                Ok(ids) if ids.iter().any(|i| i.as_str() == id.as_str()) => {}
                Ok(_) => return said(admin::OpenOutcome::NoSuchBook),
                Err(e) => return Response::Refused(self.hide(e.to_string())),
            },
        }
        // Stopped by a close of this book, or the server's (INVARIANT 10):
        // the book stays as it was.
        let stop = self.job("open", &life.stop);
        if stop.look("start") {
            return said(admin::OpenOutcome::CouldNotOpen(OPEN_STOPPED.into()));
        }
        let book = self.open_stoppable(&id, &stop);
        if matches!(book, Book::Closed) {
            self.hold(&id, book);
            return said(admin::OpenOutcome::CouldNotOpen(OPEN_STOPPED.into()));
        }
        let problem = book.problem();
        self.hold(&id, book);
        said(match problem {
            None => admin::OpenOutcome::Opened,
            Some(why) => admin::OpenOutcome::CouldNotOpen(why),
        })
    }

    /// Close the one book `id` while the server runs (INVARIANT 10). In
    /// this order: its door is shut and the book is listed closed, so a
    /// call on it is refused from here on; the calls running on it finish,
    /// on every route (SharedStore INVARIANT 3, and the gate); its MCP
    /// sessions end; its last backup is made, as at the server's close;
    /// and its store is let go, releasing its writer lock. Only then is it
    /// answered closed. Other books are never waited on, nor refused.
    fn close_one(&self, id: String) -> Response {
        let id = match BookId::parse(&id) {
            Ok(id) => id,
            Err(e) => return Response::Refused(self.hide(e.to_string())),
        };
        let said = |outcome| Response::CloseBook {
            id: id.to_string(),
            outcome,
        };
        // From now the admin jobs working on THIS book that can run long
        // stop (INVARIANT 10), so this book's turn is not held for long by
        // one of them; no other book's job is touched, and no other book's
        // change is waited for.
        let life = self.life(&id);
        let closing = self.closing_one(&life);
        let one = life.lock.lock().unwrap_or_else(|e| e.into_inner());
        let answer = self.close_book_held(&id, said);
        // The stop is lowered while the turn is still held: an open of this
        // book that was waiting for it is not stopped by a close that is over.
        drop(closing);
        drop(one);
        answer
    }

    /// [`Self::close_one`], the book's turn held.
    fn close_book_held(
        &self,
        id: &BookId,
        said: impl Fn(admin::CloseOutcome) -> Response,
    ) -> Response {
        if self.closing.is_cancelled() {
            return Response::Refused("the books are closing: nothing was done".into());
        }
        let held = self.held().get(id.as_str()).cloned();
        let parts = match held.as_ref().and_then(Book::parts) {
            Some(parts) => parts,
            None if held.is_some() => return said(admin::CloseOutcome::WasNotOpen),
            None => {
                return match self.root.list_books() {
                    Ok(ids) if ids.iter().any(|i| i.as_str() == id.as_str()) => {
                        said(admin::CloseOutcome::WasNotOpen)
                    }
                    Ok(_) => said(admin::CloseOutcome::NoSuchBook),
                    Err(e) => Response::Refused(self.hide(e.to_string())),
                };
            }
        };
        let rt = tokio::runtime::Handle::current();
        // No new call gets in; the book is listed closed.
        parts.gate.shut();
        self.hold(id, Book::Closed);
        let backup = rt.block_on(self.release(id, &parts, true, Instant::now(), None));
        eprintln!("wanderbooks server: the books {id} were closed");
        said(match backup {
            None => admin::CloseOutcome::Closed,
            Some(why) => admin::CloseOutcome::ClosedBackupIncomplete(why),
        })
    }

    /// Let go of an open book whose door is shut (INVARIANT 10; a close
    /// and a replacing restore are this): its calls finish, its sessions
    /// end, its last backup is made (a close's), and then nothing of this
    /// process's reaches its folder, however a holder of its handles from
    /// before is used. Returns why the last backup wasn't whole, if it
    /// wasn't (None: made, or no target to make it to).
    ///
    /// In this order, each step ending before the next begins:
    ///  1. the store's tools finish, and its door empties (every request
    ///     inside has finished, its record written); a request whose body
    ///     isn't whole when ANSWER_GRACE has run out is cut (its
    ///     connection closed, what it wrote removed, its record saying the
    ///     close cut it), a running call is not;
    ///  2. the answers on their way out are sent, for ANSWER_GRACE at most:
    ///     a client that never reads its answer doesn't hold the book
    ///     open (the call it was answering ran and is in the journal and
    ///     the access log; the client may retry it: INVARIANT 10);
    ///  3. its MCP sessions end;
    ///  4. the last backup, for `backup_at_close` at most; the backups are
    ///     then shut, which stops a run still going (git killed) and waits
    ///     for it to end, so the answer never comes while it runs;
    ///  5. the access log, uploads and exports close, each under the lock
    ///     its own writes go in under, so a late writer is refused;
    ///  6. the store is let go, its writer lock released.
    async fn release(
        &self,
        id: &BookId,
        parts: &Parts,
        last_backup: bool,
        begun: Instant,
        hurry: Option<&CancellationToken>,
    ) -> Option<String> {
        let Parts {
            gate,
            store,
            access,
            uploads,
            exports,
            backups,
            ended,
        } = parts;
        // The steps of the backups a close doesn't wait for (a drill, a
        // format upgrade not yet journaled) stop now, so none holds the
        // store's tools or the backups' turn while the close waits for
        // what it does wait for (INVARIANT 10).
        backups.hurry();
        // No tool starts, and every one that did start has finished. A
        // request that has begun and isn't whole when the grace runs out
        // (counted from `begun`, the start of the close) is cut, so one
        // that never finishes sending can't hold the book; a call whose
        // body is whole and that is running is left to end.
        let cutter = {
            let (gate, at) = (gate.clone(), begun + self.timers.answer_grace);
            tokio::spawn(async move {
                tokio::time::sleep_until(at.into()).await;
                gate.cut_unfinished();
            })
        };
        store.close().await;
        unless_hurried(hurry, gate.emptied()).await;
        cutter.abort();
        let answered = unless_hurried(
            hurry,
            tokio::time::timeout(self.timers.answer_grace, gate.answered()),
        )
        .await;
        if matches!(answered, Some(Err(_))) {
            eprintln!(
                "wanderbooks server: a client didn't take its answer; closing the books {id} without it"
            );
        }
        self.end_sessions(id, ended);
        let problem = if last_backup && !hurry.is_some_and(CancellationToken::is_cancelled) {
            let made = tokio::time::timeout(self.timers.backup_at_close, backups.run("close"));
            match unless_hurried(hurry, made).await {
                Some(Ok(Ok(status))) => backup_incomplete(&status),
                Some(Ok(Err(why))) => Some(why),
                Some(Err(_)) => Some(format!(
                    "it took longer than {} seconds, and was stopped",
                    self.timers.backup_at_close.as_secs().max(1)
                )),
                None => None,
            }
        } else {
            None
        };
        // A run still going is stopped, and waited for: nothing of the
        // backups runs once this returns.
        let left = backups.shut().await;
        access.close();
        uploads.close();
        exports.close();
        store.let_go().await;
        // A git stopped at its bound that left locks in the owner's folder:
        // said with the file, as the standing says it (ledger_store backup
        // INVARIANT 5b), not only that the backup took too long.
        let problem = match (problem, left.is_empty()) {
            (problem, true) => problem,
            (None, false) => Some(left.join("; ")),
            (Some(why), false) => Some(format!("{why}; {}", left.join("; "))),
        };
        problem.map(|why| self.hide(why))
    }

    /// The door of every open book.
    fn gates(&self) -> Vec<Arc<Gate>> {
        self.held()
            .values()
            .filter_map(|b| b.parts().map(|p| p.gate))
            .collect()
    }

    /// End the MCP sessions of the book `id` and forget their bindings.
    fn end_sessions(&self, id: &BookId, ended: &CancellationToken) {
        ended.cancel();
        self.session_tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, bound| bound.book != id.as_str());
    }

    /// Who is connected to each open book (INVARIANT 12).
    fn sessions_listed(&self) -> Response {
        let bound: Vec<(String, String, String, i64)> = self
            .session_tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(sid, b)| (sid.clone(), b.name.clone(), b.book.clone(), b.at))
            .collect();
        let open: Vec<(String, Arc<crate::sessions::Board>)> = self
            .held()
            .iter()
            .filter_map(|(id, b)| match b {
                Book::Open { board, .. } => Some((id.clone(), board.clone())),
                Book::Failed(_) | Book::Closed => None,
            })
            .collect();
        let when = |at: i64| {
            chrono::DateTime::from_timestamp(at, 0)
                .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
                .unwrap_or_default()
        };
        Response::Sessions(
            open.into_iter()
                .map(|(book, board)| {
                    let mut mine: Vec<_> = bound.iter().filter(|(_, _, b, _)| *b == book).collect();
                    mine.sort_by(|a, b| (a.3, &a.0).cmp(&(b.3, &b.0)));
                    let live: Vec<String> = mine.iter().map(|(sid, ..)| sid.clone()).collect();
                    let mut notes = board.snapshot(&live);
                    let sessions = mine
                        .into_iter()
                        .map(|(sid, name, _, at)| {
                            let note = notes.remove(sid).unwrap_or_default();
                            admin::SessionInfo {
                                token: name.clone(),
                                client_name: note.client.as_ref().map(|c| c.name.clone()),
                                client_version: note.client.as_ref().map(|c| c.version.clone()),
                                agent: note.agent.map(|a| admin::SessionAgent {
                                    model: a.model,
                                    reasoning_effort: a.reasoning_effort,
                                    harness: a.harness,
                                }),
                                connected_at: when(*at),
                                running: u32::try_from(note.running).unwrap_or(u32::MAX),
                                last: note.last.map(|l| admin::LastCall {
                                    tool: l.subject,
                                    outcome: match l.outcome {
                                        Outcome::Ok => "ok".to_string(),
                                        Outcome::Error => "error".to_string(),
                                        Outcome::Refused(code) => format!("refused: {code}"),
                                    },
                                    at: when(l.at),
                                }),
                            }
                        })
                        .collect();
                    admin::BookSessions { book, sessions }
                })
                .collect(),
        )
    }

    /// What is said of a book that exists but isn't open, to an admin
    /// request that needs it open.
    fn not_open(&self, id: &BookId) -> String {
        closed_words(id.as_str())
    }

    /// Restore a backup (admin INVARIANT 4; ledger_store::root INVARIANT
    /// 7): a preview, or the books restored and opened. Books it replaces
    /// are let go first, and opened again if the restore fails.
    fn restore(
        &self,
        id: String,
        source: String,
        replace: bool,
        confirmed: bool,
        expect_head: Option<(u64, String)>,
    ) -> Response {
        // admin INVARIANT 3: never the root, nor the address given.
        let said = |text: String| {
            let text = if source.chars().count() > 1 {
                text.replace(&source, "the backup given")
            } else {
                text
            };
            Response::Refused(self.hide(text))
        };
        let id = match BookId::parse(&id) {
            Ok(id) => id,
            Err(e) => return said(e.to_string()),
        };
        let git = match ledger_store::backup::Git::find() {
            Ok(git) => git,
            Err(e) => return said(e.to_string()),
        };
        // A preview only reports (whether books of that id exist among
        // what it says), so it changes nothing and takes no book's turn: a
        // close of this id never waits behind its clone and its queue for
        // the books lock (INVARIANT 10). Only the server's close stops it.
        if !confirmed {
            let stop = self.job("restore", &self.server_stop);
            return match self
                .root
                .restore_preview_stoppable(&git, &source, &id, &stop)
            {
                Ok(p) => Response::RestorePreview(admin::RestorePreview {
                    head: p.head,
                    blobs: p.blobs,
                    vault_files: p.vault_files,
                    exists: p.exists,
                    origin: p.origin,
                }),
                Err(e) => said(e.to_string()),
            };
        }
        // A restore takes this book's turn only (INVARIANT 10), and with it
        // the stop of THIS id (and the server's): a close of the id, which
        // waits for the turn, stops the restore before its point of no
        // return, whether it replaces the books or makes new ones.
        let life = self.life(&id);
        let _one = life.lock.lock().unwrap_or_else(|e| e.into_inner());
        let stop = self.job("restore", &life.stop);
        let Some(expect_head) = expect_head else {
            return said("a restore is confirmed against the preview's head: preview first".into());
        };
        // Stopped by a close before anything is let go (INVARIANT 10).
        if stop.look("begin") {
            return said("stopped because the books are closing: nothing was restored".into());
        }
        let held = self.held().contains_key(id.as_str());
        if replace && !held {
            return said(format!(
                "there are no books called {:?} to replace: nothing was restored",
                id.as_str()
            ));
        }
        if !replace && held {
            return said(format!(
                "there are already books called {:?}: restore them under another id (--id), or replace them (--replace; they are kept in legacy/)",
                id.as_str()
            ));
        }
        // The books replaced are let go as a close lets go of a book, but
        // for its last backup (INVARIANT 10): their door shut, the calls
        // inside finished (a download answered or refused, an upload's part
        // file gone), their sessions ended, their backups stopped and
        // waited for (none of theirs reaches a target after the backup is
        // read), their logs and files closed, their writer lock released.
        // The door is shut first and the book listed closed (never taken
        // out of the list: a call that comes while the restore runs is
        // told the books aren't open, as in a close, never that there are
        // no such books).
        let old = if replace {
            self.held().get(id.as_str()).cloned()
        } else {
            None
        };
        if let Some(parts) = old.as_ref().and_then(Book::parts) {
            let rt = tokio::runtime::Handle::current();
            parts.gate.shut();
            self.hold(&id, Book::Closed);
            rt.block_on(self.release(&id, &parts, false, Instant::now(), None));
        }
        match self
            .root
            .restore_stoppable(&git, &source, &id, &expect_head, replace, &stop)
        {
            Ok(r) => {
                // Restored: kept whether or not it opens; its backups told
                // whose books these were (backup.rs INVARIANTS 6, 7). Past its
                // point of no return a stop doesn't undo it: the books are
                // in place, and not opened if the server is closing.
                let book = self.open_stoppable(&id, &stop);
                let problem = book.problem().or_else(|| {
                    matches!(book, Book::Closed).then(|| STOPPED_AFTER_PLACED.to_string())
                });
                self.hold(&id, book);
                Response::Restored(admin::Restored {
                    id: r.id.to_string(),
                    head: r.head,
                    blobs_verified: r.blobs_verified,
                    vault_files: r.vault_files,
                    replaced: r.replaced,
                    origin: r.origin,
                    problem,
                })
            }
            Err(e) => {
                // The books it would have replaced, let go above, open
                // again (only this restore runs, so nothing else holds them).
                if replace {
                    let book = self.open_stoppable(&id, &stop);
                    self.hold(&id, book);
                }
                said(e.to_string())
            }
        }
    }
}

/// The settings a settings file moves in at adoption, checked: they
/// parse, they apply, and the vault key file it names (which the books
/// don't keep) is the one given.
fn settings_to_move(
    toml: &str,
    config_json: Option<&str>,
    currency: &str,
    with_vault: bool,
    vault_key: Option<&Path>,
) -> Result<ledger_config::codec::Import, String> {
    let import = ledger_config::codec::import(
        toml,
        config_json,
        &ledger_core::settings::Settings::default(),
        currency,
    )
    .map_err(|e| format!("the settings file can't be moved in: {e}"))?;
    let mut after = ledger_core::settings::Settings::default();
    after
        .apply(
            &import.changes,
            &moved_in_at_adoption(&import.changes),
            u64::MAX,
        )
        .map_err(|e| format!("the settings file can't be moved in: {e}"))?;
    if let Some(named) = import.vault_key_file.as_ref().filter(|_| with_vault) {
        let same = vault_key.is_some_and(|given| {
            match (std::fs::canonicalize(named), std::fs::canonicalize(given)) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            }
        });
        if !same {
            return Err(
                "the settings file names the TIN vault's key file ([vault] key_file), which the books don't keep: adopt with --vault-key naming that same file, or the vault wouldn't open"
                    .into(),
            );
        }
    }
    Ok(import)
}

/// Check that `key` opens every record in the vault folder `dir`; how
/// many there are.
fn vault_opens(dir: &Path, key: &Path) -> Result<usize, String> {
    let vault = ledger_vault::Vault::open(dir, key, false).map_err(|_| {
        "the vault key given isn't one this vault can use (64 hex digits): nothing was adopted"
            .to_string()
    })?;
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("the vault folder given can't be read: {}", e.kind()))?;
    let mut records = 0;
    for entry in entries {
        let entry = entry.map_err(|e| format!("the vault folder can't be read: {}", e.kind()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".bin") else {
            continue;
        };
        let opened = match stem.split_once('-') {
            Some(("tin", n)) => n.parse().ok().map(|n| vault.tin(n).map(|_| ())),
            Some(("w9", n)) => n.parse().ok().map(|n| vault.w9(n).map(|_| ())),
            _ => None,
        };
        match opened {
            Some(Ok(())) => records += 1,
            Some(Err(_)) => {
                return Err(
                    "the vault key given doesn't open the vault's records: give the key they were sealed with; nothing was adopted"
                        .into(),
                );
            }
            None => {}
        }
    }
    Ok(records)
}

/// How long a session may go without a message before it ends: far
/// longer than any call (rmcp's idle timer doesn't count a call in
/// progress, so a call longer than it would lose its answer while its work
/// went on). It only bounds sessions whose client vanished without ending
/// them; the bridge ends its own, even when signalled.
const SESSION_IDLE: Duration = Duration::from_secs(24 * 60 * 60);

/// The book's MCP sessions (see SESSION_IDLE).
fn sessions() -> LocalSessionManager {
    let mut sessions = LocalSessionManager::default();
    sessions.session_config.keep_alive = Some(SESSION_IDLE);
    sessions
}

/// Who and when, for settings moved in from a file at adoption: dated in
/// the company's time zone when the file sets one.
fn moved_in_at_adoption(
    changes: &[ledger_core::settings::Change],
) -> ledger_core::settings::Source {
    let zone = changes
        .iter()
        .find(|c| c.key == "company.timezone")
        .and_then(|c| c.value.as_ref())
        .and_then(|v| v.as_str())
        .map(str::to_string);
    ledger_core::settings::Source {
        decided_by: "the settings file".into(),
        decided_on: ledger_core::settings::today_in(zone.as_deref()),
        citation: None,
        note: Some(
            "moved in by the owner when the books were adopted (wanderbooks books adopt)".into(),
        ),
    }
}

/// The client's check of the server (plan 3.9: both ends check).
pub(crate) fn check_peer_std(stream: &ipc::StdStream) -> Result<(), String> {
    if stream.peer_is_us() {
        Ok(())
    } else {
        Err("the books server's socket belongs to another account: not used".into())
    }
}

/// A listener that hands on only this account's connections
/// (INVARIANT 2).
struct OwnerOnly(ipc::Listener);

impl OwnerOnly {
    async fn next(&mut self) -> ipc::Accepted {
        loop {
            match self.0.accept().await {
                Ok(stream) if stream.peer_is_us() => return stream,
                Ok(stream) => {
                    eprintln!("wanderbooks server: refused a connection from another account");
                    drop(stream);
                }
                Err(e) => {
                    // Out of descriptors and the like: wait, as axum does,
                    // rather than spin. A peer that hung up is no matter.
                    if !matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionAborted
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::ConnectionRefused
                    ) {
                        eprintln!("wanderbooks server: accept failed: {e}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

impl axum::serve::Listener for OwnerOnly {
    type Io = ipc::Accepted;
    type Addr = ipc::Addr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let stream = self.next().await;
            if let Ok(addr) = ipc::Listener::addr_of(&stream) {
                return (stream, addr);
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.0.local_addr()
    }
}

/// Counts an MCP request in progress, from its arrival until its answer's
/// body is dropped (sent whole, or its client gone): it rides in that
/// body, which a session fills later. Whether a tool is still running is
/// counted by its store (SharedStore::close), not here.
struct InFlight(Arc<Books>);

impl InFlight {
    fn new(books: &Arc<Books>) -> Self {
        books.in_flight.fetch_add(1, Ordering::SeqCst);
        Self(books.clone())
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
        self.0.finished.notify_waiters();
    }
}

/// The MCP session header.
pub(crate) const SESSION_HEADER: &str = "mcp-session-id";

/// The most sessions open at once (each bound to its token): past it a
/// new session is refused until one ends, never a binding forgotten
/// (INVARIANT 8; review of slice 4: clearing them let a flood unbind
/// live sessions).
const MAX_SESSIONS: usize = 100_000;

/// What a request asks for, as the access log names it: an MCP request
/// (action `connect`, its method), an upload (`upload`, its id) or a
/// download (`download`, the attachment's id).
struct Asked {
    action: &'static str,
    subject: String,
}

impl Asked {
    fn connect(method: &axum::http::Method) -> Self {
        Self {
            action: "connect",
            subject: method.as_str().to_string(),
        }
    }
}

/// A request refused at the door: recorded in the book's access log
/// (access.rs INVARIANT 3), then answered with `status` and `why`.
///
/// The record is written by a blocking task that carries the request's
/// place inside the book's door (`inside`) until the record is on disk,
/// even if the client hangs up and this future is dropped meanwhile: a
/// close of the book waits for it (INVARIANT 10), so it never lands in the
/// access log of a book that was let go, nor, once the book is opened again,
/// against the new log's head.
async fn refused_at_door(
    book: &Book,
    inside: Inside,
    principal: Principal,
    asked: &Asked,
    code: &'static str,
    status: axum::http::StatusCode,
    why: String,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Book::Open { access, store, .. } = book {
        let access = access.clone();
        let record = Access {
            principal,
            client: None,
            request_id: None,
            action: asked.action.into(),
            // Masked, then cut, by the log itself (access.rs INVARIANT 2).
            subject: asked.subject.clone(),
            args_digest: None,
            result: Outcome::Refused(code.into()),
            head_after: None,
            agent: None,
        };
        let written = tokio::task::spawn_blocking(move || {
            let _inside = inside;
            access.record(&record)
        })
        .await;
        if !matches!(written, Ok(Ok(_))) {
            store.stop();
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                [(REFUSED_HEADER, "not-recorded")],
                crate::access::NOT_RECORDED,
            )
                .into_response();
        }
    }
    let mut response = (status, [(REFUSED_HEADER, code)], why).into_response();
    if status == axum::http::StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            axum::http::header::WWW_AUTHENTICATE,
            axum::http::HeaderValue::from_static("Bearer realm=\"wanderbooks\""),
        );
    }
    response
}

/// The answer to a request a close cut while its body was still arriving
/// (INVARIANT 10): recorded in the access log as cut by the close, and the
/// connection ends with it.
async fn cut_by_the_close(
    book: &Book,
    inside: Inside,
    principal: Principal,
    asked: &Asked,
) -> axum::response::Response {
    let mut response = refused_at_door(
        book,
        inside,
        principal,
        asked,
        crate::access::CUT,
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "the books closed before this request was all sent: it wasn't run".into(),
    )
    .await;
    close_connection(&mut response);
    response
}

fn close_connection(response: &mut axum::response::Response) {
    response.headers_mut().insert(
        axum::http::header::CONNECTION,
        axum::http::HeaderValue::from_static("close"),
    );
}

/// The door to `book` (INVARIANT 8): the books held open, the server not
/// closing, and a live token for those books; else the answer refusing
/// the request (recorded in the books' access log, where there is one).
/// A request let through is inside the book's door (INVARIANT 10) until
/// the returned [`Inside`] is dropped: a close of the book waits for it,
/// so it must live as long as the request touches the book.
async fn door(
    books: &Arc<Books>,
    book: &str,
    asked: &Asked,
    headers: &axum::http::HeaderMap,
) -> Result<(Book, Token, Inside), Box<axum::response::Response>> {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    let closing = books.closing.is_cancelled();
    let found = books.held().get(book).cloned();
    // Inside the book's door before anything of the book is touched, its
    // access log included; a door that has shut (the book is closing, or
    // closed) is the same refusal as a book listed closed.
    let entered = match &found {
        Some(Book::Open { gate, .. }) => Some(gate.enter()),
        _ => None,
    };
    let shut_door = matches!(entered, Some(None));
    let (found, inside) = match entered {
        Some(Some(inside)) => (found, Some(inside)),
        Some(None) => (Some(Book::Closed), None),
        None => (found, None),
    };
    let held = match found {
        Some(Book::Closed) if !closing => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                [(REFUSED_HEADER, "closed")],
                closed_words(book),
            )
                .into_response()
                .into());
        }
        Some(Book::Failed(why)) if !closing => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                [(REFUSED_HEADER, "not-open")],
                format!("the books {book} didn't open: {why}"),
            )
                .into_response()
                .into());
        }
        None if !closing => {
            return Err((
                StatusCode::NOT_FOUND,
                [(REFUSED_HEADER, "no-such-books")],
                format!(
                    "there are no books called {:?}: `wanderbooks books list` lists them",
                    book.chars().take(64).collect::<String>()
                ),
            )
                .into_response()
                .into());
        }
        Some(b @ Book::Open { .. }) => b,
        _ => {
            let mut response = (
                StatusCode::SERVICE_UNAVAILABLE,
                "the books are closing: this request wasn't run",
            )
                .into_response();
            // The server is closing and this book's door is already shut
            // (INVARIANT 15): the same refusal, as a client knows it, as
            // one at an open door (not recorded: the door is shut).
            if shut_door {
                response.headers_mut().insert(
                    REFUSED_HEADER,
                    axum::http::HeaderValue::from_static(crate::access::CLOSING),
                );
            }
            return Err(response.into());
        }
    };
    let Some(inside) = inside else {
        unreachable!("an open book was entered above")
    };
    let token = books.bearer(headers);
    let principal = |t: &Token| Principal {
        id: t.id.clone(),
        kind: "agent".into(),
        label: t.name.clone(),
    };
    if closing {
        let who = token
            .as_ref()
            .map_or_else(|_| Principal::unknown(), principal);
        return Err(refused_at_door(
            &held,
            inside,
            who,
            asked,
            crate::access::CLOSING,
            StatusCode::SERVICE_UNAVAILABLE,
            "the books are closing: this request wasn't run".into(),
        )
        .await
        .into());
    }
    let token = match token {
        Ok(t) => t,
        Err((code, why)) => {
            return Err(refused_at_door(
                &held,
                inside,
                Principal::unknown(),
                asked,
                code,
                StatusCode::UNAUTHORIZED,
                why,
            )
            .await
            .into());
        }
    };
    if token.book != book {
        return Err(refused_at_door(
            &held,
            inside,
            principal(&token),
            asked,
            "wrong_books",
            StatusCode::FORBIDDEN,
            format!("this token is for the books {}, not {book}", token.book),
        )
        .await
        .into());
    }
    Ok((held, token, inside))
}

/// `PUT /b/{book}/u/{upload}`: the bytes of an upload begun with
/// `books.begin_upload`, from the token that began it (INVARIANT 9; plan
/// 4.7): at most the upload's limit, kept only if exactly the size and
/// SHA-256 declared, once; recorded in the access log (action `upload`).
async fn upload(
    axum::extract::State(books): axum::extract::State<Arc<Books>>,
    axum::extract::Path((book, id)): axum::extract::Path<(String, String)>,
    request: axum::extract::Request,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use ledger_store::uploads::UploadError;
    let _counted = InFlight::new(&books);
    books
        .last_arrival
        .store(books.started.elapsed().as_millis() as u64, Ordering::SeqCst);
    let asked = Asked {
        action: "upload",
        subject: id.clone(),
    };
    let (held, token, inside) = match door(&books, &book, &asked, request.headers()).await {
        Ok(passed) => passed,
        Err(refused) => {
            // What was sent is read and dropped (up to the most an upload
            // can be: only this account reaches the socket), so the client
            // gets the refusal, not a connection cut mid-send.
            use futures_util::StreamExt;
            let mut rest = request.into_body().into_data_stream();
            let mut read = 0u64;
            while let Some(Ok(chunk)) = rest.next().await {
                read += chunk.len() as u64;
                if read > ledger_store::uploads::MAX_UPLOAD {
                    break;
                }
            }
            return *refused;
        }
    };
    let Book::Open {
        access,
        uploads,
        store,
        ..
    } = &held
    else {
        unreachable!("the door lets only open books through")
    };
    // Its body watched: a close whose grace ran out cuts it while it is
    // still arriving, which ends `receive` with its part file removed.
    let (request, arrival) = inside.arriving(request);
    let cut_seen = arrival.clone();
    // Streamed to the disk as it comes, never held whole; `receive` reads
    // no more than the size declared and one byte. The bytes and their
    // record are one blocking task, which runs to its end even if the
    // client hangs up: never bytes kept without their record, and a close
    // waits for both (the count goes with them).
    use futures_util::TryStreamExt;
    let stream = request
        .into_body()
        .into_data_stream()
        .map_err(std::io::Error::other);
    let mut body = tokio_util::io::SyncIoBridge::new(tokio_util::io::StreamReader::new(stream));
    let (uploads, access, stopped) = (uploads.clone(), access.clone(), store.is_stopped());
    let task = tokio::task::spawn_blocking(move || {
        let _counted = _counted;
        // Inside the book's door until its bytes and record are done: a
        // close of the book waits for them (INVARIANT 10).
        let _inside = inside;
        // The books server stopped after an internal error: nothing more
        // comes in (SharedStore INVARIANT 2).
        let received = if stopped {
            Err(UploadError::Bad(crate::server::STOPPED.into()))
        } else {
            uploads.receive(&id, &token.id, &mut body, chrono::Utc::now().timestamp())
        };
        // Refused: the rest of what was sent (up to the most an upload
        // can be) is read and dropped, so the client gets the answer, not
        // a connection cut while it was still sending.
        if received.is_err() {
            use std::io::Read;
            let rest = ledger_store::uploads::MAX_UPLOAD + 1;
            let _ = std::io::copy(&mut (&mut body).take(rest), &mut std::io::sink());
        }
        let (status, outcome, text) = match &received {
            _ if cut_seen.was_cut() => (
                StatusCode::SERVICE_UNAVAILABLE,
                Outcome::Refused(crate::access::CUT.into()),
                "the books closed before this upload was all sent: send it again once they are open"
                    .to_string(),
            ),
            Ok(g) => (
                StatusCode::OK,
                Outcome::Ok,
                serde_json::json!({"received": g.id, "size": g.size, "sha256": g.sha256})
                    .to_string(),
            ),
            Err(e) => {
                let (status, code) = match e {
                    _ if stopped => (StatusCode::SERVICE_UNAVAILABLE, "stopped"),
                    UploadError::Unknown(_) => (StatusCode::NOT_FOUND, "no_such_upload"),
                    UploadError::NotYours => (StatusCode::FORBIDDEN, "not_yours"),
                    UploadError::AlreadySent => (StatusCode::CONFLICT, "already_sent"),
                    UploadError::InUse => (StatusCode::CONFLICT, "in_use"),
                    UploadError::WrongSize { .. }
                    | UploadError::TooMuch { .. }
                    | UploadError::WrongHash => (StatusCode::BAD_REQUEST, "not_as_declared"),
                    _ => (StatusCode::INTERNAL_SERVER_ERROR, "failed"),
                };
                (status, Outcome::Refused(code.into()), e.to_string())
            }
        };
        let record = Access {
            principal: Principal {
                id: token.id.clone(),
                kind: "agent".into(),
                label: token.name.clone(),
            },
            client: None,
            request_id: None,
            action: "upload".into(),
            subject: id.clone(),
            args_digest: None,
            result: outcome.clone(),
            head_after: None,
            agent: None,
        };
        let recorded = access.record(&record).is_ok();
        if !recorded && received.is_ok() {
            // Never bytes kept without their record: sent again later.
            uploads.discard(&id);
        }
        (status, outcome, text, recorded)
    });
    let Ok((status, outcome, text, true)) = task.await else {
        store.stop();
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(REFUSED_HEADER, "not-recorded")],
            crate::access::NOT_RECORDED,
        )
            .into_response();
    };
    let mut response = match outcome {
        Outcome::Refused(code) => (status, [(REFUSED_HEADER, code)], text).into_response(),
        _ => (status, text).into_response(),
    };
    if arrival.was_cut() {
        close_connection(&mut response);
    }
    response
}

/// `GET /b/{book}/a/{attachment}`: a stored file's bytes, for a token
/// with books:read (INVARIANT 9; what `wanderbooks download` asks for);
/// recorded in the access log (action `download`).
async fn download(
    axum::extract::State(books): axum::extract::State<Arc<Books>>,
    axum::extract::Path((book, attachment)): axum::extract::Path<(String, String)>,
    request: axum::extract::Request,
) -> axum::response::Response {
    fetch(books, book, Wanted::Attachment(attachment), request).await
}

/// `GET /b/{book}/x/{export}`: a file the books made for an agent
/// (exports.rs; D-29), only to the token it was made for (INVARIANT 9);
/// recorded in the access log (action `export`).
async fn export(
    axum::extract::State(books): axum::extract::State<Arc<Books>>,
    axum::extract::Path((book, id)): axum::extract::Path<(String, String)>,
    request: axum::extract::Request,
) -> axum::response::Response {
    fetch(books, book, Wanted::Export(id), request).await
}

/// What a GET fetches.
enum Wanted {
    /// A stored receipt or document, by its id.
    Attachment(String),
    /// A file the books made, by its id.
    Export(String),
}

/// A download or an export: the door, the read and its record, the
/// headers (INVARIANT 9).
async fn fetch(
    books: Arc<Books>,
    book: String,
    wanted: Wanted,
    request: axum::extract::Request,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    // GET only (axum hands a HEAD to a GET route): a HEAD would read the
    // file and record a download that sent nothing.
    if request.method() != axum::http::Method::GET {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(axum::http::header::ALLOW, "GET")],
            "a download is a GET",
        )
            .into_response();
    }
    let _counted = InFlight::new(&books);
    books
        .last_arrival
        .store(books.started.elapsed().as_millis() as u64, Ordering::SeqCst);
    let (action, subject) = match &wanted {
        Wanted::Attachment(id) => ("download", id.clone()),
        Wanted::Export(id) => ("export", id.clone()),
    };
    let asked = Asked {
        action,
        subject: subject.clone(),
    };
    let (held, token, inside) = match door(&books, &book, &asked, request.headers()).await {
        Ok(passed) => passed,
        Err(refused) => return *refused,
    };
    let Book::Open {
        access,
        store,
        exports,
        ..
    } = &held
    else {
        unreachable!("the door lets only open books through")
    };
    // The read and its record are one blocking task, run to its end even
    // if the client hangs up, and counted until then.
    let (access, store_for, exports) = (access.clone(), store.clone(), exports.clone());
    let task = tokio::task::spawn_blocking(move || {
        let _counted = _counted;
        // Inside the book's door until the read and its record are done
        // (INVARIANT 10).
        let _inside = inside;
        use ledger_store::exports::ExportError;
        let not_found = |code, what: &str| {
            (
                StatusCode::NOT_FOUND,
                code,
                format!("there is no such {what}"),
            )
        };
        let found = if store_for.is_stopped() {
            // Stopped after an internal error: as for uploads.
            Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "stopped",
                crate::server::STOPPED.to_string(),
            ))
        // An export is bound to the token it was made for, whose tool
        // passed its own scope check; a stored file needs books:read.
        } else if matches!(wanted, Wanted::Attachment(_)) && !token.scopes.contains(&Scope::Read) {
            Err((
                StatusCode::FORBIDDEN,
                "insufficient_scope",
                format!(
                    "this agent's token can't do that: a download needs books:read, and its token ({:?}) hasn't it",
                    token.name
                ),
            ))
        } else {
            match &wanted {
                Wanted::Export(id) => {
                    match exports.get(id, &token.id, chrono::Utc::now().timestamp()) {
                        Ok((x, bytes)) => Ok((x.file_name, x.mime, bytes)),
                        Err(e @ ExportError::Unknown(_)) => {
                            Err((StatusCode::NOT_FOUND, "no_such_export", e.to_string()))
                        }
                        Err(e @ ExportError::NotYours) => {
                            Err((StatusCode::FORBIDDEN, "not_yours", e.to_string()))
                        }
                        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, "failed", e.to_string())),
                    }
                }
                // An id as the books write it (no sign, no leading zeros),
                // so the log holds no alias of it.
                Wanted::Attachment(a) => {
                    match a.parse::<u64>().ok().filter(|n| n.to_string() == *a) {
                        None => Err(not_found("no_such_attachment", "attachment")),
                        Some(id) => match store_for.attachment_file(id) {
                            Ok(Some(file)) => Ok(file),
                            Ok(None) => Err(not_found("no_such_attachment", "attachment")),
                            // The stored file damaged: not a missing one.
                            Err(why) => Err((StatusCode::INTERNAL_SERVER_ERROR, "failed", why)),
                        },
                    }
                }
            }
        };
        let record = Access {
            principal: Principal {
                id: token.id.clone(),
                kind: "agent".into(),
                label: token.name.clone(),
            },
            client: None,
            request_id: None,
            action: action.into(),
            subject,
            args_digest: None,
            result: match &found {
                Ok(_) => Outcome::Ok,
                Err((_, code, _)) => Outcome::Refused((*code).into()),
            },
            head_after: None,
            agent: None,
        };
        let recorded = access.record(&record).is_ok();
        (found, recorded)
    });
    let Ok((found, true)) = task.await else {
        store.stop();
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(REFUSED_HEADER, "not-recorded")],
            crate::access::NOT_RECORDED,
        )
            .into_response();
    };
    match found {
        Ok((name, mime, bytes)) => {
            let digest = format!(
                "sha-256=:{}:",
                base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    <sha2::Sha256 as sha2::Digest>::digest(&bytes)
                )
            );
            let mime = axum::http::HeaderValue::from_str(&mime).unwrap_or(
                axum::http::HeaderValue::from_static("application/octet-stream"),
            );
            (
                StatusCode::OK,
                [
                    (axum::http::header::CONTENT_TYPE, mime),
                    (
                        axum::http::header::CONTENT_DISPOSITION,
                        axum::http::HeaderValue::from_str(&disposition(&name))
                            .expect("only visible ASCII"),
                    ),
                    (
                        axum::http::header::X_CONTENT_TYPE_OPTIONS,
                        axum::http::HeaderValue::from_static("nosniff"),
                    ),
                    (
                        axum::http::HeaderName::from_static(DIGEST_HEADER),
                        axum::http::HeaderValue::from_str(&digest).expect("base64 is ASCII"),
                    ),
                ],
                bytes,
            )
                .into_response()
        }
        Err((status, code, why)) => (status, [(REFUSED_HEADER, code)], why).into_response(),
    }
}

/// The header a download's SHA-256 rides in (RFC 9530 Repr-Digest), so
/// `wanderbooks download` keeps only what was stored.
pub const DIGEST_HEADER: &str = "repr-digest";

/// A download's Content-Disposition (RFC 6266): its name in ASCII, every
/// other character `_`, and whole in UTF-8 as `filename*` (RFC 8187).
fn disposition(name: &str) -> String {
    let plain: String = name
        .chars()
        .map(|c| {
            if (c.is_ascii_graphic() || c == ' ') && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    format!("attachment; filename=\"{plain}\"; filename*=UTF-8''{encoded}")
}

/// `/b/{book}/mcp`: the book's MCP service, for a request carrying a token
/// for that book (INVARIANT 8).
async fn mcp(
    axum::extract::State(books): axum::extract::State<Arc<Books>>,
    axum::extract::Path(book): axum::extract::Path<String>,
    mut request: axum::extract::Request,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use tower::ServiceExt;
    // Counted before the closing check, so closing never misses it.
    let counted = (request.method() != axum::http::Method::GET).then(|| InFlight::new(&books));
    books
        .last_arrival
        .store(books.started.elapsed().as_millis() as u64, Ordering::SeqCst);
    let method = request.method().clone();
    // Inside the book's door (INVARIANT 10) until the request is answered:
    // a close of the book waits for it, and its tools are counted by the
    // store (SharedStore INVARIANT 3).
    let (held, token, inside) =
        match door(&books, &book, &Asked::connect(&method), request.headers()).await {
            Ok(passed) => passed,
            Err(refused) => return *refused,
        };
    let Book::Open { service, board, .. } = &held else {
        unreachable!("the door lets only open books through")
    };
    let (service, board) = ((**service).clone(), board.clone());
    let principal = |t: &Token| Principal {
        id: t.id.clone(),
        kind: "agent".into(),
        label: t.name.clone(),
    };
    // Its body watched: a close whose grace ran out cuts it while it is
    // still arriving (never once whole), before any tool has seen it.
    let (rewatched, arrival) = inside.arriving(request);
    request = rewatched;
    let who = principal(&token);
    let session = request
        .headers()
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    // A session is bound when it is opened (below): one bound to another
    // token is refused; one never bound isn't one this server opened, and
    // is left to the MCP service, which doesn't know it either (and says
    // so, which a client takes as "open a new one").
    let (elsewhere, full) = {
        let bound = books
            .session_tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let elsewhere = session
            .as_ref()
            .and_then(|sid| bound.get(sid))
            .is_some_and(|owner| owner.token != token.id);
        (elsewhere, session.is_none() && bound.len() >= MAX_SESSIONS)
    };
    // The cap is on opening sessions: a request opens one only if it is an
    // initialize (a 2026-07-28 call without a session opens none).
    let full = full && {
        let (parts, body) = request.into_parts();
        let bytes = axum::body::to_bytes(body, MAX_BODY)
            .await
            .unwrap_or_default();
        let opens = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .is_some_and(|m| m.get("method").and_then(|v| v.as_str()) == Some("initialize"));
        request = axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes));
        opens
    };
    if arrival.was_cut() {
        return cut_by_the_close(&held, inside, who, &Asked::connect(&method)).await;
    }
    if full {
        return refused_at_door(
            &held,
            inside,
            principal(&token),
            &Asked::connect(&method),
            "too_many_sessions",
            StatusCode::SERVICE_UNAVAILABLE,
            "the books server has too many sessions open: end some, then try again".into(),
        )
        .await;
    }
    if elsewhere {
        return refused_at_door(
            &held,
            inside,
            principal(&token),
            &Asked::connect(&method),
            "session_token",
            StatusCode::FORBIDDEN,
            "this session was opened with another token".into(),
        )
        .await;
    }
    let (token_id, token_name) = (token.id.clone(), token.name.clone());
    request
        .extensions_mut()
        .insert(crate::access::Caller { token });
    let response = match service.oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    };
    // The service reads a whole body before it hands any message to a
    // session or a tool (rmcp's `expect_json`, which ends in an error
    // answer on a body that fails), so a body cut here reached nothing:
    // no call ran, nothing is in the journal.
    if arrival.was_cut() {
        drop(response);
        return cut_by_the_close(&held, inside, who, &Asked::connect(&method)).await;
    }
    // A session the answer opened is bound to the token that opened it;
    // one ended, or found gone, is forgotten.
    {
        let mut bound = books
            .session_tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if session.is_none()
            && let Some(sid) = response
                .headers()
                .get(SESSION_HEADER)
                .and_then(|v| v.to_str().ok())
        {
            bound.insert(
                sid.to_string(),
                Bound {
                    token: token_id,
                    name: token_name,
                    book: book.clone(),
                    at: chrono::Utc::now().timestamp(),
                },
            );
        }
        let ended = method == axum::http::Method::DELETE && response.status().is_success();
        let gone = response.status() == StatusCode::NOT_FOUND;
        if (ended || gone)
            && let Some(sid) = &session
        {
            bound.remove(sid);
            board.forget(sid);
        }
    }
    // The answer may still be on its way: it counts, for the server's close
    // and for this book's, until its body ends. A standalone event stream
    // (a GET) is no answer, and ends only with its session, after the close
    // has waited for the answers.
    let answering = (method != axum::http::Method::GET).then(|| inside.answering());
    response.map(|body| {
        use http_body_util::BodyExt;
        axum::body::Body::new(body.map_frame(move |frame| {
            let _held = (&counted, &answering);
            frame
        }))
    })
}

/// The one router both doors serve (the local socket and the tunnel).
fn router(books: Arc<Books>) -> axum::Router {
    axum::Router::new()
        .route("/b/{book}/mcp", axum::routing::any(mcp))
        .route("/b/{book}/u/{upload}", axum::routing::put(upload))
        .route("/b/{book}/a/{attachment}", axum::routing::get(download))
        .route("/b/{book}/x/{export}", axum::routing::get(export))
        .fallback(not_found)
        .with_state(books)
}

/// The door in-process, for the fuzz targets (P12): the same router, fed
/// bytes without a socket. Not built unless asked for.
#[cfg(feature = "fuzzing")]
pub mod fuzz_door;

/// Anything but `/b/{book}/mcp`.
async fn not_found() -> (axum::http::StatusCode, &'static str) {
    (axum::http::StatusCode::NOT_FOUND, "not found")
}

/// One admin connection: one request, one answer (admin INVARIANT 1).
async fn serve_admin(stream: ipc::Accepted, books: Arc<Books>) {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
    let (read, mut write) = ipc::split_accepted(stream);
    let mut reader = tokio::io::BufReader::new(read.take(admin::MAX_LINE as u64 + 1));
    let mut line = String::new();
    // A request still arriving when closing begins is turned away.
    let read = tokio::select! {
        read = tokio::time::timeout(ADMIN_READ_TIMEOUT, reader.read_line(&mut line)) => Some(read),
        () = books.closing.cancelled() => None,
    };
    let answer = match read {
        None => Response::Refused("the books are closing: nothing was done".into()),
        Some(Err(_)) => Response::Refused("no request arrived in time".into()),
        Some(Ok(Err(e))) => {
            Response::Refused(format!("the request couldn't be read: {}", e.kind()))
        }
        Some(Ok(Ok(_))) if !line.ends_with('\n') => {
            Response::Refused("the request is too long, or cut off".into())
        }
        Some(Ok(Ok(_))) => match serde_json::from_str::<admin::Envelope>(&line) {
            Err(e) => Response::Refused(format!("the request isn't understood: {e}")),
            Ok(envelope) if !admin::accepts(envelope.proto) => Response::Refused(
                "this command is from another version of wanderbooks than the books server: \
                 close the books (`wanderbooks close`) and try again"
                    .into(),
            ),
            Ok(_) if books.closing.is_cancelled() => {
                Response::Refused("the books are closing: nothing was done".into())
            }
            Ok(envelope) => {
                let books = books.clone();
                tokio::task::spawn_blocking(move || books.admin(envelope.request))
                    .await
                    .unwrap_or_else(|_| {
                        Response::Refused(
                            "the books server hit an internal error doing that".into(),
                        )
                    })
            }
        },
    };
    let mut text = serde_json::to_string(&answer).unwrap_or_else(|_| {
        r#"{"refused":"the books server couldn't write its answer"}"#.to_string()
    });
    text.push('\n');
    let _ = write.write_all(text.as_bytes()).await;
    let _ = write.shutdown().await;
}

/// Bind a socket in `run/`, replacing a stale one (INVARIANT 1: only the
/// server holding the lock gets here).
fn bind(run: &Path, name: &str) -> std::io::Result<ipc::Listener> {
    ipc::Listener::bind(&ipc::Endpoint::new(run, name)?)
}

/// The address `--http` takes: loopback only (plan 3.9, P9a).
///
/// The tunnel's door is the same plaintext MCP service with a static
/// bearer token; a TLS proxy or tunnel runs in front of it on the same
/// machine. Any other address would put the books' plaintext door on the
/// network, which 3.9 and P9a do not allow, so it is refused rather than
/// bound (or warned about).
pub fn tunnel_address(text: &str) -> Result<std::net::SocketAddr, String> {
    let addr: std::net::SocketAddr = text
        .parse()
        .map_err(|e| format!("--http takes an address like 127.0.0.1:7411: {e}"))?;
    if !addr.ip().is_loopback() {
        return Err(format!(
            "--http is loopback only (127.0.0.1:7411 or [::1]:7411), not {addr}: this door is plaintext, so the TLS proxy or tunnel belongs in front of loopback"
        ));
    }
    Ok(addr)
}

fn print_usage(prog: &str) {
    eprintln!("usage: {prog} [--root DIR] [--isolation LEVEL] [--http [ADDR]] [--open BOOKS]");
    eprintln!("  The books server for the data root: started by the host, not by hand.");
    eprintln!("  It runs until its standard input ends, or SIGTERM, SIGINT or SIGHUP.");
    eprintln!("  --isolation same_user | enforced: what books.start says of how these books");
    eprintln!("    are kept from agents (enforced on the owner's own machine: P9a).");
    eprintln!("  --http [ADDR]: also serve the MCP door on loopback only (default");
    eprintln!("    127.0.0.1:7411), for a TLS proxy or a tunnel in front of it. Off unless");
    eprintln!("    given; any other address is refused.");
    eprintln!("  --open all | none | ID[,ID...]: which books are open at the start (default");
    eprintln!("    all). A book left closed opens when asked (`wanderbooks books open ID`).");
}

/// `wanderbooks server`: serve every book in the data root until told to
/// close (INVARIANT 6).
pub fn run(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut root: Option<PathBuf> = None;
    let mut backup_quiet = BACKUP_QUIET;
    let mut drill_first = DRILL_FIRST;
    let mut timers = Timers::default();
    let mut isolation = Isolation::SameUser;
    let mut tunnel: Option<std::net::SocketAddr> = None;
    let mut open_at_start = OpenAtStart::All;
    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = Some(PathBuf::from(args.next().ok_or("--root needs a folder")?)),
            // How the books are kept from agents (plan 3.8, P9a): the
            // system unit on the owner's own machine passes `enforced`;
            // the server never guesses.
            "--isolation" => {
                let value = args
                    .next()
                    .ok_or("--isolation needs same_user or enforced")?;
                isolation = Isolation::parse(&value)?;
            }
            // The tunnel's door (plan 3.9, P9a): the same MCP service on
            // loopback, for a TLS proxy or tunnel in front of it. Off
            // unless asked for; the bearer token still guards every call.
            "--http" => {
                let addr = match args.peek() {
                    Some(v) if !v.starts_with('-') => args.next().unwrap_or_default(),
                    _ => "127.0.0.1:7411".to_string(),
                };
                tunnel = Some(tunnel_address(&addr)?);
            }
            // Which books are open at the start (plan/p15-app-and-relay.md
            // 3.1): the app opens a company as its tab opens.
            "--open" => {
                open_at_start =
                    OpenAtStart::parse(&args.next().ok_or("--open needs all, none, or ids")?)?;
            }
            // How long the books stay unwritten before a backup (tests).
            "--backup-quiet" => {
                let ms: u64 = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--backup-quiet needs milliseconds")?;
                backup_quiet = Duration::from_millis(ms);
            }
            // How soon after books open a drill is first looked for (tests).
            "--drill-first" => {
                let ms: u64 = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--drill-first needs milliseconds")?;
                drill_first = Duration::from_millis(ms);
            }
            // The periods of the sweep, the backup look and the drill look
            // (tests: every timer at a few milliseconds).
            "--timers" => {
                let ms: u64 = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--timers needs milliseconds")?;
                let every = Duration::from_millis(ms.max(1));
                (timers.sweep_every, timers.backup_look, timers.drill_look) = (every, every, every);
            }
            // How long a book's last backup may take at its close (tests).
            "--close-backup" => {
                let ms: u64 = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--close-backup needs milliseconds")?;
                timers.backup_at_close = Duration::from_millis(ms);
            }
            // How long a book's close waits for answers on their way out
            // (tests).
            "--answer-grace" => {
                let ms: u64 = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--answer-grace needs milliseconds")?;
                timers.answer_grace = Duration::from_millis(ms);
            }
            // Make a drill or a format upgrade slow, as a large book makes
            // them (tests): each phase waits this long, or until the close
            // hurries it.
            "--slow-steps" => {
                let ms: u64 = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--slow-steps needs milliseconds")?;
                timers.slow_steps = Duration::from_millis(ms);
            }
            // A timer waits between taking a book's handles and acting on
            // them, while `run/timer-hold` exists (tests).
            "--timer-hold" => timers.timer_hold = true,
            "--help" | "-h" => {
                print_usage(prog);
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                print_usage(prog);
                return Err(Box::new(crate::Exit(2)));
            }
        }
    }
    let root = match Root::find(root.as_deref()) {
        Ok(root) => root,
        Err(e) => {
            eprintln!("wanderbooks server: {e}");
            return Err(Box::new(crate::Exit(2)));
        }
    };
    let run = root.run_dir().map_err(|e| e.to_string())?;
    // INVARIANT 1: the lock first, before anything else is opened.
    let lock_path = run.join(SERVER_LOCK);
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("can't open the server lock: {}", e.kind()))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    // INVARIANT 6: told to close at any point (a long wait or rebuild
    // included), the server closes cleanly as soon as it can.
    let stops = Stops::default();
    {
        let _in_runtime = rt.enter();
        watch_for_close(&stops)?;
    }
    // INVARIANT 1: a server still finishing (its host gone) has the lock;
    // this one waits for it, touching nothing, until told to close.
    let mut said = false;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {
                if stops.close.is_cancelled() {
                    return Ok(());
                }
                if !said {
                    eprintln!(
                        "wanderbooks server: another books server has these books open; waiting for it to finish"
                    );
                    said = true;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(format!("can't take the server lock: {}", e.kind()).into());
            }
        }
    }
    {
        // Only once it is ours: a refused server touches nothing.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("can't make the server lock private: {}", e.kind()))?;
        }
        #[cfg(windows)]
        ledger_store::winsec::set_owner_only(&lock_path)
            .map_err(|e| format!("can't make the server lock private: {}", e.kind()))?;
    }
    let books = Arc::new(
        Books::new(root.clone(), backup_quiet, drill_first, isolation).with_timers(timers),
    );
    {
        let loaded = books.load_principals();
        if let Err(why) = &loaded {
            eprintln!("wanderbooks server: agents' tokens can't be read: {why}");
        }
        *books.principals.write().unwrap_or_else(|e| e.into_inner()) = loaded;
    }
    // INVARIANT 3: every book chosen to be open is tried before ready; the
    // others are held closed.
    let ids = root.list_books().map_err(|e| e.to_string())?;
    if let OpenAtStart::Only(wanted) = &open_at_start
        && let Some(unknown) = wanted
            .iter()
            .find(|w| !ids.iter().any(|i| i.as_str() == w.as_str()))
    {
        eprintln!(
            "wanderbooks server: --open names books that don't exist: {unknown:?} (`wanderbooks books list` lists them)"
        );
        return Err(Box::new(crate::Exit(2)));
    }
    for id in ids {
        let book = if open_at_start.wants(&id) {
            books.open(&id)
        } else {
            Book::Closed
        };
        books.hold(&id, book);
    }
    let served = if stops.close.is_cancelled() {
        eprintln!("wanderbooks server: told to close while opening: closed");
        Ok(())
    } else {
        rt.block_on(serve(books.clone(), &run, &stops, tunnel))
    };
    // No request is running now (serve waited for each); the stores go
    // before the lock, and the process exit would release both.
    books
        .held
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    rt.shutdown_timeout(Duration::from_secs(1));
    for name in [MCP_SOCKET, ADMIN_SOCKET] {
        let _ = std::fs::remove_file(run.join(name));
    }
    drop(lock);
    served
}

/// How long, once every running request has finished, the server still
/// waits for answers on their way out (or requests still arriving).
const ANSWER_GRACE: Duration = Duration::from_secs(10);
/// How long the door stays open, refusing, after the last request came.
const LINGER: Duration = Duration::from_millis(500);
/// The longest it stays open so.
const LINGER_MAX: Duration = Duration::from_secs(3);
/// How long the HTTP side then has to end before it is dropped.
const HTTP_GRACE: Duration = Duration::from_secs(5);

/// Being told to close: once (`close`), and again (`hurry`: stop waiting
/// for answers on their way out; never for a request still running).
#[derive(Default)]
struct Stops {
    close: CancellationToken,
    hurry: CancellationToken,
}

/// Bind, say ready, serve, and close (INVARIANTS 5, 6).
async fn serve(
    books: Arc<Books>,
    run: &Path,
    stops: &Stops,
    tunnel: Option<std::net::SocketAddr>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mcp_listener = OwnerOnly(bind(run, MCP_SOCKET)?);
    let mut admin_listener = OwnerOnly(bind(run, ADMIN_SOCKET)?);
    // The tunnel's door (plan 3.9, P9a): the same service on 127.0.0.1,
    // for the TLS proxy or tunnel in front of it. Bound before ready, so
    // what starts beside the server finds it; a bind that fails is said
    // and is not fatal (3.9: the books' own sockets still serve).
    let tunnel_listener = match tunnel {
        Some(addr) => match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => Some(listener),
            Err(e) => {
                eprintln!(
                    "wanderbooks server: the tunnel's door at {addr} can't be bound ({e}); serving the local sockets only"
                );
                None
            }
        },
        None => None,
    };
    let stop = stops.close.clone();
    {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        writeln!(out, "{READY}")?;
        out.flush()?;
    }
    let held: Vec<String> = books.held().keys().cloned().collect();
    eprintln!("wanderbooks server: ready, holding {} books", held.len());
    // Every open book is backed up when it opens, once its writing has
    // stopped for a while, and again after a failed run (backup.rs).
    let (backing, backup_stop) = (books.clone(), stop.clone());
    let backup_loop = tokio::spawn(async move { keep_backed_up(backing, backup_stop).await });
    // Every open book's lapsed uploads go within SWEEP_EVERY of lapsing,
    // whatever else the server is doing (uploads.rs INVARIANT 3). A sweep
    // of a book is inside its door, as a request is (INVARIANT 10): a close
    // waits for it, and a book whose door is shut isn't swept; the book's
    // uploads and exports are closed as it is let go, so a sweep that
    // took its handles earlier can't touch a book that closed or was
    // opened again since.
    let (sweeping, sweep_stop) = (books.clone(), stop.clone());
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(sweeping.timers.sweep_every);
        loop {
            tokio::select! {
                () = sweep_stop.cancelled() => break,
                _ = tick.tick() => {
                    let handles: Vec<_> = sweeping
                        .held()
                        .values()
                        .filter_map(|b| match b {
                            Book::Open { uploads, exports, gate, .. } => {
                                Some((uploads.clone(), exports.clone(), gate.clone()))
                            }
                            Book::Failed(_) | Book::Closed => None,
                        })
                        .collect();
                    if !handles.is_empty() {
                        sweeping.timer_hold("sweep").await;
                    }
                    // Inside each book's door, entered only now: a book
                    // that closed since the handles were taken is skipped
                    // (and its handles are closed, whatever this does).
                    let held: Vec<_> = handles
                        .into_iter()
                        .filter_map(|(u, x, gate)| gate.enter().map(|inside| (u, x, inside)))
                        .collect();
                    let now = chrono::Utc::now().timestamp();
                    let _ = tokio::task::spawn_blocking(move || {
                        for (u, x, _inside) in &held {
                            u.sweep(now);
                            x.sweep(now);
                        }
                    })
                    .await;
                }
            }
        }
    });

    let app = router(books.clone());
    // The HTTP side keeps answering (refusing, once closing) until the
    // close is done: a client is never cut off mid-request by it.
    let shut = CancellationToken::new();
    let graceful = shut.clone();
    let tunnel_app = app.clone();
    let mut http = tokio::spawn(async move {
        axum::serve(mcp_listener, app)
            .with_graceful_shutdown(async move { graceful.cancelled().await })
            .await
    });
    let tunnel_http = tunnel_listener.map(|listener| {
        let app = tunnel_app;
        let graceful = shut.clone();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move { graceful.cancelled().await })
                .await
        })
    });
    let admin_jobs = TaskTracker::new();
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            stream = admin_listener.next() => {
                admin_jobs.spawn(serve_admin(stream, books.clone()));
            }
        }
    }
    drop(admin_listener);
    eprintln!("wanderbooks server: closing");
    // INVARIANT 6. New requests are refused from here on, and (INVARIANT
    // 15) every book's door is shut and the cut of what hasn't finished
    // arriving is set for one grace from now, at the start: the one thing
    // a stalled client may not do is hold the close, whatever else the
    // close is waiting for (an admin job, a slow backup of another book).
    books.closing.cancel();
    // The admin jobs that can run long stop at their next look, before
    // their point of no return (INVARIANT 10): the wait below is for jobs
    // that are stopping, not for a restore to run its course.
    books.stop_every_job();
    let begun = Instant::now();
    let cutters: Vec<_> = books
        .gates()
        .into_iter()
        .map(|gate| {
            gate.shut();
            let at = begun + books.timers.answer_grace;
            tokio::spawn(async move {
                tokio::time::sleep_until(at.into()).await;
                gate.cut_unfinished();
            })
        })
        .collect();
    // Admin work under way (a create, an adoption, a close of one book)
    // finishes.
    admin_jobs.close();
    admin_jobs.wait().await;
    drop(backup_loop);
    // Every book still open is let go the way a close of one book lets go
    // of it (`Books::release`: INVARIANT 10, the one closing path): its
    // tools finish, however long they take and whether or not their client
    // is still there (a store is never let go of mid-write, SharedStore
    // INVARIANT 3), the answers on their way out are sent, a request not
    // whole at the grace is cut, its sessions end, its last backup is made
    // for BACKUP_AT_CLOSE at most, and its logs, uploads, exports, backups
    // and store are closed. The books are let go together: one slow
    // backup doesn't hold the others' (each book's own bound is the same).
    // (Every admin job has ended: `admin_jobs.wait()` above. No book's
    // changes are in flight to be waited for.)
    let open: Vec<(BookId, Parts)> = {
        let open: Vec<(BookId, Parts)> = books
            .held()
            .iter()
            .filter_map(|(id, b)| Some((BookId::parse(id).ok()?, b.parts()?)))
            .collect();
        for (id, _) in &open {
            books.hold(id, Book::Closed);
        }
        open
    };
    let released = futures_util::future::join_all(
        open.iter()
            .map(|(id, parts)| books.release(id, parts, true, begun, Some(&stops.hurry))),
    )
    .await;
    for ((id, _), problem) in open.iter().zip(released) {
        if let Some(why) = problem {
            eprintln!("wanderbooks server: the last backup of the books {id} wasn't whole: {why}");
        }
    }
    for cutter in cutters {
        cutter.abort();
    }
    books.sessions.cancel();
    // A call that set out just before its client heard of the close still
    // meets a refusal, never a connection cut mid-way (which it couldn't
    // tell from a call that ran): the door stays open, refusing, until
    // none has come for LINGER.
    let linger = async {
        let deadline = tokio::time::Instant::now() + LINGER_MAX;
        loop {
            let since = (books.started.elapsed().as_millis() as u64)
                .saturating_sub(books.last_arrival.load(Ordering::SeqCst));
            let quiet = Duration::from_millis(since);
            if quiet >= LINGER || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(LINGER - quiet).await;
        }
    };
    tokio::select! {
        () = linger => {}
        () = stops.hurry.cancelled() => {}
    }
    shut.cancel();
    match tokio::time::timeout(HTTP_GRACE, &mut http).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => eprintln!("wanderbooks server: {e}"),
        Ok(Err(e)) => eprintln!("wanderbooks server: {e}"),
        Err(_) => http.abort(),
    }
    if let Some(mut tunnel) = tunnel_http {
        match tokio::time::timeout(HTTP_GRACE, &mut tunnel).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(e))) => eprintln!("wanderbooks server: {e}"),
            Ok(Err(e)) => eprintln!("wanderbooks server: {e}"),
            Err(_) => tunnel.abort(),
        }
    }
    eprintln!("wanderbooks server: closed");
    Ok(())
}

/// Close when standard input ends (the host let go, or died) or on
/// SIGTERM, SIGINT or SIGHUP; hurry on the next signal. Runs inside the
/// runtime, before the books are opened.
fn watch_for_close(stops: &Stops) -> std::io::Result<()> {
    let on_eof = stops.close.clone();
    // A plain thread: a blocked read must never hold up the runtime's end.
    std::thread::Builder::new()
        .name("stdin-watch".into())
        .spawn(move || {
            use std::io::Read;
            let mut sink = [0u8; 256];
            let mut stdin = std::io::stdin();
            loop {
                match stdin.read(&mut sink) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            on_eof.cancel();
        })?;
    for mut signals in ipc::stop_signals()? {
        let (close, hurry) = (stops.close.clone(), stops.hurry.clone());
        tokio::spawn(async move {
            while signals.recv().await.is_some() {
                if close.is_cancelled() {
                    hurry.cancel();
                } else {
                    close.cancel();
                }
            }
        });
    }
    Ok(())
}

/// What the backup loop knows of one open book.
struct Seen {
    /// Its backups, by identity: books restored over others are new.
    backups: Arc<crate::backup::Backups>,
    watch: Watch,
}

/// One open book's backups, as the loop has watched them.
#[derive(Debug, Clone, Copy)]
struct Watch {
    /// Its head's seq when last looked at, and when that changed.
    seq: u64,
    moved_at: Instant,
    /// The seq last committed and offered to the targets.
    backed: Option<u64>,
    /// When the last run's commit failed, if it did: nothing runs again
    /// for RETRY_AFTER.
    failed_at: Option<Instant>,
    /// When the last run left a target failing, if it did: it is tried
    /// again RETRY_AFTER on, written to or not.
    target_failed_at: Option<Instant>,
    /// When a drill is next looked for.
    drill_next: Instant,
}

impl Watch {
    /// Books first seen open now, at `seq`: a drill is first looked for
    /// `drill_first` from now.
    fn new(seq: u64, now: Instant, drill_first: Duration) -> Self {
        Watch {
            seq,
            moved_at: now,
            backed: None,
            failed_at: None,
            target_failed_at: None,
            drill_next: now + drill_first,
        }
    }

    /// A run through `seq` came to `outcome` at `now`: None when its
    /// commit failed, else whether every target holds it.
    fn ran(&mut self, seq: u64, outcome: Option<bool>, now: Instant) {
        match outcome {
            None => self.failed_at = Some(now),
            Some(all) => {
                self.backed = Some(seq);
                self.failed_at = None;
                self.target_failed_at = (!all).then_some(now);
            }
        }
    }
}

/// Why a backup is due now, if one is: the books just seen open; or
/// written and quiet for `quiet` since (not yet backed up); or a target
/// left failing RETRY_AFTER ago. Nothing within RETRY_AFTER of a failed
/// commit.
fn backup_due(
    seen: Option<&Watch>,
    seq: u64,
    now: Instant,
    quiet: Duration,
) -> Option<&'static str> {
    let Some(s) = seen else {
        return Some("open");
    };
    let after =
        |t: Option<Instant>| t.is_some_and(|f| now.duration_since(f) >= crate::backup::RETRY_AFTER);
    if s.failed_at.is_some() && !after(s.failed_at) {
        return None;
    }
    let quiet_now = if s.seq == seq {
        now.duration_since(s.moved_at) >= quiet
    } else {
        quiet.is_zero()
    };
    if s.backed != Some(seq) && quiet_now {
        return Some("write");
    }
    after(s.target_failed_at).then_some("retry")
}

/// Whether a run's every target holds the books: a target paused for
/// these books (restored as a copy of others) is no failure to try again.
fn every_target_held(status: &crate::backup::Status) -> bool {
    status
        .targets
        .iter()
        .all(|t| t.paused || t.last_error.is_none())
}

/// Why the last backup at a close wasn't whole, if it wasn't: a target
/// that failed, or lacks the books' head (a target paused for books
/// restored as a copy of others is no failure). None when there is
/// nothing to say, or no target.
fn backup_incomplete(status: &crate::backup::Status) -> Option<String> {
    let lacking: Vec<String> = status
        .targets
        .iter()
        .filter(|t| !t.paused)
        .filter_map(|t| match (&t.last_error, t.through_seq) {
            (Some(why), _) => Some(format!("{} failed: {why}", t.at)),
            (None, through) if through != Some(status.head_seq) => {
                Some(format!("{} doesn't hold the books' latest", t.at))
            }
            (None, _) => None,
        })
        .collect();
    (!lacking.is_empty()).then(|| format!("the backup to {}", lacking.join("; ")))
}

/// Whether to look for a drill due now (then again DRILL_LOOK on).
fn drill_look_due(seen: &Watch, now: Instant) -> bool {
    now >= seen.drill_next
}

/// Back every open book up (backup.rs) whenever [`backup_due`] says, and
/// drill its backups when a drill is due (looked for every DRILL_LOOK).
/// Until `stop`.
async fn keep_backed_up(books: Arc<Books>, stop: CancellationToken) {
    let mut seen: BTreeMap<String, Seen> = BTreeMap::new();
    let mut tick = tokio::time::interval(books.timers.backup_look);
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            _ = tick.tick() => {}
        }
        let open: Vec<(String, Arc<crate::backup::Backups>)> = books
            .held()
            .iter()
            .filter_map(|(id, b)| match b {
                Book::Open { backups, .. } => Some((id.clone(), backups.clone())),
                Book::Failed(_) | Book::Closed => None,
            })
            .collect();
        seen.retain(|id, _| open.iter().any(|(o, _)| o == id));
        for (id, backups) in open {
            if stop.is_cancelled() {
                return;
            }
            let Some((seq, _)) = backups.head().await else {
                continue;
            };
            let now = Instant::now();
            // Books restored over others are seen anew.
            if seen
                .get(&id)
                .is_some_and(|s| !Arc::ptr_eq(&s.backups, &backups))
            {
                seen.remove(&id);
            }
            let due = backup_due(
                seen.get(&id).map(|s| &s.watch),
                seq,
                now,
                books.backup_quiet,
            );
            let s = &mut seen
                .entry(id)
                .or_insert(Seen {
                    backups: backups.clone(),
                    watch: Watch::new(seq, now, books.drill_first),
                })
                .watch;
            if s.seq != seq {
                s.seq = seq;
                s.moved_at = now;
            }
            if let Some(why) = due {
                books.timer_hold("backup").await;
                let outcome = backups
                    .run(why)
                    .await
                    .ok()
                    .map(|status| every_target_held(&status));
                s.ran(seq, outcome, now);
            }
            if stop.is_cancelled() {
                return;
            }
            // Drills (backup.rs INVARIANT 5).
            if drill_look_due(s, now) {
                s.drill_next = now + books.timers.drill_look;
                if backups.drill_due().await {
                    books.timer_hold("drill").await;
                    backups.drill().await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backup_is_due_on_open_then_once_quiet_after_a_write() {
        let t0 = Instant::now();
        let quiet = Duration::from_secs(30);
        let s = |seq, moved: u64, backed, failed: Option<u64>| Watch {
            seq,
            moved_at: t0 + Duration::from_secs(moved),
            backed,
            failed_at: failed.map(|f| t0 + Duration::from_secs(f)),
            target_failed_at: None,
            drill_next: t0,
        };
        let at = |secs: u64| t0 + Duration::from_secs(secs);
        // First seen: at once.
        assert_eq!(backup_due(None, 5, t0, quiet), Some("open"));
        // Backed up through the head: nothing.
        assert_eq!(
            backup_due(Some(&s(5, 0, Some(5), None)), 5, at(100), quiet),
            None
        );
        // Written: not until it has been quiet `quiet`.
        assert_eq!(
            backup_due(Some(&s(5, 0, Some(5), None)), 6, at(1), quiet),
            None
        );
        assert_eq!(
            backup_due(Some(&s(6, 10, Some(5), None)), 6, at(39), quiet),
            None
        );
        assert_eq!(
            backup_due(Some(&s(6, 10, Some(5), None)), 6, at(40), quiet),
            Some("write")
        );
        // No quiet asked for: at once.
        assert_eq!(
            backup_due(Some(&s(5, 0, Some(5), None)), 6, at(1), Duration::ZERO),
            Some("write")
        );
        // A failure waits RETRY_AFTER.
        let retry = crate::backup::RETRY_AFTER.as_secs();
        let failed = s(6, 10, Some(5), Some(50));
        assert_eq!(
            backup_due(Some(&failed), 6, at(50 + retry - 1), quiet),
            None
        );
        assert_eq!(
            backup_due(Some(&failed), 6, at(50 + retry), quiet),
            Some("write")
        );
        // A drill is first looked for `drill_first` after the books are
        // seen open, never at once.
        let first = Duration::from_secs(60);
        let w = Watch::new(1, t0, first);
        assert!(!drill_look_due(&w, t0));
        assert!(!drill_look_due(&w, t0 + first - Duration::from_millis(1)));
        assert!(drill_look_due(&w, t0 + first));
        assert_eq!((w.backed, w.failed_at, w.seq), (None, None, 1));
        // A run's outcome: its commit failed (nothing again for a
        // while), or made and some target failing (tried again later,
        // while writes still go to the others), or all well.
        let mut w = Watch::new(1, t0, first);
        w.ran(4, None, at(5));
        assert_eq!(
            (w.backed, w.failed_at, w.target_failed_at),
            (None, Some(at(5)), None)
        );
        w.ran(4, Some(false), at(9));
        assert_eq!(
            (w.backed, w.failed_at, w.target_failed_at),
            (Some(4), None, Some(at(9)))
        );
        w.seq = 5;
        w.moved_at = at(10);
        assert_eq!(backup_due(Some(&w), 5, at(10 + 30), quiet), Some("write"));
        w.backed = Some(5);
        assert_eq!(backup_due(Some(&w), 5, at(9 + retry - 1), quiet), None);
        assert_eq!(backup_due(Some(&w), 5, at(9 + retry), quiet), Some("retry"));
        // A paused target is no failure; another's error is.
        let target = |paused: bool, error: Option<&str>| crate::backup::TargetStatus {
            target: "repository".into(),
            at: "x".into(),
            last_ok_at: None,
            through_seq: None,
            stale: false,
            last_error: error.map(str::to_string),
            last_drill_at: None,
            drill_error: None,
            paused,
        };
        let status = |targets| crate::backup::Status {
            head_seq: 5,
            targets,
            note: None,
        };
        assert!(every_target_held(&status(vec![
            target(true, Some("paused")),
            target(false, None)
        ])));
        assert!(!every_target_held(&status(vec![
            target(false, Some("no")),
            target(false, None)
        ])));
        assert!(every_target_held(&status(vec![])));
        w.ran(5, Some(true), at(9 + retry));
        assert_eq!((w.failed_at, w.target_failed_at), (None, None));
        assert_eq!(backup_due(Some(&w), 5, at(9 + retry * 3), quiet), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_pair_is_this_account_at_both_ends() {
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        assert_eq!(ipc::peer_uid(&a).unwrap(), ipc::own_uid());
        assert!(ipc::peer_is_us(&b));
        assert!(check_peer_std(&a).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_pair_names_this_process() {
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        if cfg!(target_os = "linux") {
            assert_eq!(ipc::peer_pid(&a), Some(std::process::id()));
        }
        let file = std::fs::File::open("/dev/null").unwrap();
        assert_eq!(ipc::peer_pid(&file), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_descriptor_that_isnt_a_socket_is_never_taken_for_us() {
        let file = std::fs::File::open("/dev/null").unwrap();
        assert!(ipc::peer_uid(&file).is_err());
        assert!(!ipc::peer_is_us(&file));
    }

    /// A socket with no other end: the system answers for it (Linux: no
    /// process, no user), and that answer names neither this process nor
    /// this account; the client's check refuses it. Linux only: the
    /// answer and `SOCK_CLOEXEC` are Linux's (the portable job found it).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_socket_with_no_other_end_names_no_process_and_isnt_us() {
        use std::os::fd::FromRawFd;
        // SAFETY: socket(2) has no preconditions; the descriptor is checked
        // and then owned by the stream alone.
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        assert!(fd >= 0);
        // SAFETY: a fresh descriptor of a Unix stream socket, owned by none.
        let lone = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
        assert_eq!(ipc::peer_pid(&lone), None);
        assert!(!ipc::peer_is_us(&lone));
        assert_eq!(
            check_peer_std(&lone).unwrap_err(),
            "the books server's socket belongs to another account: not used"
        );
    }

    /// A session ends only after a day without a message (SESSION_IDLE),
    /// never rmcp's few minutes: a call longer than that keeps its answer.
    #[test]
    fn a_session_ends_only_after_a_day_without_a_message() {
        assert_eq!(
            sessions().session_config.keep_alive,
            Some(Duration::from_secs(86_400))
        );
    }

    /// Settings moved in at adoption are dated in the time zone the file
    /// sets (company.timezone), not by any other entry's text. The two
    /// zones are 26 hours apart, so their dates always differ.
    #[test]
    fn settings_moved_in_are_dated_in_the_files_own_time_zone() {
        let change = |key: &str, value: &str| ledger_core::settings::Change {
            key: key.into(),
            value: Some(serde_json::json!(value)),
        };
        let changes = [
            change("company.name", "Etc/GMT+12"),
            change("company.timezone", "Pacific/Kiritimati"),
        ];
        let source = moved_in_at_adoption(&changes);
        assert_eq!(
            source.decided_on,
            ledger_core::settings::today_in(Some("Pacific/Kiritimati"))
        );
        assert_ne!(
            source.decided_on,
            ledger_core::settings::today_in(Some("Etc/GMT+12"))
        );
        assert_eq!(source.decided_by, "the settings file");
    }

    /// A server's books in a fresh root under `home`, as `run` holds them,
    /// without its sockets: each test drives the handlers itself.
    fn books_in(home: &Path) -> Arc<Books> {
        let (root, _) = Root::setup(home, Some(&home.join("root")), None).unwrap();
        let books = Arc::new(Books::new(
            root,
            BACKUP_QUIET,
            DRILL_FIRST,
            Isolation::SameUser,
        ));
        let loaded = books.load_principals();
        *books.principals.write().unwrap() = loaded;
        books
    }

    /// Make the books `id` through the admin request.
    fn create(books: &Books, id: &str) {
        match books.admin(Request::Create {
            id: id.into(),
            currency: "USD".into(),
        }) {
            Response::Created { problem: None, .. } => {}
            other => panic!("{other:?}"),
        }
    }

    /// A token for `book` named `name` with every scope (made, or made
    /// again): its id and its text.
    fn token(books: &Books, book: &str, name: &str) -> (String, String) {
        match books.admin(Request::TokenCreate {
            book: book.into(),
            name: name.into(),
            scopes: ["read", "write", "post", "admin"]
                .map(String::from)
                .to_vec(),
            replace: true,
        }) {
            Response::TokenCreated { token, text } => (token.id, text.0),
            other => panic!("{other:?}"),
        }
    }

    /// A session of `book` bound to the token `token`.
    fn bound_to(token: &str, book: &str) -> Bound {
        Bound {
            token: token.into(),
            name: "test".into(),
            book: book.into(),
            at: 0,
        }
    }

    /// The sessions bound, by id.
    fn bound(books: &Books) -> Vec<String> {
        books
            .session_tokens
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    /// The status, the refusal code and the text of an answer.
    async fn answer(r: axum::response::Response) -> (u16, Option<String>, String) {
        let code = r
            .headers()
            .get(REFUSED_HEADER)
            .map(|v| v.to_str().unwrap().to_string());
        let status = r.status().as_u16();
        let body = axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, code, String::from_utf8_lossy(&body).into_owned())
    }

    /// A PUT of `body` for upload `id` of the books `acme`.
    fn put(id: &str, bearer: Option<&str>, body: axum::body::Body) -> axum::extract::Request {
        let mut r = axum::http::Request::builder()
            .method("PUT")
            .uri(format!("http://localhost/b/acme/u/{id}"));
        if let Some(b) = bearer {
            r = r.header("authorization", format!("Bearer {b}"));
        }
        r.body(body).unwrap()
    }

    /// Send a PUT to the upload route as the router would.
    async fn send_upload(
        books: &Arc<Books>,
        id: &str,
        bearer: Option<&str>,
        body: axum::body::Body,
    ) -> axum::response::Response {
        upload(
            axum::extract::State(books.clone()),
            axum::extract::Path(("acme".to_string(), id.to_string())),
            put(id, bearer, body),
        )
        .await
    }

    /// A request body that arrives as the test sends it, one chunk at a
    /// time (the sender's `reserve` returns once the last chunk was taken).
    fn fed() -> (tokio::sync::mpsc::Sender<Vec<u8>>, axum::body::Body) {
        let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
        let stream = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv()
                .await
                .map(|b| (Ok::<_, std::io::Error>(axum::body::Bytes::from(b)), rx))
        });
        (tx, axum::body::Body::from_stream(stream))
    }

    /// The upload's grant, for `token_id`, of `bytes`.
    fn grant(books: &Books, token_id: &str, bytes: &[u8]) -> ledger_store::uploads::Grant {
        let held = books.held();
        let Some(Book::Open { uploads, .. }) = held.get("acme") else {
            panic!("acme isn't open");
        };
        uploads
            .grant(
                token_id,
                &ledger_store::journal::sha256_hex(bytes),
                bytes.len() as u64,
                "application/pdf",
                "upload.pdf",
                "document",
                chrono::Utc::now().timestamp(),
            )
            .unwrap()
    }

    /// The last access record of `acme`: its action and its outcome.
    fn last_access(books: &Books) -> (String, Outcome) {
        let dir = books.root.book_dir(&BookId::parse("acme").unwrap());
        let (records, _) = ledger_store::access::read(&dir).unwrap();
        let last = records.last().unwrap();
        (last.access.action.clone(), last.access.result.clone())
    }

    #[test]
    fn a_book_that_didnt_open_says_why_and_one_open_says_nothing() {
        let why = "its access log can't be used".to_string();
        assert_eq!(Book::Failed(why.clone()).problem(), Some(why));
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        assert_eq!(books.held()["acme"].problem(), None);
    }

    /// INVARIANT 6: once closing, every request is refused as closing,
    /// even one for books that didn't open or don't exist (until then
    /// told so), so a client knows the server is going, not that its
    /// books are gone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn once_closing_every_request_is_refused_as_closing() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        books.hold(
            &BookId::parse("broken").unwrap(),
            Book::Failed("its access log can't be used".into()),
        );
        let asked = Asked::connect(&axum::http::Method::POST);
        let headers = axum::http::HeaderMap::new();
        let refused = |books: Arc<Books>, book: &'static str| {
            let (asked, headers) = (&asked, &headers);
            async move {
                match door(&books, book, asked, headers).await {
                    Ok(_) => panic!("{book}: let through"),
                    Err(r) => answer(*r).await,
                }
            }
        };
        let (status, code, text) = refused(books.clone(), "broken").await;
        assert_eq!((status, code.as_deref()), (503, Some("not-open")), "{text}");
        let (status, code, text) = refused(books.clone(), "nobody").await;
        assert_eq!(
            (status, code.as_deref()),
            (404, Some("no-such-books")),
            "{text}"
        );
        books.closing.cancel();
        for book in ["broken", "nobody"] {
            let (status, code, text) = refused(books.clone(), book).await;
            assert_eq!((status, code), (503, None), "{book}: {text}");
            assert_eq!(text, "the books are closing: this request wasn't run");
        }
        let (status, code, text) = refused(books.clone(), "acme").await;
        assert_eq!(
            (status, code.as_deref()),
            (503, Some(crate::access::CLOSING))
        );
        assert_eq!(text, "the books are closing: this request wasn't run");
    }

    /// INVARIANT 9: nothing comes in once the books stopped (SharedStore
    /// INVARIANT 2): an upload is refused as stopped, and recorded so.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_upload_to_books_that_stopped_is_refused_as_stopped() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        let (id, text) = token(&books, "acme", "agent");
        let bytes = b"%PDF-1.4 stopped".to_vec();
        let g = grant(&books, &id, &bytes);
        match books.held().get("acme") {
            Some(Book::Open { store, .. }) => store.stop(),
            _ => panic!("acme isn't open"),
        }
        let r = send_upload(&books, &g.id, Some(&text), bytes.into()).await;
        let (status, code, said) = answer(r).await;
        assert_eq!((status, code.as_deref()), (503, Some("stopped")), "{said}");
        assert!(said.contains("stopped taking requests"), "{said}");
        assert_eq!(
            last_access(&books),
            ("upload".to_string(), Outcome::Refused("stopped".into()))
        );
    }

    /// INVARIANT 9: an upload is received from one request at a time;
    /// a second sent while the first is still arriving is refused as in
    /// use, and the first is kept.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_upload_still_arriving_is_refused_to_a_second_sender() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        let (id, text) = token(&books, "acme", "agent");
        let bytes: Vec<u8> = b"%PDF-1.4 "
            .iter()
            .copied()
            .chain((0..2_000_000u32).map(|i| (i % 251) as u8))
            .collect();
        let g = grant(&books, &id, &bytes);
        let (tx, body) = fed();
        let first = tokio::spawn({
            let (books, id, text) = (books.clone(), g.id.clone(), text.clone());
            async move { send_upload(&books, &id, Some(&text), body).await }
        });
        tx.send(bytes[..1_000_000].to_vec()).await.unwrap();
        // Arriving: its part is on the disk.
        let part = books
            .root
            .book_dir(&BookId::parse("acme").unwrap())
            .join(ledger_store::uploads::UPLOADS_DIR)
            .join(format!("{}.part", g.id));
        let deadline = Instant::now() + Duration::from_secs(30);
        while !part.exists() {
            assert!(Instant::now() < deadline, "the first upload never began");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let second = send_upload(&books, &g.id, Some(&text), bytes.clone().into()).await;
        let (status, code, said) = answer(second).await;
        assert_eq!((status, code.as_deref()), (409, Some("in_use")), "{said}");
        assert_eq!(
            last_access(&books),
            ("upload".to_string(), Outcome::Refused("in_use".into()))
        );
        tx.send(bytes[1_000_000..].to_vec()).await.unwrap();
        drop(tx);
        let (status, _, said) = answer(first.await.unwrap()).await;
        assert_eq!(status, 200, "{said}");
    }

    /// INVARIANT 9: a refused upload's bytes are read and dropped up to
    /// the most an upload can be, so a client sending no more than that
    /// is never cut off mid-send, and no further, so one that never stops
    /// sending is still answered: refused at the door (no token) and past
    /// it (no such upload) alike.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refused_upload_is_read_up_to_the_most_an_upload_can_be() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        let (_, text) = token(&books, "acme", "agent");
        let max = ledger_store::uploads::MAX_UPLOAD as usize;
        let mib = 1024 * 1024;
        assert_eq!(max % mib, 0);
        for (bearer, status, code) in [
            (None, 401, crate::access::UNAUTHORIZED),
            (Some(text.clone()), 404, "no_such_upload"),
        ] {
            let (tx, body) = fed();
            let answered = tokio::spawn({
                let books = books.clone();
                async move {
                    let id = "up_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
                    send_upload(&books, id, bearer.as_deref(), body).await
                }
            });
            for _ in 0..max / mib {
                tx.send(vec![b'x'; mib]).await.unwrap();
            }
            // Every byte up to the most taken, and a moment to act on it:
            // still reading.
            let more = tx.reserve().await.unwrap();
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert!(
                !answered.is_finished(),
                "{status}: answered before the most an upload can be was read"
            );
            // One byte past it: answered, though the body never ends.
            more.send(vec![b'x']);
            let r = tokio::time::timeout(Duration::from_secs(60), answered)
                .await
                .unwrap_or_else(|_| panic!("{status}: never answered"))
                .unwrap();
            let (got, refused, said) = answer(r).await;
            assert_eq!((got, refused.as_deref()), (status, Some(code)), "{said}");
            drop(tx);
        }
    }

    /// INVARIANT 8: a token revoked or replaced lets go of the sessions it
    /// opened, and only those: not another book's token of its name, nor
    /// its own book's token of another name.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_replaced_or_revoked_token_lets_go_of_its_own_sessions_alone() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        create(&books, "beta");
        // Oldest first: another book's token of the same name, then one of
        // the same book with another name, then the one replaced.
        let (beta, _) = token(&books, "beta", "agent");
        let (other, _) = token(&books, "acme", "other");
        let (agent, _) = token(&books, "acme", "agent");
        {
            let mut bound = books.session_tokens.lock().unwrap();
            for (sid, owner) in [("s-beta", &beta), ("s-other", &other), ("s-agent", &agent)] {
                bound.insert(sid.into(), bound_to(owner, "acme"));
            }
        }
        token(&books, "acme", "agent");
        assert_eq!(bound(&books), ["s-beta", "s-other"]);
        match books.admin(Request::TokenRevoke {
            book: "beta".into(),
            name: "agent".into(),
            id: None,
        }) {
            Response::TokenRevoked(t) => assert_eq!(t.id, beta),
            other => panic!("{other:?}"),
        }
        assert_eq!(bound(&books), ["s-other"]);
    }

    /// A token check answers by the SHA-256 asked, for the books and name
    /// asked alone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_token_check_says_whether_the_text_is_the_live_one() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        let (old_id, old) = token(&books, "acme", "agent");
        let (new_id, new) = token(&books, "acme", "agent");
        assert_ne!(old_id, new_id);
        let check = |book: &str, name: &str, text: &str| match books.admin(Request::TokenCheck {
            book: book.into(),
            name: name.into(),
            sha256: ledger_store::journal::sha256_hex(text.as_bytes()),
        }) {
            Response::TokenChecked { live, matches } => (live.map(|t| t.id), matches),
            other => panic!("{other:?}"),
        };
        assert_eq!(check("acme", "agent", &new), (Some(new_id.clone()), true));
        assert_eq!(check("acme", "agent", &old), (Some(new_id), false));
        assert_eq!(check("acme", "other", &new), (None, false));
        assert_eq!(check("beta", "agent", &new), (None, false));
    }

    /// An MCP request to `acme` with `bearer`, on `session` if given.
    fn mcp_request(
        method: axum::http::Method,
        bearer: &str,
        session: Option<&str>,
        body: serde_json::Value,
    ) -> axum::extract::Request {
        let mut r = axum::http::Request::builder()
            .method(method)
            .uri("http://localhost/b/acme/mcp")
            .header("host", "localhost")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("authorization", format!("Bearer {bearer}"));
        if let Some(sid) = session {
            r = r
                .header(SESSION_HEADER, sid)
                .header("mcp-protocol-version", "2025-06-18");
        }
        r.body(axum::body::Body::from(body.to_string())).unwrap()
    }

    async fn send_mcp(
        books: &Arc<Books>,
        request: axum::extract::Request,
    ) -> axum::response::Response {
        mcp(
            axum::extract::State(books.clone()),
            axum::extract::Path("acme".to_string()),
            request,
        )
        .await
    }

    fn initialize() -> serde_json::Value {
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}}})
    }

    /// INVARIANT 8: a session stays bound to the token that opened it
    /// through every request it serves, and is forgotten when it ends.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_session_stays_bound_to_its_token_until_it_ends() {
        use axum::http::Method;
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        let (agent_id, agent) = token(&books, "acme", "agent");
        let (_, other) = token(&books, "acme", "other");
        let r = send_mcp(
            &books,
            mcp_request(Method::POST, &agent, None, initialize()),
        )
        .await;
        let sid = r.headers()[SESSION_HEADER].to_str().unwrap().to_string();
        let (status, _, said) = answer(r).await;
        assert_eq!(status, 200, "{said}");
        let initialized =
            serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let list = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
        for body in [initialized, list.clone()] {
            let r = send_mcp(&books, mcp_request(Method::POST, &agent, Some(&sid), body)).await;
            let (status, _, said) = answer(r).await;
            assert!((200..300).contains(&status), "{status}: {said}");
            assert_eq!(
                books
                    .session_tokens
                    .lock()
                    .unwrap()
                    .get(&sid)
                    .map(|b| &b.token),
                Some(&agent_id)
            );
        }
        let r = send_mcp(&books, mcp_request(Method::POST, &other, Some(&sid), list)).await;
        let (status, code, _) = answer(r).await;
        assert_eq!((status, code.as_deref()), (403, Some("session_token")));
        // Ended: forgotten.
        let r = send_mcp(
            &books,
            mcp_request(Method::DELETE, &agent, Some(&sid), serde_json::json!({})),
        )
        .await;
        let (status, _, said) = answer(r).await;
        assert!((200..300).contains(&status), "{status}: {said}");
        assert!(bound(&books).is_empty());
    }

    /// INVARIANT 8: past MAX_SESSIONS bindings only a request that opens
    /// a session (an initialize) is refused; any other still reaches the
    /// MCP service.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn past_the_most_sessions_only_opening_another_is_refused() {
        use axum::http::Method;
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        let (agent_id, agent) = token(&books, "acme", "agent");
        {
            let mut bound = books.session_tokens.lock().unwrap();
            for n in 0..MAX_SESSIONS {
                bound.insert(format!("s-{n}"), bound_to(&agent_id, "acme"));
            }
        }
        let r = send_mcp(
            &books,
            mcp_request(Method::POST, &agent, None, initialize()),
        )
        .await;
        let (status, code, said) = answer(r).await;
        assert_eq!(
            (status, code.as_deref()),
            (503, Some("too_many_sessions")),
            "{said}"
        );
        let list = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
        let r = send_mcp(&books, mcp_request(Method::POST, &agent, None, list)).await;
        let (status, code, said) = answer(r).await;
        assert_ne!(
            code.as_deref(),
            Some("too_many_sessions"),
            "{status}: {said}"
        );
        assert_eq!(bound(&books).len(), MAX_SESSIONS);
    }

    /// The owner passphrase the upgrade tests set.
    const PASS: &str = "correct horse battery";

    /// Books `id` as an older version wrote them (journal format 1), with
    /// a chart, an entry and a vault key, held by the server.
    fn old_books(books: &Books, id: &str) -> PathBuf {
        let id = BookId::parse(id).unwrap();
        let dir = books.root.book_dir(&id);
        std::fs::create_dir(&dir).unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        let mut s =
            Store::create_in_format(&dir, ledger_core::Currency::USD, &Default::default()).unwrap();
        let cash = s
            .add_account("1000", "Cash", ledger_core::AccountType::Asset, None, false)
            .unwrap();
        let cap = s
            .add_account(
                "3000",
                "Capital",
                ledger_core::AccountType::Equity,
                None,
                false,
            )
            .unwrap();
        let e = s
            .create_entry(chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(), "In")
            .unwrap();
        let m =
            ledger_core::Money::new(rust_decimal::Decimal::from(500), ledger_core::Currency::USD);
        s.add_line(e, cash, ledger_core::Side::Debit, m).unwrap();
        s.add_line(e, cap, ledger_core::Side::Credit, m).unwrap();
        s.post_entry(e).unwrap();
        drop(s);
        let keys = dir.join(ledger_store::root::KEYS_DIR);
        std::fs::create_dir(&keys).unwrap();
        std::fs::write(
            keys.join(ledger_store::root::LEGACY_KEY_FILE),
            "ab".repeat(32),
        )
        .unwrap();
        books.hold(&id, books.open(&id));
        dir
    }

    /// The books `id` as listed.
    fn listed(books: &Books, id: &str) -> admin::BookStatus {
        match books.admin(Request::List) {
            Response::Books(list) => list.into_iter().find(|b| b.id == id).unwrap(),
            other => panic!("{other:?}"),
        }
    }

    /// Ask for the upgrade of `id` with `pass`, off the runtime (as the
    /// admin socket does).
    async fn upgrade(books: &Arc<Books>, id: &str, pass: &str) -> Response {
        let (books, id, pass) = (books.clone(), id.to_string(), pass.to_string());
        tokio::task::spawn_blocking(move || {
            books.admin(Request::UpgradeFormat {
                id,
                passphrase: admin::Passphrase(pass),
            })
        })
        .await
        .unwrap()
    }

    /// git, on the books' own archive: what it said.
    fn archive_git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg(format!(
                "--git-dir={}",
                // A canonical path on Windows carries `\\?\`, which git refuses.
                ledger_store::root::plain_path(&dir.join("archive.git")).display()
            ))
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap()
    }

    /// plan/served.md 3.9's `format_upgrade_waits_for_consent`, the
    /// server's part: books an older version wrote are served on their old
    /// format and listed as waiting; the go-ahead is refused without the
    /// owner passphrase (checked before anything is done: no backup, no
    /// line), and the books stay served as they were; with it, a backup of
    /// the books as they were, their keys in it, is committed and proven,
    /// then the whole format is declared in one line; asked again, nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn format_upgrade_waits_for_consent() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        let dir = old_books(&books, "acme");
        let journal = || std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap();
        let before = journal();
        let waiting = listed(&books, "acme").format_upgrade.expect("it waits");
        // The database file it uses: this version's schema's own.
        assert_eq!(
            listed(&books, "acme").database.as_deref(),
            Some(ledger_store::db_file().as_str())
        );
        // Format 1 waits for this version's whole format: format 2 and
        // every feature it knows.
        let adds = ledger_store::JournalFormat::default()
            .upgrade()
            .unwrap()
            .adds();
        assert_eq!(adds[0], "journal format 2");
        assert_eq!(waiting.format, 1);
        assert_eq!(waiting.adds, adds);
        // No passphrase set yet, then a wrong one, then none: refused,
        // nothing done.
        match upgrade(&books, "acme", PASS).await {
            Response::Refused(why) => {
                assert!(why.contains("no owner passphrase is set yet"), "{why}");
                assert!(why.contains("nothing was changed"), "{why}");
            }
            other => panic!("{other:?}"),
        }
        let auth = books.root.auth_dir().unwrap();
        ledger_store::passphrase::set(&auth, PASS, None).unwrap();
        for wrong in ["not the passphrase at all", ""] {
            match upgrade(&books, "acme", wrong).await {
                Response::Refused(why) => {
                    assert_eq!(why, "that isn't the owner passphrase: nothing was changed")
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(journal(), before);
        assert!(
            !dir.join("archive.git").exists(),
            "no backup before consent"
        );
        assert!(listed(&books, "acme").format_upgrade.is_some());
        assert_eq!(books.held()["acme"].problem(), None, "still served");
        // Books that don't exist, or didn't open.
        assert!(matches!(
            upgrade(&books, "nobody", PASS).await,
            Response::Refused(_)
        ));
        // The go-ahead.
        let done = match upgrade(&books, "acme", PASS).await {
            Response::FormatUpgraded(u) => u,
            other => panic!("{other:?}"),
        };
        assert_eq!(done.id, "acme");
        assert_eq!(done.added, adds);
        let backup = done.backup.clone().expect("backed up first");
        let after = journal();
        assert!(after.starts_with(&before));
        let added: Vec<serde_json::Value> = String::from_utf8(after[before.len()..].to_vec())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(added.len(), 1, "one line");
        assert_eq!(added[0]["type"], "journal.format_set");
        let c = ledger_store::format::current();
        // The declaration also gives older books their id (plan P15 6a).
        let mut declared = added[0]["payload"].clone();
        let books_id = declared.as_object_mut().unwrap().remove("books_id");
        assert_eq!(books_id.and_then(|v| v.as_str().map(str::len)), Some(32));
        assert_eq!(
            declared,
            serde_json::json!({"format": c.format, "features": c.features})
        );
        assert_eq!(done.head.0, backup.head.0 + 1);
        // The backup: the books as they were, keys and all, the latest
        // commit of their archive.
        assert_eq!(
            archive_git(&dir, &["rev-parse", "HEAD"]).trim(),
            backup.commit
        );
        assert_eq!(
            archive_git(&dir, &["show", &format!("{}:journal.jsonl", backup.commit)]).as_bytes(),
            before.as_slice()
        );
        assert_eq!(
            archive_git(
                &dir,
                &["show", &format!("{}:keys/legacy.key", backup.commit)]
            ),
            "ab".repeat(32)
        );
        // Both on the backup record.
        let record =
            std::fs::read_to_string(ledger_store::ops::ops_dir(&dir).join(crate::backup::LOG_FILE))
                .unwrap();
        assert!(record.contains(r#""kind":"format_upgrade""#), "{record}");
        assert!(record.contains(&backup.commit), "{record}");
        assert!(
            !ledger_store::ops::ops_dir(&dir)
                .join("upgrade-check")
                .exists()
        );
        // Waiting for nothing now; asked again, nothing is done.
        assert_eq!(listed(&books, "acme").format_upgrade, None);
        match upgrade(&books, "acme", PASS).await {
            Response::FormatUpgraded(u) => {
                assert!(u.added.is_empty() && u.backup.is_none(), "{u:?}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(journal(), after);
        // Books this version made wait for nothing, and asking for their
        // upgrade changes nothing of them: not even a backup repository.
        create(&books, "beta");
        assert_eq!(listed(&books, "beta").format_upgrade, None);
        let beta = books.root.book_dir(&BookId::parse("beta").unwrap());
        match upgrade(&books, "beta", PASS).await {
            Response::FormatUpgraded(u) => {
                assert!(u.added.is_empty() && u.backup.is_none(), "{u:?}");
            }
            other => panic!("{other:?}"),
        }
        assert!(!beta.join("archive.git").exists(), "nothing was changed");
    }

    /// The backup is trusted only once proven, and only the one just made:
    /// an archive whose head names a branch other than the one a backup's
    /// clone takes (a damaged archive) takes the new commit where no
    /// clone finds it, so what is proven isn't what was made: nothing is
    /// upgraded, and it is recorded. Once the archive is right, its latest
    /// commit (which already holds the books) is proven and the upgrade
    /// made.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_upgrade_unless_the_backup_just_made_is_proven() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        let dir = old_books(&books, "acme");
        ledger_store::passphrase::set(&books.root.auth_dir().unwrap(), PASS, None).unwrap();
        let store = match &books.held()["acme"] {
            Book::Open { store, .. } => store.clone(),
            Book::Failed(why) => panic!("{why}"),
            Book::Closed => panic!("closed"),
        };
        let (d, s) = (dir.clone(), store.clone());
        let first = tokio::task::spawn_blocking(move || {
            let git = ledger_store::backup::Git::find().unwrap();
            let head = s.with_store(|s| s.head()).unwrap();
            ledger_store::backup::commit(&git, &d, "acme", head).unwrap()
        })
        .await
        .unwrap()
        .expect("a first backup");
        archive_git(&dir, &["symbolic-ref", "HEAD", "refs/heads/elsewhere"]);
        // A second later: the commit made now, of the same files, isn't
        // byte for byte the first (git dates commits to the second).
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let before = std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap();
        match upgrade(&books, "acme", PASS).await {
            Response::Refused(why) => {
                assert!(why.contains("didn't prove whole"), "{why}");
                assert!(why.contains("nothing was upgraded"), "{why}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap(),
            before
        );
        assert!(listed(&books, "acme").format_upgrade.is_some());
        let record = || {
            std::fs::read_to_string(ledger_store::ops::ops_dir(&dir).join(crate::backup::LOG_FILE))
                .unwrap()
        };
        assert!(
            record().contains(r#""why":"format_upgrade""#),
            "the commit made: {}",
            record()
        );
        assert!(record().contains(r#""ok":false"#), "{}", record());
        // The archive put right: its latest commit holds the books.
        archive_git(&dir, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        match upgrade(&books, "acme", PASS).await {
            Response::FormatUpgraded(u) => {
                let b = u.backup.clone().expect("proven");
                assert_eq!(b.commit, first.commit, "{u:?}");
                assert_eq!(b.head, first.head);
                assert_eq!(u.head.0, first.head.0 + 1);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(listed(&books, "acme").format_upgrade, None);
    }

    /// A backup that can't be made upgrades nothing: the books stay on
    /// their old format, served and waiting, and the failure is recorded.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_upgrade_without_a_backup() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        let dir = old_books(&books, "acme");
        ledger_store::passphrase::set(&books.root.auth_dir().unwrap(), PASS, None).unwrap();
        // A file no book keeps: the backup refuses it.
        std::fs::write(dir.join(".gitattributes"), "* -text\n").unwrap();
        let before = std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap();
        match upgrade(&books, "acme", PASS).await {
            Response::Refused(why) => {
                assert!(why.contains("couldn't be made"), "{why}");
                assert!(why.contains("nothing was upgraded"), "{why}");
                assert!(!why.contains(&home.path().display().to_string()), "{why}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap(),
            before
        );
        assert!(listed(&books, "acme").format_upgrade.is_some());
        let record =
            std::fs::read_to_string(ledger_store::ops::ops_dir(&dir).join(crate::backup::LOG_FILE))
                .unwrap();
        assert!(
            record.contains(r#""kind":"format_upgrade","ok":false"#)
                || (record.contains(r#""kind":"format_upgrade""#)
                    && record.contains(r#""ok":false"#)),
            "{record}"
        );
    }

    /// An admin request, on a thread of its own (an admin request blocks).
    async fn ask(books: &Arc<Books>, request: Request) -> Response {
        let books = books.clone();
        tokio::task::spawn_blocking(move || books.admin(request))
            .await
            .unwrap()
    }

    fn closed(id: &str) -> Response {
        Response::CloseBook {
            id: id.into(),
            outcome: admin::CloseOutcome::Closed,
        }
    }

    fn opened(id: &str, outcome: admin::OpenOutcome) -> Response {
        Response::OpenBook {
            id: id.into(),
            outcome,
        }
    }

    /// Headers carrying `bearer`.
    fn bearing(bearer: &str) -> axum::http::HeaderMap {
        let mut h = axum::http::HeaderMap::new();
        h.insert("authorization", format!("Bearer {bearer}").parse().unwrap());
        h
    }

    /// What the door says of `book`, for a request bearing `bearer`: let
    /// through (None), or its status, refusal code and words.
    async fn at_the_door(
        books: &Arc<Books>,
        book: &str,
        bearer: &str,
    ) -> Option<(u16, Option<String>, String)> {
        let asked = Asked::connect(&axum::http::Method::POST);
        match door(books, book, &asked, &bearing(bearer)).await {
            Ok(_) => None,
            Err(r) => Some(answer(*r).await),
        }
    }

    /// INVARIANT 10, 11: a closed book's calls are refused as "closed" (not
    /// as a book that doesn't exist), with words an agent can relay; the
    /// other book never notices; the book is listed closed, with no
    /// problem; it opens again, and serves again.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_closed_book_is_refused_as_closed_and_the_other_still_serves() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        create(&books, "beta");
        let (_, acme) = token(&books, "acme", "agent");
        let (_, beta) = token(&books, "beta", "agent");
        assert_eq!(at_the_door(&books, "acme", &acme).await, None);

        assert_eq!(
            ask(&books, Request::CloseBook { id: "acme".into() }).await,
            closed("acme")
        );
        let (status, code, said) = at_the_door(&books, "acme", &acme).await.unwrap();
        assert_eq!((status, code.as_deref()), (503, Some("closed")));
        assert_eq!(
            said,
            "The books \"acme\" aren't open: open them in Wanderbooks."
        );
        // Told apart from books that don't exist.
        let (status, code, _) = at_the_door(&books, "nobody", &acme).await.unwrap();
        assert_eq!((status, code.as_deref()), (404, Some("no-such-books")));
        // The other book is untouched.
        assert_eq!(at_the_door(&books, "beta", &beta).await, None);
        let listed = books.held()["acme"].problem();
        assert_eq!(listed, None, "closed is not a problem");
        match ask(&books, Request::List).await {
            Response::Books(list) => {
                let a = list.iter().find(|b| b.id == "acme").unwrap();
                assert!(!a.open && a.closed && a.problem.is_none(), "{a:?}");
                let b = list.iter().find(|b| b.id == "beta").unwrap();
                assert!(b.open && !b.closed, "{b:?}");
            }
            other => panic!("{other:?}"),
        }

        // Closed twice; and a name that is no book.
        assert_eq!(
            ask(&books, Request::CloseBook { id: "acme".into() }).await,
            Response::CloseBook {
                id: "acme".into(),
                outcome: admin::CloseOutcome::WasNotOpen
            }
        );
        assert_eq!(
            ask(
                &books,
                Request::CloseBook {
                    id: "nobody".into()
                }
            )
            .await,
            Response::CloseBook {
                id: "nobody".into(),
                outcome: admin::CloseOutcome::NoSuchBook
            }
        );

        // Open: once; again; a name that is no book.
        assert_eq!(
            ask(&books, Request::OpenBook { id: "acme".into() }).await,
            opened("acme", admin::OpenOutcome::Opened)
        );
        assert_eq!(
            ask(&books, Request::OpenBook { id: "acme".into() }).await,
            opened("acme", admin::OpenOutcome::AlreadyOpen)
        );
        assert_eq!(
            ask(
                &books,
                Request::OpenBook {
                    id: "nobody".into()
                }
            )
            .await,
            opened("nobody", admin::OpenOutcome::NoSuchBook)
        );
        assert_eq!(at_the_door(&books, "acme", &acme).await, None);
    }

    /// INVARIANT 10: a close lets go of the store, releasing its writer
    /// lock, so another process could open the book; and while another
    /// holds it, opening says so, as at the start.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_closed_book_can_be_opened_by_another_program_and_not_by_two() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        let dir = books.root.book_dir(&BookId::parse("acme").unwrap());
        // Held: the same program can't open the store a second time.
        assert!(Store::open(&dir).is_err(), "the server holds the books");
        assert_eq!(
            ask(&books, Request::CloseBook { id: "acme".into() }).await,
            closed("acme")
        );
        // Let go: another opens, writes (an account), and holds it.
        let other = Store::open(&dir).expect("the lock was released");
        match ask(&books, Request::OpenBook { id: "acme".into() }).await {
            Response::OpenBook {
                outcome: admin::OpenOutcome::CouldNotOpen(why),
                ..
            } => assert!(
                why.contains("another program has these books open"),
                "{why}"
            ),
            other => panic!("{other:?}"),
        }
        match ask(&books, Request::List).await {
            Response::Books(list) => {
                let a = list.iter().find(|b| b.id == "acme").unwrap();
                assert!(!a.open && !a.closed && a.problem.is_some(), "{a:?}");
            }
            other => panic!("{other:?}"),
        }
        // Closing a book that didn't open: it was not open.
        assert_eq!(
            ask(&books, Request::CloseBook { id: "acme".into() }).await,
            Response::CloseBook {
                id: "acme".into(),
                outcome: admin::CloseOutcome::WasNotOpen
            }
        );
        drop(other);
        assert_eq!(
            ask(&books, Request::OpenBook { id: "acme".into() }).await,
            opened("acme", admin::OpenOutcome::Opened)
        );
    }

    /// INVARIANT 10: a close answers only once the requests already inside
    /// the book's door have left; a request arriving meanwhile is
    /// refused at once (as closed), and another book is not held up.
    /// INVARIANT 10: a close stops a format upgrade that has not yet
    /// journaled its `journal.format_set` line, instead of waiting for it
    /// (the upgrade is counted by the store's close): the upgrade says it
    /// was not done, the books are byte for byte as they were, and the
    /// close is answered. The upgrade is held in its backup's proof by the
    /// test knob that makes each phase wait a minute.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_close_stops_a_format_upgrade_before_it_is_journaled() {
        let home = tempfile::tempdir().unwrap();
        let (root, _) = Root::setup(home.path(), Some(&home.path().join("root")), None).unwrap();
        let books = Arc::new(
            Books::new(root, BACKUP_QUIET, DRILL_FIRST, Isolation::SameUser).with_timers(Timers {
                slow_steps: Duration::from_secs(60),
                answer_grace: Duration::from_millis(300),
                ..Timers::default()
            }),
        );
        let loaded = books.load_principals();
        *books.principals.write().unwrap() = loaded;
        let dir = old_books(&books, "acme");
        let auth = books.root.auth_dir().unwrap();
        ledger_store::passphrase::set(&auth, PASS, None).unwrap();
        let journal = || std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap();
        let before = journal();
        let backups = match books.held().get("acme") {
            Some(Book::Open { backups, .. }) => backups.clone(),
            _ => panic!("acme isn't open"),
        };
        let upgrading = tokio::spawn({
            let books = books.clone();
            async move { upgrade(&books, "acme", PASS).await }
        });
        // It is inside the proof of its backup, waiting.
        let by = Instant::now() + Duration::from_secs(60);
        while backups.pauses_begun() == 0 {
            assert!(Instant::now() < by, "the upgrade never reached its proof");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let started = Instant::now();
        let closed = ask(&books, Request::CloseBook { id: "acme".into() }).await;
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "held by the upgrade"
        );
        assert!(
            matches!(
                closed,
                Response::CloseBook {
                    outcome: admin::CloseOutcome::Closed
                        | admin::CloseOutcome::ClosedBackupIncomplete(_),
                    ..
                }
            ),
            "{closed:?}"
        );
        match upgrading.await.unwrap() {
            Response::Refused(why) => assert!(why.contains("nothing was upgraded"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            journal(),
            before,
            "an upgrade stopped by a close changed the books"
        );
    }

    /// INVARIANT 15: a body read to its end is whole and never cut,
    /// whatever follows (the cut firing, the body polled again); one not
    /// read to its end when the cut fires is cut at once, however much of
    /// it has come.
    #[tokio::test]
    async fn a_body_read_to_its_end_is_never_cut_and_one_not_yet_is() {
        use futures_util::StreamExt;
        let gate = Arc::new(Gate::default());
        // Whole: every byte read to the end, then the cut.
        let inside = gate.enter().unwrap();
        let (tx, body) = fed();
        let (request, whole) = inside.arriving(axum::extract::Request::new(body));
        let mut data = request.into_body().into_data_stream();
        tx.send(vec![1, 2, 3]).await.unwrap();
        assert_eq!(data.next().await.unwrap().unwrap().len(), 3);
        drop(tx);
        assert!(data.next().await.is_none());
        gate.cut_unfinished();
        assert!(!whole.was_cut(), "a body read to its end was cut");
        // Asked again after the cut: still the end, not a cut.
        assert!(data.next().await.is_none());
        assert!(!whole.was_cut());
        drop(inside);
        // Not whole: part of it came, the cut fires, the next read fails
        // (a gate's cut is for good: this is another book's).
        let gate = Arc::new(Gate::default());
        let inside = gate.enter().unwrap();
        let (tx, body) = fed();
        let (request, partial) = inside.arriving(axum::extract::Request::new(body));
        let mut data = request.into_body().into_data_stream();
        tx.send(vec![1, 2, 3]).await.unwrap();
        assert!(data.next().await.unwrap().is_ok());
        assert!(!partial.was_cut());
        gate.cut_unfinished();
        // More bytes are on their way, however quickly: cut all the same.
        let _ = tx.try_send(vec![4, 5, 6]);
        assert!(data.next().await.unwrap().is_err());
        assert!(partial.was_cut());
        drop(inside);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_close_waits_for_the_requests_inside_and_refuses_the_next() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        create(&books, "beta");
        let (_, acme) = token(&books, "acme", "agent");
        let (_, beta) = token(&books, "beta", "agent");
        let gate = match books.held().get("acme") {
            Some(Book::Open { gate, .. }) => gate.clone(),
            _ => panic!("acme isn't open"),
        };
        let inside = gate.enter().expect("the door is open");
        let closing = tokio::spawn({
            let books = books.clone();
            async move { ask(&books, Request::CloseBook { id: "acme".into() }).await }
        });
        // The door shuts as the close begins.
        while gate.enter().is_some() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let (status, code, _) = at_the_door(&books, "acme", &acme).await.unwrap();
        assert_eq!((status, code.as_deref()), (503, Some("closed")));
        assert_eq!(at_the_door(&books, "beta", &beta).await, None);
        // A negative window: it can only fail if the close was answered,
        // never because the machine was slow.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!closing.is_finished(), "closed with a request still inside");
        drop(inside);
        assert_eq!(closing.await.unwrap(), closed("acme"));
    }

    /// INVARIANT 3, 10: books left closed at the start are listed closed
    /// and open when asked; the option takes all, none, or ids.
    #[test]
    fn the_books_open_at_the_start_are_chosen_by_option() {
        assert_eq!(OpenAtStart::parse("all"), Ok(OpenAtStart::All));
        assert_eq!(OpenAtStart::parse("none"), Ok(OpenAtStart::Only(vec![])));
        assert_eq!(
            OpenAtStart::parse("acme, beta"),
            Ok(OpenAtStart::Only(vec!["acme".into(), "beta".into()]))
        );
        assert!(OpenAtStart::parse("acme,../etc").is_err());
        assert!(OpenAtStart::parse("").is_err());
        let acme = BookId::parse("acme").unwrap();
        assert!(OpenAtStart::All.wants(&acme));
        assert!(!OpenAtStart::Only(vec![]).wants(&acme));
        assert!(OpenAtStart::Only(vec!["acme".into()]).wants(&acme));
    }

    /// INVARIANT 12: the live sessions of each open book, as the server
    /// knows them: the token's NAME, the client, the agent's report, the
    /// calls, the last call; never the token; one that ended isn't listed;
    /// and a book's sessions end with the book.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_sessions_request_says_who_is_connected_and_never_their_token() {
        use axum::http::Method;
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        create(&books, "beta");
        let (_, agent) = token(&books, "acme", "agent");
        let sessions_of = |r: Response, book: &str| match r {
            Response::Sessions(all) => {
                all.into_iter()
                    .find(|b| b.book == book)
                    .unwrap_or_else(|| panic!("{book} isn't listed"))
                    .sessions
            }
            other => panic!("{other:?}"),
        };
        assert!(sessions_of(ask(&books, Request::Sessions).await, "acme").is_empty());
        let r = send_mcp(
            &books,
            mcp_request(Method::POST, &agent, None, initialize()),
        )
        .await;
        let sid = r.headers()[SESSION_HEADER].to_str().unwrap().to_string();
        let (status, _, said) = answer(r).await;
        assert_eq!(status, 200, "{said}");
        let r = send_mcp(
            &books,
            mcp_request(
                Method::POST,
                &agent,
                Some(&sid),
                serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            ),
        )
        .await;
        assert!(answer(r).await.0 < 300);
        // Who the client is, from `initialize`, before it has made a call.
        // (The notification is handled by the session's task, a moment
        // after it was accepted.)
        let mut early = sessions_of(ask(&books, Request::Sessions).await, "acme");
        for _ in 0..100 {
            if early.iter().all(|s| s.client_name.is_some()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            early = sessions_of(ask(&books, Request::Sessions).await, "acme");
        }
        assert_eq!(early.len(), 1, "{early:?}");
        assert_eq!(early[0].client_name.as_deref(), Some("test"), "{early:?}");
        assert!(early[0].last.is_none() && early[0].agent.is_none());
        for body in [
            serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
                "name": "books.start",
                "arguments": {"agent": {"model": "m-1", "harness": "h-1"}}}}),
        ] {
            let r = send_mcp(&books, mcp_request(Method::POST, &agent, Some(&sid), body)).await;
            let (status, _, said) = answer(r).await;
            assert!((200..300).contains(&status), "{status}: {said}");
        }
        let listed = ask(&books, Request::Sessions).await;
        let text = serde_json::to_string(&listed).unwrap();
        assert!(!text.contains(&agent), "a token's text is never listed");
        let sessions = sessions_of(listed, "acme");
        assert_eq!(sessions.len(), 1, "{sessions:?}");
        let s = &sessions[0];
        assert_eq!(s.token, "agent");
        assert_eq!(
            (s.client_name.as_deref(), s.client_version.as_deref()),
            (Some("test"), Some("0"))
        );
        let said = s.agent.as_ref().expect("the agent's self-report");
        assert_eq!(
            (said.model.as_deref(), said.harness.as_deref()),
            (Some("m-1"), Some("h-1"))
        );
        assert_eq!(s.running, 0);
        let last = s.last.as_ref().expect("a last call");
        assert_eq!(
            (last.tool.as_str(), last.outcome.as_str()),
            ("books.start", "ok")
        );
        assert!(s.connected_at.ends_with('Z'), "{}", s.connected_at);
        assert!(sessions_of(ask(&books, Request::Sessions).await, "beta").is_empty());

        // Ended: not listed.
        let r = send_mcp(
            &books,
            mcp_request(Method::DELETE, &agent, Some(&sid), serde_json::json!({})),
        )
        .await;
        assert!(answer(r).await.0 < 300);
        assert!(sessions_of(ask(&books, Request::Sessions).await, "acme").is_empty());

        // Closed with the book: a session that was open is gone, and the
        // reopened book has none.
        let r = send_mcp(
            &books,
            mcp_request(Method::POST, &agent, None, initialize()),
        )
        .await;
        let again = r.headers()[SESSION_HEADER].to_str().unwrap().to_string();
        assert_eq!(answer(r).await.0, 200);
        assert_eq!(bound(&books), std::slice::from_ref(&again));
        assert_eq!(
            ask(&books, Request::CloseBook { id: "acme".into() }).await,
            closed("acme")
        );
        assert!(bound(&books).is_empty());
        let list = Request::Sessions;
        match ask(&books, list).await {
            Response::Sessions(all) => {
                assert!(
                    all.iter().all(|b| b.book != "acme"),
                    "closed books list none"
                )
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            ask(&books, Request::OpenBook { id: "acme".into() }).await,
            opened("acme", admin::OpenOutcome::Opened)
        );
        assert!(sessions_of(ask(&books, Request::Sessions).await, "acme").is_empty());
        // The old session id isn't one of the reopened book's.
        let r = send_mcp(
            &books,
            mcp_request(
                Method::POST,
                &agent,
                Some(&again),
                serde_json::json!({"jsonrpc": "2.0", "id": 9, "method": "tools/list"}),
            ),
        )
        .await;
        assert_eq!(answer(r).await.0, 404);
    }

    /// Everything in a folder, by relative path.
    fn digest_of_tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        fn walk(base: &Path, at: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            let Ok(entries) = std::fs::read_dir(at) else {
                return;
            };
            for e in entries.flatten() {
                let path = e.path();
                if path.is_dir() {
                    walk(base, &path, out);
                } else if let Ok(bytes) = std::fs::read(&path) {
                    out.insert(
                        path.strip_prefix(base).unwrap().display().to_string(),
                        bytes,
                    );
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(dir, dir, &mut out);
        out
    }

    /// A refusal's access record is written by a blocking task that keeps
    /// the request's place inside the book's door: when the client hangs up
    /// and the request's future is dropped while the record still waits for
    /// a thread, a close still waits for it (INVARIANT 10). Before, the
    /// place went with the future, and the record could land after the
    /// close was answered, and after a reopening against the new log's head.
    #[test]
    fn a_refusal_being_recorded_keeps_its_place_when_its_client_hangs_up() {
        let home = tempfile::tempdir().unwrap();
        // One blocking thread, held: the record can only wait for it.
        let rt = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        let (gate, access) = match books.held().get("acme") {
            Some(Book::Open { gate, access, .. }) => (gate.clone(), access.clone()),
            _ => panic!("acme is open"),
        };
        rt.block_on(async {
            let (release, held) = std::sync::mpsc::channel::<()>();
            let blocker = tokio::task::spawn_blocking(move || {
                let _ = held.recv();
            });
            // A request with a token nobody knows: refused at the door; its
            // future is dropped while the record waits for the thread.
            let gone = tokio::time::timeout(
                Duration::from_millis(100),
                at_the_door(&books, "acme", "no such token"),
            )
            .await;
            assert!(gone.is_err(), "the record was written with no thread free");
            assert_eq!(
                gate.inside.load(Ordering::SeqCst),
                1,
                "the request's place inside the door went with its future"
            );
            // A close begun now waits for it.
            gate.shut();
            let waiting = tokio::time::timeout(Duration::from_millis(100), gate.emptied()).await;
            assert!(waiting.is_err(), "the door emptied with a record pending");
            release.send(()).unwrap();
            blocker.await.unwrap();
            tokio::time::timeout(Duration::from_secs(10), gate.emptied())
                .await
                .expect("the door empties once the record is written");
            assert_eq!(access.head().seq, 1, "the record was written");
        });
    }

    /// Whatever of a book this process holds from before its close does
    /// nothing after it, though the same book is open again (INVARIANT
    /// 10): its access log, uploads, exports and backups write, sweep, run
    /// and record nothing, so the reopened book's files, and the chain
    /// of its logs, are only what the reopened book wrote.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn handles_from_before_a_close_do_nothing_after_the_book_is_open_again() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        let dir = old_books(&books, "acme");
        let auth = books.root.auth_dir().unwrap();
        ledger_store::passphrase::set(&auth, PASS, None).unwrap();
        let (access, uploads, exports, backups) = match books.held().get("acme") {
            Some(Book::Open {
                access,
                uploads,
                exports,
                backups,
                ..
            }) => (
                access.clone(),
                uploads.clone(),
                exports.clone(),
                backups.clone(),
            ),
            _ => panic!("acme is open"),
        };
        assert_eq!(
            ask(&books, Request::CloseBook { id: "acme".into() }).await,
            closed("acme")
        );
        assert_eq!(
            ask(&books, Request::OpenBook { id: "acme".into() }).await,
            opened("acme", admin::OpenOutcome::Opened)
        );
        // The reopened book's own backups write records (its format
        // upgrade is a backup and a record), moving the log's head on from
        // where the old handle left it: an old handle that appended now
        // would break the chain.
        match upgrade(&books, "acme", PASS).await {
            Response::FormatUpgraded(_) => {}
            other => panic!("{other:?}"),
        }
        let before = digest_of_tree(&dir);
        // Stray files a sweep would take, and a grant it would lapse.
        let now = chrono::Utc::now().timestamp() + 10_000_000;
        let stray = dir
            .join(ledger_store::uploads::UPLOADS_DIR)
            .join("stray.bin");
        std::fs::write(&stray, b"x").unwrap();
        let before = {
            let mut b = before;
            b.insert(
                format!("{}/stray.bin", ledger_store::uploads::UPLOADS_DIR),
                b"x".to_vec(),
            );
            b
        };
        // The late record of a refused call, a timer's sweep, an upload, an
        // export, a run, a drill and an upgrade, all through old handles.
        let late = Access {
            principal: Principal::unknown(),
            client: None,
            request_id: None,
            action: "connect".into(),
            subject: "POST".into(),
            args_digest: None,
            result: Outcome::Refused("closing".into()),
            head_after: None,
            agent: None,
        };
        assert!(access.record(&late).is_err());
        uploads.sweep(now);
        exports.sweep(now);
        assert!(
            uploads
                .grant(
                    "t",
                    &ledger_store::journal::sha256_hex(b"b"),
                    1,
                    "text/plain",
                    "a.txt",
                    "receipt",
                    now
                )
                .is_err()
        );
        assert!(exports.put("t", "a.csv", "text/csv", b"x", now).is_err());
        assert!(backups.run("write").await.is_err());
        assert!(backups.run_with("asked", true).await.is_err());
        backups.drill().await;
        assert!(!backups.drill_due().await);
        assert!(backups.upgrade_format().await.is_err());
        assert_eq!(
            digest_of_tree(&dir),
            before,
            "an old handle changed the reopened book's files"
        );
        // The reopened book's own logs are whole and take records.
        ledger_store::access::read(&dir).expect("the access log is whole");
        let new_backups = match books.held().get("acme") {
            Some(Book::Open { backups, .. }) => backups.clone(),
            _ => panic!("acme is open"),
        };
        assert!(!Arc::ptr_eq(&backups, &new_backups));
        ledger_store::chain::read(
            &ledger_store::ops::ops_dir(&dir).join(crate::backup::LOG_FILE),
            "wanderbooks-backup-v1",
        )
        .expect("the backup record is whole");
        let _ = std::fs::remove_file(stray);
    }
}
