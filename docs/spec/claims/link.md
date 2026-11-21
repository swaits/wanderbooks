# Claims: the link

The product rules of the app's side of the relay (plan P15, unit B3;
decisions D-80, D-82, D-85, D-87 in `plan/p15-app-and-relay.md`). No authority
states them: they are wanderbooks' own (`WB-link-<n>`). The code is the crate
`crates/ledger-link` (the app's identity, line and TLS and certificates) and,
joining it to the books server's remote door, `crates/ledger-host/src/web.rs`.
The relay itself is `crates/relay` (`relay.md`); the remote door and its
sign-in are `remote.md`.

Rules: none from bookspec.

---

## Product rules

**WB-link-1** An installation has one durable address, `https://<secret>.<four-words>.wanderbooks.link`, and its key never leaves the machine.

```text
GOAL: a person has one address that is the same every time web assistants
      are turned on, that nobody can list or guess, and a key the relay is
      only ever sent the public half of; and can have a new address, after
      which the old one stops working.
SOURCE: D-82, D-85, D-87 in plan/p15-app-and-relay.md; section 5 ("Names":
        one name per installation, each company a path under it; the secret
        label stops listing and guessing); the relay's own rule (WB-relay-3:
        a label is 25 to 63 characters of a-z and 0-9).
INVARIANTS:
  1. The name is four words from the relay's list, picked by the app and
     kept; the relay claims it for the app's key at the first line.
  2. The secret label is 32 random characters of a-z and 0-9, about 165
     bits (128 are needed), made by rejection sampling.
  3. The key (Ed25519), the label and the certificate's key are in the
     data root's private link folder: owner-only files (0600 on Unix, the
     owner-only access list the token files use on Windows), written whole
     or not at all.
  4. A new address replaces the label at once and the old host name is no
     longer given a certificate; if the relay has lost the name, a whole new
     identity is made.
  5. No key, label or token is in a log, an error, a status's debug text or
     a test's output.
  6. The label is not a secret from the network. It travels in the caller's
     TLS hello (the server name) and in the DNS queries that find the
     address, so the relay, the vendor's resolvers and the path in between
     all see it. The sign-in is the lock; the label only stops listing and
     guessing. Nothing here says otherwise to the person.
  7. The address kept is read from the folder once and held: the window
     asks every frame, and the key beside it is read only where it signs.
SCENARIOS:
  1. The first turn-on makes an address; off and on again, and a restart of
     the app, give the same one, and the name is claimed once.
  2. A new address differs in its label only; the old one gets no TLS.
  3. A damaged identity file is an error in words and is never overwritten.
```

*Enforced:* `Identity::load_or_create`, `Identity::new_label`, `Identity::reset`, `new_label`, `write` (module `private`, crate `ledger-link`).
*Checked:* `ledger-link` `tests/link.rs` (`the_key_the_certificate_and_the_label_are_private`, `a_new_address_stops_the_old_one`, `off_closes_the_line_and_the_address_is_the_same_on_again_and_after_a_restart`, `a_relay_that_lost_the_name_says_so_and_a_new_address_carries_on`, `the_kept_address_is_read_once_not_every_frame`) and unit tests in `src/identity.rs` and `src/private.rs`.

**WB-link-2** The app holds one outbound line to the relay and keeps it, by itself, through anything.

```text
GOAL: the line comes back after the network drops, the relay restarts, the
      relay is down for a while or the computer sleeps and wakes on another
      network, never spins hot against a relay that is down, and ends when
      it is told to or when the app does.
SOURCE: plan/p15-app-and-relay.md section 6a ("Unit B3's duties: ping at
        least every 60 s (a line silent for 180 s is dropped); ... a yamux
        stream limit at least the relay's"); D-80; WB-relay-2 (the proof
        that is signed for each session).
INVARIANTS:
  1. The line is yamux over TLS 1.3 to the relay, proved with the app's key
     over that session (the relay crate's own client); the line runs in the
     app's own process and ends with it, so it can never keep a door open for
     books that closed.
  2. A failed dial waits a growing, jittered time (at most five minutes);
     a nudge starts it again. A line that held for a minute is redialled
     after a wait spread over 0 to 30 seconds, and only then does the
     backoff start over; a line that ended sooner (a relay that accepts and
     drops) waits as a failed dial does. So a relay that restarts is not
     met by every app in the same second, and one that misbehaves is not
     redialled at a steady rate for ever.
  3. The line is pinged every 30 seconds and a ping not answered in 15
     ends it and a new one is made.
  4. A wall-clock step much longer than the time that passed (a sleep) ends
     the line and dials again at once.
  5. The line offers the relay more streams than the relay's limits need.
SCENARIOS:
  1. The relay drops the line; the app dials again and the same name is
     served, with no new claim.
  2. An idle line, pinged, outlives several of the relay's silence limits;
     without pings the relay drops it and the app dials again.
  3. A relay that is not there is dialled with a backoff, not in a loop.
```

*Enforced:* `Backoff`, `Redial`, `WakeDetector`, `Timing` (module `line`) and `Inner::hold` and `Inner::line_loop` (module `link`), crate `ledger-link`.
*Checked:* `ledger-link` `tests/link.rs` (`the_line_comes_back_after_the_relay_drops_it`, `pings_keep_an_idle_line_and_silence_loses_it`, `an_unreachable_relay_says_so_in_words_and_recovers_by_itself`) and unit tests in `src/line.rs` (`the_backoff_grows_stays_under_its_ceiling_and_resets`, `a_wall_clock_jump_with_no_monotonic_jump_is_a_sleep`, `a_line_that_ends_young_keeps_the_backoff_and_one_that_held_is_spread`).

**WB-link-3** The app ends the web caller's TLS itself, with its own certificate, and hands the plain stream to the remote door after the secret and the relay's header.

```text
GOAL: what the relay forwards is TLS it cannot read; the app decrypts it
      with a key that never left the machine, for the app's own host name
      only; and the remote door can tell the connection came from the app's
      line and whose address the caller has.
SOURCE: D-80 (encrypted end to end), plan/p15-app-and-relay.md section 4
        rule 8 (the per-start secret, then the PROXY header); WB-remote-1;
        WB-relay-6.
INVARIANTS:
  1. TLS (1.3 and 1.2, with HTTP/2 or HTTP/1.1 by ALPN) is ended with the
     certificate the app holds, and only for the exact host name of its
     address: any other name, the apex and a stranger's guess get no
     certificate.
  2. The first 64 bytes of every connection to the door are the door's
     per-start secret as given, then the relay's PROXY version 2 header,
     then the caller's plain bytes; the door is on 127.0.0.1 only.
  3. At most 16 callers are served at once, and a caller whose handshake
     does not finish in time is dropped; with no door, a caller is turned
     away at once.
  4. Nothing the relay could use to read traffic leaves the machine.
  5. Who can read what, said plainly: the relay that carries the
     connection cannot read it. The assistant's maker (for example
     Anthropic or OpenAI) reads what the person lets that assistant read,
     which is what it is sent. "Nobody can read what passes through" is
     not true and is not said; the Connect page says the two sentences
     above.
SCENARIOS:
  1. A caller's TLS through the relay reaches a stand-in door, which is
     given the secret first, then the caller's address as the relay saw
     it, then plain HTTP; what crossed the relay holds neither the
     request, the reply, the secret nor any key.
  2. With no door the caller's connection ends; with it back, it works.
```

*Enforced:* `Resolver`, `server_config` (module `certs`) and `Inner::serve_caller` (module `link`), crate `ledger-link`.
*Checked:* `ledger-link` `tests/link.rs` (`a_caller_through_the_relay_reaches_the_door_and_the_relay_sees_only_ciphertext`, `a_stalled_handshake_is_dropped_and_no_door_means_no_service`) and `ledger-cli` `tests/web_link.rs` (`a_web_assistant_signs_in_through_the_relay_and_reads_the_books`).

**WB-link-4** The app's certificate is its own, from the authority the relay lists, kept across restarts and renewed ahead of expiry.

```text
GOAL: a certificate for the app's own name, ordered with a key made here,
      kept so a restart orders nothing, renewed when the authority says (or
      two thirds through its life), from whichever of a fixed set of public
      authorities the relay lists, so a swap among them needs no release;
      and nothing the relay says can make the app believe a certificate
      from an authority it made up, or send a request to a server it
      named.
SOURCE: D-87, plan/p15-app-and-relay.md section 5 ("Authorities are data,
        not code"; the proof record is published by the relay for the app's
        own name); RFC 8555 (ACME, DNS-01, external account binding);
        RFC 9773 (renewal information).
INVARIANTS:
  1. The order is for the wildcard under the app's name; the proof goes to
     the relay over the app's own line and is cleared afterwards; the app
     gives an authority no contact.
  2. The authorities are the relay's list, in its order; one marked not
     usable is skipped; a failure falls to the next; all failing is a
     state in words, retried with a growing, jittered wait while the line
     stays up.
  3. The certificate and its key are kept in one private file and read at
     start; a certificate for another name, or one that does not parse, is
     ordered again.
  4. Renewal is at the time the authority's renewal information suggests
     (asked again from time to time), else two thirds through the life,
     naming the certificate it replaces; the certificate in use serves
     until it ends if a renewal fails.
  5. An account binding is sent only when the relay gave one for this app.
  6. A certificate is believed only if its chain ends at one of the public
     roots the program ships, for the app's wildcard, at issue and at every
     load (`CertTrust`; only a test builds one of its own roots, and
     nothing at run time can). A lifetime over 100 days is refused, and a
     renewal is planned no later than two thirds of the life or 60 days,
     whichever is sooner. A kept certificate that fails any of this is
     ignored and ordered again. The same name keeps a good wildcard across
     "Get a new address"; a name that is given up (a lost one, a first claim
     that found it taken) has none carried over.
  7. A certificate is never served at or after its end, however far off
     the next renewal try is: the resolver refuses it, no wait runs past it,
     and the state stops saying it is in place.
  8. The app sends a request only to an authority's directory on a host
     compiled into it (`DIRECTORY_HOSTS`: Let's Encrypt production and
     staging, Google Trust Services, ZeroSSL, Buypass and SSL.com, each
     from its own documentation), over https and with no port or name in
     the address. The relay still chooses which of them and in what order;
     one on any other host is skipped and said so. A kept account is used
     only if its directory is on the list too. Adding a brand-new
     authority therefore takes a release of the app: that is the price of
     the relay not being trusted with where the app sends its requests.
  9. The certificate work starts when the line is up, so it is for the name
     the relay accepted; a first claim that finds the name taken picks
     another before any certificate is ordered, and the door is started as
     the address that was settled (`Link::wait_settled`).
SCENARIOS:
  1. Order, store, reuse across a restart: one order in all.
  2. The renewal window opens: a second order names the first.
  3. The first authority fails and the second issues; one needing a binding
     the relay has none for is never asked.
  4. Every authority failing says so in words and mends when one works.
  5. A certificate from an authority the relay made up, or that lives over
     100 days, is refused and never stored or served; one on disk is
     discarded; one that has ended is not served; a directory on another
     host is not contacted; a name taken on the first claim is replaced
     before any order.
```

*Enforced:* `StoredCert`, `CertStore`, `CertTrust`, `Resolver`, `renewal_cap` (module `certs`), `AcmeIssuer`, `DIRECTORY_HOSTS` (module `acme`), `Link::wait_settled`, `Inner::cert_loop` and `Inner::order` (module `link`), crate `ledger-link`.
*Checked:* `ledger-link` `tests/link.rs` (`the_first_authority_fails_and_the_second_issues`, `an_authority_that_is_not_usable_is_skipped`, `every_authority_failing_is_said_and_retried_until_one_works`, `the_certificate_is_renewed_at_the_authoritys_suggested_time`, `without_renewal_information_it_renews_two_thirds_through`) and `tests/acme.rs` (`an_order_goes_through_account_proof_finalize_and_download`, `a_renewal_reuses_the_account_names_what_it_replaces_and_reads_the_window`, `a_binding_is_sent_when_there_is_one_and_never_invented`, `a_refusal_comes_back_in_the_authoritys_words`, `a_directory_on_a_host_the_app_does_not_know_is_refused_before_any_request`, `a_kept_account_for_an_unlisted_directory_is_not_used`) and `tests/link.rs` (`a_certificate_from_an_authority_the_relay_made_up_is_never_stored_or_served`, `a_stored_certificate_from_a_made_up_authority_is_discarded_and_ordered_again`, `a_name_taken_on_the_first_claim_settles_before_the_certificate_and_the_address`, `an_expired_certificate_is_not_served_and_the_state_says_so`) and unit tests in `src/certs.rs` (`a_certificate_from_an_authority_the_relay_made_up_is_refused`, `a_lifetime_over_the_cap_is_refused`, `renewal_is_never_later_than_two_thirds_or_sixty_days`, `a_stored_certificate_that_fails_is_ignored_when_read`, `the_resolver_serves_only_its_host_and_only_until_the_certificate_ends`) and `src/acme.rs` (`every_listed_host_is_accepted_at_its_directory_and_no_other_is`).

**WB-link-5** Every state of the link ends in plain words and mends by itself.

```text
GOAL: no network, a relay that is down or busy, an authority that is
      rate-limited, a clock that is wrong, a laptop that slept: each is said
      in words a person can use, says what is not affected, and recovers
      without the person unless it needs them.
SOURCE: plan/p15-app-and-relay.md 3.3 ("progress in plain words ... the
        state when the relay cannot be reached, in words, with nothing lost
        locally").
INVARIANTS:
  1. One place makes the words (`Trouble::words`, `LinkStatus::words`); a
     trouble that mends itself says it keeps trying and that the local
     assistants still work; one that needs the person says what to do.
  2. A relay certificate that has ended or not begun is a wrong clock, not
     a relay that is down.
  3. A name that does not resolve is no internet; an unanswered relay is
     the relay down; a relay that does not know the name is the name lost,
     and a new address makes a new identity.
  4. The link's state is read from memory and never waits; the window and
     the local assistants never depend on the network.
  5. The window's words are the app's own, chosen from the kind of
     failure. What the relay or an authority said is shown only as a short
     quoted detail: control and invisible characters removed, white space
     made single spaces, cut to about 120 characters, and the whole text
     of failures bounded (`detail`, `clean`).
SCENARIOS:
  1. Each trouble has words; the mending ones say so.
  2. A relay that is not there is "can't reach its relay ... keeps
     trying", and the link connects when it is.
  3. A name that cannot be resolved is no internet; an expired relay
     certificate is a wrong clock.
```

*Enforced:* `Trouble`, `LinkStatus`, `Step` (module `status`), `trouble_of_io`, `trouble_of_refusal` (module `line`), crate `ledger-link`.
*Checked:* `ledger-link` `tests/link.rs` (`an_unreachable_relay_says_so_in_words_and_recovers_by_itself`, `no_internet_is_said_as_that`, `a_wrong_clock_is_said_as_that`, `an_authority_that_shouts_is_shown_short_and_plain`) and `tests/acme.rs` (`what_the_authority_says_is_short_plain_and_quoted`) and unit tests in `src/status.rs`, `src/text.rs` (`long_text_with_tricks_comes_out_short_and_plain`) and `src/line.rs` (`errors_read_as_the_right_trouble`).

**WB-link-6** Web assistants are on per company, off by default, and the line and the door are open only while some company is on.

```text
GOAL: nothing is dialled, listened on or offered until a person turns a
      company on; turning it off closes the line and the door and keeps the
      address; what was on when the app closed is on again at the next
      start.
SOURCE: plan/p15-app-and-relay.md 3.3 ("Allow web assistants" per company,
        off by default; "Turning it off closes the line and the door; the
        name is kept"), section 5 (one name per installation); D-84.
INVARIANTS:
  1. Turning on starts the door (127.0.0.1) as the app's one address and
     offers the company; turning off withdraws the company and, with none
     left, closes the line and then the door.
  2. A new address restarts the door at the new address: every token and
     sign-in made at the old one is refused.
  3. The switches are kept in a private file and put back for the
     companies that are open; a failure at any step leaves nothing
     half-on.
  4. Off is only off when the books server said so. A refusal, or any
     reply but its success, leaves the company shown as on, with the error
     in words (it is still reachable by anyone holding a token); a server
     that cannot be reached at all closes the whole line and the door, for
     every company, and says so.
  5. The door is started as the address the relay has accepted: the name is
     claimed and settled first.
SCENARIOS:
  1. On, a web assistant signs in through the relay and reads the books;
     off, the line and the door are gone and the address is kept; on again,
     the same address; a new address, and the old one is refused.
```

*Enforced:* `WebAssistants` (`turn_on`, `turn_off`, `new_address`, `restore`) in `crates/ledger-host/src/web.rs`.
*Checked:* `ledger-cli` `tests/web_link.rs` (`a_web_assistant_signs_in_through_the_relay_and_reads_the_books`, `off_closes_the_line_and_the_door_and_on_again_is_the_same_address`, `a_new_address_refuses_what_the_old_one_signed_in`, `the_switches_are_put_back_at_the_next_start`) and unit tests in `crates/ledger-host/src/web.rs` (`one_company_off_is_unreachable_at_once_while_another_still_works`, `a_refusal_leaves_the_company_shown_as_on_with_the_reason`, `an_unreachable_server_closes_the_whole_line_and_door_and_says_so`).
