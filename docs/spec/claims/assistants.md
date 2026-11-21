# Claims: assistants

Wanderbooks' own product rules for how the window deals with the assistants
that do the bookkeeping (plan P15, `plan/p15-app-and-relay.md` 3.3). No
authority outside this product states them. They are `WB-assist-<n>`. The
word in the window is "assistant", never "MCP", "agent", "server" or "token".

Rules: none from bookspec's list; these are product rules only.

---

### Product rules

**WB-assist-1 Every company carries one of three states, as a shape and a word,
read from the server's own answer.** A company in the window is in exactly one
state: no assistant connected, connected (with the program's name: "Claude
Code"), or working (a call of an assistant is running); or not known, when the
books server has not answered lately (starting, gone, slow, or none). The
state comes from the server's sessions request, never from anything the
window did. A tab carries the shape (plugs apart, plugs together, a turning
arc: three silhouettes, so it reads without colour) with the words in its
hover; the front company's state is said in words once, at the right of the
top bar, with the last thing done in words ("Posted an entry, 10 seconds
ago"). The sidebar's links are not given up for it: they all fit at 1479x519
(`tests/sidenav.rs`). Asking is off the UI thread, one request at a time, at
most every 2 seconds; an answer older than 15 seconds is not believed; a
server that is slow or gone is "not known" and no frame waits for it. What an
assistant says of itself is shown as said and trusted for nothing.
*Enforced:* `ledger_gui::connections` (`Connections::poll`, `status_within`,
`Status::word`, `Status::detail`), `ledger_gui::connect::paint_mark` and
`chip`, `ledger_gui::tabs::Tab::link`, `ledger_host::app_server::AppServer::roster`.
*Checked:* `ledger-gui` `tests/assistants.rs`
(`each_state_is_read_from_the_servers_answer_and_is_the_companys_own`,
`a_slow_server_is_never_waited_for_and_one_request_is_out_at_a_time`,
`a_server_that_is_gone_or_refuses_is_not_known_and_the_window_goes_on`,
`the_three_states_are_three_different_shapes`,
`the_state_marks_the_top_bar_and_the_pages_look_as_blessed`); `tests/sidenav.rs`.

**WB-assist-2 Closing the window, or a tab, with an assistant connected warns
first, then closes if confirmed (D-79).** Asked to close the window, or a
company's tab, while an assistant is connected to what would close, the window
says so in plain words, names the companies and who is connected, says that
what an assistant is in the middle of finishes first and that nothing is cut
off half way, and closes only when the person says so; "Keep it open" leaves
everything as it was. With no assistant connected it does not warn. One
dialog is up at a time: a job that holds the window has its own dialog and no
warning is raised meanwhile; the warning comes before, and never beside, the
server's close ("Closing your books" and the close report of WB-app-4); a
close the person confirmed goes straight to that close and is not warned
again. A server that does not answer is not a reason to hold a close.
*Enforced:* `ledger_gui::app::App` (`guard_assistants`, `ask_close`),
`ledger_gui::connections::Connections::roster_within`.
*Checked:* `ledger-gui` `tests/assistants.rs`
(`closing_the_window_with_an_assistant_connected_warns_and_then_closes_if_confirmed`,
`closing_the_window_with_no_assistant_connected_does_not_warn`,
`a_server_that_does_not_answer_does_not_hold_a_close`,
`closing_a_tab_with_an_assistant_connected_warns_and_only_then`).

**WB-assist-3 "Ask your assistant" is where a button that edits would be.** The
window only reads the books; to change anything a person asks an assistant.
On each page where a person might look for a button that edits (transactions,
invoices and bills, parties, accounts, fixed assets, projects, documents, the
company's settings) there is "Ask your assistant", which copies one ready
sentence naming the company and what the page is for ("In my Wanderbooks books
for Acme, please create a new invoice.") and says it did. With no assistant
connected it says so and links to the Connect page. It writes nothing, and it
is not offered in a window that runs no books server. A banner saying
"read-only" is not the design.
*Enforced:* `ledger_gui::widgets::page_header_ask`,
`ledger_gui::connect::ask_button`, `ask_feedback`, `sentence`.
*Checked:* `ledger-gui` `tests/assistants.rs`
(`ask_your_assistant_copies_one_ready_sentence`,
`a_window_with_no_server_offers_no_assistant`).

**WB-assist-4 The Connect page connects an assistant on this computer and ends
a key.** The page ("Assistants", under the company) says in its first sentence
that everything here is for reading and that changes go through an assistant.
It has a row for each of Claude Code, Codex, Gemini CLI, Grok Build, Muse Code
and Cursor. Claude Code and Codex have a button that does what `wanderbooks
connect` does (a key made and kept on this computer, the bridge added to the
assistant's own list) and, folded, the exact command it runs. For the other
four the page gives the program and arguments an assistant must run
(`<this program> mcp --book ID`, with `--root` when a root was named), the
one-time "Make the key" step, and says the tool is untested: the repository
documents no configuration format for them, and none is invented. The page
lists every key to the company with who is connected on it, what it may do and
when it connected and last acted, and "Revoke", asked once, which calls what
`wanderbooks token revoke` calls (the server ends the key and the sessions on
it; its copy in the token file is removed). The window writes no books. The
web half of the page (the switch, the address, the steps for Claude and
ChatGPT) is plan unit B3's, and the "Claude desktop app" row (a one-click
extension) is unit C1's. What unit B1 puts in `connect::State::web_section` is
the question to the person when a web assistant asks to connect (WB-remote-5):
a card that says who is asking, where it is sent back to, the address it asked
from and what it will be able to do (read and draft; posting only if ticked),
with "Allow" and "Don't allow", over any page of the company
(`connect::State::web_prompt`) and on the Connect page; and, for each web
connection allowed, what it may do, with "Let it post entries" and "Stop
letting it post". Revoke is the same row of "Who can reach these books".
*Enforced:* `ledger_gui::connect::State::show`, `ledger_host::commands`
(`bridge_serve_args`, `command_line`, `ensure_key`, `forget_token_file`),
`ledger_host::app_server::AppServer::revoke_token`.
*Checked:* `ledger-gui` `tests/assistants.rs`
(`the_top_bar_state_opens_the_connect_page_which_lists_and_revokes`,
`the_connect_buttons_do_what_connect_does`).

**WB-assist-5** The company's name and "call books.start first" stand in the first tools' own descriptions too.

```text
GOAL: a product that keeps only the first 512 characters of the server's
      instructions, or drops them, still tells its model which company this is
      and where to begin.
SOURCE: plan/p15-app-and-relay.md 3.4 (ChatGPT uses only the first 512
        characters; claude.ai is reported to drop them, unverified since).
INVARIANTS:
  1. The first 512 characters of the instructions stand alone and name the
     company, books.start and books.handbook.
  2. The descriptions of books.start and of the tools a model reaches for
     first begin with the company and "call books.start first in every
     session", for local and remote callers alike; the lead is at most 160
     characters.
SCENARIOS:
  1. A list of tools names the company in each of the first tools and in
     no other.
```

*Enforced:* `first_tools_lead`, `connect_text`, `list_tools` (the server's `BooksServer`).
*Checked:* `ledger-mcp` unit test `the_company_and_the_entry_point_are_in_the_first_tools_too`.
