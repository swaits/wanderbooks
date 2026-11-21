# Claims: app

Wanderbooks' own product rules for the desktop app as the host of the books
(plan P15, `plan/p15-app-and-relay.md`). No authority outside this product
states them: bookspec's rules are about the books, and these are about who
holds them open and how an assistant is told. They are `WB-app-<n>`.

Rules: none from bookspec's list; these are product rules only.

---

### Product rules

**WB-app-1 The app is the host: the books server is its child, and no one
else's.** Starting Wanderbooks starts the books server as a child process,
from the app's own executable; closing the window closes the server's
input, and the server closes as it always does (running calls finish,
nothing is killed, the last backup of each company is made). A window that
dies any way at all (a crash, a kill, a driver fault) closes the same
input, because the system closes the pipe, so no server is left behind and
no company is half closed. Starting is refused, in words, when another
program already has these books open: the host's lock (`run/host.lock`) or
the server's (`run/server.lock`) is held, the second including a server
still finishing a backup after its window went. The server is a process and
not a thread so that a fault in the window can never take a book down in
the middle of a write (D-84). A server the window started is either managed
by the window or stopped: one that is alive but not yet ready (opening big
companies, or waiting for another server to finish) is waited for, in view;
a start that fails, or a window closed before the start has the child,
closes the child's input and waits for it, so none is left running that
nothing manages. Nothing marks a root as the app's: an earlier build wrote a permanent
`run/app-hosted` file, which means nothing now and is removed at the next
start of the app's server; a root that has one works as one that does not.
Whose the books are is read from what runs (WB-app-5). The server is in a
process group of its own on Unix, so a Ctrl+C at the terminal the window
was started from reaches the window alone and never closes the books under
an open window; on Windows it breaks away from the job a launcher runs the
window in (and, where that job does not allow it, runs inside it), so a
launcher whose job kills everything in it when the window dies does not kill
the server in the middle of a backup: the window's death reaches it as the
end of its input, and it closes cleanly.
*Enforced:* `ledger_host::app_server::AppServer` (`start_in`, `stop`, its
drop), `AppHost::start` and `AppHost::stop`, `ANOTHER_HAS_THEM`; the server's own close, ledger-mcp
`local::run` (INVARIANT 6).
*Checked:* `ledger-cli` `app_server.rs` (`the_server_is_a_child_that_closes_when_its_input_does`,
`a_window_that_dies_leaves_no_server_and_no_half_closed_book`,
`a_start_is_refused_when_another_program_has_the_books`,
`a_stop_that_runs_out_of_time_leaves_the_server_closing_and_says_so`,
`a_server_that_never_gets_ready_is_stopped_with_the_window_and_not_left_behind`,
`a_window_closed_before_the_child_was_stored_stops_the_child`,
`a_late_server_is_waited_for_and_stays_the_windows_to_manage`,
`a_mark_an_earlier_build_left_is_ignored_and_removed`,
`a_terminal_ctrl_c_reaches_the_window_and_never_the_server`). The Windows
breakaway is checked by type only; no test runs it.

**WB-app-2 A tab is a company open in the server.** A company has a tab
exactly while the server has it open to assistants: the companies the
window restores as tabs are opened when the server starts, opening a tab
asks the server to open that company, closing a tab asks it to close that
company. The window itself reads the journal on its own, read-only, and
never writes the books. A request that fails is shown to the person in
words and is never swallowed: a close that says its last backup was not
whole, a company that would not open for assistants, a server that stopped
or never started. A request for one company never waits for another's. A start that failed
(another program had the books) is tried again by the next tab that opens,
or by the "Try again" button beside its notice; a server still starting is
said to be starting. A company the window wanted open that the server does
not have open when it says ready (it failed to open) is a notice for that
company. The format upgrade of a company, asked in the window, is an admin
request to this same server (`UpgradeFormat`), since there is no host.
*Enforced:* `ledger_gui::serving::Serving` (`start`, `open`, `close`,
`take_notices`), `ledger_gui::app::App::open` and `App::close`,
`ledger_host::app_server::AppServer::open_book` and `close_book`,
`AppHost::not_open` and `AppHost::upgrade_format`, `Serving::retry`.
*Checked:* `ledger-gui` `tests/serving.rs` (`a_tab_opening_asks_the_server_to_open_that_company`,
`a_tab_closing_asks_the_server_to_close_that_company`,
`the_companies_the_window_restores_are_the_servers_to_open`,
`a_close_that_was_not_whole_is_said_to_the_person`,
`a_refused_start_is_said_and_nothing_is_asked_of_the_server`,
`a_failed_start_is_tried_again_when_the_next_tab_opens`,
`the_person_can_try_a_failed_start_again`, `a_server_still_starting_is_said_to_be_starting`,
`a_company_that_did_not_open_at_the_start_is_said_and_asked_for_again`);
`ledger-cli` `app_server.rs` (`opening_and_closing_a_company_drives_the_servers_book`,
`a_company_the_server_does_not_have_open_is_told`,
`the_format_upgrade_goes_to_the_windows_own_server`);
`ledger-host` `app_server.rs` tests (`a_close_with_an_incomplete_backup_is_a_notice_never_a_plain_success`,
`an_open_that_did_not_happen_says_why`).

**WB-app-3 The bridge speaks to the running app and says so in words.**
`wanderbooks mcp --book X` connects to the root's `run/mcp.sock`. With no
host holding the root (every root but a headless `wanderbooks serve`'s), it
holds no lease and starts no host: no program starts one but `serve`. With
nothing running, every call is answered "Wanderbooks isn't running: start
Wanderbooks and open X (on a computer with no screen, run `wanderbooks
serve`), then try again."; with a server holding the books and no socket yet
(the app opening large companies, or finishing a last backup), "Wanderbooks
is still opening its companies, or finishing the last backup of the last
session: X can be reached in a moment. Try again shortly."; with the app
running but the company not open in it, "Company X isn't open: open it in
Wanderbooks." None is final: the call after the app starts, or the company
is opened, goes through, in a session already running. A root a host holds
(`wanderbooks serve`) is served as it was: the bridge holds a lease on the
host; and when that host goes and nothing takes its place the bridge says
the words above instead of starting another.
*Enforced:* `ledger_host::bridge` (`serves_the_app`, `keep_in_app`, `keep`,
`not_running_words`, `opening_words`, `not_open_words`), INVARIANT 12;
`ledger_host::opener::Opener::hold` (INVARIANTS 6, 7).
*Checked:* `ledger-host` bridge tests (`with_the_app_not_running_every_call_is_answered_in_words`,
`with_a_server_still_opening_the_answer_says_so_and_not_that_nothing_runs`,
`with_the_app_running_a_company_that_is_not_open_is_said_in_words`,
`a_host_bridge_passes_the_servers_words_on_as_they_are`,
`a_root_is_the_line_to_the_app_when_no_host_holds_it`);
`ledger-cli` `app_server.rs` (`the_bridge_says_the_app_isnt_running_then_works_once_it_is`: not
running, then running with the company closed, then open, then closed
again, then the window closed, in one session, with no host started);
`open_while_held.rs` (`a_command_leaves_nothing_running_and_serve_closes_after_its_last_holder`:
a bridge with nothing running starts no host).

**WB-app-4 Closing the window is bounded and visible.** Asked to close, the
window stays up and says it is closing the books (finishing what is
running, making each company's last backup) until the server has exited,
or until 90 seconds have passed. Each open company is closed first, so the
window can say, in words, each company whose closing backup was not whole,
a server that exited badly, and one that is still finishing by itself; when
it has anything to say it stays up, showing it, with a "Close anyway"
button, and closing the window again is not that. It never kills the server. A close does not begin
while a job that changes the books is running (a restore, a format
upgrade: they hold the window already). A server that has exited on its own
is shown to the person at once, as a notice that assistants cannot reach
their companies.
*Enforced:* `ledger_gui::app::App` (`guard_server_close`),
`ledger_gui::serving::Serving::begin_stop`, `ledger_host::app_server::STOP_WAIT`.
*Checked:* `ledger-gui` `tests/serving.rs` (`closing_the_window_stops_the_server_and_waits_for_it`,
`a_server_that_will_not_exit_is_left_closing_and_the_window_closes`,
`a_close_that_was_not_whole_keeps_the_window_up_until_the_person_closes_anyway`,
`a_server_that_exits_on_its_own_is_said_once`); `ledger-cli` `app_server.rs`
(`a_stop_that_runs_out_of_time_leaves_the_server_closing_and_says_so`,
`a_closing_backup_that_was_not_whole_is_said_when_the_window_closes`,
`a_server_that_exits_badly_is_said_when_the_window_closes`).

**WB-app-5 A command finds what runs, never starts a host, and does its
work or says to start Wanderbooks.** Every `wanderbooks …` command that
needs the books, and the window's own menu work, finds what holds the root:
a host (`wanderbooks serve`: it takes a lease, as before), the app's server
(no host holds the root, a server holds `run/server.lock`, and its admin
socket answers: it asks that server and leaves it running), or nothing. A
server that holds the lock and has no admin socket yet is opening its
companies or finishing a backup: the command says so ("Wanderbooks is still
opening (or closing) the books: waiting for it…") and waits, for as long as
its own wait lasts (`open_wait`, 120 s), then says it is still opening and
to try again; it never fails with a connection error and never starts a
second server beside it. With nothing running, no command starts a host.
One that only reads, or manages the books' files and tokens, runs a books
server of its own, the one the app starts, for the length of the work and
closes it (waiting for its last backup, 90 s at most, and saying so if it
is still finishing); it takes the same `run/server.lock` and `run/books.lock`
the app's server takes, so two programs never hold one set of books, and a
start is refused in words when another has them. One that needs a company
held open by Wanderbooks says "Start Wanderbooks first: nothing has the
books open. Open the Wanderbooks app, or, on a computer with no screen, run
`wanderbooks serve` and leave it running." `wanderbooks serve` is the only
program that starts a detached host (the headless path); it keeps the lease
protocol, the 10 s close delay, the scrubbed environment, the restart onto a
replaced program and its daily look for an update.

| Command | The app (or `serve`) running | Nothing running |
|---|---|---|
| `books list` | the server's list, open or closed | the folder's companies, "not open", no lock taken (nothing is read of them) |
| `books create`, `adopt`, `restore` | the server does it | its own server, no company open, closed after |
| `books put-aside` | works on the folder itself, as it always has: it never asked a server, and moves only copies outside the root into `legacy/` after reading the company's journal head; it takes no lock | the same |
| `books back-up ID`, `books upgrade ID`, `books extract ID N` | the server does it (the upgrade as an admin request when there is no host) | its own server with ID open, closed after |
| `books open ID`, `books close ID` | the server opens or closes ID | "Start Wanderbooks first" (there is no server to open it in) |
| `token create`, `list`, `check`, `revoke` | the server's admin socket | its own server (ID open when one is named) |
| `token print` | the token file | the token file (it never asked a server) |
| `connect`, the books home | a token as above, the pointer file | the same, its own server for the token |
| `upload`, `download` | the book's door, with the token's file | "Start Wanderbooks first" |
| `doctor` | says whose the books are and lists them, tokens and assistants | says the books are closed and Wanderbooks isn't running, and how to start it |
| `close` | on a host, closes the books; on the app, says the app's tabs and window do | "already closed" |
| `serve` | on the app, refuses ("the app has these books open"); on a host, joins it | starts the host |
| `mcp` | the bridge (WB-app-3) | the bridge says it in words |

There is no "Close the books" in the app: closing a company's tab closes
the company (WB-app-2) and closing the window closes them all (WB-app-4).
*Enforced:* `ledger_host::opener::Opener::hold` (`HoldError::NotRunning`,
`admin_answers`), `ledger_host::commands` (`direct_for`, `reach`,
`Reached::release`, `listed_while_closed`, `upgrade_by_admin`, `doctor`,
`close`).
*Checked:* `ledger-cli` `app_server.rs`
(`a_command_with_the_app_running_goes_to_its_server_and_leaves_it_running`,
`with_nothing_running_a_command_runs_its_own_server_or_says_to_start_wanderbooks`,
`a_command_waits_in_words_for_a_server_that_is_still_opening`);
`open_while_held.rs` (`a_command_leaves_nothing_running_and_serve_closes_after_its_last_holder`,
`serve_and_the_bridges_share_one_host_and_one_server`,
`books_commands_hold_the_books_while_they_run`,
`a_command_ends_at_its_deadline_during_a_restart`,
`the_viewers_new_books_are_made_by_the_server`);
`format_upgrade.rs` (`format_upgrade_waits_for_consent`: a command's own
server shows the upgrade its company waits for).
