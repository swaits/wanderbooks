//! The link, end to end on localhost: an in-process relay (the relay
//! crate's own), an in-process certificate authority that checks the proof
//! in the relay's DNS, a stand-in for the remote door, and an assistant.
//! Nothing here touches a network beyond 127.0.0.1.
#![allow(missing_docs)]

mod support;

use ledger_link::{CertTrust, Dial, Step, Trouble};
use std::sync::atomic::Ordering;
use std::time::Duration;
use support::*;

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// WB-link-1 to WB-link-3: claim, prove, line up, certificate; then a web
/// caller's TLS connection through the relay is ended at the app and the
/// door is given the secret, the caller's address and plain HTTP; and
/// what crossed the relay was ciphertext.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_caller_through_the_relay_reaches_the_door_and_the_relay_sees_only_ciphertext() {
    let w = world(LIST, |_| {}).await;
    assert_eq!(w.link.status().step(), Step::Off);
    w.link.turn_on().unwrap();
    let s = until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    assert!(s.claimed && s.line_up && s.cert_expires.is_some());
    assert_eq!(w.env.relay.shared().lines.len(), 1);
    assert_eq!(
        w.env.relay.shared().metrics.claims.load(Ordering::Relaxed),
        1
    );

    let host = w.host();
    // The address is `<secret>.<four words>.<domain>`, the secret 25 or
    // more characters of a-z and 0-9.
    let parts: Vec<&str> = host.split('.').collect();
    assert_eq!(parts.len(), 4, "{host}");
    assert!(
        parts[0].len() >= 25
            && parts[0]
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    );
    assert_eq!(format!("{}.{}", parts[2], parts[3]), DOMAIN);

    let reply = w
        .assistant(&host)
        .await
        .expect("TLS completes with the app");
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.ends_with(BODY));
    // The door was given the secret first, then the caller's address as
    // the relay saw it, then plain HTTP with no TLS in it.
    let seen = w.door.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].secret_ok);
    let from_relay = w.caller_tap.source_ports.lock().unwrap().clone();
    assert_eq!(seen[0].caller, from_relay.first().copied());
    assert!(seen[0].request.starts_with("GET /ping HTTP/1.1\r\n"));

    // What crossed the relay, both ways, is TLS: the request and the reply
    // are not in it in the clear.
    for crossed in [
        w.caller_tap.seen_up(),
        w.caller_tap.seen_down(),
        w.tap.seen_up(),
        w.tap.seen_down(),
    ] {
        assert!(!crossed.is_empty());
        assert!(!contains(&crossed, b"GET /ping"));
        assert!(!contains(&crossed, BODY.as_bytes()));
        assert!(!contains(&crossed, SECRET.as_bytes()));
    }
    // The caller's own bytes carried the name in the clear (the relay
    // routes on it) and the app's certificate only inside TLS 1.3.
    assert!(contains(&w.caller_tap.seen_up(), host.as_bytes()));
    // Nothing private was sent to the relay: not the line's key, not the
    // certificate's key, not the account.
    let key = std::fs::read(w.dir.path().join("link/line.key")).unwrap();
    let cert: serde_json::Value =
        serde_json::from_slice(&std::fs::read(w.dir.path().join("link/cert.json")).unwrap())
            .unwrap();
    let cert_key = cert["key_pem"].as_str().unwrap().to_owned();
    for crossed in [w.tap.seen_up(), w.tap.seen_down(), w.caller_tap.seen_up()] {
        assert!(!contains(&crossed, &key));
        assert!(!contains(&crossed, cert_key.as_bytes()));
        assert!(!contains(&crossed, b"PRIVATE KEY"));
    }
}

/// WB-link-1: the key and certificate files are private, and the secret
/// label is in no status text.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_key_the_certificate_and_the_label_are_private() {
    let w = world(LIST, |_| {}).await;
    w.link.turn_on().unwrap();
    until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    let dir = w.dir.path().join("link");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode =
            |p: std::path::PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.clone()), 0o700);
        for f in ["identity.json", "line.key", "cert.json"] {
            assert_eq!(mode(dir.join(f)), 0o600, "{f}");
        }
    }
    let host = w.host();
    let label = host.split('.').next().unwrap();
    let status = format!("{:?} {}", w.link.status(), w.link.status().words());
    assert!(!status.contains(label));
    // The label is not the name's, nor guessable from the files' times.
    assert!(!host.split('.').nth(1).unwrap().contains(label));
}

/// WB-link-1, 4: off means off (the line ends, the certificate stops being
/// served), the name is kept, on again is the same address, and a restart
/// of the app reuses the stored certificate: one order in all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn off_closes_the_line_and_the_address_is_the_same_on_again_and_after_a_restart() {
    let mut w = world(LIST, |_| {}).await;
    w.link.turn_on().unwrap();
    until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    let host = w.host();
    assert_eq!(w.fake.orders(), 1);

    w.link.turn_off();
    let s = w.link.status();
    assert_eq!(s.step(), Step::Off);
    assert!(s.address.is_none());
    // The relay lost the line, and a caller is turned away.
    eventually("the relay lost the line", || {
        w.env.relay.shared().lines.is_empty()
    })
    .await;
    assert!(w.assistant(&host).await.is_err());
    // The name is kept.
    assert_eq!(
        w.link
            .kept_address()
            .unwrap()
            .trim_start_matches("https://"),
        host
    );

    w.link.turn_on().unwrap();
    until(&w.link, "connected again", |s| s.step() == Step::Connected).await;
    assert_eq!(w.host(), host);
    assert!(w.assistant(&host).await.unwrap().ends_with(BODY));
    assert_eq!(
        w.fake.orders(),
        1,
        "a certificate already held is not ordered again"
    );
    // The name was claimed once.
    assert_eq!(
        w.env.relay.shared().metrics.claims.load(Ordering::Relaxed),
        1
    );

    // The app is closed and started again.
    w.link.turn_off();
    w.restart(|_| {});
    w.link.turn_on().unwrap();
    until(&w.link, "connected after a restart", |s| {
        s.step() == Step::Connected
    })
    .await;
    assert_eq!(w.host(), host);
    assert_eq!(w.fake.orders(), 1, "a restart orders nothing");
    assert_eq!(
        w.env.relay.shared().metrics.claims.load(Ordering::Relaxed),
        1
    );
    assert!(w.assistant(&host).await.unwrap().ends_with(BODY));
}

/// WB-link-1: a new address replaces the secret label and the old one
/// stops working at once; the name, the key and the certificate stay.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_address_stops_the_old_one() {
    let w = world(LIST, |_| {}).await;
    w.link.turn_on().unwrap();
    until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    let old = w.host();
    assert!(w.assistant(&old).await.is_ok());

    let new = w.link.new_address().unwrap();
    let new = new.trim_start_matches("https://").to_owned();
    assert_ne!(new, old);
    // Same name, a different label.
    assert_eq!(
        old.split_once('.').unwrap().1,
        new.split_once('.').unwrap().1
    );
    until(&w.link, "connected again", |s| s.step() == Step::Connected).await;
    assert_eq!(w.host(), new);
    assert!(
        w.assistant(&old).await.is_err(),
        "the old address no longer gets a certificate"
    );
    assert!(w.assistant(&new).await.unwrap().ends_with(BODY));
    assert_eq!(w.fake.orders(), 1, "the wildcard certificate still serves");
    // The apex and a made-up label get nothing either.
    let name = new.split_once('.').unwrap().1.to_owned();
    assert!(w.assistant(&name).await.is_err());
    assert!(
        w.assistant(&format!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.{name}"))
            .await
            .is_err()
    );
}

/// WB-link-2: the relay drops the line, and the app dials again by itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_line_comes_back_after_the_relay_drops_it() {
    let w = world(LIST, |_| {}).await;
    w.link.turn_on().unwrap();
    until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    let before = w.tap.accepted.load(Ordering::SeqCst);
    let host = w.host();

    w.tap.sever();
    eventually("the app dialled again", || {
        w.tap.accepted.load(Ordering::SeqCst) > before
    })
    .await;
    until(&w.link, "connected again", |s| {
        s.line_up && s.step() == Step::Connected
    })
    .await;
    eventually("the relay has the line", || {
        w.env.relay.shared().lines.len() == 1
    })
    .await;
    assert!(w.assistant(&host).await.unwrap().ends_with(BODY));
    // The name was not claimed again; the proof is the key.
    assert_eq!(
        w.env.relay.shared().metrics.claims.load(Ordering::Relaxed),
        1
    );
}

/// WB-link-2: pings keep an idle line. The relay drops a line silent for
/// its idle limit; this one outlives several of them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pings_keep_an_idle_line_and_silence_loses_it() {
    let mut limits = relay_limits();
    limits.line_idle_ms = 700;
    let w = world_with(limits, LIST, |t| {
        t.ping_every = Duration::from_millis(150);
    })
    .await;
    w.link.turn_on().unwrap();
    until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    let dialled = w.tap.accepted.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(
        w.tap.accepted.load(Ordering::SeqCst),
        dialled,
        "an idle but pinged line is never dropped and redialled"
    );
    assert!(w.link.status().line_up);
    // Without pings the same relay drops it.
    let silent = world_with(
        {
            let mut l = relay_limits();
            l.line_idle_ms = 500;
            l
        },
        LIST,
        |t| t.ping_every = Duration::from_secs(3600),
    )
    .await;
    silent.link.turn_on().unwrap();
    until(&silent.link, "connected", |s| s.step() == Step::Connected).await;
    let first = silent.tap.accepted.load(Ordering::SeqCst);
    eventually("the silent line was dropped and dialled again", || {
        silent.tap.accepted.load(Ordering::SeqCst) > first
    })
    .await;
}

fn relay_limits() -> relay::Limits {
    support::common::quick_limits()
}

/// WB-link-5: with the relay unreachable the state says so in words and
/// mends itself; the people at the window lose nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unreachable_relay_says_so_in_words_and_recovers_by_itself() {
    let w = world(LIST, |_| {}).await;
    w.tap.refuse.store(true, Ordering::SeqCst);
    w.link.turn_on().unwrap();
    let s = until(&w.link, "a trouble", |s| s.trouble.is_some()).await;
    assert_eq!(s.trouble, Some(Trouble::RelayDown));
    assert_eq!(s.step(), Step::GettingAddress);
    assert!(s.problem().unwrap().contains("keeps trying"));
    assert!(s.problem().unwrap().contains("on this computer still work"));
    // Several failed dials, spaced by backoff: not a hot loop.
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(w.tap.accepted.load(Ordering::SeqCst) < 25);
    // The network comes back.
    w.tap.refuse.store(false, Ordering::SeqCst);
    let s = until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    assert!(s.trouble.is_none() && s.cert_trouble.is_none());
}

/// WB-link-5: no internet at all (the relay's name does not resolve) is
/// said as that.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_internet_is_said_as_that() {
    let w = world(LIST, |_| {}).await;
    // Rebuild the link with a dial that cannot resolve.
    let mut cfg = config(&w);
    cfg.dial = Dial::Host("relay.nonexistent.invalid".into(), 443);
    cfg.dir = w.dir.path().join("link2");
    let link = ledger_link::Link::start_on(tokio::runtime::Handle::current(), cfg);
    link.turn_on().unwrap();
    let s = until(&link, "a trouble", |s| s.trouble.is_some()).await;
    assert_eq!(s.trouble, Some(Trouble::NoInternet));
    assert!(s.problem().unwrap().contains("can't reach the internet"));
    link.turn_off();
}

/// WB-link-5: a relay whose certificate is not valid yet or has ended
/// means this clock is wrong; it is said so, and the state keeps trying.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wrong_clock_is_said_as_that() {
    use rcgen::{CertificateParams, KeyPair};
    let w = world(LIST, |_| {}).await;
    // A TLS server with a certificate that ended yesterday.
    let mut params = CertificateParams::new(vec![RELAY_HOST.to_string()]).unwrap();
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(30);
    params.not_after = now - time::Duration::days(1);
    let key = KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    let chain = vec![cert.der().clone()];
    let pk = rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
        key.serialize_der(),
    ));
    let server = relay::tls::server_config(chain, pk).unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(server);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((s, _)) = listener.accept().await else {
                return;
            };
            let a = acceptor.clone();
            tokio::spawn(async move {
                let _ = a.accept(s).await;
            });
        }
    });
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.der().clone()).unwrap();
    let mut cfg = config(&w);
    cfg.dial = Dial::Addr(addr);
    cfg.relay_tls = relay::tls::client_config(roots).unwrap();
    cfg.dir = w.dir.path().join("link3");
    let link = ledger_link::Link::start_on(tokio::runtime::Handle::current(), cfg);
    link.turn_on().unwrap();
    let s = until(&link, "a trouble", |s| s.trouble.is_some()).await;
    assert_eq!(s.trouble, Some(Trouble::ClockWrong));
    assert!(s.problem().unwrap().contains("clock"));
    assert!(Trouble::ClockWrong.mends_itself());
    link.turn_off();
}

/// WB-link-1, 5: a relay that no longer knows the name says so in words,
/// and a new address (a whole new identity) claims a new name and carries on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_relay_that_lost_the_name_says_so_and_a_new_address_carries_on() {
    let mut w = world(LIST, |_| {}).await;
    // A second relay, with an empty registry, whose certificate this app
    // trusts too.
    let fresh = support::common::start_with(
        support::common::quick_limits(),
        relay::authorities::AuthorityList::from_toml(LIST).unwrap(),
    )
    .await;
    let mut roots = fresh.relay_ca.roots();
    roots
        .roots
        .extend(w.env.relay_ca.roots().roots.iter().cloned());
    let mut cfg = config(&w);
    cfg.relay_tls = relay::tls::client_config(roots).unwrap();
    w.link = ledger_link::Link::start_on(tokio::runtime::Handle::current(), cfg);
    w.link.set_door(Some(w.door.target.clone()));
    w.link.turn_on().unwrap();
    until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    let old = w.host();
    *w.tap.upstream.lock().unwrap() = fresh.router();
    w.tap.sever();
    let s = until(&w.link, "the name is lost", |s| {
        s.trouble == Some(Trouble::NameLost)
    })
    .await;
    assert!(s.problem().unwrap().contains("Get a new address"));
    let new = w.link.new_address().unwrap();
    assert_ne!(new.trim_start_matches("https://"), old);
    until(&w.link, "claimed a new name", |s| s.claimed && s.line_up).await;
    assert_eq!(
        fresh.relay.shared().metrics.claims.load(Ordering::Relaxed),
        1
    );
}

/// WB-link-4: the authorities are the relay's list, in its order; one
/// marked not usable is skipped; the first failing falls to the second;
/// the certificate is kept 0600 and reused; and the wildcard is for this
/// name and none other.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_first_authority_fails_and_the_second_issues() {
    let w = world(LIST, |_| {}).await;
    w.fake.state.lock().unwrap().fail.insert("first".into());
    w.link.turn_on().unwrap();
    until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    let orders: Vec<String> = w
        .fake
        .state
        .lock()
        .unwrap()
        .orders
        .iter()
        .map(|o| o.authority.clone())
        .collect();
    assert_eq!(orders, ["first", "second"]);
    let host = w.host();
    let name = host.split_once('.').unwrap().1;
    let id = w.fake.state.lock().unwrap().orders[1].identifier.clone();
    assert_eq!(id, format!("*.{name}"));
    // The relay's DNS holds no proof once the order is done.
    let proofs = dns_txt(
        w.env.relay.dns_udp_addr(),
        &format!("_acme-challenge.{name}."),
    )
    .await;
    assert!(proofs.is_empty(), "{proofs:?}");
}

/// WB-link-4: an authority that needs a binding the relay has none for is
/// listed and not usable: it is skipped, and the next is used.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_authority_that_is_not_usable_is_skipped() {
    const LIST_BINDING: &str = r#"
[[authority]]
id = "needs-binding"
directory = "https://fake.example/b"
caa_domain = "b.example"
needs_binding = true

[[authority]]
id = "plain"
directory = "https://fake.example/p"
caa_domain = "p.example"
"#;
    let w = world(LIST_BINDING, |_| {}).await;
    w.link.turn_on().unwrap();
    until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    let orders: Vec<String> = w
        .fake
        .state
        .lock()
        .unwrap()
        .orders
        .iter()
        .map(|o| o.authority.clone())
        .collect();
    assert_eq!(
        orders,
        ["plain"],
        "the one that is not usable was never asked"
    );
}

/// WB-link-4: every authority failing is a state in words that is retried
/// and mends itself; the line stays up meanwhile.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_authority_failing_is_said_and_retried_until_one_works() {
    let w = world(LIST, |_| {}).await;
    {
        let mut st = w.fake.state.lock().unwrap();
        st.fail.insert("first".into());
        st.fail.insert("second".into());
    }
    w.link.turn_on().unwrap();
    let s = until(&w.link, "a certificate trouble", |s| {
        s.cert_trouble.is_some()
    })
    .await;
    assert_eq!(s.step(), Step::GettingCertificate);
    assert!(s.line_up);
    let words = s.problem().unwrap();
    assert!(
        words.contains("could not get a certificate") && words.contains("keeps trying"),
        "{words}"
    );
    // Retried, with a wait.
    eventually("several tries", || w.fake.orders() >= 4).await;
    // The authority comes back.
    w.fake.state.lock().unwrap().fail.clear();
    let s = until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    assert!(s.cert_trouble.is_none());
}

/// WB-link-4: renewal ahead of expiry, at the time the authority's
/// renewal information suggests, naming the certificate it replaces; the
/// certificate that serves keeps serving while a renewal fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_certificate_is_renewed_at_the_authoritys_suggested_time() {
    let w = world(LIST, |_| {}).await;
    w.fake.state.lock().unwrap().window = Some((1, 2));
    w.link.turn_on().unwrap();
    let s = until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    let first_end = s.cert_expires.unwrap();
    assert_eq!(w.fake.orders(), 1);
    eventually("a renewal", || w.fake.orders() >= 2).await;
    {
        let st = w.fake.state.lock().unwrap();
        assert!(!st.orders[0].replaces);
        assert!(
            st.orders[1].replaces,
            "a renewal names the certificate it replaces"
        );
        assert_eq!(st.orders[0].authority, st.orders[1].authority);
    }
    until(&w.link, "the new certificate serves", |s| {
        s.cert_expires.is_some_and(|e| e >= first_end) && s.cert_trouble.is_none()
    })
    .await;
    assert!(w.assistant(&w.host()).await.unwrap().ends_with(BODY));

    // A renewal that fails leaves the one in use serving.
    {
        let mut st = w.fake.state.lock().unwrap();
        st.fail.insert("first".into());
        st.fail.insert("second".into());
        st.window = Some((1, 2));
    }
    let n = w.fake.orders();
    eventually("a failed renewal try", || w.fake.orders() > n + 1).await;
    let s = until(&w.link, "the renewal trouble", |s| s.cert_trouble.is_some()).await;
    assert!(s.cert_expires.is_some());
    assert!(
        s.cert_trouble
            .unwrap()
            .contains("the one in use still works")
    );
    assert!(w.assistant(&w.host()).await.unwrap().ends_with(BODY));
}

/// WB-link-4: with no renewal information a certificate is renewed two
/// thirds of the way through its life.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_renewal_information_it_renews_two_thirds_through() {
    let w = world(LIST, |_| {}).await;
    // The authority's certificates are good for an hour from now and began
    // an hour ago: two thirds of two hours is 20 minutes from now.
    w.link.turn_on().unwrap();
    until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    let cert: serde_json::Value =
        serde_json::from_slice(&std::fs::read(w.dir.path().join("link/cert.json")).unwrap())
            .unwrap();
    let (b, e, r) = (
        cert["not_before"].as_i64().unwrap(),
        cert["not_after"].as_i64().unwrap(),
        cert["renew_at"].as_i64().unwrap(),
    );
    assert_eq!(r, b + (e - b) * 2 / 3);
    assert_eq!(w.fake.orders(), 1);
}

/// WB-link-3: a caller that never finishes its handshake is dropped, and
/// a connection while the door is gone is closed, not held.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stalled_handshake_is_dropped_and_no_door_means_no_service() {
    let w = world(LIST, |t| t.handshake = Duration::from_millis(500)).await;
    w.link.turn_on().unwrap();
    until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    let host = w.host();
    // No door: the caller's connection ends.
    w.link.set_door(None);
    let r = w.assistant(&host).await;
    assert!(r.is_err() || r.unwrap().is_empty());
    w.link.set_door(Some(w.door.target.clone()));
    assert!(w.assistant(&host).await.unwrap().ends_with(BODY));
}

/// WB-link-4 (INVARIANTS 7 and 8 of certs.rs): the relay names the
/// authorities, so a certificate from one it made up is refused (the
/// program trusts the public roots, not the relay's list): nothing is
/// stored or served, the state says so, and a lifetime over the cap is
/// refused too.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_certificate_from_an_authority_the_relay_made_up_is_never_stored_or_served() {
    let mut w = world(LIST, |_| {}).await;
    // The real program's trust: the public roots. The fake authority is
    // not one of them.
    w.remake(|cfg| cfg.trust = CertTrust::public().unwrap());
    w.link.turn_on().unwrap();
    let s = until(&w.link, "the refusal is said", |s| {
        s.cert_trouble
            .as_deref()
            .is_some_and(|t| t.contains("public certificate authority"))
    })
    .await;
    assert_ne!(s.step(), Step::Connected);
    assert!(s.cert_expires.is_none());
    assert!(!w.dir.path().join("link/cert.json").exists());
    assert!(w.fake.orders() >= 1);
    // Nobody is served a certificate from it.
    assert!(w.assistant(&w.host()).await.is_err());
    w.link.turn_off();

    // A certificate from an authority the program does trust, but that
    // lives over 100 days, is refused as well.
    w.remake(|_| {});
    w.fake.state.lock().unwrap().lifetime = 101 * 86_400;
    w.link.turn_on().unwrap();
    let s = until(&w.link, "the lifetime is refused", |s| {
        s.cert_trouble
            .as_deref()
            .is_some_and(|t| t.contains("100 days"))
    })
    .await;
    assert!(s.cert_expires.is_none());
    assert!(!w.dir.path().join("link/cert.json").exists());
}

/// WB-link-4 (INVARIANT 7): a certificate kept on disk that the program
/// would not trust is discarded and ordered again, and "a new address"
/// does not bring it back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stored_certificate_from_a_made_up_authority_is_discarded_and_ordered_again() {
    let mut w = world(LIST, |_| {}).await;
    w.link.turn_on().unwrap();
    until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    assert_eq!(w.fake.orders(), 1);
    w.link.turn_off();
    // The file is there, from the authority the test trusts; a program that
    // trusts only the public roots reads it as no certificate at all.
    assert!(w.dir.path().join("link/cert.json").exists());
    w.remake(|cfg| cfg.trust = CertTrust::public().unwrap());
    w.link.turn_on().unwrap();
    until(&w.link, "it orders again", |s| {
        s.cert_trouble
            .as_deref()
            .is_some_and(|t| t.contains("public certificate authority"))
    })
    .await;
    assert!(w.fake.orders() >= 2, "the stored one was not believed");
    assert!(w.link.status().cert_expires.is_none());
    // A new address carries on the same way: nothing is loaded from the
    // file that fails.
    w.link.new_address().unwrap();
    until(&w.link, "the refusal again", |s| {
        s.cert_trouble.is_some() && s.line_up
    })
    .await;
    assert!(w.link.status().cert_expires.is_none());
    assert!(w.assistant(&w.host()).await.is_err());
}

/// WB-link-3: a first claim that finds the name taken picks another, and
/// the address, the certificate and the door all follow the one that was
/// claimed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_name_taken_on_the_first_claim_settles_before_the_certificate_and_the_address() {
    let w = world(LIST, |_| {}).await;
    // The identity is made, and somebody else (another key) takes its name
    // first.
    let dir = w.dir.path().join("link");
    let id = ledger_link::identity::Identity::load_or_create(&dir).unwrap();
    let taken = id.name();
    let (other, _) = relay::proto::AppKey::generate();
    let _held = w.env.connect(taken, &other, true).await.unwrap();

    w.link.turn_on().unwrap();
    w.link.wait_settled(Duration::from_secs(20)).unwrap();
    let settled = w.link.status();
    assert!(settled.claimed);
    let address = settled.address.clone().unwrap();
    let host = address.trim_start_matches("https://").to_owned();
    let name = host.split_once('.').unwrap().1.to_owned();
    assert_ne!(name, format!("{taken}.{DOMAIN}"), "another name was picked");

    until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    // Every certificate order was for the name that was claimed.
    {
        let st = w.fake.state.lock().unwrap();
        assert!(!st.orders.is_empty());
        for o in &st.orders {
            assert_eq!(o.identifier, format!("*.{name}"));
        }
    }
    assert_eq!(w.link.status().address.as_deref(), Some(address.as_str()));
    assert_eq!(w.link.kept_address().as_deref(), Some(address.as_str()));
    assert!(w.assistant(&host).await.unwrap().ends_with(BODY));
}

/// WB-link-4 (INVARIANT 9 of certs.rs): a certificate is not served after
/// its end, even when it cannot be renewed, and the state says it is gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_expired_certificate_is_not_served_and_the_state_says_so() {
    // Renewals that fail are tried again only after an hour: nothing but
    // the end of the certificate wakes the app.
    let w = world(LIST, |t| {
        t.cert_retry_min = Duration::from_secs(3600);
        t.cert_retry_max = Duration::from_secs(3600);
        t.ari_recheck = Duration::from_secs(3600);
    })
    .await;
    w.fake.state.lock().unwrap().lifetime = 3;
    w.link.turn_on().unwrap();
    until(&w.link, "connected", |s| s.step() == Step::Connected).await;
    let host = w.host();
    {
        let mut st = w.fake.state.lock().unwrap();
        st.fail.insert("first".into());
        st.fail.insert("second".into());
    }
    // Served while it lasts ...
    let early = w.link.status().cert_expires.unwrap();
    // ... and, once it has ended, neither served nor said to be in place.
    until(&w.link, "the certificate has ended", |s| {
        s.cert_expires.is_none()
    })
    .await;
    assert!(ledger_link::certs::now() >= early);
    assert_ne!(w.link.status().step(), Step::Connected);
    assert!(w.assistant(&host).await.is_err());
}

/// WB-link-5 (text.rs): what a certificate authority says to the app does
/// not reach the window unbounded.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_authority_that_shouts_is_shown_short_and_plain() {
    let w = world(LIST, |_| {}).await;
    {
        let mut st = w.fake.state.lock().unwrap();
        st.fail.insert("first".into());
        st.fail.insert("second".into());
        st.say = Some(format!("send money\u{7}\n\u{202e}{}", "now ".repeat(5000)));
    }
    w.link.turn_on().unwrap();
    let s = until(&w.link, "the trouble is said", |s| s.cert_trouble.is_some()).await;
    let words = s.problem().unwrap();
    assert!(
        words.chars().count() < 1200,
        "{} characters",
        words.chars().count()
    );
    assert!(
        !words.chars().any(|c| c.is_control() || c == '\u{202e}'),
        "{words:?}"
    );
    assert!(words.contains("send money"));
    assert!(words.starts_with("Wanderbooks could not get a certificate yet."));
}

/// The window asks for the kept address every frame: it is read from the
/// folder once, not each time (the key lives in the same file).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_kept_address_is_read_once_not_every_frame() {
    let w = world(LIST, |_| {}).await;
    assert!(w.link.kept_address().is_none(), "before the first start");
    w.link.turn_on().unwrap();
    until(&w.link, "claimed", |s| s.claimed).await;
    let first = w.link.kept_address().unwrap();
    w.link.turn_off();
    // With the file gone, the answer is still the one held: nothing reads
    // the folder again.
    for f in std::fs::read_dir(w.dir.path().join("link")).unwrap() {
        let f = f.unwrap().path();
        if f.is_file() {
            std::fs::remove_file(f).unwrap();
        }
    }
    assert_eq!(w.link.kept_address().as_deref(), Some(first.as_str()));
}
