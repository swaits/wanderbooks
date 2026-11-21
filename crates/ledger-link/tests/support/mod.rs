//! Shared setup for the link's tests: an in-process relay (the relay
//! crate's own test setup), an in-process certificate authority, a stand-in
//! for the remote door, a tap between the app and the relay, and an
//! assistant. Everything stays on 127.0.0.1.
#![allow(dead_code, missing_docs)]

#[path = "../../../relay/tests/common/mod.rs"]
pub mod common;

use futures::future::BoxFuture;
use hickory_proto::op::{Message, Query};
use hickory_proto::rr::{Name as DnsName, RData, RecordType};
use ledger_link::certs::{Issued, Issuer, OrderRequest};
use ledger_link::{CertTrust, Dial, DoorTarget, Link, LinkConfig, LinkStatus, Timing};
use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair, KeyUsagePurpose};
use relay::proto::{Authority, Caller};
use rustls::RootCertStore;
use rustls::pki_types::ServerName;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_rustls::TlsConnector;
use zeroize::Zeroizing;

pub use common::{DOMAIN, RELAY_HOST};

/// The words the fake authority speaks through.
pub struct Order {
    pub authority: String,
    pub identifier: String,
    pub replaces: bool,
    pub had_binding: bool,
}

pub struct FakeState {
    pub orders: Vec<Order>,
    /// Authority ids that refuse.
    pub fail: std::collections::HashSet<String>,
    /// Seconds the certificate lasts from now.
    pub lifetime: i64,
    /// What a refusal says, when it is not the fixed words.
    pub say: Option<String>,
    /// The renewal window the authority suggests, as seconds from now.
    pub window: Option<(i64, i64)>,
}

/// An in-process certificate authority: it asks the relay's own DNS for
/// the proof, as a real one would, and signs with a CA of its own.
pub struct Fake {
    ca: CertifiedIssuer<'static, KeyPair>,
    dns: SocketAddr,
    pub state: Mutex<FakeState>,
}

pub fn txt_at(m: &Message) -> Vec<String> {
    m.answers
        .iter()
        .filter_map(|r| match &r.data {
            RData::TXT(t) => Some(t.to_string()),
            _ => None,
        })
        .collect()
}

pub async fn dns_txt(addr: SocketAddr, name: &str) -> Vec<String> {
    let mut q = Message::query();
    q.queries.push(Query::query(
        DnsName::from_ascii(name).unwrap(),
        RecordType::TXT,
    ));
    let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sock.send_to(&q.to_vec().unwrap(), addr).await.unwrap();
    let mut buf = [0u8; 2048];
    let (len, _) = tokio::time::timeout(Duration::from_secs(5), sock.recv_from(&mut buf))
        .await
        .expect("an answer")
        .unwrap();
    txt_at(&Message::from_vec(&buf[..len]).unwrap())
}

impl Fake {
    pub fn new(dns: SocketAddr) -> Arc<Self> {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "fake authority");
        let ca = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
        Arc::new(Fake {
            ca,
            dns,
            state: Mutex::new(FakeState {
                orders: vec![],
                fail: Default::default(),
                lifetime: 3600,
                say: None,
                window: None,
            }),
        })
    }

    pub fn roots(&self) -> RootCertStore {
        let mut roots = RootCertStore::empty();
        roots.add(self.ca.der().clone()).unwrap();
        roots
    }

    pub fn orders(&self) -> usize {
        self.state.lock().unwrap().orders.len()
    }
}

fn proof_value() -> String {
    // 43 base64url characters, as a DNS-01 value is.
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let r = relay::proto::random32();
    (0..43)
        .map(|i| char::from(alphabet[usize::from(r[i % 32] % 64)]))
        .collect()
}

impl Issuer for Fake {
    fn issue<'a>(&'a self, req: OrderRequest<'a>) -> BoxFuture<'a, Result<Issued, String>> {
        Box::pin(async move {
            let (fails, lifetime, say) = {
                let mut st = self.state.lock().unwrap();
                st.orders.push(Order {
                    authority: req.authority.id.clone(),
                    identifier: req.identifier.clone(),
                    replaces: req.replaces.is_some(),
                    had_binding: req.authority.binding.is_some(),
                });
                (
                    st.fail.contains(&req.authority.id),
                    st.lifetime,
                    st.say.clone(),
                )
            };
            if fails {
                return Err(say.unwrap_or_else(|| "the fake authority says no".into()));
            }
            // The proof goes to the relay, and the authority reads it from
            // the relay's own DNS, as a real one does.
            let value = proof_value();
            req.dns.publish(&value).await?;
            let name = req.identifier.trim_start_matches("*.");
            let seen = dns_txt(self.dns, &format!("_acme-challenge.{name}.")).await;
            req.dns.clear().await;
            if !seen.contains(&value) {
                return Err("the proof of the name was not in the DNS".into());
            }
            let mut params = CertificateParams::new(vec![req.identifier.clone()]).unwrap();
            let now = time::OffsetDateTime::now_utc();
            params.not_before = now - time::Duration::hours(1);
            params.not_after = now + time::Duration::seconds(lifetime);
            let key = KeyPair::generate().unwrap();
            let cert = params.signed_by(&key, &self.ca).unwrap();
            Ok(Issued {
                chain_pem: format!("{}{}", cert.pem(), self.ca.pem()),
                key_pem: Zeroizing::new(key.serialize_pem()),
            })
        })
    }

    fn renewal_window<'a>(
        &'a self,
        _authority: &'a Authority,
        _chain_pem: &'a str,
    ) -> BoxFuture<'a, Option<(i64, i64)>> {
        Box::pin(async move {
            let w = self.state.lock().unwrap().window;
            let now = ledger_link::certs::now();
            w.map(|(a, b)| (now + a, now + b))
        })
    }
}

/// What crosses between two ends, recorded, and cut on request.
pub struct Tap {
    pub addr: SocketAddr,
    pub upstream: Arc<Mutex<SocketAddr>>,
    pub accepted: Arc<AtomicUsize>,
    pub refuse: Arc<AtomicBool>,
    pub to_upstream: Arc<Mutex<Vec<u8>>>,
    pub from_upstream: Arc<Mutex<Vec<u8>>>,
    cut: watch::Sender<u64>,
    /// The address the relay sees connections from.
    pub source_ports: Arc<Mutex<Vec<SocketAddr>>>,
}

impl Tap {
    pub async fn new(upstream: SocketAddr) -> Tap {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let tap = Tap {
            addr,
            upstream: Arc::new(Mutex::new(upstream)),
            accepted: Arc::new(AtomicUsize::new(0)),
            refuse: Arc::new(AtomicBool::new(false)),
            to_upstream: Arc::new(Mutex::new(vec![])),
            from_upstream: Arc::new(Mutex::new(vec![])),
            cut: watch::channel(0).0,
            source_ports: Arc::new(Mutex::new(vec![])),
        };
        let (up, accepted, refuse, to, from, cut, ports) = (
            tap.upstream.clone(),
            tap.accepted.clone(),
            tap.refuse.clone(),
            tap.to_upstream.clone(),
            tap.from_upstream.clone(),
            tap.cut.clone(),
            tap.source_ports.clone(),
        );
        tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    return;
                };
                accepted.fetch_add(1, Ordering::SeqCst);
                if refuse.load(Ordering::SeqCst) {
                    continue; // dropped at once
                }
                let target = *up.lock().unwrap();
                let (to, from, ports) = (to.clone(), from.clone(), ports.clone());
                let mut watcher = cut.subscribe();
                tokio::spawn(async move {
                    let Ok(server) = TcpStream::connect(target).await else {
                        return;
                    };
                    if let Ok(local) = server.local_addr() {
                        ports.lock().unwrap().push(local);
                    }
                    let (mut cr, mut cw) = client.into_split();
                    let (mut sr, mut sw) = server.into_split();
                    let a = async {
                        let mut buf = [0u8; 8192];
                        loop {
                            let n = cr.read(&mut buf).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            to.lock().unwrap().extend_from_slice(&buf[..n]);
                            if sw.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                        let _ = sw.shutdown().await;
                    };
                    let b = async {
                        let mut buf = [0u8; 8192];
                        loop {
                            let n = sr.read(&mut buf).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            from.lock().unwrap().extend_from_slice(&buf[..n]);
                            if cw.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                        let _ = cw.shutdown().await;
                    };
                    tokio::select! {
                        () = async { tokio::join!(a, b); } => {}
                        _ = watcher.changed() => {}
                    }
                });
            }
        });
        tap
    }

    /// Cut every connection now (the network dropped them).
    pub fn sever(&self) {
        self.cut.send_modify(|n| *n += 1);
    }

    pub fn seen_up(&self) -> Vec<u8> {
        self.to_upstream.lock().unwrap().clone()
    }

    pub fn seen_down(&self) -> Vec<u8> {
        self.from_upstream.lock().unwrap().clone()
    }
}

/// What the stand-in door was given.
#[derive(Clone, Debug)]
pub struct Seen {
    pub secret_ok: bool,
    pub caller: Option<SocketAddr>,
    pub request: String,
}

/// A stand-in for the remote door: it wants the secret, then the PROXY
/// header, then plain HTTP, and answers a fixed body.
pub struct Door {
    pub target: DoorTarget,
    pub seen: Arc<Mutex<Vec<Seen>>>,
}

pub const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub const BODY: &str = "the door says hello";

impl Door {
    pub async fn new() -> Door {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(vec![]));
        let record = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    return;
                };
                let record = record.clone();
                tokio::spawn(async move {
                    let mut given = [0u8; 64];
                    if s.read_exact(&mut given).await.is_err() {
                        return;
                    }
                    let secret_ok = given.as_slice() == SECRET.as_bytes();
                    let caller = Caller::read(&mut s).await.ok().map(|c| c.source);
                    let mut req = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = s.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        req.extend_from_slice(&buf[..n]);
                    }
                    record.lock().unwrap().push(Seen {
                        secret_ok,
                        caller,
                        request: String::from_utf8_lossy(&req).into_owned(),
                    });
                    if secret_ok {
                        let _ = s
                            .write_all(
                                format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
                                    BODY.len()
                                )
                                .as_bytes(),
                            )
                            .await;
                    }
                });
            }
        });
        Door {
            target: DoorTarget::new(addr, SECRET).unwrap(),
            seen,
        }
    }
}

/// The timings the tests run at.
pub fn fast() -> Timing {
    Timing {
        ping_every: Duration::from_millis(200),
        ping_timeout: Duration::from_secs(2),
        tick: Duration::from_millis(100),
        connect_timeout: Duration::from_secs(3),
        backoff_min: Duration::from_millis(50),
        backoff_max: Duration::from_millis(400),
        handshake: Duration::from_secs(3),
        cert_retry_min: Duration::from_millis(100),
        cert_retry_max: Duration::from_millis(500),
        order_timeout: Duration::from_secs(10),
        ari_recheck: Duration::from_secs(1),
        wake_gap: Duration::from_secs(30),
        line_stable: Duration::from_millis(300),
        redial_spread: Duration::from_millis(200),
    }
}

pub const LIST: &str = r#"
[[authority]]
id = "first"
directory = "https://fake.example/first"
caa_domain = "first.example"

[[authority]]
id = "second"
directory = "https://fake.example/second"
caa_domain = "second.example"
"#;

pub struct World {
    pub env: common::Env,
    pub tap: Tap,
    pub caller_tap: Tap,
    pub door: Door,
    pub fake: Arc<Fake>,
    pub dir: tempfile::TempDir,
    pub link: Link,
}

pub fn config(w: &World) -> LinkConfig {
    LinkConfig {
        dir: w.dir.path().join("link"),
        domain: DOMAIN.into(),
        relay_host: RELAY_HOST.into(),
        dial: Dial::Addr(w.tap.addr),
        relay_tls: w.env.line_tls(),
        issuer: w.fake.clone(),
        trust: CertTrust::for_tests(w.fake.roots()),
        timing: fast(),
    }
}

pub async fn world(list: &str, timing: impl FnOnce(&mut Timing)) -> World {
    world_with(common::quick_limits(), list, timing).await
}

pub async fn world_with(
    limits: relay::Limits,
    list: &str,
    timing: impl FnOnce(&mut Timing),
) -> World {
    let authorities = relay::authorities::AuthorityList::from_toml(list).unwrap();
    let env = common::start_with(limits, authorities).await;
    let tap = Tap::new(env.router()).await;
    let caller_tap = Tap::new(env.router()).await;
    let door = Door::new().await;
    let fake = Fake::new(env.relay.dns_udp_addr());
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = LinkConfig {
        dir: dir.path().join("link"),
        domain: DOMAIN.into(),
        relay_host: RELAY_HOST.into(),
        dial: Dial::Addr(tap.addr),
        relay_tls: env.line_tls(),
        issuer: fake.clone(),
        trust: CertTrust::for_tests(fake.roots()),
        timing: fast(),
    };
    timing(&mut cfg.timing);
    let link = Link::start_on(tokio::runtime::Handle::current(), cfg);
    link.set_door(Some(door.target.clone()));
    World {
        env,
        tap,
        caller_tap,
        door,
        fake,
        dir,
        link,
    }
}

impl World {
    /// The link made again with `change` applied to its setup (the same
    /// folder, relay and authority).
    pub fn remake(&mut self, change: impl FnOnce(&mut LinkConfig)) {
        let mut cfg = config(self);
        change(&mut cfg);
        let old = std::mem::replace(
            &mut self.link,
            Link::start_on(tokio::runtime::Handle::current(), cfg),
        );
        drop(old);
        self.link.set_door(Some(self.door.target.clone()));
    }

    /// A second link on the same folder (the app was restarted).
    pub fn restart(&mut self, timing: impl FnOnce(&mut Timing)) {
        let mut cfg = config(self);
        timing(&mut cfg.timing);
        let old = std::mem::replace(
            &mut self.link,
            Link::start_on(tokio::runtime::Handle::current(), cfg),
        );
        drop(old);
        self.link.set_door(Some(self.door.target.clone()));
    }

    /// An assistant: TLS to `host` through the tap, one GET, the whole
    /// reply.
    pub async fn assistant(&self, host: &str) -> std::io::Result<String> {
        let tcp = TcpStream::connect(self.tap_for_callers()).await?;
        let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(self.fake.roots())
        .with_no_client_auth();
        let mut tls = tokio::time::timeout(
            Duration::from_secs(5),
            TlsConnector::from(Arc::new(cfg))
                .connect(ServerName::try_from(host.to_owned()).unwrap(), tcp),
        )
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "the handshake never finished")
        })??;
        tls.write_all(
            format!("GET /ping HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await?;
        let mut out = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(10), tls.read_to_end(&mut out)).await;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn tap_for_callers(&self) -> SocketAddr {
        // Callers reach the relay's router the way the internet does: not
        // through the app's tap, but through one of their own, which is
        // also where what crosses is recorded.
        self.caller_tap.addr
    }

    pub fn host(&self) -> String {
        self.link
            .status()
            .address
            .or_else(|| self.link.kept_address())
            .unwrap()
            .trim_start_matches("https://")
            .to_owned()
    }
}

pub async fn until(link: &Link, what: &str, mut ok: impl FnMut(&LinkStatus) -> bool) -> LinkStatus {
    for _ in 0..400 {
        let s = link.status();
        if ok(&s) {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("never: {what}; last status {:?}", link.status());
}

pub async fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..400 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("never: {what}");
}
