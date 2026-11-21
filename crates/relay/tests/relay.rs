//! The relay, end to end on loopback: real TLS, real lines, real DNS
//! packets. Nothing leaves 127.0.0.1.
#![allow(missing_docs)]

mod common;

use std::sync::atomic::Ordering;
use std::time::Duration;

use common::*;
use hickory_proto::op::{Message, Query, ResponseCode};
use hickory_proto::rr::{Name as DnsName, RData, RecordType};
use relay::Limits;
use relay::client::{self, ClientError};
use relay::proto::{self, AppKey, Challenge, Refusal, Verdict, read_frame, write_frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

const NAME: &str = "ledger-accrual-tally-mint";
const OTHER: &str = "ledger-accrual-tally-vault";

fn count(c: &std::sync::atomic::AtomicU64) -> u64 {
    c.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------- routing

/// WB-relay-1 scenario 1: an assistant reaches the app by name, TLS runs
/// end to end with the app's certificate (which the relay could not make,
/// and which the assistant trusts a different authority for), bytes go
/// both ways, and the app is told the caller's real address.
#[tokio::test]
async fn an_assistant_reaches_the_app_by_name_with_the_apps_own_certificate() {
    let env = start(quick_limits()).await;
    let (key, _) = AppKey::generate();
    let mut app = env.app(n(NAME), &key, true).await;
    assert!(!app.line.is_closed());

    let mut tls = env
        .assistant(&host_of(&n(NAME)))
        .await
        .expect("TLS completes with the app");
    let local = tls.get_ref().0.local_addr().unwrap();
    tls.write_all(b"hello app").await.unwrap();
    let mut buf = [0u8; 64];
    let got = tls.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..got], b"echo:hello app");
    // Both ways, more than once, and a large body in each direction.
    let big = vec![7u8; 200_000];
    let (mut rd, mut wr) = tokio::io::split(tls);
    let sender = tokio::spawn({
        let big = big.clone();
        async move {
            wr.write_all(&big).await.unwrap();
            wr
        }
    });
    // The app echoes each chunk it reads with a prefix, so at least all
    // of it comes back.
    let mut echoed = 0usize;
    while echoed < big.len() {
        let k = match tokio::time::timeout(Duration::from_secs(10), rd.read(&mut buf)).await {
            Ok(r) => r.unwrap(),
            Err(_) => panic!("stalled after {echoed} bytes echoed"),
        };
        assert!(k > 0);
        echoed += k;
    }
    let _wr = sender.await.unwrap();
    // The address the app was told is the one the assistant connected from.
    let seen = tokio::time::timeout(Duration::from_secs(5), app.callers.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(seen, local);
    assert_eq!(count(&env.relay.shared().metrics.routed), 1);
    env.relay.shutdown().await;
}

/// WB-relay-1 invariant 1: the certificate the assistant is shown is the
/// app's; the relay has no certificate an assistant trusts for the name,
/// so a relay that terminated TLS could not have completed this.
#[tokio::test]
async fn the_relay_holds_no_certificate_the_assistant_would_accept() {
    let env = start(quick_limits()).await;
    let (key, _) = AppKey::generate();
    let _app = env.app(n(NAME), &key, true).await;
    // An assistant that trusts only the relay's own authority is refused
    // by the app's certificate, which proves who answered.
    let tcp = TcpStream::connect(env.router()).await.unwrap();
    let relay_only =
        tokio_rustls::TlsConnector::from(relay::tls::client_config(env.relay_ca.roots()).unwrap());
    let host = rustls::pki_types::ServerName::try_from(host_of(&n(NAME))).unwrap();
    let err = relay_only
        .connect(host, tcp)
        .await
        .expect_err("the app's certificate is not the relay's authority's");
    assert!(
        err.to_string().to_lowercase().contains("unknown")
            || err.to_string().to_lowercase().contains("issuer"),
        "{err}"
    );
    env.relay.shutdown().await;
}

/// WB-relay-1 invariant 2 / scenario 2: unknown names and offline apps
/// are closed alike, as are bad shapes and no name at all.
#[tokio::test]
async fn unknown_names_offline_apps_and_odd_hosts_are_closed_the_same_way() {
    let env = start(quick_limits()).await;
    // Offline: claimed once, then the line closed.
    let (key, _) = AppKey::generate();
    let line = env.connect(n(OTHER), &key, true).await.unwrap();
    line.close().await;
    eventually("the line is gone", || env.relay.shared().lines.is_empty()).await;

    let unknown = host_of(&n(NAME));
    let offline = host_of(&n(OTHER));
    let mut errors = Vec::new();
    for host in [
        unknown.as_str(),
        offline.as_str(),
        "wanderbooks.link",
        &format!("short.{NAME}.wanderbooks.link"),
        &format!("{SECRET}.not-a-listed-name-here.wanderbooks.link"),
        &format!("{SECRET}.{NAME}.example.com"),
    ] {
        let e = env.assistant(host).await.expect_err(host);
        errors.push(format!("{e}"));
    }
    assert!(errors.iter().all(|e| e == &errors[0]), "{errors:?}");
    assert!(
        errors[0].to_lowercase().contains("unrecognis")
            || errors[0].to_lowercase().contains("unrecogniz"),
        "{}",
        errors[0]
    );
    let m = &env.relay.shared().metrics;
    assert_eq!(count(&m.unknown_or_offline), 6);
    assert_eq!(count(&m.routed), 0);

    // No server name at all (an IP address as the host): closed too.
    let tcp = TcpStream::connect(env.router()).await.unwrap();
    let ip_host = rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap();
    assert!(env.assistant_tls().connect(ip_host, tcp).await.is_err());
    env.relay.shutdown().await;
}

/// WB-relay-2 scenario: the owner reconnecting replaces the old line.
#[tokio::test]
async fn a_reconnect_with_the_right_key_replaces_the_old_line() {
    let env = start(quick_limits()).await;
    let (key, _) = AppKey::generate();
    let first = env.app(n(NAME), &key, true).await;
    let mut second = env.app(n(NAME), &key, false).await;
    eventually("the first line is closed", || first.line.is_closed()).await;
    assert_eq!(env.relay.shared().lines.len(), 1);
    let mut tls = env.assistant(&host_of(&n(NAME))).await.unwrap();
    tls.write_all(b"x").await.unwrap();
    let mut b = [0u8; 16];
    assert!(tls.read(&mut b).await.unwrap() > 0);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), second.callers.recv())
            .await
            .unwrap()
            .is_some()
    );
    env.relay.shutdown().await;
}

// ------------------------------------------------------------------ proof

/// WB-relay-2 / WB-relay-4: first claim wins; another key cannot take the
/// name, a reconnect needs the right key, and nothing is claimed by a
/// reconnect.
#[tokio::test]
async fn a_name_cannot_be_claimed_twice_and_a_wrong_key_is_refused() {
    let env = start(quick_limits()).await;
    let (a, _) = AppKey::generate();
    let (b, _) = AppKey::generate();
    let line = env.connect(n(NAME), &a, true).await.unwrap();
    assert!(line.verdict.claimed);

    let refusal = |r: Result<client::Line, ClientError>| match r {
        Err(ClientError::Refused(why)) => why,
        Err(other) => panic!("not a refusal: {other}"),
        Ok(_) => panic!("the line came up"),
    };
    assert_eq!(
        refusal(env.connect(n(NAME), &b, true).await),
        Refusal::Taken
    );
    assert_eq!(
        refusal(env.connect(n(NAME), &b, false).await),
        Refusal::BadProof
    );
    assert_eq!(
        refusal(env.connect(n(OTHER), &a, false).await),
        Refusal::UnknownName
    );
    // The owner still gets back in, without claiming again.
    let again = env.connect(n(NAME), &a, false).await.unwrap();
    assert!(!again.verdict.claimed);
    // And the same key claiming again is not "new".
    let third = env.connect(n(NAME), &a, true).await.unwrap();
    assert!(!third.verdict.claimed);
    assert_eq!(count(&env.relay.shared().metrics.claims), 1);
    assert_eq!(env.relay.shared().registry.count().await.unwrap(), 1);
    env.relay.shutdown().await;
}

/// WB-relay-2 invariant 1: a recorded proof is good for one session.
#[tokio::test]
async fn a_replayed_or_relayed_proof_is_refused() {
    let env = start(quick_limits()).await;
    let (key, _) = AppKey::generate();
    let name = n(NAME);
    // Session A: a real, accepted handshake, by hand.
    let mut a = client::tls_connect(env.router(), env.line_tls(), RELAY_HOST)
        .await
        .unwrap();
    let exporter_a = client::session_exporter(&a).unwrap();
    let ch: Challenge = read_frame(&mut a).await.unwrap();
    let nonce_a = proto::unhex::<32>(&ch.nonce).unwrap();
    let hello_a = client::make_hello(&key, &name, true, &exporter_a, &nonce_a);
    write_frame(&mut a, &hello_a).await.unwrap();
    let verdict: Verdict = read_frame(&mut a).await.unwrap();
    assert!(verdict.ok);

    // Session B replays session A's whole hello.
    let mut b = client::tls_connect(env.router(), env.line_tls(), RELAY_HOST)
        .await
        .unwrap();
    let exporter_b = client::session_exporter(&b).unwrap();
    assert_ne!(exporter_a, exporter_b);
    let ch_b: Challenge = read_frame(&mut b).await.unwrap();
    write_frame(&mut b, &hello_a).await.unwrap();
    let v: Verdict = read_frame(&mut b).await.unwrap();
    assert_eq!((v.ok, v.refusal), (false, Some(Refusal::BadProof)));

    // Session C: the right nonce, signed over another session's exporter
    // (what a relay in the middle of two sessions could do).
    let mut c = client::tls_connect(env.router(), env.line_tls(), RELAY_HOST)
        .await
        .unwrap();
    let ch_c: Challenge = read_frame(&mut c).await.unwrap();
    let nonce_c = proto::unhex::<32>(&ch_c.nonce).unwrap();
    let relayed = client::make_hello(&key, &name, false, &exporter_a, &nonce_c);
    write_frame(&mut c, &relayed).await.unwrap();
    let v: Verdict = read_frame(&mut c).await.unwrap();
    assert_eq!((v.ok, v.refusal), (false, Some(Refusal::BadProof)));

    // Session D: the right exporter, session B's old nonce.
    let mut d = client::tls_connect(env.router(), env.line_tls(), RELAY_HOST)
        .await
        .unwrap();
    let exporter_d = client::session_exporter(&d).unwrap();
    let _ch_d: Challenge = read_frame(&mut d).await.unwrap();
    let nonce_b = proto::unhex::<32>(&ch_b.nonce).unwrap();
    let stale = client::make_hello(&key, &name, false, &exporter_d, &nonce_b);
    write_frame(&mut d, &stale).await.unwrap();
    let v: Verdict = read_frame(&mut d).await.unwrap();
    assert_eq!((v.ok, v.refusal), (false, Some(Refusal::BadProof)));
    assert_eq!(count(&env.relay.shared().metrics.refused_proof), 3);
    drop(a);
    env.relay.shutdown().await;
}

/// A key that is not the name's, with a valid signature of its own, and
/// garbage in every field.
#[tokio::test]
async fn malformed_hellos_are_refused_without_touching_the_registry() {
    let env = start(quick_limits()).await;
    let (key, _) = AppKey::generate();
    let mut s = client::tls_connect(env.router(), env.line_tls(), RELAY_HOST)
        .await
        .unwrap();
    let ex = client::session_exporter(&s).unwrap();
    let ch: Challenge = read_frame(&mut s).await.unwrap();
    let nonce = proto::unhex::<32>(&ch.nonce).unwrap();
    let mut hello = client::make_hello(&key, &n(NAME), true, &ex, &nonce);
    hello.name = "relay".into(); // not four listed words
    write_frame(&mut s, &hello).await.unwrap();
    let v: Verdict = read_frame(&mut s).await.unwrap();
    assert_eq!(v.refusal, Some(Refusal::BadRequest));
    assert_eq!(env.relay.shared().registry.count().await.unwrap(), 0);

    // A frame that is not JSON ends the line without a verdict.
    let mut s = client::tls_connect(env.router(), env.line_tls(), RELAY_HOST)
        .await
        .unwrap();
    let _ch: Challenge = read_frame(&mut s).await.unwrap();
    s.write_all(&[0, 4, b'n', b'o', b'p', b'e']).await.unwrap();
    let mut rest = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut rest))
        .await
        .unwrap();
    // A frame longer than the limit does too.
    let mut s = client::tls_connect(env.router(), env.line_tls(), RELAY_HOST)
        .await
        .unwrap();
    let _ch: Challenge = read_frame(&mut s).await.unwrap();
    s.write_all(&[0xff, 0xff]).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut rest))
        .await
        .unwrap();
    assert_eq!(env.relay.shared().registry.count().await.unwrap(), 0);
    env.relay.shutdown().await;
}

/// WB-relay-4: a blocked name cannot connect; the limit on new names holds.
#[tokio::test]
async fn blocked_names_and_the_limit_on_new_names() {
    let limits = Limits {
        claims_per_window: 2,
        claims_per_ip: 100,
        ..Limits::default()
    };
    let env = start(limits).await;
    let (k1, _) = AppKey::generate();
    let (k2, _) = AppKey::generate();
    let (k3, _) = AppKey::generate();
    env.connect(n(NAME), &k1, true).await.unwrap();
    env.connect(n(OTHER), &k2, true).await.unwrap();
    match env
        .connect(n("ledger-accrual-tally-audit"), &k3, true)
        .await
    {
        Err(ClientError::Refused(Refusal::RateLimited)) => {}
        other => panic!("{:?}", other.err().map(|e| e.to_string())),
    }
    assert!(
        env.relay
            .shared()
            .registry
            .set_blocked(&n(NAME), true)
            .await
            .unwrap()
    );
    match env.connect(n(NAME), &k1, false).await {
        Err(ClientError::Refused(Refusal::Blocked)) => {}
        other => panic!("{:?}", other.err().map(|e| e.to_string())),
    }
    env.relay.shutdown().await;
}

/// The per-address limit on claims, in memory.
#[tokio::test]
async fn one_address_cannot_claim_many_names() {
    let limits = Limits {
        claims_per_window: 1000,
        claims_per_ip: 2,
        ..Limits::default()
    };
    let env = start(limits).await;
    let mut outcomes = Vec::new();
    for _ in 0..4 {
        let (k, _) = AppKey::generate();
        let r = env.connect(relay::names::Name::random(), &k, true).await;
        outcomes.push(match r {
            Ok(_) => "ok",
            Err(ClientError::Refused(Refusal::RateLimited)) => "limited",
            Err(_) => "error",
        });
    }
    assert_eq!(outcomes, ["ok", "ok", "limited", "limited"]);
    env.relay.shutdown().await;
}

// -------------------------------------------------------------- DNS + TXT

async fn dns(addr: std::net::SocketAddr, name: &str, ty: RecordType) -> Message {
    let mut q = Message::query();
    q.queries
        .push(Query::query(DnsName::from_ascii(name).unwrap(), ty));
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sock.send_to(&q.to_vec().unwrap(), addr).await.unwrap();
    let mut buf = [0u8; 2048];
    let (len, _) = tokio::time::timeout(Duration::from_secs(5), sock.recv_from(&mut buf))
        .await
        .expect("an answer")
        .unwrap();
    Message::from_vec(&buf[..len]).unwrap()
}

fn txt_values(m: &Message) -> Vec<String> {
    m.answers
        .iter()
        .filter_map(|r| match &r.data {
            RData::TXT(t) => Some(t.to_string()),
            _ => None,
        })
        .collect()
}

/// WB-relay-5 scenario: a proof record only for one's own name, proven by
/// the key, over the proven line; the request holds no name to forge.
#[tokio::test]
async fn a_proof_record_is_published_for_ones_own_name_and_no_other() {
    let env = start(quick_limits()).await;
    let (ka, _) = AppKey::generate();
    let (kb, _) = AppKey::generate();
    let a = env.connect(n(NAME), &ka, true).await.unwrap();
    let b = env.connect(n(OTHER), &kb, true).await.unwrap();
    let dns_addr = env.relay.dns_udp_addr();
    let q_a = format!("_acme-challenge.{NAME}.{DOMAIN}.");
    let q_b = format!("_acme-challenge.{OTHER}.{DOMAIN}.");

    assert!(txt_values(&dns(dns_addr, &q_a, RecordType::TXT).await).is_empty());
    let r = a.publish_txt(PROOF_A).await.unwrap();
    assert!(r.ok, "{r:?}");
    assert_eq!(
        txt_values(&dns(dns_addr, &q_a, RecordType::TXT).await),
        [PROOF_A]
    );
    // The other app's name has nothing, and the first cannot put anything there.
    assert!(txt_values(&dns(dns_addr, &q_b, RecordType::TXT).await).is_empty());
    b.publish_txt(PROOF_B).await.unwrap();
    assert_eq!(
        txt_values(&dns(dns_addr, &q_b, RecordType::TXT).await),
        [PROOF_B]
    );
    assert_eq!(
        txt_values(&dns(dns_addr, &q_a, RecordType::TXT).await),
        [PROOF_A]
    );

    // Values that are not DNS-01 proofs are refused; the record is untouched.
    for bad in [
        "",
        "short",
        &"x".repeat(44),
        "not base64url!!!not base64url!!!not base64url!!!",
    ] {
        let r = a.publish_txt(bad).await.unwrap();
        assert!(
            !r.ok && r.error.as_deref() == Some("bad_value"),
            "{bad:?}: {r:?}"
        );
    }
    assert_eq!(
        txt_values(&dns(dns_addr, &q_a, RecordType::TXT).await),
        [PROOF_A]
    );

    // Clearing removes only the app's own.
    let r = a.request(proto::Request::ClearTxt).await.unwrap();
    assert!(r.ok);
    assert!(txt_values(&dns(dns_addr, &q_a, RecordType::TXT).await).is_empty());
    assert_eq!(
        txt_values(&dns(dns_addr, &q_b, RecordType::TXT).await),
        [PROOF_B]
    );

    // The same record over TCP, and the apex and a wildcard-style name.
    let mut tcp = TcpStream::connect(env.relay.dns_tcp_addr()).await.unwrap();
    let mut q = Message::query();
    q.queries.push(Query::query(
        DnsName::from_ascii(&q_b).unwrap(),
        RecordType::TXT,
    ));
    let wire = q.to_vec().unwrap();
    let mut framed = u16::try_from(wire.len()).unwrap().to_be_bytes().to_vec();
    framed.extend(wire);
    tcp.write_all(&framed).await.unwrap();
    let mut len = [0u8; 2];
    tcp.read_exact(&mut len).await.unwrap();
    let mut body = vec![0u8; usize::from(u16::from_be_bytes(len))];
    tcp.read_exact(&mut body).await.unwrap();
    assert_eq!(txt_values(&Message::from_vec(&body).unwrap()), [PROOF_B]);

    let host = dns(
        dns_addr,
        &format!("{SECRET}.{NAME}.{DOMAIN}."),
        RecordType::A,
    )
    .await;
    assert!(
        matches!(host.answers.as_slice(), [r] if matches!(&r.data, RData::A(a) if a.to_string() == "198.51.100.7"))
    );
    let outside = dns(dns_addr, "example.com.", RecordType::A).await;
    assert_eq!(outside.metadata.response_code, ResponseCode::Refused);
    env.relay.shutdown().await;
}

/// WB-relay-5: only the line itself can ask; a connection that has not
/// proved a name cannot open a control stream (it has no line).
#[tokio::test]
async fn publishing_needs_a_proven_line() {
    let env = start(quick_limits()).await;
    let (key, _) = AppKey::generate();
    // Reach TLS and the challenge, then walk away without a proof.
    let mut s = client::tls_connect(env.router(), env.line_tls(), RELAY_HOST)
        .await
        .unwrap();
    let _ch: Challenge = read_frame(&mut s).await.unwrap();
    drop(s);
    // And a refused hello gives no line to publish on.
    let r = env.connect(n(NAME), &key, false).await;
    assert!(matches!(r, Err(ClientError::Refused(Refusal::UnknownName))));
    let dns_addr = env.relay.dns_udp_addr();
    let q = format!("_acme-challenge.{NAME}.{DOMAIN}.");
    assert!(txt_values(&dns(dns_addr, &q, RecordType::TXT).await).is_empty());
    env.relay.shutdown().await;
}

// ------------------------------------------------------------- authorities

const LIST: &str = r#"
[[authority]]
id = "letsencrypt"
directory = "http://127.0.0.1:14000/dir"

[[authority]]
id = "zerossl"
directory = "http://127.0.0.1:14001/dir"
needs_binding = true
binding_kid = "kid-1"
binding_hmac_key = "a2V5"
"#;

/// WB-relay-5 scenario: the list is served in its order, with bindings,
/// and an edit to the file changes what the next request gets, with no
/// change to the app.
#[tokio::test]
async fn the_authority_list_is_served_and_follows_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authorities.toml");
    std::fs::write(&path, LIST).unwrap();
    let list = relay::authorities::AuthorityList::from_file(path.clone()).unwrap();
    let env = start_with(quick_limits(), list).await;
    let (key, _) = AppKey::generate();
    let line = env.connect(n(NAME), &key, true).await.unwrap();

    let got = line.authorities().await.unwrap();
    let ids: Vec<_> = got.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["letsencrypt", "zerossl"]);
    assert!(!got[0].needs_binding && got[0].binding.is_none());
    assert!(got[1].needs_binding);
    assert_eq!(
        got[1].binding.as_ref().map(|b| b.kid.as_str()),
        Some("kid-1")
    );
    assert_eq!(got[0].directory, "http://127.0.0.1:14000/dir");

    // Swap the authorities: ZeroSSL first, a third added.
    let swapped = r#"
[[authority]]
id = "zerossl"
directory = "http://127.0.0.1:14001/dir"
needs_binding = true
binding_kid = "kid-2"
binding_hmac_key = "a2V5"

[[authority]]
id = "gts"
directory = "http://127.0.0.1:14002/dir"

[[authority]]
id = "letsencrypt"
directory = "http://127.0.0.1:14000/dir"
"#;
    std::fs::write(&path, swapped).unwrap();
    // The relay rereads on its half-minute tick; here, call it.
    let _ = env.relay.shared().authorities.reload();
    let got = line.authorities().await.unwrap();
    let ids: Vec<_> = got.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["zerossl", "gts", "letsencrypt"]);
    assert_eq!(
        got[0].binding.as_ref().map(|b| b.kid.as_str()),
        Some("kid-2")
    );
    env.relay.shutdown().await;
}

// ------------------------------------------------------------------ bounds

/// WB-relay-7: a half-open ClientHello, a silent peer and a slow drip all
/// time out; none holds anything past the hello timeout.
#[tokio::test]
async fn a_silent_or_dripping_peer_is_closed_at_the_hello_timeout() {
    let env = start(Limits {
        hello_timeout_ms: 400,
        ..quick_limits()
    })
    .await;
    // Silent.
    let mut silent = TcpStream::connect(env.router()).await.unwrap();
    // A drip: the start of a handshake record, one byte at a time.
    let mut drip = TcpStream::connect(env.router()).await.unwrap();
    let started = std::time::Instant::now();
    for b in [0x16u8, 3, 1, 0x01, 0x00, 1, 2, 3] {
        if drip.write_all(&[b]).await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut buf = [0u8; 8];
    let a = tokio::time::timeout(Duration::from_secs(5), silent.read(&mut buf))
        .await
        .expect("closed in time");
    assert!(matches!(a, Ok(0) | Err(_)));
    let b = tokio::time::timeout(Duration::from_secs(5), drip.read(&mut buf))
        .await
        .expect("closed in time");
    assert!(matches!(b, Ok(0) | Err(_)));
    assert!(started.elapsed() < Duration::from_secs(4));
    eventually("both counted", || {
        count(&env.relay.shared().metrics.hello_timeouts) == 2
    })
    .await;
    env.relay.shutdown().await;
}

/// WB-relay-1 / WB-relay-7: bytes that are not a TLS hello are closed
/// without a reply that says anything.
#[tokio::test]
async fn bytes_that_are_not_a_hello_are_closed() {
    let env = start(quick_limits()).await;
    for junk in [
        &b"GET / HTTP/1.1\r\nHost: x\r\n\r\n"[..],
        &[0x16, 3, 3, 0x40, 0x01][..],
        &[0u8; 64][..],
    ] {
        let mut s = TcpStream::connect(env.router()).await.unwrap();
        s.write_all(junk).await.unwrap();
        let mut rest = Vec::new();
        let r = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut rest)).await;
        assert!(r.is_ok(), "closed in time");
        assert!(rest.is_empty(), "{rest:?}");
    }
    eventually("counted", || {
        count(&env.relay.shared().metrics.bad_hello) == 3
    })
    .await;
    env.relay.shutdown().await;
}

/// WB-relay-7: one address may have only so many unfinished hellos.
#[tokio::test]
async fn one_address_cannot_hold_many_unfinished_hellos() {
    let env = start(Limits {
        max_pending_per_ip: 2,
        hello_timeout_ms: 5_000,
        ..quick_limits()
    })
    .await;
    let _a = TcpStream::connect(env.router()).await.unwrap();
    let _b = TcpStream::connect(env.router()).await.unwrap();
    eventually("two pending", || {
        count(&env.relay.shared().metrics.accepted) == 2
    })
    .await;
    let mut c = TcpStream::connect(env.router()).await.unwrap();
    let mut buf = [0u8; 4];
    let r = tokio::time::timeout(Duration::from_secs(2), c.read(&mut buf))
        .await
        .expect("closed at once, not at the timeout");
    assert!(matches!(r, Ok(0) | Err(_)));
    assert_eq!(count(&env.relay.shared().metrics.shed), 1);
    env.relay.shutdown().await;
}

/// WB-relay-7: a line handshake that stalls is dropped at its timeout.
#[tokio::test]
async fn a_line_handshake_that_never_finishes_is_dropped() {
    let env = start(Limits {
        handshake_timeout_ms: 400,
        ..quick_limits()
    })
    .await;
    let mut s = client::tls_connect(env.router(), env.line_tls(), RELAY_HOST)
        .await
        .unwrap();
    let _ch: Challenge = read_frame(&mut s).await.unwrap();
    // No hello. The relay closes the connection at the timeout.
    let mut rest = Vec::new();
    let r = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut rest)).await;
    assert!(r.is_ok(), "closed in time");
    env.relay.shutdown().await;
}

/// WB-relay-7: a silent line is dropped after its idle time, a line that
/// pings stays, and the routing table forgets the dropped one.
#[tokio::test]
async fn a_silent_line_is_dropped_and_a_pinging_one_is_not() {
    let env = start(Limits {
        line_idle_ms: 600,
        ..quick_limits()
    })
    .await;
    let (k1, _) = AppKey::generate();
    let (k2, _) = AppKey::generate();
    let silent = env.connect(n(NAME), &k1, true).await.unwrap();
    let chatty = env.connect(n(OTHER), &k2, true).await.unwrap();
    assert_eq!(env.relay.shared().lines.len(), 2);
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(chatty.request(proto::Request::Ping).await.unwrap().ok);
    }
    eventually("the silent line is gone", || silent.is_closed()).await;
    assert_eq!(env.relay.shared().lines.len(), 1);
    assert!(!chatty.is_closed());
    env.relay.shutdown().await;
}

/// WB-relay-7: streams per app are capped; a caller over it is closed
/// like any refusal and the app's other callers are untouched.
#[tokio::test]
async fn streams_per_app_are_capped() {
    let env = start(Limits {
        streams_per_app: 2,
        ..quick_limits()
    })
    .await;
    let (key, _) = AppKey::generate();
    let _app = env.app(n(NAME), &key, true).await;
    let host = host_of(&n(NAME));
    let mut held = Vec::new();
    for _ in 0..2 {
        let mut t = env.assistant(&host).await.unwrap();
        t.write_all(b"x").await.unwrap();
        let mut b = [0u8; 8];
        assert!(t.read(&mut b).await.unwrap() > 0);
        held.push(t);
    }
    assert!(
        env.assistant(&host).await.is_err(),
        "a third caller is over the cap"
    );
    assert_eq!(count(&env.relay.shared().metrics.app_busy), 1);
    // A closed caller frees its place.
    drop(held.pop());
    eventually("a place is free", || {
        env.relay.shared().lines.get(&n(NAME)).is_some()
    })
    .await;
    let mut ok = false;
    for _ in 0..40 {
        if env.assistant(&host).await.is_ok() {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "the place was never freed");
    env.relay.shutdown().await;
}

/// WB-relay-7: the number of lines is capped.
#[tokio::test]
async fn lines_are_capped() {
    let env = start(Limits {
        max_lines: 1,
        ..quick_limits()
    })
    .await;
    let (k1, _) = AppKey::generate();
    let (k2, _) = AppKey::generate();
    let _one = env.connect(n(NAME), &k1, true).await.unwrap();
    match env.connect(n(OTHER), &k2, true).await {
        Err(ClientError::Refused(Refusal::Busy)) => {}
        other => panic!("{:?}", other.err().map(|e| e.to_string())),
    }
    // The first app can still reconnect: that replaces, not adds.
    let _again = env.connect(n(NAME), &k1, false).await.unwrap();
    env.relay.shutdown().await;
}

/// WB-relay-7: a slow reader behind the relay back-pressures the caller;
/// the relay does not buffer for it, and ends the connection at the stall
/// limit. The caller tries to send 256 MiB and manages a small fraction.
#[tokio::test]
async fn a_slow_reader_back_pressures_and_is_cut_off_at_the_stall_limit() {
    let env = start(Limits {
        write_stall_ms: 800,
        ..quick_limits()
    })
    .await;
    let (key, _) = AppKey::generate();
    let mut line = env.connect(n(NAME), &key, true).await.unwrap();
    // The app takes the caller's stream and then never reads from it.
    let held = tokio::spawn(async move {
        let incoming = line.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(incoming);
        line
    });
    let host = host_of(&n(NAME));
    let mut tcp = TcpStream::connect(env.router()).await.unwrap();
    // Not TLS-complete (the app never answers): the caller sends its
    // hello and then a flood of "application data" bytes.
    let hello = common::client_hello_bytes(&host);
    tcp.write_all(&hello).await.unwrap();
    let chunk = vec![0xabu8; 64 * 1024];
    let started = std::time::Instant::now();
    let mut sent = 0usize;
    let ended = loop {
        match tokio::time::timeout(Duration::from_secs(10), tcp.write_all(&chunk)).await {
            Ok(Ok(())) => {
                sent += chunk.len();
                if sent >= 256 * 1024 * 1024 {
                    break "finished";
                }
            }
            Ok(Err(_)) => break "reset",
            Err(_) => break "blocked",
        }
    };
    let _ = started;
    assert_ne!(
        ended, "finished",
        "the relay took {sent} bytes for a reader that reads none"
    );
    assert!(sent < 32 * 1024 * 1024, "{sent} bytes were absorbed");
    eventually("the relay gave up on the stalled stream", || {
        // Routed and then closed: the stream's permit is back.
        count(&env.relay.shared().metrics.routed) == 1
    })
    .await;
    held.abort();
    env.relay.shutdown().await;
}

/// WB-relay-7: DNS over UDP is rate limited per address from a fixed table.
#[tokio::test]
async fn dns_over_udp_is_rate_limited_per_address() {
    let env = start(Limits {
        dns_qps_per_ip: 5,
        ..quick_limits()
    })
    .await;
    let addr = env.relay.dns_udp_addr();
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut q = Message::query();
    q.queries.push(Query::query(
        DnsName::from_ascii("wanderbooks.link.").unwrap(),
        RecordType::A,
    ));
    let wire = q.to_vec().unwrap();
    for _ in 0..200 {
        sock.send_to(&wire, addr).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut answered = 0;
    let mut buf = [0u8; 2048];
    while let Ok(Ok(_)) =
        tokio::time::timeout(Duration::from_millis(100), sock.recv_from(&mut buf)).await
    {
        answered += 1;
    }
    assert!((5..=30).contains(&answered), "{answered} of 200 answered");
    env.relay.shutdown().await;
}

/// WB-relay-7: TCP DNS is capped per address and a connection that says
/// nothing is closed.
#[tokio::test]
async fn dns_over_tcp_is_capped_and_times_out() {
    let env = start(Limits {
        dns_tcp_per_ip: 2,
        dns_tcp_timeout_ms: 400,
        ..quick_limits()
    })
    .await;
    let a = TcpStream::connect(env.relay.dns_tcp_addr()).await.unwrap();
    let b = TcpStream::connect(env.relay.dns_tcp_addr()).await.unwrap();
    let mut c = TcpStream::connect(env.relay.dns_tcp_addr()).await.unwrap();
    let mut buf = [0u8; 4];
    let r = tokio::time::timeout(Duration::from_secs(2), c.read(&mut buf))
        .await
        .expect("closed at once");
    assert!(matches!(r, Ok(0) | Err(_)));
    let mut a = a;
    let r = tokio::time::timeout(Duration::from_secs(3), a.read(&mut buf))
        .await
        .expect("closed at the timeout");
    assert!(matches!(r, Ok(0) | Err(_)));
    drop(b);
    env.relay.shutdown().await;
}

/// Shutting down closes the lines.
#[tokio::test]
async fn shutdown_closes_every_line() {
    let env = start(quick_limits()).await;
    let (key, _) = AppKey::generate();
    let line = env.connect(n(NAME), &key, true).await.unwrap();
    let shared = env.relay.shared().clone();
    env.relay.shutdown().await;
    eventually("the line ends", || line.is_closed()).await;
    assert!(shared.lines.get(&n(NAME)).is_none() || line.is_closed());
}
