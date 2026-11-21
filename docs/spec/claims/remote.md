# Claims: the remote door

The product rules of how web assistants (Claude and ChatGPT on the web; Gemini,
Grok and muse.ai best effort) reach a company, and the sign-in behind it (plan
P15, unit B1; decisions D-81, D-83, D-86, D-88 in `plan/p15-app-and-relay.md`;
the ten sign-in rules of its section 4). No authority outside this product
states them: they are wanderbooks' own (`WB-remote-<n>`). They stand on public
specifications for the protocol (OAuth 2.1, RFC 7636, 7591, 8414, 8707, 9728 and
9700, the MCP authorization specification) and on nothing else. The code is the
`remote` module of the books server (`crates/ledger-mcp/src/local/remote/`).

The door listens on 127.0.0.1 only, off until the app asks. The relay (unit B2)
and the app's line to it with its certificate (unit B3) put it on the internet;
the relay writes the caller's address in a PROXY header before the caller's own
bytes (WB-relay-6), and this door reads it.

Rules: none from bookspec.

---

## Product rules

**WB-remote-1** The door is off until asked, is loopback only, and knows each caller's address only from the relay's header.

```text
GOAL: nothing reaches the books from the internet unless the app turned the
      door on, and the address of a caller is the one place no caller can
      write.
SOURCE: WB-relay-6 (the caller's address), D-80, D-83 in
        plan/p15-app-and-relay.md; the PROXY protocol specification version 2;
        plan/p15-app-and-relay.md section 4, rule 8.
INVARIANTS:
  1. The door listens only after a RemoteStart admin request, on 127.0.0.1;
     it stops on RemoteStop and when the server closes. It serves a company
     only while that company is offered (RemoteAllow) and open.
  2. Every connection begins with the relay's exact header (a version 2
     PROXY header for TCP over IPv4 or IPv6); anything else, a cut header,
     or silence for five seconds is dropped before a byte of HTTP is read.
  3. The address in the header is what the access record and the limits use;
     no HTTP header (X-Forwarded-For and its kind) is read for it; a second
     header sent inside the caller's own bytes is HTTP text, refused as such.
  4. Connections are bounded, and a request's headers must arrive in time.
SCENARIOS:
  1. A call through the door is recorded with the address in the header, not
     with the one a caller's forwarding headers claim.
  2. A connection with no header, or with a header inside a request, gets
     nothing.
  3. The door is off before it is asked, and refuses connections after it is
     stopped.
```

*Enforced:* `read_header`, `accept_loop`, `start_door`, `stop_door`, `Asked` (the server's `local` module).
*Checked:* `ledger-mcp` unit tests `the_relays_header_reads_back_its_source`, `anything_but_the_exact_form_is_refused`, `the_recorded_address_is_the_relays_and_a_forged_one_changes_nothing`, `the_door_is_off_until_asked_and_stops_when_told`.

**WB-remote-2** There is one public address, and every address the door says is built from it.

```text
GOAL: a product that learns where to sign in is never told another place,
      whatever it sends in its request.
SOURCE: RFC 9728 and RFC 8414 (the metadata documents and where they are
        found); plan/p15-app-and-relay.md section 4, rule 1 (a year of
        breakage in another project came of building these from the request).
INVARIANTS:
  1. The issuer is the address given to RemoteStart (https, a host, no path)
     and no other; a request for another Host is refused (421).
  2. The protected-resource document is served at the root, after the
     well-known name with the resource's path appended, and before it; the
     authorization server's at its two well-known names, with a path
     appended or prepended too. Every address in them is the issuer's.
  3. A token, code or refresh token made under one issuer is refused under
     another.
SCENARIOS:
  1. Every place a product looks answers the same document.
  2. The public address changes: what was made under the old one stops.
```

*Enforced:* `start_door`, `server_document`, `resource_document`, `Remote::authenticate`.
*Checked:* `ledger-mcp` unit tests `the_metadata_is_the_pinned_address_at_every_place_products_look`, `an_expired_token_and_a_token_for_another_issuer_are_refused`.

**WB-remote-3** An unsigned request is a 401 that says where to sign in, `initialize` included, and is never written to the books' record.

```text
GOAL: a product that was told nothing finds the sign-in; a stranger cannot
      fill the disk by knocking.
SOURCE: RFC 6750 (the challenge), RFC 9728; plan/p15-app-and-relay.md section
        4, rule 4 (one product shows "connected" and never signs in when its
        first request is let through).
INVARIANTS:
  1. A request to a company's path with no token, a wrong one or an expired
     one is a 401 whose WWW-Authenticate names that company's
     protected-resource metadata; a presented bad token adds
     error="invalid_token". `initialize` is no exception.
  2. A key made for the local socket opens nothing on this door.
  3. Such a request is counted in memory and not written to the access record
     (a request with a good token that is refused is).
SCENARIOS:
  1. initialize, a tool call and a notification, unsigned, are each 401.
  2. Twenty more unsigned requests leave the access record where it was.
```

*Enforced:* `unauthorized`, `Remote::authenticate`, `door` (the server's `local` module).
*Checked:* `ledger-mcp` unit test `an_unsigned_request_is_a_401_that_says_where_to_sign_in_initialize_included`.

**WB-remote-4** A client introduces itself in one of three ways, and is sent back only to an address it registered.

```text
GOAL: any of the products can sign in, and none can be sent to an address a
      stranger chose.
SOURCE: RFC 7591 (dynamic registration), the IETF draft for OAuth client ID
        metadata documents, RFC 6749 section 10.15 (open redirectors), RFC
        7636 (PKCE), RFC 8707 (resource); plan/p15-app-and-relay.md section 4,
        rules 2 and 3.
INVARIANTS:
  1. A client is a client ID metadata document (an https address, fetched
     only if every address its host has is public, with no redirect, at most
     64 KiB, within five seconds, its client_id the address it came from),
     a dynamic registration, or a client id and secret the app made.
  2. A redirect address is https, a loopback http address, or an app's own
     scheme, with no fragment or userinfo; a client may carry up to twenty;
     the one asked for must equal one registered exactly. A mismatch is a
     page, never a redirect.
  3. PKCE with S256 is required, and its verifier is checked in constant
     time; `resource` must be one company's address under the issuer (or
     absent where one company is on offer); a code is single-use, lives
     sixty seconds, and ends what it made if used twice; the consent is a
     302.
  4. A secret (a client's, a code, a token) is kept as its SHA-256 only.
SCENARIOS:
  1. Each of the three kinds signs in end to end and reads the books.
  2. Wrong redirect, no PKCE, the plain method, a wrong resource, a wrong
     verifier and a reused code are each refused.
```

*Enforced:* `Remote::register`, `Remote::resolve_client`, `Remote::authorize`, `Remote::token`, `parse` (client documents), `is_public`, `valid_redirect`.
*Checked:* `ledger-mcp` unit tests `a_registered_client_signs_in_and_reads_the_books`, `a_client_metadata_document_signs_in`, `a_typed_client_id_and_secret_signs_in`, `a_redirect_that_is_not_registered_is_never_redirected_to`, `pkce_the_resource_and_the_verifier_are_required`, `only_public_addresses_are_fetched`, `a_document_must_be_what_it_says_it_is`.

**WB-remote-5** No assistant connects without the person, in the app's window.

```text
GOAL: nothing a stranger sends over the internet can approve a connection.
SOURCE: D-83 in plan/p15-app-and-relay.md (a new assistant is approved in
        the app's window, by whoever is at the machine).
INVARIANTS:
  1. /authorize never approves: it parks the request for five minutes and
     answers a page that says to look at Wanderbooks (no form, no button,
     no script, every outside word escaped) and looks again by itself.
  2. Only an admin request (the owner-only admin socket) turns a waiting
     request into a code, by an id that is not the browser's: the page's own
     id only reads the outcome, once (302 with the code, or with
     access_denied after a refusal or a timeout).
  3. The window is told who is asking: the product by where it is sent
     back to (never by what it calls itself), that address, the caller's
     address, the company, whether posting was asked for.
  4. Waiting requests are bounded in all and per address.
SCENARIOS:
  1. The person allows: the browser is sent back with a code.
  2. The person refuses, or no one answers: the browser is sent back with
     access_denied, and nothing was made.
  3. A guess of the browser's id decides nothing.
```

*Enforced:* `Remote::authorize`, `Remote::wait`, `Remote::decide`, `Remote::pending_list`, `waiting` and `problem` (the page), `ledger_gui::connect::State::web_prompt`, `ledger_host::app_server::AppServer::web_decide`.
*Checked:* `ledger-mcp` unit tests `the_window_is_told_who_asks_and_the_page_only_says_to_look`, `a_refusal_and_a_timeout_send_the_browser_back_denied_and_make_nothing`, `waiting_requests_are_bounded_per_address_and_in_all`, `a_strangers_words_never_become_markup`; `ledger-gui` `tests/assistants.rs` (`a_web_assistant_asking_is_put_to_the_person_over_any_page_in_plain_words`, `refusing_and_allowing_with_posting_are_their_own_answers`, `a_request_that_ran_out_goes_away_and_posting_is_turned_on_and_off_on_the_connect_page`).

**WB-remote-6** An approval is the owner's decision about a product, a company and what it may do, and it survives a product registering again.

```text
GOAL: Claude and Grok register anew on most sign-ins; the person is not
      asked again, and revoking ends everything made under the approval.
SOURCE: plan/p15-app-and-relay.md section 4, rule 6; RFC 9700 (refresh token
        rotation and reuse); D-86.
INVARIANTS:
  1. An approval is one token of the books' principals (so "Who can reach
     these books" lists it, and revoking it by id ends it) plus a record
     naming the product (by its known redirect hosts, or its document
     address, or its name and redirects) and the company.
  2. A sign-in from the same product for the same company, while its
     principals' token is live and it was used within sixty days, is sent
     back with a code at once; for another company it is a new question.
  3. An access token lives ten minutes and is memory only; a refresh token
     lives sixty days since last use, is kept (hashed) across restarts, is
     rotated at every use, and one used twice ends its whole family.
  4. A revoked connection's access token is a 401 at once, and its refresh
     token is invalid_grant; a new sign-in asks the person again.
SCENARIOS:
  1. A second registration of Claude is signed in without a question.
  2. A refresh after ten days, and after a restart, still works.
  3. Revoke in the list ends access and refresh.
```

*Enforced:* `Remote::decide`, `Remote::issue`, `Remote::refresh`, `Remote::authenticate`, `Remote::save`.
*Checked:* `ledger-mcp` unit tests `an_approval_survives_a_product_registering_again`, `a_code_used_twice_ends_what_it_made_and_so_does_a_refresh_token`, `revoking_the_connection_ends_the_access_token_and_the_refresh_token`, `refresh_tokens_survive_a_restart_and_only_their_hashes_are_kept`.

**WB-remote-7** A failure to sign in never bans an address; what a request creates is bounded.

```text
GOAL: a stranger can fill neither memory nor disk, and a vendor's shared
      address is never shut out for another's failures.
SOURCE: plan/p15-app-and-relay.md section 4, rule 7 (the vendors' addresses
        are shared; rate-limit per approval instead).
INVARIANTS:
  1. Registered clients (2000 in all, thirty an hour from one address,
     600 an hour in all), waiting requests (twenty, five per address), codes,
     access tokens, refresh tokens, approvals, families, fetches of client
     documents and the limiter's own table are all bounded.
  2. A registration nobody used for a sign-in goes after an hour; the oldest
     unused goes first when the table is full; none unused and full is a
     refusal.
  3. Nothing bans an address; a refusal says to wait. An approval's own
     calls are limited to 600 a minute.
SCENARIOS:
  1. Thirty-one registrations from one address: the thirty-first waits;
     another address is not shut out.
  2. Unused registrations expire.
```

*Enforced:* `Remote::allow`, `Remote::make_room`, `Remote::sweep`.
*Checked:* `ledger-mcp` unit tests `registration_is_bounded_and_unused_registrations_expire`, `waiting_requests_are_bounded_per_address_and_in_all`.

**WB-remote-8** A token for one company never reaches another, and a company not offered is not there.

```text
GOAL: the lock on the front door is per company.
SOURCE: D-78 in plan/p15-app-and-relay.md (a connection names its company and
        is never redirected to another).
INVARIANTS:
  1. An access token carries the company of its approval; at another
     company's path it is a 403, recorded as that token's refusal with the
     caller's address.
  2. A company that is not offered (or not open) answers 404 on its path and
     has no metadata.
  3. A code and a refresh token carry their company and never make a token
     for another.
SCENARIOS:
  1. A token for acme at beta's path is 403; with beta not offered, 404.
```

*Enforced:* `door` (the server's `local` module), `Remote::is_allowed`, `resource_document`.
*Checked:* `ledger-mcp` unit test `a_token_for_one_company_never_reaches_another`.

**WB-remote-9** A web assistant may read and draft; posting is the owner's to turn on, per connection.

```text
GOAL: an assistant that also reads email, and can be steered by it, cannot
      post by default.
SOURCE: D-86 in plan/p15-app-and-relay.md.
INVARIANTS:
  1. An approval grants read and write (drafts), not post and never admin;
     the access record (admin) is not reachable through this door whatever
     the token says.
  2. The owner raises or lowers posting per approval (RemoteSetPost): the
     token that carries it is replaced by one with the new scopes in one
     principals record, and the same access tokens carry on.
  3. A key made for an assistant that takes only a key (RemoteMakeKey) is a
     principals token the door accepts, shown once, with the same scopes;
     no other principals token is accepted here.
SCENARIOS:
  1. A new connection can't post; raised, it can; lowered, it can't.
```

*Enforced:* `scopes_for`, `Remote::set_post`, `Remote::make_key`, `Remote::authenticate`.
*Checked:* `ledger-mcp` unit tests `the_default_is_read_and_draft_and_the_person_raises_it_to_post`, `an_unsigned_request_is_a_401_that_says_where_to_sign_in_initialize_included` (an agent's own key opens nothing here).

**WB-remote-10** A remote call is the same call: one write path, plain JSON, no session.

```text
GOAL: a write through the door is exactly once (D-88), and the door adds no
      second way to the books.
SOURCE: D-88 in plan/p15-app-and-relay.md; WB-format-8 to 10.
INVARIANTS:
  1. Every remote call is handled by the same server as a local one
     (`call_tool`): a write without its request_key is refused, and a repeat
     of a key returns the first answer and writes nothing. The remote door
     has no tool of its own.
  2. It is stateless and answers plain JSON: no session id is made, and one
     a caller names is not heard.
SCENARIOS:
  1. A draft without a key is refused; with one, twice, it is one draft and
     the same answer.
```

*Enforced:* `mcp_via` (the server's `local` module), `call_tool` (the server's `BooksServer`).
*Checked:* `ledger-mcp` unit tests `a_remote_write_needs_its_request_key_and_a_repeat_returns_the_first_answer`, `a_registered_client_signs_in_and_reads_the_books`.
