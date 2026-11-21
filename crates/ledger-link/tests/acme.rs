//! The ACME client against an in-process authority: the real
//! `instant-acme` client speaks RFC 8555 to a small authority written
//! here (it checks the proof in the relay's own DNS, signs the CSR it is
//! given with a CA of its own, and offers renewal information). No
//! network beyond 127.0.0.1: the "HTTP" is a function call.
#![allow(missing_docs)]

mod support;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use bytes::Bytes;
use futures::future::BoxFuture;
use http_body_util::BodyExt;
use instant_acme::{BodyWrapper, BytesResponse, Error, HttpClient};
use ledger_link::certs::{DnsProof, Issuer, OrderRequest, inspect, leaf_der};
use ledger_link::{AcmeIssuer, certs};
use rcgen::{CertificateSigningRequestParams, CertifiedIssuer, KeyPair};
use relay::proto::{Authority, Binding};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use support::common::{self, DOMAIN, n};

const BASE: &str = "https://acme.test";

#[derive(Default)]
struct Server {
    nonce: u64,
    accounts: HashMap<String, String>, // kid -> thumbprint
    new_accounts: Vec<serde_json::Value>,
    orders: Vec<Order>,
    ari_gets: u32,
    refuse_orders: bool,
    /// What a refusal says in its detail (else a short one).
    detail: Option<String>,
    certs: HashMap<usize, String>,
}

struct Order {
    identifier: String,
    replaces: Option<String>,
    thumbprint: String,
    token: String,
    authz_valid: bool,
    done: bool,
}

struct FakeAcme {
    ca: CertifiedIssuer<'static, KeyPair>,
    dns: std::net::SocketAddr,
    state: Mutex<Server>,
    window: (i64, i64),
}

fn jws(body: &[u8]) -> (serde_json::Value, serde_json::Value) {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap();
    let protected: serde_json::Value =
        serde_json::from_slice(&B64.decode(v["protected"].as_str().unwrap()).unwrap()).unwrap();
    let payload = match v["payload"].as_str().unwrap() {
        "" => serde_json::Value::Null,
        p => serde_json::from_slice(&B64.decode(p).unwrap()).unwrap(),
    };
    (protected, payload)
}

fn thumbprint(jwk: &serde_json::Value) -> String {
    let canon = format!(
        r#"{{"crv":{},"kty":{},"x":{},"y":{}}}"#,
        jwk["crv"], jwk["kty"], jwk["x"], jwk["y"]
    );
    B64.encode(Sha256::digest(canon.as_bytes()))
}

type Reply = (u16, Vec<(&'static str, String)>, Bytes);

fn json(status: u16, extra: Vec<(&'static str, String)>, v: serde_json::Value) -> Reply {
    let mut h = vec![("content-type", "application/json".to_string())];
    h.extend(extra);
    (status, h, Bytes::from(v.to_string()))
}

impl FakeAcme {
    fn roots(&self) -> rustls::RootCertStore {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(self.ca.der().clone()).unwrap();
        roots
    }

    async fn handle(&self, method: &str, path: &str, body: &[u8]) -> Reply {
        let nonce = {
            let mut s = self.state.lock().unwrap();
            s.nonce += 1;
            format!("nonce-{}", s.nonce)
        };
        let with_nonce = |mut r: Reply| {
            r.1.push(("replay-nonce", nonce.clone()));
            r
        };
        match (method, path) {
            ("GET", "/dir") => with_nonce(json(
                200,
                vec![],
                serde_json::json!({
                    "newNonce": format!("{BASE}/nonce"),
                    "newAccount": format!("{BASE}/acct"),
                    "newOrder": format!("{BASE}/order"),
                    "renewalInfo": format!("{BASE}/ari"),
                    "meta": {}
                }),
            )),
            (_, "/nonce") => with_nonce((200, vec![], Bytes::new())),
            ("POST", "/acct") => {
                let (protected, payload) = jws(body);
                let kid = format!("{BASE}/acct/1");
                let mut s = self.state.lock().unwrap();
                s.accounts
                    .insert(kid.clone(), thumbprint(&protected["jwk"]));
                s.new_accounts.push(payload);
                with_nonce(json(
                    201,
                    vec![("location", kid)],
                    serde_json::json!({"status":"valid"}),
                ))
            }
            ("POST", "/order") => {
                let (protected, payload) = jws(body);
                let mut s = self.state.lock().unwrap();
                if s.refuse_orders {
                    return with_nonce(json(
                        429,
                        vec![],
                        serde_json::json!({"type":"urn:ietf:params:acme:error:rateLimited","detail":s.detail.clone().unwrap_or_else(|| "too many certificates already issued".into())}),
                    ));
                }
                let thumb = s.accounts[protected["kid"].as_str().unwrap()].clone();
                let identifier = payload["identifiers"][0]["value"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let replaces = payload["replaces"].as_str().map(str::to_owned);
                let id = s.orders.len() + 1;
                s.orders.push(Order {
                    identifier,
                    replaces: replaces.clone(),
                    thumbprint: thumb,
                    token: format!("token-{id}"),
                    authz_valid: false,
                    done: false,
                });
                let mut order = serde_json::json!({
                    "status":"pending",
                    "identifiers": payload["identifiers"],
                    "authorizations":[format!("{BASE}/authz/{id}")],
                    "finalize": format!("{BASE}/finalize/{id}"),
                });
                if let Some(r) = replaces {
                    order["replaces"] = r.into();
                }
                with_nonce(json(
                    201,
                    vec![("location", format!("{BASE}/order/{id}"))],
                    order,
                ))
            }
            ("POST", p) if p.starts_with("/authz/") => {
                let id: usize = p[7..].parse().unwrap();
                let s = self.state.lock().unwrap();
                let o = &s.orders[id - 1];
                let ident = o.identifier.trim_start_matches("*.");
                with_nonce(json(
                    200,
                    vec![],
                    serde_json::json!({
                        "status": if o.authz_valid {"valid"} else {"pending"},
                        "identifier": {"type":"dns","value": ident},
                        "wildcard": o.identifier.starts_with("*."),
                        "challenges":[{"type":"dns-01","url":format!("{BASE}/chall/{id}"),
                                       "token":o.token,"status":"pending"}]
                    }),
                ))
            }
            ("POST", p) if p.starts_with("/chall/") => {
                let id: usize = p[7..].parse().unwrap();
                let (ident, expected) = {
                    let s = self.state.lock().unwrap();
                    let o = &s.orders[id - 1];
                    let ka = format!("{}.{}", o.token, o.thumbprint);
                    (
                        o.identifier.trim_start_matches("*.").to_owned(),
                        B64.encode(Sha256::digest(ka.as_bytes())),
                    )
                };
                let seen = support::dns_txt(self.dns, &format!("_acme-challenge.{ident}.")).await;
                let ok = seen.contains(&expected);
                self.state.lock().unwrap().orders[id - 1].authz_valid = ok;
                with_nonce(json(
                    200,
                    vec![],
                    serde_json::json!({"type":"dns-01","url":format!("{BASE}/chall/{id}"),
                        "token":format!("token-{id}"),"status": if ok {"valid"} else {"invalid"}}),
                ))
            }
            ("POST", p) if p.starts_with("/order/") => {
                let id: usize = p[7..].parse().unwrap();
                let s = self.state.lock().unwrap();
                let o = &s.orders[id - 1];
                let mut order = serde_json::json!({
                    "status": if o.done {"valid"} else if o.authz_valid {"ready"} else {"pending"},
                    "identifiers":[{"type":"dns","value":o.identifier}],
                    "authorizations":[format!("{BASE}/authz/{id}")],
                    "finalize": format!("{BASE}/finalize/{id}"),
                });
                if o.done {
                    order["certificate"] = format!("{BASE}/cert/{id}").into();
                }
                if let Some(r) = &o.replaces {
                    order["replaces"] = r.clone().into();
                }
                with_nonce(json(200, vec![], order))
            }
            ("POST", p) if p.starts_with("/finalize/") => {
                let id: usize = p[10..].parse().unwrap();
                let (_, payload) = jws(body);
                let csr = B64.decode(payload["csr"].as_str().unwrap()).unwrap();
                let der = rustls::pki_types::CertificateSigningRequestDer::from(csr);
                let mut params = CertificateSigningRequestParams::from_der(&der).unwrap();
                let now = time::OffsetDateTime::now_utc();
                params.params.not_before = now - time::Duration::hours(1);
                params.params.not_after = now + time::Duration::days(60);
                params.params.use_authority_key_identifier_extension = true;
                let cert = params.signed_by(&self.ca).unwrap();
                let mut s = self.state.lock().unwrap();
                s.orders[id - 1].done = true;
                s.certs
                    .insert(id, format!("{}{}", cert.pem(), self.ca.pem()));
                let o = &s.orders[id - 1];
                with_nonce(json(
                    200,
                    vec![],
                    serde_json::json!({
                        "status":"valid",
                        "identifiers":[{"type":"dns","value":o.identifier}],
                        "authorizations":[format!("{BASE}/authz/{id}")],
                        "finalize": format!("{BASE}/finalize/{id}"),
                        "certificate": format!("{BASE}/cert/{id}"),
                        "replaces": o.replaces,
                    }),
                ))
            }
            ("POST", p) if p.starts_with("/cert/") => {
                let id: usize = p[6..].parse().unwrap();
                let pem = self.state.lock().unwrap().certs[&id].clone();
                with_nonce((
                    200,
                    vec![("content-type", "application/pem-certificate-chain".into())],
                    Bytes::from(pem),
                ))
            }
            ("GET", p) if p.starts_with("/ari/") => {
                self.state.lock().unwrap().ari_gets += 1;
                let now = time::OffsetDateTime::now_utc();
                let at = |secs: i64| {
                    (now + time::Duration::seconds(secs))
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap()
                };
                json(
                    200,
                    vec![("retry-after", "3600".into())],
                    serde_json::json!({"suggestedWindow":{"start":at(self.window.0),"end":at(self.window.1)}}),
                )
            }
            _ => json(
                404,
                vec![],
                serde_json::json!({"type":"urn:ietf:params:acme:error:malformed","detail":"no such path"}),
            ),
        }
    }
}

struct Http(Arc<FakeAcme>);

impl HttpClient for Http {
    fn request(
        &self,
        req: http::Request<BodyWrapper<Bytes>>,
    ) -> Pin<Box<dyn Future<Output = Result<BytesResponse, Error>> + Send>> {
        let fake = self.0.clone();
        Box::pin(async move {
            let (parts, body) = req.into_parts();
            let bytes = body
                .collect()
                .await
                .map_err(|e| Error::Other(e.into()))?
                .to_bytes();
            let path = parts.uri.path().to_owned();
            assert_eq!(parts.uri.host(), Some("acme.test"));
            let (status, headers, out) = fake.handle(parts.method.as_str(), &path, &bytes).await;
            let mut b = http::Response::builder().status(status);
            for (k, v) in headers {
                b = b.header(k, v);
            }
            let (parts, _) = b.body(()).unwrap().into_parts();
            Ok(BytesResponse {
                parts,
                body: Box::new(out),
            })
        })
    }
}

struct Proof(relay::client::LineHandle);

impl DnsProof for Proof {
    fn publish<'a>(&'a self, value: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let r = self.0.publish_txt(value).await.map_err(|e| e.to_string())?;
            if r.ok {
                Ok(())
            } else {
                Err(r.error.unwrap_or_default())
            }
        })
    }
    fn clear(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let _ = self.0.request(relay::proto::Request::ClearTxt).await;
        })
    }
}

struct Rig {
    env: common::Env,
    acme: Arc<FakeAcme>,
    issuer: AcmeIssuer,
    line: relay::client::Line,
    dir: tempfile::TempDir,
    name: relay::names::Name,
}

async fn rig(window: (i64, i64)) -> Rig {
    let env = common::start(common::quick_limits()).await;
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
    let ca = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
    let acme = Arc::new(FakeAcme {
        ca,
        dns: env.relay.dns_udp_addr(),
        state: Mutex::new(Server::default()),
        window,
    });
    let dir = tempfile::tempdir().unwrap();
    let a2 = acme.clone();
    let issuer = AcmeIssuer::new(dir.path())
        .with_directory_hosts_for_tests(&["acme.test"])
        .with_http(Arc::new(move || {
            Some(Box::new(Http(a2.clone())) as Box<dyn HttpClient>)
        }));
    let (key, _) = relay::proto::AppKey::generate();
    let name = n("ledger-accrual-tally-mint");
    let line = env.connect(name, &key, true).await.unwrap();
    Rig {
        env,
        acme,
        issuer,
        line,
        dir,
        name,
    }
}

fn authority(binding: bool) -> Authority {
    Authority {
        id: "fake".into(),
        directory: format!("{BASE}/dir"),
        needs_binding: binding,
        usable: true,
        binding: binding.then(|| Binding {
            kid: "kid-1".into(),
            hmac_key: B64.encode(b"0123456789abcdef0123456789abcdef"),
        }),
    }
}

/// WB-link-4: account, order, the DNS proof through the relay, finalize
/// and download, in the real client; the key is made here; the account is
/// kept privately; the relay's DNS is left clean.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_order_goes_through_account_proof_finalize_and_download() {
    let r = rig((60, 120)).await;
    let proof = Proof(r.line.handle());
    let a = authority(false);
    let ident = format!("*.{}.{DOMAIN}", r.name);
    let issued = r
        .issuer
        .issue(OrderRequest {
            authority: &a,
            identifier: ident.clone(),
            replaces: None,
            dns: &proof,
        })
        .await
        .unwrap();
    let der = leaf_der(&issued.chain_pem).unwrap();
    let (nb, na, names) = inspect(&der).unwrap();
    assert_eq!(names, std::slice::from_ref(&ident));
    assert!(nb < certs::now() && na > certs::now());
    assert!(issued.key_pem.contains("PRIVATE KEY"));
    // The certificate and key go together (the store checks it).
    assert!(
        certs::StoredCert::new(
            "ledger-accrual-tally-mint",
            "fake",
            &issued,
            &ident,
            None,
            &certs::CertTrust::for_tests(r.acme.roots())
        )
        .is_ok()
    );

    // The account is kept in a private file, with no contact in it.
    let file = r.dir.path().join("acme/fake.json");
    assert!(file.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    {
        let st = r.acme.state.lock().unwrap();

        assert_eq!(st.new_accounts.len(), 1);
        assert!(
            st.new_accounts[0]["contact"]
                .as_array()
                .is_none_or(|c| c.is_empty())
        );
        assert_eq!(st.new_accounts[0]["termsOfServiceAgreed"], true);
    }
    // The relay holds no proof once the order is done.
    let left = support::dns_txt(
        r.env.relay.dns_udp_addr(),
        &format!("_acme-challenge.{}.{DOMAIN}.", r.name),
    )
    .await;
    assert!(left.is_empty(), "{left:?}");
}

/// WB-link-4: a second order reuses the account, a renewal names the
/// certificate it replaces, and the authority's renewal window is read.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_renewal_reuses_the_account_names_what_it_replaces_and_reads_the_window() {
    let r = rig((600, 900)).await;
    let proof = Proof(r.line.handle());
    let a = authority(false);
    let ident = format!("*.{}.{DOMAIN}", r.name);
    let first = r
        .issuer
        .issue(OrderRequest {
            authority: &a,
            identifier: ident.clone(),
            replaces: None,
            dns: &proof,
        })
        .await
        .unwrap();
    let second = r
        .issuer
        .issue(OrderRequest {
            authority: &a,
            identifier: ident.clone(),
            replaces: leaf_der(&first.chain_pem),
            dns: &proof,
        })
        .await
        .unwrap();
    assert_ne!(first.chain_pem, second.chain_pem);
    assert_ne!(
        *first.key_pem, *second.key_pem,
        "a fresh key for each certificate"
    );
    {
        let st = r.acme.state.lock().unwrap();
        assert_eq!(st.new_accounts.len(), 1, "the account was reused");
        assert!(st.orders[0].replaces.is_none());
        let id = st.orders[1].replaces.clone().unwrap();
        assert!(id.contains('.'), "an ARI certificate id: {id}");
    }
    let (s, e) = r
        .issuer
        .renewal_window(&a, &first.chain_pem)
        .await
        .expect("the authority offers a window");
    let now = certs::now();
    assert!(
        (now + 590..=now + 610).contains(&s) && (now + 890..=now + 910).contains(&e),
        "{s} {e} {now}"
    );
    assert_eq!(r.acme.state.lock().unwrap().ari_gets, 1);
}

/// WB-link-4: an authority that needs an account binding gets the one the
/// relay gave, and without one nothing is asked of it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_binding_is_sent_when_there_is_one_and_never_invented() {
    let r = rig((60, 120)).await;
    let proof = Proof(r.line.handle());
    let ident = format!("*.{}.{DOMAIN}", r.name);
    let mut no = authority(true);
    no.binding = None;
    no.usable = false;
    let err = r
        .issuer
        .issue(OrderRequest {
            authority: &no,
            identifier: ident.clone(),
            replaces: None,
            dns: &proof,
        })
        .await
        .err()
        .unwrap();
    assert!(err.contains("needs an account binding"), "{err}");
    assert!(r.acme.state.lock().unwrap().new_accounts.is_empty());

    let yes = authority(true);
    r.issuer
        .issue(OrderRequest {
            authority: &yes,
            identifier: ident,
            replaces: None,
            dns: &proof,
        })
        .await
        .unwrap();
    let st = r.acme.state.lock().unwrap();
    let eab = &st.new_accounts[0]["externalAccountBinding"];
    assert!(eab.is_object(), "{:?}", st.new_accounts);
    let protected: serde_json::Value =
        serde_json::from_slice(&B64.decode(eab["protected"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(protected["kid"], "kid-1");
}

/// WB-link-4: an authority's refusal comes back in its own words.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refusal_comes_back_in_the_authoritys_words() {
    let r = rig((60, 120)).await;
    r.acme.state.lock().unwrap().refuse_orders = true;
    let proof = Proof(r.line.handle());
    let err = r
        .issuer
        .issue(OrderRequest {
            authority: &authority(false),
            identifier: format!("*.{}.{DOMAIN}", r.name),
            replaces: None,
            dns: &proof,
        })
        .await
        .err()
        .unwrap();
    assert!(err.contains("too many certificates"), "{err}");
}

fn order_for(r: &Rig, authority: &Authority) -> OrderRequest<'static> {
    // The proof is never reached by the tests that use this: they stop at
    // the directory.
    struct Never;
    impl DnsProof for Never {
        fn publish<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async { Err("not reached".into()) })
        }
        fn clear(&self) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
    }
    static NEVER: Never = Never;
    OrderRequest {
        authority: Box::leak(Box::new(authority.clone())),
        identifier: format!("*.{}.{DOMAIN}", r.name),
        replaces: None,
        dns: &NEVER,
    }
}

/// WB-link-4 (invariant 6): the relay's list can name any directory, and
/// the app contacts only a host compiled into it: no request is made for
/// any other, however it is spelled.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_directory_on_a_host_the_app_does_not_know_is_refused_before_any_request() {
    let r = rig((60, 120)).await;
    let reached = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (a2, seen) = (r.acme.clone(), reached.clone());
    // The default list: no host of a test in it.
    let issuer = AcmeIssuer::new(r.dir.path()).with_http(Arc::new(move || {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Some(Box::new(Http(a2.clone())) as Box<dyn HttpClient>)
    }));
    for directory in [
        "https://acme.test/dir",
        "https://evil.example/directory",
        "http://acme-v02.api.letsencrypt.org/directory",
        "https://acme-v02.api.letsencrypt.org@evil.example/directory",
        "https://acme-v02.api.letsencrypt.org:8443/directory",
        "https://acme-v02.api.letsencrypt.org.evil.example/directory",
        "https://evil.example\\@acme-v02.api.letsencrypt.org/directory",
        "https://127.0.0.1:9/directory",
        "not an address",
    ] {
        let mut a = authority(false);
        a.directory = directory.into();
        let err = issuer.issue(order_for(&r, &a)).await.err().unwrap();
        assert!(err.contains("does not use it"), "{directory}: {err}");
    }
    assert_eq!(
        reached.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "not one request was made"
    );
    assert!(r.acme.state.lock().unwrap().new_accounts.is_empty());
}

/// WB-link-4 (invariant 6): an account kept for a directory that is not on
/// the list is not used; a new one is made at the authority named.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_kept_account_for_an_unlisted_directory_is_not_used() {
    let r = rig((60, 120)).await;
    let order = |r: &Rig| OrderRequest {
        authority: Box::leak(Box::new(authority(false))),
        identifier: format!("*.{}.{DOMAIN}", r.name),
        replaces: None,
        dns: Box::leak(Box::new(Proof(r.line.handle()))),
    };
    r.issuer.issue(order(&r)).await.unwrap();
    assert_eq!(r.acme.state.lock().unwrap().new_accounts.len(), 1);
    // The same account file, as if the relay had once had it made at another
    // host.
    let file = r.dir.path().join("acme/fake.json");
    let mut kept: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(kept["directory"], format!("{BASE}/dir"));
    kept["directory"] = "https://evil.example/dir".into();
    std::fs::write(&file, serde_json::to_vec(&kept).unwrap()).unwrap();
    r.issuer.issue(order(&r)).await.unwrap();
    assert_eq!(
        r.acme.state.lock().unwrap().new_accounts.len(),
        2,
        "the account kept for another host was left alone and a new one made"
    );
}

/// WB-link-5: what an authority says is a short, plain, quoted detail in
/// the app's own words, never its text as it came.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_the_authority_says_is_short_plain_and_quoted() {
    let r = rig((60, 120)).await;
    r.acme.state.lock().unwrap().refuse_orders = true;
    r.acme.state.lock().unwrap().detail = Some(format!(
        "pay now\u{7}\n\u{202e}{}",
        "give us your password ".repeat(500)
    ));
    let proof = Proof(r.line.handle());
    let err = r
        .issuer
        .issue(OrderRequest {
            authority: &authority(false),
            identifier: format!("*.{}.{DOMAIN}", r.name),
            replaces: None,
            dns: &proof,
        })
        .await
        .err()
        .unwrap();
    assert!(
        err.chars().count() < 260,
        "{} characters",
        err.chars().count()
    );
    assert!(
        !err.chars().any(|c| c.is_control() || c == '\u{202e}'),
        "{err:?}"
    );
    assert!(err.contains("too many certificates"), "{err}");
    assert!(err.contains("It said \u{201c}pay now give us"), "{err}");
    assert!(err.contains("\u{2026}\u{201d}"), "{err}");
}
