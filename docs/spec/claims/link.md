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
SCENARIOS:
  1. The first turn-on makes an address; off and on again, and a restart of
     the app, give the same one, and the name is claimed once.
  2. A new address differs in its label only; the old one gets no TLS.
  3. A damaged identity file is an error in words and is never overwritten.
```

*Enforced:* `Identity::load_or_create`, `Identity::new_label`, `Identity::reset`, `new_label`, `write` (module `private`, crate `ledger-link`).
*Checked:* `ledger-link` `tests/link.rs` (`the_key_the_certificate_and_the_label_are_private`, `a_new_address_stops_the_old_one`, `off_closes_the_line_and_the_address_is_the_same_on_again_and_after_a_restart`, `a_relay_that_lost_the_name_says_so_and_a_new_address_carries_on`) and unit tests in `src/identity.rs` and `src/private.rs`.

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
     a nudge or a held line starts it again.
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

*Enforced:* `Backoff`, `WakeDetector`, `Timing` (module `line`) and `Inner::hold` and `Inner::line_loop` (module `link`), crate `ledger-link`.
*Checked:* `ledger-link` `tests/link.rs` (`the_line_comes_back_after_the_relay_drops_it`, `pings_keep_an_idle_line_and_silence_loses_it`, `an_unreachable_relay_says_so_in_words_and_recovers_by_itself`) and unit tests in `src/line.rs` (`the_backoff_grows_stays_under_its_ceiling_and_resets`, `a_wall_clock_jump_with_no_monotonic_jump_is_a_sleep`).

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
      two thirds through its life), from whichever authority the relay lists
      so a swap needs no release.
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
SCENARIOS:
  1. Order, store, reuse across a restart: one order in all.
  2. The renewal window opens: a second order names the first.
  3. The first authority fails and the second issues; one needing a binding
     the relay has none for is never asked.
  4. Every authority failing says so in words and mends when one works.
```

*Enforced:* `StoredCert`, `CertStore`, `default_renewal` (module `certs`), `AcmeIssuer` (module `acme`), `Inner::cert_loop` and `Inner::order` (module `link`), crate `ledger-link`.
*Checked:* `ledger-link` `tests/link.rs` (`the_first_authority_fails_and_the_second_issues`, `an_authority_that_is_not_usable_is_skipped`, `every_authority_failing_is_said_and_retried_until_one_works`, `the_certificate_is_renewed_at_the_authoritys_suggested_time`, `without_renewal_information_it_renews_two_thirds_through`) and `tests/acme.rs` (`an_order_goes_through_account_proof_finalize_and_download`, `a_renewal_reuses_the_account_names_what_it_replaces_and_reads_the_window`, `a_binding_is_sent_when_there_is_one_and_never_invented`, `a_refusal_comes_back_in_the_authoritys_words`).

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
SCENARIOS:
  1. Each trouble has words; the mending ones say so.
  2. A relay that is not there is "can't reach its relay ... keeps
     trying", and the link connects when it is.
  3. A name that cannot be resolved is no internet; an expired relay
     certificate is a wrong clock.
```

*Enforced:* `Trouble`, `LinkStatus`, `Step` (module `status`), `trouble_of_io`, `trouble_of_refusal` (module `line`), crate `ledger-link`.
*Checked:* `ledger-link` `tests/link.rs` (`an_unreachable_relay_says_so_in_words_and_recovers_by_itself`, `no_internet_is_said_as_that`, `a_wrong_clock_is_said_as_that`) and unit tests in `src/status.rs` and `src/line.rs` (`errors_read_as_the_right_trouble`).

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
SCENARIOS:
  1. On, a web assistant signs in through the relay and reads the books;
     off, the line and the door are gone and the address is kept; on again,
     the same address; a new address, and the old one is refused.
```

*Enforced:* `WebAssistants` (`turn_on`, `turn_off`, `new_address`, `restore`) in `crates/ledger-host/src/web.rs`.
*Checked:* `ledger-cli` `tests/web_link.rs` (`a_web_assistant_signs_in_through_the_relay_and_reads_the_books`, `off_closes_the_line_and_the_door_and_on_again_is_the_same_address`, `a_new_address_refuses_what_the_old_one_signed_in`, `the_switches_are_put_back_at_the_next_start`).
