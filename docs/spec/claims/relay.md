# Claims: relay

The product rules of the relay on `wanderbooks.link` (plan P15, unit B2;
decisions D-80, D-82, D-85, D-87 in `plan/p15-app-and-relay.md`). No
authority states them: they are wanderbooks' own (`WB-relay-<n>`). The
relay is the crate `crates/relay`; it depends on none of the `ledger-*`
crates. A person runs the desktop app at home; web assistants can only call
servers on the public internet; the app dials out to the relay and holds a
line open, and the relay routes an assistant's TLS connection, by its server
name, down that line. The relay holds no key or certificate for any app and
cannot read the traffic.

Rules: none from bookspec.

---

## Product rules

**WB-relay-1** The router reads the server name in the first TLS message and forwards the encrypted bytes, and nothing else.

```text
GOAL: route an assistant's connection to the app that owns the name in
      its first message, without terminating TLS for that name, and close
      anything else the same way.
SOURCE: RFC 8446 section 4.1.2 and RFC 6066 section 3 (the ClientHello and
        the server_name extension); D-80 in plan/p15-app-and-relay.md
        (the relay cannot read traffic).
INVARIANTS:
  1. The router reads only the first TLS record, to its server name, and
     then copies bytes both ways; the only TLS the relay ends is the
     line's, for its own host. It holds no certificate for any app.
  2. An unknown name, an app that is not connected, an app over its stream
     limit, a host that is not `<secret>.<four words>.<domain>`, and every
     malformed first message are closed in the same way (a TLS
     `unrecognized_name` alert, then close, or close with nothing).
  3. The ClientHello parser never panics and never reads beyond what it
     was given, whatever the bytes; a hello split over several records is
     refused.
  4. Bytes cross in both directions with one buffer per direction; a close
     in one direction is passed on, and the other direction carries on
     until it ends.
SCENARIOS:
  1. An assistant connects with an app's name; TLS completes between the
     assistant and the app with the app's own certificate, bytes go both
     ways, and a relay holding no certificate the assistant trusts is
     enough.
  2. An unknown name and an offline app are closed with the same alert.
  3. Random bytes, mutated hellos and every cut of a real hello are
     refused without a panic.
```

*Enforced:* `route`, `read_hello`, `hello::server_name`, `hello::record_len`, `Zone::classify`, `splice::splice` (crate `relay`).
*Checked:* `relay` `tests/relay.rs` (`an_assistant_reaches_the_app_by_name_with_the_apps_own_certificate`, `the_relay_holds_no_certificate_the_assistant_would_accept`, `unknown_names_offline_apps_and_odd_hosts_are_closed_the_same_way`, `bytes_that_are_not_a_hello_are_closed`); unit tests in `src/hello.rs` (`random_bytes_never_panic`, `a_mutated_hello_never_panics`, `a_real_rustls_hello_gives_its_name`, `not_tls_and_cut_off_bytes_are_refused`) and `src/splice.rs` (`bytes_cross_both_ways_and_closes_are_passed_on`, `a_large_transfer_arrives_whole_in_small_buffers`).

**WB-relay-2** An app proves it holds the key for its name, for one TLS session only, every time it connects.

```text
GOAL: neither a stranger nor a recording of an old proof, nor a relay that
      sat between two sessions, can take a name's line.
SOURCE: D-85 in plan/p15-app-and-relay.md (the key is pinned in the
        relay's table); RFC 8032 (Ed25519); RFC 5705 and RFC 8446 section
        7.5 (TLS exporters).
INVARIANTS:
  1. The line is TLS 1.3. The relay sends a fresh 32-byte nonce; the app
     answers with its name, its public key, whether it is claiming the
     name, and an Ed25519 signature over a context string, this session's
     TLS exporter value, the nonce, the claim flag and the name.
  2. The signature is checked before any registry work, so a peer without
     the key never reaches the registry.
  3. A name's line is brought up only for the key the registry holds for
     it (or the key that has just claimed it); a reconnect with the right
     key replaces the old line.
  4. Every handshake frame is at most 2 048 bytes, and the whole
     handshake, TLS included, finishes within `handshake_timeout_ms`.
SCENARIOS:
  1. A recorded hello from an accepted session, sent on a new session, is
     refused; so is a hello signed over another session's exporter and one
     signed over an old nonce.
  2. A wrong key for an existing name is refused; the owner reconnects.
  3. A hello with a name that is not four listed words, a frame that is
     not JSON and a frame over the limit end the line without touching
     the registry.
```

*Enforced:* `decide`, `handshake`, `proto::proof_message`, `proto::verify`, `read_frame`, `write_frame`, `client::make_hello`.
*Checked:* `relay` `tests/relay.rs` (`a_replayed_or_relayed_proof_is_refused`, `a_name_cannot_be_claimed_twice_and_a_wrong_key_is_refused`, `malformed_hellos_are_refused_without_touching_the_registry`, `a_reconnect_with_the_right_key_replaces_the_old_line`, `a_line_handshake_that_never_finishes_is_dropped`); unit tests in `src/proto.rs` (`a_proof_holds_for_one_session_and_nothing_else`, `frames_are_bounded`).

**WB-relay-3** An app's name is four words from the curated finance and accounting list.

```text
GOAL: a recognisable public name that is never a word a customer would not
      want in an address, and that no stranger can use to claim the
      relay's own hosts or a challenge name.
SOURCE: D-82 in plan/p15-app-and-relay.md; the list is
        crates/relay/words.txt.
INVARIANTS:
  1. A name is four indices into the list: a name that exists is four
     listed words, hyphen-joined in lower case, and parsing is exact.
  2. The list is sorted, has no repeat, holds words of 3 to 11 lower-case
     letters, and holds none of a set of unwelcome words; it has 1 454
     words, so a name is 42.0 bits.
  3. A host is routed only as `<secret>.<name>.<domain>`, the secret being
     16 to 63 lower-case letters or digits; the apex, the relay's host and
     every other shape are not an app's.
SCENARIOS:
  1. `ledger-accrual-tally-mint` parses and prints as itself;
     `ledger-accrual-tally`, an unlisted word, `relay` and `ns1` do not.
  2. `<secret>.ledger-accrual-tally-mint.wanderbooks.link` is that app's;
     the same without the secret, with a short secret or under another
     domain is not.
```

*Enforced:* `Name::parse`, `Name::random`, `words`, `Zone::classify`, `Zone::below`.
*Checked:* unit tests in `src/names.rs` (`the_list_is_sorted_unique_plain_and_welcome`, `a_name_is_four_listed_words_and_prints_as_itself`, `a_host_is_routed_only_as_secret_dot_name_dot_domain`, `below_names_the_labels_under_the_zone`).

**WB-relay-4** The registry is the one durable table: name, public key, created, blocked.

```text
GOAL: a name belongs to the first key that claims it and to no other,
      across restarts, with nothing else about an app kept.
SOURCE: D-85 in plan/p15-app-and-relay.md.
INVARIANTS:
  1. A name has at most one key; a later claim, with the same key or
     another, never changes the row.
  2. A claim is durable before it is acknowledged, and the limit on new
     names (50 a week by default, the certificate authority's own limit)
     is counted from the rows' creation times in the same transaction, so
     a restart does not reset it; one address may claim a few names a day.
  3. Requests to the database go through a bounded queue; past it the
     relay says busy.
  4. The operator can block a name; a blocked name's line is refused.
SCENARIOS:
  1. Twenty racing claims of one name: one wins.
  2. A claim, a reopen, a lookup: the key is there; the window limit
     counts rows made before the reopen.
  3. Two names are claimed under a limit of two; a third is refused, and
     a blocked name cannot connect.
```

*Enforced:* `Registry::claim`, `Registry::lookup`, `Registry::set_blocked`, `Registry::open`.
*Checked:* unit tests in `src/registry.rs` (`the_first_claim_wins_and_the_row_never_changes`, `racing_claims_have_one_winner`, `a_claim_outlives_a_restart_and_the_window_counts_old_rows`, `blocking_a_name_is_kept`); `relay` `tests/relay.rs` (`blocked_names_and_the_limit_on_new_names`, `one_address_cannot_claim_many_names`).

**WB-relay-5** The relay is the domain's DNS and publishes an app's proof record for its own name only; certificate authorities are a list it serves.

```text
GOAL: an app can prove its name to a certificate authority and no other
      name, the relay never proves a name on its own account, and the
      authority an app asks is chosen by the relay's list at the time.
SOURCE: D-87 in plan/p15-app-and-relay.md; RFC 8555 sections 7.3.4 and 8.4
        (external account binding, the DNS-01 challenge); RFC 4592
        (wildcards); RFC 6891 (EDNS); RFC 8482 (ANY).
INVARIANTS:
  1. A publish request carries no name: the name is the line's, proven at
     the handshake. A value is exactly 43 base64url characters; a name
     holds at most four values, for ten minutes each, and publishes at
     most twelve times an hour; the store holds at most 100 000 names.
  2. DNS answers every name under the zone with the relay's addresses
     (the name servers with their own), including names beneath a label
     that has a record of its own; TXT is answered only at
     `_acme-challenge.<name>` with the live published values; a name
     outside the zone is refused; recursion is never offered.
  3. A UDP answer never exceeds the client's size (512 without EDNS, at
     most 1 232), longer ones are truncated; UDP is rate limited per
     address from a fixed table; TCP is capped in connections, per
     address, in queries and in time.
  4. The list of authorities is served in its order, each with its
     directory address and whether it needs an account binding, and the
     binding when it does; it is read again when its file changes, and a
     bad file never replaces a good list.
SCENARIOS:
  1. Two apps each publish; each name answers its own value and not the
     other's; a bad value is refused; clearing removes only one's own.
  2. A for `<secret>.<name>.<domain>` is the relay's address with and
     without a TXT record published for that name.
  3. The list is served as written, then the file is edited to put
     another authority first and add a third, and the next request
     returns that order, with no change to the app.
```

*Enforced:* `serve_control`, `TxtStore::publish`, `TxtStore::get`, `DnsService::answer`, `serve_udp`, `serve_tcp`, `AuthorityList::for_app`, `AuthorityList::reload`, `is_proof_value`.
*Checked:* `relay` `tests/relay.rs` (`a_proof_record_is_published_for_ones_own_name_and_no_other`, `publishing_needs_a_proven_line`, `the_authority_list_is_served_and_follows_the_file`, `dns_over_udp_is_rate_limited_per_address`, `dns_over_tcp_is_capped_and_times_out`); unit tests in `src/dns.rs` (`every_name_under_the_zone_is_at_the_relay_even_beneath_a_label_with_a_record`, `the_proof_record_is_answered_for_its_own_name_until_it_expires`, `outside_the_zone_is_refused_and_odd_queries_are_answered_plainly`, `garbage_never_panics_and_is_mostly_ignored`), `src/txt.rs` (`a_value_is_answered_until_its_ttl`, `only_proof_values_are_taken`, `values_per_name_and_names_in_all_are_bounded`, `publishes_are_counted_per_hour`) and `src/authorities.rs` (`the_list_is_served_in_order_with_its_binding`, `a_bad_list_is_refused`, `an_edit_changes_what_is_served_and_a_bad_edit_does_not`).

**WB-relay-6** The app is told the caller's real address by the relay, first, in a header no caller can forge.

```text
GOAL: the app sees the address the assistant connected from, so it can
      decide on it, without trusting anything the caller sent (the Home
      Assistant CVE-2023-41894 lesson: a connection that arrives through
      the relay is not a local connection).
SOURCE: the PROXY protocol specification, version 2; D-80 in
        plan/p15-app-and-relay.md.
INVARIANTS:
  1. The relay writes a PROXY version 2 header from the socket's own peer
     address as the first bytes of the stream, before any byte the caller
     sent; a mapped IPv4 address is written as IPv4.
  2. The header parses only in its exact forms (28 bytes for IPv4, 52 for
     IPv6, no extensions); the app drops a stream whose header does not.
SCENARIOS:
  1. The app sees the assistant's own address and port.
  2. Headers for IPv4 and IPv6 round-trip; a cut, a wrong signature and a
     wrong length are refused.
```

*Enforced:* `route`, `Caller::encode`, `Caller::read`.
*Checked:* `relay` `tests/relay.rs` (`an_assistant_reaches_the_app_by_name_with_the_apps_own_certificate`); unit test in `src/proto.rs` (`a_caller_header_round_trips_for_v4_and_v6_and_nothing_else_parses`).

**WB-relay-7** Every buffer, queue, table and time in the relay is bounded.

```text
GOAL: no peer, however many or however slow, makes the relay hold
      unbounded bytes, tasks or map entries; idle lines are cheap.
SOURCE: D-80 in plan/p15-app-and-relay.md (bounded in memory, sized for
        hundreds of thousands of idle apps on one modest host).
INVARIANTS:
  1. `#![forbid(unsafe_code)]`. Every limit is a field of `Limits` with a
     default; nothing else decides a size or a time.
  2. A connection is counted against a global and a per-address limit
     before any work; a hello not complete in `hello_timeout_ms` is
     closed, and a peer may have only a few unfinished hellos. Per-address
     tables are fixed in size and do not grow with the number of
     addresses.
  3. Per app: `streams_per_app` caller streams, `control_streams_per_app`
     control streams, the line's receive windows capped, a line silent for
     `line_idle_ms` dropped; across apps `max_lines`.
  4. A spliced connection holds one buffer per direction, reads only when
     the other side has taken the last write, ends when a write is stuck
     for `write_stall_ms`, when it is idle for `stream_idle_ms` or when it
     reaches `stream_max_age_ms`.
  5. An idle line costs about 15 KiB of the relay's memory (measured,
     release build, 20 000 lines), and that figure does not grow under
     churn.
SCENARIOS:
  1. A silent peer and a peer that sends a byte every 100 ms are both
     closed at the hello timeout.
  2. A third caller for an app limited to two is closed; a place freed is
     reusable.
  3. A caller sends 256 MiB to an app that never reads: the relay takes a
     few MiB at most, then closes the stream at the stall limit.
  4. A line that never pings is dropped after its idle time and leaves
     the routing table; one that pings stays.
  5. 20 000 idle lines, brought down and up three times, hold flat.
```

*Enforced:* `Limits`, `IpGate::acquire`, `RateTable::allow`, `accept_loop`, `mux::spawn`, `splice::splice`, `LineMap::insert`.
*Checked:* `relay` `tests/relay.rs` (`a_silent_or_dripping_peer_is_closed_at_the_hello_timeout`, `one_address_cannot_hold_many_unfinished_hellos`, `streams_per_app_are_capped`, `lines_are_capped`, `a_slow_reader_back_pressures_and_is_cut_off_at_the_stall_limit`, `a_silent_line_is_dropped_and_a_pinging_one_is_not`, `shutdown_closes_every_line`); `relay` `tests/memory.rs` (`idle_line_memory`, run by hand: ignored in a normal run); unit tests in `src/gate.rs` (`a_limit_of_two_admits_two_and_frees_on_drop`, `every_permit_gives_back_what_it_took`, `ipv6_counts_by_the_slash_64`, `a_rate_table_allows_a_burst_then_the_rate`), `src/mux.rs` (`a_line_holds_only_its_streams`, `a_silent_line_is_closed_after_its_idle_time`, `a_large_transfer_both_ways_completes`) and `src/splice.rs` (`a_peer_that_never_reads_stalls_the_splice_and_buffers_little`, `a_silent_connection_is_closed_after_its_idle_time`, `a_busy_connection_still_ends_at_its_maximum_age`).
