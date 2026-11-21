//! Shared setup for the relay's tests: certificates, a relay on loopback,
//! a stand-in for the app, and a stand-in for an assistant. Everything
//! stays on 127.0.0.1.
#![allow(dead_code, missing_docs)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair, KeyUsagePurpose};
use relay::authorities::AuthorityList;
use relay::client::{self, Incoming, Line, LineOptions};
use relay::config::{Config, NameServer};
use relay::names::Name;
use relay::proto::AppKey;
use relay::{Limits, Relay};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
use rustls::{RootCertStore, ServerConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};

pub const DOMAIN: &str = "wanderbooks.link";
pub const RELAY_HOST: &str = "relay.wanderbooks.link";
pub const SECRET: &str = "k3j9x0q7m2z8c5v1b4n6a9s2d";
pub const PROOF_A: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";
pub const PROOF_B: &str = "ZYXWVUTSRQPONMLKJIHGFEDCBAzyxwvutsrq-_01234";

pub struct Ca {
    issuer: CertifiedIssuer<'static, KeyPair>,
}

impl Ca {
    pub fn new(cn: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, cn);
        let issuer = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
        Ca { issuer }
    }

    pub fn roots(&self) -> RootCertStore {
        let mut roots = RootCertStore::empty();
        roots.add(self.issuer.der().clone()).unwrap();
        roots
    }

    pub fn leaf(&self, names: &[&str]) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
        let key = KeyPair::generate().unwrap();
        let params =
            CertificateParams::new(names.iter().map(|s| s.to_string()).collect::<Vec<_>>())
                .unwrap();
        let cert = params.signed_by(&key, &self.issuer).unwrap();
        (
            vec![cert.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
    }

    pub fn pem(&self) -> String {
        self.issuer.pem()
    }
}

/// A relay on loopback with its own CA, and an app CA for the app's
/// certificate (so the relay's certificate is not one an assistant trusts).
pub struct Env {
    pub relay: Relay,
    pub relay_ca: Ca,
    pub app_ca: Ca,
    pub dir: tempfile::TempDir,
}

pub fn quick_limits() -> Limits {
    Limits {
        // The tests claim many names from one address.
        claims_per_window: 10_000,
        claims_per_ip: 10_000,
        ..Limits::default()
    }
}

pub async fn start(limits: Limits) -> Env {
    start_with(limits, AuthorityList::empty()).await
}

pub async fn start_with(limits: Limits, authorities: AuthorityList) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let relay_ca = Ca::new("relay test ca");
    let app_ca = Ca::new("app test ca");
    let (chain, key) = relay_ca.leaf(&[RELAY_HOST]);
    let tls = relay::tls::server_config(chain, key).unwrap();
    let cfg = Config {
        router_addr: "127.0.0.1:0".parse().unwrap(),
        dns_addr: "127.0.0.1:0".parse().unwrap(),
        registry_path: dir.path().join("registry.sqlite3"),
        address_v4: vec!["198.51.100.7".parse().unwrap()],
        nameservers: vec![NameServer {
            name: "ns1.wanderbooks.link".into(),
            v4: vec!["198.51.100.53".parse().unwrap()],
            v6: vec![],
        }],
        limits,
        ..Config::default()
    };
    let relay = Relay::start(cfg, tls, authorities).await.unwrap();
    Env {
        relay,
        relay_ca,
        app_ca,
        dir,
    }
}

impl Env {
    pub fn router(&self) -> SocketAddr {
        self.relay.router_addr()
    }

    pub fn line_tls(&self) -> Arc<rustls::ClientConfig> {
        relay::tls::client_config(self.relay_ca.roots()).unwrap()
    }

    pub async fn connect(
        &self,
        name: Name,
        key: &AppKey,
        claim: bool,
    ) -> Result<Line, client::ClientError> {
        client::connect(
            self.router(),
            self.line_tls(),
            RELAY_HOST,
            name,
            key,
            claim,
            LineOptions::default(),
        )
        .await
    }

    /// An app: claim (or reconnect) and serve callers with TLS from the
    /// app's own certificate, echoing what they say, prefixed.
    pub async fn app(&self, name: Name, key: &AppKey, claim: bool) -> App {
        let line = self
            .connect(name, key, claim)
            .await
            .expect("the line comes up");
        App::serve(line, name, &self.app_ca)
    }

    pub fn assistant_tls(&self) -> TlsConnector {
        TlsConnector::from(Arc::new(
            rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(self.app_ca.roots())
            .with_no_client_auth(),
        ))
    }

    /// An assistant: TCP to the router, TLS with this server name.
    pub async fn assistant(
        &self,
        host: &str,
    ) -> std::io::Result<tokio_rustls::client::TlsStream<TcpStream>> {
        let tcp = TcpStream::connect(self.router()).await?;
        self.assistant_tls()
            .connect(ServerName::try_from(host.to_owned()).unwrap(), tcp)
            .await
    }
}

pub fn host_of(name: &Name) -> String {
    format!("{SECRET}.{name}.{DOMAIN}")
}

pub fn n(words: &str) -> Name {
    Name::parse(words).unwrap()
}

/// The app's side once the line is up.
pub struct App {
    pub callers: tokio::sync::mpsc::UnboundedReceiver<std::net::SocketAddr>,
    pub line: relay::client::LineHandle,
    pub task: tokio::task::JoinHandle<()>,
}

impl App {
    fn serve(mut line: Line, name: Name, app_ca: &Ca) -> App {
        let (chain, key) = app_ca.leaf(&[&format!("*.{name}.{DOMAIN}")]);
        let mut cfg =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(chain, key)
                .unwrap();
        cfg.send_tls13_tickets = 0;
        let acceptor = TlsAcceptor::from(Arc::new(cfg));
        let (seen, callers) = tokio::sync::mpsc::unbounded_channel();
        let handle = line.handle();
        let task = tokio::spawn(async move {
            while let Some(Incoming { caller, stream }) = line.accept().await {
                let _ = seen.send(caller.source);
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(stream).await else {
                        return;
                    };
                    let mut buf = [0u8; 4096];
                    loop {
                        let Ok(n) = tls.read(&mut buf).await else {
                            return;
                        };

                        if n == 0 {
                            let _ = tls.shutdown().await;
                            return;
                        }
                        let mut out = b"echo:".to_vec();
                        out.extend_from_slice(&buf[..n]);
                        if tls.write_all(&out).await.is_err() || tls.flush().await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        App {
            callers,
            line: handle,
            task,
        }
    }
}

pub async fn eventually<F: FnMut() -> bool>(what: &str, mut f: F) {
    for _ in 0..200 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("never happened: {what}");
}

/// A real TLS 1.3 ClientHello for `host`, as bytes.
pub fn client_hello_bytes(host: &str) -> Vec<u8> {
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(RootCertStore::empty())
    .with_no_client_auth();
    let name = ServerName::try_from(host.to_owned()).unwrap();
    let mut conn = rustls::ClientConnection::new(Arc::new(cfg), name).unwrap();
    let mut out = Vec::new();
    conn.write_tls(&mut out).unwrap();
    out
}
