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
nothing manages. The `run/app-hosted` marker, written by every start, is
permanent: nothing ever removes it, so a root the app has served is the
app's for good. A headless user who ran the desktop app once on a root sees,
from then on, an agent's bridge (`wanderbooks mcp`) answer "Wanderbooks isn't
running" when no app runs, and not start a host on demand as it did before;
a host they start themselves (`wanderbooks serve`) holds the root and the
bridge uses it as always (WB-app-3). Deleting the marker by hand gives the
root back to the on-demand host.
*Enforced:* `ledger_host::app_server::AppServer` (`start_in`, `stop`, its
`Drop`), `AppHost::start` and `AppHost::stop`, `ANOTHER_HAS_THEM`; the server's own close, ledger-mcp
`local::run` (INVARIANT 6).
*Checked:* `ledger-cli` `app_server.rs` (`the_server_is_a_child_that_closes_when_its_input_does`,
`a_window_that_dies_leaves_no_server_and_no_half_closed_book`,
`a_start_is_refused_when_another_program_has_the_books`,
`a_stop_that_runs_out_of_time_leaves_the_server_closing_and_says_so`,
`a_server_that_never_gets_ready_is_stopped_with_the_window_and_not_left_behind`,
`a_window_closed_before_the_child_was_stored_stops_the_child`,
`a_late_server_is_waited_for_and_stays_the_windows_to_manage`).

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
`wanderbooks mcp --book X` connects to the root's `run/mcp.sock`. For a
root the app has served (it wrote `run/app-hosted`) with no host holding
it, the bridge holds no lease and starts no host. With the app not
running, every call is answered "Wanderbooks isn't running: start
Wanderbooks and open X."; with the app running but the company not open in
it, "Company X isn't open: open it in Wanderbooks." Neither answer is
final: the call after the app starts, or the company is opened, goes
through, in a session already running. A root no app has served, and any
root a host holds (`wanderbooks serve`, the headless host), behave as they
did: the bridge holds a lease and starts a host.
*Enforced:* `ledger_host::bridge` (`serves_the_app`, `keep_in_app`,
`not_running_words`, `not_open_words`), INVARIANT 12.
*Checked:* `ledger-host` bridge tests (`with_the_app_not_running_every_call_is_answered_in_words`,
`with_the_app_running_a_company_that_is_not_open_is_said_in_words`,
`a_host_bridge_passes_the_servers_words_on_as_they_are`);
`ledger-cli` `app_server.rs` (`the_bridge_says_the_app_isnt_running_then_works_once_it_is`: not
running, then running with the company closed, then open, then closed
again, then the window closed, in one session, with no host started).

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
