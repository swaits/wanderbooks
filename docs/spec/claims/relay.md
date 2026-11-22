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
  4. A caller who has not shown the name's key learns nothing about which
     names exist: a name nobody has claimed, a name held by another key,
     and a blocked name are refused with one answer (`bad_proof`). Only
     the holder of a blocked name's key is told `blocked`; a claim of a
     held name, blocked or not, is `taken`.
  5. Every handshake frame is at most 2 048 bytes, and the whole
     handshake, TLS included, finishes within `handshake_timeout_ms`.
SCENARIOS:
  1. A recorded hello from an accepted session, sent on a new session, is
     refused; so is a hello signed over another session's exporter and one
     signed over an old nonce.
  2. A wrong key for an existing name is refused; the owner reconnects.
  3. A hello with a name that is not four listed words, a frame that is
     not JSON and a frame over the limit end the line without touching
     the registry.
  4. Another key's reconnect to a name that is claimed, one that is
     unclaimed and one that is blocked get the same refusal.
```

*Enforced:* `decide`, `handshake`, `proto::proof_message`, `proto::verify`, `read_frame`, `write_frame`, `client::make_hello`.
*Checked:* `relay` `tests/relay.rs` (`a_replayed_or_relayed_proof_is_refused`, `a_name_cannot_be_claimed_twice_and_a_wrong_key_is_refused`, `a_stranger_cannot_tell_which_names_exist_or_are_blocked`, `malformed_hellos_are_refused_without_touching_the_registry`, `a_reconnect_with_the_right_key_replaces_the_old_line`, `a_line_handshake_that_never_finishes_is_dropped`); unit tests in `src/proto.rs` (`a_proof_holds_for_one_session_and_nothing_else`, `frames_are_bounded`).

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
     letters, and holds none of a set of unwelcome words (nor "executor",
     "testament" or "probate"); it has 1 427 words, so a name is 41.9
     bits, and the floor is 40 bits (1 024 words), which a test states.
     Where two words sound alike (peer and pier) one is listed, and where
     one is the inflection of another (accrue and accrued) the base form
     is: a name read aloud is unambiguous.
  3. A host is routed only as `<secret>.<name>.<domain>`, the secret being
     25 to 63 lower-case letters or digits; the apex, the relay's host and
     every other shape are not an app's. The app makes the secret from at
     least 128 random bits: 25 or more random base 36 characters (25 of
     them are 129.2 bits). The relay cannot check that the characters are
     random, so it refuses a label too short to hold 128 bits, and the app
     is the one that must not make it from less.
SCENARIOS:
  1. `ledger-accrual-tally-mint` parses and prints as itself;
     `ledger-accrual-tally`, an unlisted word, `relay` and `ns1` do not.
  2. `<secret>.ledger-accrual-tally-mint.wanderbooks.link` is that app's;
     the same without the secret, with a secret under 25 characters
     (`aaaaaaaaaaaaaaaa`) or under another domain is not.
```

*Enforced:* `Name::parse`, `Name::random`, `words`, `Zone::classify`, `Zone::below`.
*Checked:* unit tests in `src/names.rs` (`the_list_is_sorted_unique_plain_and_welcome`, `a_name_is_four_listed_words_and_prints_as_itself`, `four_words_are_at_least_40_bits`, `no_word_is_an_inflection_or_a_homophone_of_another`, `a_host_is_routed_only_as_secret_dot_name_dot_domain`, `a_secret_label_under_25_characters_is_not_routed`, `below_names_the_labels_under_the_zone`).

**WB-relay-4** The registry is the one durable table: name, public key, created, blocked, first publish.

```text
GOAL: a name belongs to the first key that claims it and to no other,
      across restarts, with nothing else about an app kept.
SOURCE: D-85 in plan/p15-app-and-relay.md.
INVARIANTS:
  1. A name has at most one key; a later claim, with the same key or
     another, never changes the row.
  2. A claim is durable before it is acknowledged. Claiming a name does
     not spend the certificate authority's budget: the claim is limited
     by a safety ceiling on the registry (5 000 new names a day, counted
     from the rows' creation times in the same transaction, so a restart
     does not reset it) and, in memory, per address and per network: 5 a
     day for one IPv4 address or IPv6 /64, 20 a day for one IPv4 /24 or
     IPv6 /48 (an IPv4-mapped IPv6 address counts as its IPv4 address).
  3. The certificate authority's issuance limit (50 new certificates a
     week for the domain) is charged when a name first publishes a proof
     record, once per name: the registry keeps the time of a name's first
     publish and counts them in the window in one transaction, so a
     restart does not reset the 50 a week, and a renewal (a name already
     charged) is free. First publishes have per-prefix limits of their own
     in memory, over the same window: 3 a week for one IPv4 address or
     IPv6 /64, 10 a week for one IPv4 /24 or IPv6 /48.
  4. Requests to the database go through a bounded queue; past it the
     relay says busy.
  5. The operator can block a name; a blocked name's line is refused, a
     blocked name's live line is closed at its next publish and by a
     sweep of the registry every 10 seconds (`block_check_ms`), so a block
     written by the operator's command reaches the running relay within
     about 10 seconds, and an open line does not outlast it.
SCENARIOS:
  1. Twenty racing claims of one name: one wins.
  2. A claim, a reopen, a lookup: the key is there; the window limit
     counts rows made before the reopen.
  3. Two names are claimed under a limit of two; a third is refused, and
     a blocked name cannot connect.
  4. Four names claim under a certificate budget of two: all four claim,
     the first two publish, the third's first publish is refused, and the
     first renews at no charge; a restart keeps the count.
  5. The operator blocks a name whose line is up: the line closes, the
     assistant can no longer reach it, and the name's owner is told
     `blocked` when it reconnects.
```

*Enforced:* `Registry::claim`, `Registry::lookup`, `Registry::set_blocked`, `Registry::charge_publish`, `Registry::blocked_names`, `Registry::open`, `PrefixLimit::allow`, `publish`.
*Checked:* unit tests in `src/registry.rs` (`the_first_claim_wins_and_the_row_never_changes`, `racing_claims_have_one_winner`, `a_claim_outlives_a_restart_and_the_window_counts_old_rows`, `blocking_a_name_is_kept`, `the_certificate_budget_charges_a_name_once_and_survives_a_restart`, `the_blocked_names_are_listed_and_a_file_without_the_column_is_upgraded`); `relay` `tests/relay.rs` (`blocked_names_and_the_limit_on_new_names`, `one_address_cannot_claim_many_names`, `the_certificate_budget_is_spent_at_first_publish_not_at_claim`, `first_publishes_are_limited_per_address`, `blocking_a_name_closes_its_live_line`, `a_blocked_name_cannot_publish_and_its_line_is_closed`); unit tests in `src/gate.rs` (`fifty_addresses_in_one_slash_64_get_the_slash_64s_limit_not_fifty_times_it`, `one_slash_48_cannot_use_many_slash_64s_to_beat_the_limit`, `a_mapped_address_is_counted_with_its_ipv4_address`, `a_prefix_limit_resets_with_its_window_and_a_refusal_counts_nothing`).

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
     address (IPv4 address or IPv6 /64) from a fixed table, 20 a second
     with a burst of 40, and a sender over its rate is answered
     truncated with no data (so a real resolver retries over TCP, and
     nothing larger than the query is sent to a spoofed source); all UDP
     answers together are limited to 5 000 a second, past which there is
     no answer; a UDP "connection reset" from the receive is not an error
     of the socket and is passed over at once; TCP is capped in
     connections, per address, in queries and in time.
  4. The list of authorities is served in its order, each with its
     directory address, whether it needs an account binding, whether it
     is usable now, and the binding when it has one; it is read again
     when its file changes, and a bad file never replaces a good list.
  5. The list file never carries a binding secret, and none is ever
     served from it: a file with `binding_kid` or `binding_hmac_key` is
     refused when read, in words. A binding comes only from a per-app
     source (an implementation of `BindingSource` that asks the authority
     for one per app); with none installed, as in `wanderbooks-relay`
     today, an authority that needs a binding is listed, served without
     one and marked not usable yet.
  6. DNS answers CAA at the apex (a certificate authority finds it by
     climbing from any name below, RFC 8659) with one `issue` record per
     listed authority, `issue "<caa_domain>; validationmethods=dns-01"`
     (RFC 8657), from each authority's `caa_domain` in the same list, so
     swapping authorities is an edit to one file; with no authority
     listed there is no CAA record. The relay's own host alone has a set
     of its own, the same authorities with no method restriction, so its
     certificate can be had by HTTP-01 and renewed by certbot's timer; no
     other name below the apex has one, so an app's name still climbs to
     the apex and DNS-01.
SCENARIOS:
  1. Two apps each publish; each name answers its own value and not the
     other's; a bad value is refused; clearing removes only one's own.
  2. A for `<secret>.<name>.<domain>` is the relay's address with and
     without a TXT record published for that name.
  3. The list is served as written, then the file is edited to put
     another authority first and add a third, and the next request
     returns that order, and the zone's CAA record follows, with no
     change to the app.
  4. An authority that needs a binding, with no per-app source, is served
     without one and marked not usable; a file with a binding secret is
     not loaded.
  5. A hundred queries from one address in a burst: a few are answered,
     the rest truncated; past the global ceiling, none.
```

*Enforced:* `serve_control`, `TxtStore::publish`, `TxtStore::get`, `DnsService::answer`, `serve_udp`, `serve_tcp`, `AuthorityList::for_app`, `AuthorityList::caa_issuers`, `AuthorityList::reload`, `DnsService::truncated`, `is_proof_value`.
*Checked:* `relay` `tests/relay.rs` (`a_proof_record_is_published_for_ones_own_name_and_no_other`, `publishing_needs_a_proven_line`, `the_authority_list_is_served_and_follows_the_file`, `no_shared_binding_secret_is_ever_served`, `dns_over_udp_over_the_rate_is_answered_truncated`, `dns_over_udp_has_a_global_ceiling`, `dns_over_tcp_is_capped_and_times_out`); unit tests in `src/dns.rs` (`every_name_under_the_zone_is_at_the_relay_even_beneath_a_label_with_a_record`, `the_proof_record_is_answered_for_its_own_name_until_it_expires`, `outside_the_zone_is_refused_and_odd_queries_are_answered_plainly`, `garbage_never_panics_and_is_mostly_ignored`, `caa_at_the_apex_permits_the_listed_authorities_by_dns_01_only`, `caa_at_the_relay_host_permits_the_listed_authorities_by_any_method`, `no_authority_list_means_no_caa_even_at_the_relay_host`, `caa_is_made_from_the_list_and_follows_it`, `the_truncated_answer_is_the_question_with_the_tc_bit_and_nothing_more`), `src/txt.rs` (`a_value_is_answered_until_its_ttl`, `only_proof_values_are_taken`, `values_per_name_and_names_in_all_are_bounded`, `publishes_are_counted_per_hour`) and `src/authorities.rs` (`the_list_is_served_in_order_and_a_binding_comes_only_from_a_source`, `a_list_file_that_carries_a_binding_secret_is_refused_in_words`, `a_bad_list_is_refused`, `an_edit_changes_what_is_served_and_a_bad_edit_does_not`, `a_binding_source_is_asked_only_for_authorities_that_need_one`).

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
      unbounded bytes, tasks or map entries; idle lines are cheap; and
      one address or one network cannot stop the relay for the others.
SOURCE: D-80 in plan/p15-app-and-relay.md (bounded in memory, sized for
        hundreds of thousands of idle apps on one modest host).
INVARIANTS:
  1. `#![forbid(unsafe_code)]`. Every limit is a field of `Limits` with a
     default; nothing else decides a size or a time.
  2. Addresses are counted as what a subscriber holds: an IPv4 address,
     or an IPv6 /64 (host reach), and an IPv4 /24 or IPv6 /48 (net
     reach). An IPv4-mapped IPv6 address (`::ffff:a.b.c.d`, as a
     dual-stack listener reports an IPv4 caller) is made canonical first,
     in every table: the connection gates, the handshake gates, the claim
     and first-publish limits, the DNS rate tables.
  3. A connection is counted against a global limit and the per-address
     limits before any work; a hello not complete in `hello_timeout_ms` is
     closed; a peer may have few unfinished hellos (32 per address, 256
     per network, 4 096 in all). The server name is not known before the
     hello is read, so a reserve of 512 hello places cannot be kept "for
     the relay's own host name": it is kept for addresses that held a
     proven line in the last 12 to 24 hours (a two-generation Bloom
     filter of 2^23 bits, filled only by a line that passed its proof),
     and is used only when the 4 096 are taken. A flood from addresses
     that never proved a name cannot use it; an app that comes back from a
     new address during a flood retries until the flood ends.
  4. A line handshake holds a place in the pool of 1 024, and a place for
     its address (4) and its network (64), from the moment the router
     sees the relay's host name until the verdict, so one address cannot
     hold the pool by finishing a hello and stalling. Per-address tables
     are fixed in size and do not grow with the number of addresses.
  5. Per app: `streams_per_app` (8) caller streams, `control_streams_per_app`
     (2) control streams, and 4 more the line may count while a stream
     just closed is forgotten; a line silent for `line_idle_ms` dropped;
     across apps `max_lines`. Every stream may hold a 256 KiB receive
     window, which the yamux crate fixes (its 0.14 has no way to start a
     stream smaller, and refuses a connection window below the sum), so
     the most one line can be made to hold is 14 streams of 256 KiB,
     3.5 MiB (it was 52 streams, 13 MiB).
  6. A spliced connection holds one buffer per direction, reads only when
     the other side has taken the last write, and ends when a write of
     `n` bytes is not done in `write_stall_ms` (5 s) plus `n` over
     `min_stream_bytes_per_sec` (2 KiB a second), so a reader that takes
     bytes slowly enough to keep a window full is cut, not outwaited; and
     when it is idle for `stream_idle_ms` or reaches `stream_max_age_ms`.
  7. An idle line costs about 15 KiB of the relay's memory (measured,
     release build, 20 000 lines, after these limits: 15.4 KiB, and
     15.6 KiB after two rounds of churn), and that figure does not grow
     under churn.
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
  6. Two handshakes that get the challenge and stall, from one address
     under a limit of two: a third is refused at once, and a place freed
     is reusable.
  7. With the hello pool full, an address that held a line gets in by the
     reserve and a stranger does not; the reserve is itself bounded.
  8. Fifty addresses in one IPv6 /64 get one /64's share; a hundred /64s
     in one /48 get one /48's; two IPv4 callers on a dual-stack listener
     do not share a slot.
  9. A reader taking 128 bytes a second behind a 1 KiB a second floor is
     cut; one at 2 KiB a second is not.
```

The limits, with what they are for (all in `Limits`, all configurable):

| Limit | Default | Counted by |
|---|---|---|
| `max_pending_hellos` / `hello_reserve` | 4 096 / 512 | all / known line addresses only |
| `max_pending_per_ip` / `_per_net` | 32 / 256 | IPv4 address or /64 / IPv4 /24 or /48 |
| `max_handshakes` | 1 024 | all |
| `max_handshakes_per_ip` / `_per_net` | 4 / 64 | until the verdict |
| `max_connections_per_ip` | 20 000 | AI vendors share a few addresses |
| `streams_per_app` / `control_streams_per_app` | 8 / 2 (+4 slack) | per line |
| `write_stall_ms` / `min_stream_bytes_per_sec` | 5 000 / 2 048 | per write |
| `claims_per_window` / `claim_window_ms` | 5 000 / 24 h | all names (safety ceiling) |
| `claims_per_ip` / `claims_per_net` | 5 / 20 a day | new names per prefix |
| `cert_publishes_per_window` / `cert_window_ms` | 50 / 7 d | names' first publishes (the authority's limit) |
| `cert_publishes_per_ip` / `_per_net` | 3 / 10 a week | first publishes per prefix |
| `txt_publishes_per_hour` | 12 | per name |
| `block_check_ms` | 10 000 | one registry query |
| `dns_qps_per_ip` / `dns_qps_global` | 20 / 5 000 | UDP, truncated / silent past it |

*Residual:* the certificate budget and the pools are cut per prefix, which
makes them cost an attacker more, not impossible: an attacker with many
networks (a provider's /32 holds 65 536 IPv6 /48s) can still spend the
week's 50 first publishes, or fill the hello pool. The answers are the
reserve for known lines, renewals being free, a second authority in the
list when the first's limit is reached, and asking the authority to raise
the domain's limit.

*Enforced:* `Limits`, `IpGate::acquire`, `PrefixLimit::allow`, `KnownAddrs`, `RateTable::allow`, `Shared::admit`, `accept_loop`, `serve_line`, `mux::spawn`, `splice::splice`, `LineMap::insert`.
*Checked:* `relay` `tests/relay.rs` (`a_silent_or_dripping_peer_is_closed_at_the_hello_timeout`, `one_address_cannot_hold_many_unfinished_hellos`, `one_network_cannot_hold_many_unfinished_hellos`, `a_known_line_address_still_gets_a_hello_place_when_the_pool_is_full`, `one_address_cannot_hold_the_pool_of_line_handshakes`, `streams_per_app_are_capped`, `lines_are_capped`, `a_slow_reader_back_pressures_and_is_cut_off_at_the_stall_limit`, `a_silent_line_is_dropped_and_a_pinging_one_is_not`, `shutdown_closes_every_line`); `relay` `tests/memory.rs` (`idle_line_memory`, run by hand: ignored in a normal run); unit tests in `src/gate.rs` (`a_limit_of_two_admits_two_and_frees_on_drop`, `every_permit_gives_back_what_it_took`, `ipv6_counts_by_the_slash_64`, `a_net_gate_counts_a_slash_48_and_an_ipv4_slash_24`, `two_ipv4_callers_on_a_dual_stack_listener_do_not_share_a_slot`, `known_addresses_are_remembered_for_one_to_two_periods`, `a_rate_table_allows_a_burst_then_the_rate`), `src/mux.rs` (`a_line_holds_only_its_streams`, `a_silent_line_is_closed_after_its_idle_time`, `a_large_transfer_both_ways_completes`) and `src/splice.rs` (`a_peer_that_never_reads_stalls_the_splice_and_buffers_little`, `a_reader_under_the_floor_rate_is_cut_and_one_at_the_floor_is_not`, `a_silent_connection_is_closed_after_its_idle_time`, `a_busy_connection_still_ends_at_its_maximum_age`).
