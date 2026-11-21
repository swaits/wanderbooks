//! The app's own certificate: kept on disk, served by rustls, ordered
//! from whichever authority the relay lists, renewed ahead of expiry.
//!
//! ```text
//! GOAL: the app terminates the web caller's TLS with a certificate whose
//!       key was made here and never left; the certificate survives a
//!       restart without a new order; it is renewed ahead of expiry (at
//!       the time the authority's renewal information suggests, else two
//!       thirds of the way through its life); and the authority is data:
//!       the relay's list, in order, so a swap needs no release.
//! SOURCE: WB-link-3 (TLS at the app), WB-link-4 (certificates and their
//!         renewal), D-80, D-87, plan p15-app-and-relay.md section 5
//!         ("Authorities are data, not code"); RFC 8555 (ACME), RFC 9773
//!         (renewal information).
//! INVARIANTS:
//!   1. The private key is made by the app's side of the order and is
//!      written only to the app's own private file (private.rs): it is
//!      never in a request to the relay, a status, a log or an error.
//!   2. The certificate stored is for exactly the wildcard under this
//!      name; one for another name, or one that does not parse, is
//!      ignored and ordered again.
//!   3. A stored certificate is read at start: a restart before its
//!      renewal time orders nothing.
//!   4. The authorities are tried in the relay's order; one marked not
//!      usable is skipped; a failure of one falls to the next; all failing
//!      is a state in words that is retried with a growing, jittered wait.
//!   5. The certificate that is serving keeps serving while a renewal
//!      fails, until it ends.
//!   6. The server name must be this app's host (`<secret>.<name>.<domain>`):
//!      a hello for any other name gets no certificate, so a stranger who
//!      knows the four words but not the label learns nothing and reaches
//!      nothing.
//!   7. A certificate is believed only if its chain ends at one of the
//!      public roots the program ships ([`CertTrust::public`]), for the
//!      app's wildcard, at the moment it is issued and at every load: the
//!      relay names the authorities, so a certificate from one it made up
//!      (a self-signed one valid for centuries) is refused, and one kept
//!      on disk that fails is ignored and ordered again. Only a test makes
//!      a [`CertTrust`] of its own roots; nothing at run time can.
//!   8. A lifetime over [`MAX_LIFETIME_DAYS`] days is refused, and the
//!      renewal is never later than two thirds of the life or
//!      [`MAX_RENEW_DAYS`] days into it, whichever is sooner.
//!   9. A certificate is never served at or after its `not_after`, even
//!      when a renewal is failing or the next look is hours away.
//! SCENARIOS:
//!   1. An order with no stored certificate stores one and serves it; a
//!      second start reads it and orders nothing.
//!   2. The renewal time passes: a second order is made and replaces the
//!      first, naming the first as the one it replaces.
//!   3. The first authority fails and the second issues.
//!   4. A hello for another host name gets no certificate.
//!   5. A certificate from an authority that is not in the trusted roots,
//!      one that lives over the cap, and one past its end are refused or
//!      not served (unit tests here, `tests/link.rs`).
//! ```

use crate::private;
use futures::future::BoxFuture;
use relay::proto::Authority;
use rustls::RootCertStore;
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::ServerCertVerifier;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

/// The relay publishes and clears the proof record of the app's name.
pub trait DnsProof: Send + Sync {
    /// Publish the DNS-01 proof value (the 43 characters) at
    /// `_acme-challenge.<name>`.
    fn publish<'a>(&'a self, value: &'a str) -> BoxFuture<'a, Result<(), String>>;
    /// Remove the app's proof records.
    fn clear(&self) -> BoxFuture<'_, ()>;
}

/// One order.
pub struct OrderRequest<'a> {
    /// The authority to order from.
    pub authority: &'a Authority,
    /// The one name asked for: `*.<four-words>.<domain>`.
    pub identifier: String,
    /// The certificate this one replaces (DER), if there is one still
    /// valid from this authority: renewal information names it.
    pub replaces: Option<Vec<u8>>,
    /// Where the proof goes.
    pub dns: &'a dyn DnsProof,
}

/// A certificate got from an authority.
pub struct Issued {
    /// The certificate and its chain, PEM, leaf first.
    pub chain_pem: String,
    /// The private key made for it, PEM. Never leaves the app.
    pub key_pem: Zeroizing<String>,
}

/// An authority's protocol, behind one small trait: everything that
/// differs between authorities beyond ACME itself (renewal information,
/// lifetimes, limits) is behind it, and the tests give it an in-process
/// authority.
pub trait Issuer: Send + Sync {
    /// Order a certificate (the whole of RFC 8555: account, order,
    /// DNS-01 proof through `req.dns`, finalize, download). An error is
    /// in words.
    fn issue<'a>(&'a self, req: OrderRequest<'a>) -> BoxFuture<'a, Result<Issued, String>>;

    /// The renewal window the authority suggests for this certificate
    /// (unix seconds, start and end), if it offers one.
    fn renewal_window<'a>(
        &'a self,
        authority: &'a Authority,
        chain_pem: &'a str,
    ) -> BoxFuture<'a, Option<(i64, i64)>>;
}

/// Seconds since the epoch.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// The longest a certificate may live (INVARIANT 8): public authorities
/// issue for 90 days or fewer, so a longer one is not from one.
pub const MAX_LIFETIME_DAYS: i64 = 100;

/// The latest, in days from the start of its life, a renewal is planned
/// (INVARIANT 8).
pub const MAX_RENEW_DAYS: i64 = 60;

const DAY: i64 = 86_400;

/// Which certificate authorities this app believes (INVARIANT 7): the
/// public roots the program ships. Not data, not the relay's to change.
#[derive(Clone)]
pub struct CertTrust {
    verifier: Arc<WebPkiServerVerifier>,
}

impl std::fmt::Debug for CertTrust {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertTrust").finish_non_exhaustive()
    }
}

impl CertTrust {
    /// The public roots the program ships (Mozilla's, as
    /// `webpki-root-certs`).
    pub fn public() -> Result<Self, String> {
        let mut roots = RootCertStore::empty();
        for der in webpki_root_certs::TLS_SERVER_ROOT_CERTS {
            // A root the parser can't read is skipped, not fatal.
            let _ = roots.add(der.clone());
        }
        Self::from_roots(roots)
    }

    /// Trust exactly these roots: for a test's own authority. Nothing in
    /// the program calls this; the relay cannot reach it (INVARIANT 7).
    pub fn for_tests(roots: RootCertStore) -> Self {
        Self::from_roots(roots)
            .unwrap_or_else(|why| panic!("a test's roots must make a verifier: {why}"))
    }

    fn from_roots(roots: RootCertStore) -> Result<Self, String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
            .build()
            .map_err(|_| "no trusted certificate authorities".to_string())?;
        Ok(CertTrust { verifier })
    }

    /// Whether `chain_pem` (leaf first) is a good chain, at time `t`, for
    /// the wildcard `identifier` (`*.<name>.<domain>`).
    pub fn check(&self, chain_pem: &str, identifier: &str, t: i64) -> Result<(), String> {
        let chain: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(chain_pem.as_bytes())
                .collect::<Result<_, _>>()
                .map_err(|_| "the certificate can't be read".to_string())?;
        let (leaf, rest) = chain
            .split_first()
            .ok_or("the authority sent no certificate")?;
        // A host the wildcard covers: the chain is checked for a name a
        // caller will really use.
        let host = identifier.replacen('*', "check", 1);
        let name = ServerName::try_from(host)
            .map_err(|_| "the certificate's name is not a host name".to_string())?;
        let at = UnixTime::since_unix_epoch(Duration::from_secs(u64::try_from(t).unwrap_or(0)));
        self.verifier
            .verify_server_cert(leaf, rest, &name, &[], at)
            .map(|_| ())
            .map_err(|_| {
                "the certificate does not come from a public certificate authority this program trusts"
                    .to_string()
            })
    }
}

/// The certificate and its key, as kept (one file, so one atomic write).
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredCert {
    v: u8,
    /// The four-word name it is for.
    pub name: String,
    /// The authority that issued it.
    pub authority: String,
    /// The chain, PEM.
    pub chain_pem: String,
    key_pem: String,
    /// Valid from (unix seconds).
    pub not_before: i64,
    /// Valid until (unix seconds).
    pub not_after: i64,
    /// When to order the next one (unix seconds).
    pub renew_at: i64,
}

impl std::fmt::Debug for StoredCert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the key (INVARIANT 1).
        f.debug_struct("StoredCert")
            .field("name", &self.name)
            .field("authority", &self.authority)
            .field("not_after", &self.not_after)
            .field("renew_at", &self.renew_at)
            .finish_non_exhaustive()
    }
}

const FILE: &str = "cert.json";

/// The first certificate of a chain, DER.
pub fn leaf_der(chain_pem: &str) -> Option<Vec<u8>> {
    CertificateDer::pem_slice_iter(chain_pem.as_bytes())
        .next()?
        .ok()
        .map(|c| c.as_ref().to_vec())
}

/// A certificate's validity (unix seconds) and its names.
pub fn inspect(der: &[u8]) -> Option<(i64, i64, Vec<String>)> {
    let (_, cert) = x509_parser::parse_x509_certificate(der).ok()?;
    let v = cert.validity();
    let names = cert
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|san| {
            san.value
                .general_names
                .iter()
                .filter_map(|n| match n {
                    x509_parser::extensions::GeneralName::DNSName(d) => Some((*d).to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    Some((v.not_before.timestamp(), v.not_after.timestamp(), names))
}

impl StoredCert {
    /// A certificate just issued, checked: it must be for `identifier`,
    /// valid now, chain to a trusted root, live no longer than the cap and
    /// have a key that matches (INVARIANTS 2, 7 and 8).
    pub fn new(
        name: &str,
        authority: &str,
        issued: &Issued,
        identifier: &str,
        renew_at: Option<i64>,
        trust: &CertTrust,
    ) -> Result<Self, String> {
        let der = leaf_der(&issued.chain_pem)
            .ok_or("the authority sent something that is not a certificate")?;
        let (not_before, not_after, names) =
            inspect(&der).ok_or("the authority sent a certificate that can't be read")?;
        if !names.iter().any(|n| n.eq_ignore_ascii_case(identifier)) {
            return Err("the authority sent a certificate for another name".into());
        }
        let t = now();
        if not_after <= t {
            return Err("the authority sent a certificate that has already ended".into());
        }
        let cap = renewal_cap(not_before, not_after);
        check_life(not_before, not_after)?;
        trust.check(&issued.chain_pem, identifier, t)?;
        certified(&issued.chain_pem, &issued.key_pem)
            .ok_or("the certificate and its key do not go together")?;
        let renew_at = renew_at.unwrap_or(cap).min(cap);
        Ok(StoredCert {
            v: 1,
            name: name.to_owned(),
            authority: authority.to_owned(),
            chain_pem: issued.chain_pem.clone(),
            key_pem: issued.key_pem.to_string(),
            not_before,
            not_after,
            renew_at,
        })
    }

    /// A stored certificate, checked again as it is read (INVARIANTS 2, 7,
    /// 8): the same tests as when it was issued, against the file's own
    /// words and the clock now. A renewal time set too late is brought in.
    fn vetted(mut self, name: &str, identifier: &str, trust: &CertTrust) -> Option<Self> {
        if self.v != 1 || self.name != name || self.certified().is_none() {
            return None;
        }
        let der = self.der()?;
        let (not_before, not_after, names) = inspect(&der)?;
        if !names.iter().any(|n| n.eq_ignore_ascii_case(identifier)) {
            return None;
        }
        check_life(not_before, not_after).ok()?;
        trust.check(&self.chain_pem, identifier, now()).ok()?;
        // What is in the certificate is what counts, not what the file says
        // about it.
        self.not_before = not_before;
        self.not_after = not_after;
        self.renew_at = self.renew_at.min(renewal_cap(not_before, not_after));
        Some(self)
    }

    /// The key and chain, ready for rustls.
    pub fn certified(&self) -> Option<Arc<CertifiedKey>> {
        certified(&self.chain_pem, &self.key_pem)
    }

    /// The certificate's DER (for renewal information).
    pub fn der(&self) -> Option<Vec<u8>> {
        leaf_der(&self.chain_pem)
    }

    /// Whether it still serves.
    pub fn valid_at(&self, t: i64) -> bool {
        self.not_after > t
    }
}

/// Two thirds of the way through its life: a third is left to renew in.
pub fn default_renewal(not_before: i64, not_after: i64) -> i64 {
    not_before + (not_after - not_before) * 2 / 3
}

/// The latest a renewal may be planned (INVARIANT 8): two thirds of the
/// life or [`MAX_RENEW_DAYS`] days in, whichever is sooner.
pub fn renewal_cap(not_before: i64, not_after: i64) -> i64 {
    default_renewal(not_before, not_after).min(not_before.saturating_add(MAX_RENEW_DAYS * DAY))
}

fn check_life(not_before: i64, not_after: i64) -> Result<(), String> {
    if not_after.saturating_sub(not_before) > MAX_LIFETIME_DAYS * DAY {
        return Err(format!(
            "the certificate would live longer than {MAX_LIFETIME_DAYS} days, which no public authority issues"
        ));
    }
    Ok(())
}

fn certified(chain_pem: &str, key_pem: &str) -> Option<Arc<CertifiedKey>> {
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(chain_pem.as_bytes())
        .collect::<Result<_, _>>()
        .ok()?;
    if chain.is_empty() {
        return None;
    }
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).ok()?;
    let provider = rustls::crypto::ring::default_provider();
    let signing = provider.key_provider.load_private_key(key).ok()?;
    let ck = CertifiedKey::new(chain, signing);
    ck.keys_match().ok()?;
    Some(Arc::new(ck))
}

/// The certificate on disk.
pub struct CertStore {
    path: PathBuf,
}

impl CertStore {
    /// The store in the link folder `dir`.
    pub fn new(dir: &Path) -> Self {
        CertStore {
            path: dir.join(FILE),
        }
    }

    /// The stored certificate for `name`, if there is a good one
    /// (INVARIANTS 2, 7, 8): one that fails any check is as if there were
    /// none, and is ordered again. A file that does not read is the same.
    pub fn load(&self, name: &str, identifier: &str, trust: &CertTrust) -> Option<StoredCert> {
        let bytes = std::fs::read(&self.path).ok()?;
        let c: StoredCert = serde_json::from_slice(&bytes).ok()?;
        c.vetted(name, identifier, trust)
    }

    /// Keep a certificate, privately and atomically.
    pub fn save(&self, c: &StoredCert) -> std::io::Result<()> {
        let text = serde_json::to_vec(c).map_err(std::io::Error::other)?;
        private::write(&self.path, &text)
    }

    /// Forget the certificate (a new identity).
    pub fn forget(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// What rustls serves: the current certificate, to the app's own host
/// name and no other (INVARIANT 6), and never at or after its end
/// (INVARIANT 9).
pub struct Resolver {
    host: RwLock<String>,
    key: RwLock<Option<(Arc<CertifiedKey>, i64)>>,
}

impl std::fmt::Debug for Resolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolver").finish_non_exhaustive()
    }
}

impl Resolver {
    /// A resolver with no certificate and no host.
    pub fn new() -> Arc<Self> {
        Arc::new(Resolver {
            host: RwLock::new(String::new()),
            key: RwLock::new(None),
        })
    }

    /// The one host name served.
    pub fn set_host(&self, host: &str) {
        *self.host.write().unwrap_or_else(|e| e.into_inner()) = host.to_ascii_lowercase();
    }

    /// The certificate to serve, with when it ends (unix seconds), or none.
    pub fn set_cert(&self, cert: Option<(Arc<CertifiedKey>, i64)>) {
        *self.key.write().unwrap_or_else(|e| e.into_inner()) = cert;
    }

    /// Whether a certificate is being served.
    pub fn has_key(&self) -> bool {
        self.key.read().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    /// The certificate for a hello naming `sni` at time `t`: only for the
    /// app's own host, and only while it lasts.
    pub fn pick(&self, sni: &str, t: i64) -> Option<Arc<CertifiedKey>> {
        let host = self.host.read().unwrap_or_else(|e| e.into_inner());
        if host.is_empty() || !sni.eq_ignore_ascii_case(&host) {
            return None;
        }
        drop(host);
        let held = self.key.read().unwrap_or_else(|e| e.into_inner());
        held.as_ref()
            .filter(|(_, not_after)| t < *not_after)
            .map(|(key, _)| key.clone())
    }
}

impl ResolvesServerCert for Resolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.pick(hello.server_name()?, now())
    }
}

/// The server settings the app terminates TLS with: TLS 1.3 and 1.2, the
/// resolver, and HTTP/2 or HTTP/1.1 by ALPN.
pub fn server_config(resolver: Arc<Resolver>) -> Arc<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap_or_else(|_| unreachable!("the ring provider has the default versions"))
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    // The line is the relay's; a ticket resumed there is one more thing
    // to hold for nothing.
    cfg.send_tls13_tickets = 0;
    Arc::new(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair};

    const IDENT: &str = "*.a-b-c-d.wanderbooks.link";

    fn ca() -> CertifiedIssuer<'static, KeyPair> {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap()
    }

    /// A certificate for `IDENT` from `ca`, living `days` days from an hour
    /// ago.
    fn issue(ca: &CertifiedIssuer<'static, KeyPair>, days: i64) -> Issued {
        let mut params = CertificateParams::new(vec![IDENT.to_string()]).unwrap();
        let t = time::OffsetDateTime::now_utc();
        params.not_before = t - time::Duration::hours(1);
        params.not_after = t + time::Duration::days(days);
        let key = KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, ca).unwrap();
        Issued {
            chain_pem: format!("{}{}", cert.pem(), ca.pem()),
            key_pem: Zeroizing::new(key.serialize_pem()),
        }
    }

    fn trusting(ca: &CertifiedIssuer<'static, KeyPair>) -> CertTrust {
        let mut roots = RootCertStore::empty();
        roots.add(ca.der().clone()).unwrap();
        CertTrust::for_tests(roots)
    }

    #[test]
    fn two_thirds_of_a_life() {
        assert_eq!(default_renewal(0, 90), 60);
        assert_eq!(default_renewal(100, 106), 104);
    }

    #[test]
    fn a_certificate_from_an_authority_the_program_trusts_is_kept() {
        let ca = ca();
        let c = StoredCert::new("a-b-c-d", "x", &issue(&ca, 60), IDENT, None, &trusting(&ca))
            .expect("a good certificate");
        assert!(c.valid_at(now()));
    }

    #[test]
    fn a_certificate_from_an_authority_the_relay_made_up_is_refused() {
        // The authority the relay listed signs for any name, for 8000 days
        // (and, separately, for 60): neither chains to a public root.
        let fake = ca();
        let public = CertTrust::public().unwrap();
        let why =
            StoredCert::new("a-b-c-d", "x", &issue(&fake, 60), IDENT, None, &public).unwrap_err();
        assert!(why.contains("public certificate authority"), "{why}");
        // Valid until the year 9999, as a hostile relay's would be.
        assert!(
            StoredCert::new("a-b-c-d", "x", &issue(&fake, 8000), IDENT, None, &public).is_err()
        );
        // A self-signed leaf with nothing above it is no better.
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(vec![IDENT.to_string()]).unwrap();
        params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(60);
        let own = params.self_signed(&key).unwrap();
        let issued = Issued {
            chain_pem: own.pem(),
            key_pem: Zeroizing::new(key.serialize_pem()),
        };
        assert!(StoredCert::new("a-b-c-d", "x", &issued, IDENT, None, &public).is_err());
    }

    #[test]
    fn a_lifetime_over_the_cap_is_refused() {
        let ca = ca();
        let trust = trusting(&ca);
        let why =
            StoredCert::new("a-b-c-d", "x", &issue(&ca, 101), IDENT, None, &trust).unwrap_err();
        assert!(why.contains("100 days"), "{why}");
        assert!(StoredCert::new("a-b-c-d", "x", &issue(&ca, 99), IDENT, None, &trust).is_ok());
    }

    #[test]
    fn renewal_is_never_later_than_two_thirds_or_sixty_days() {
        let ca = ca();
        let trust = trusting(&ca);
        // 99 days: two thirds is 66 days in, so sixty days is sooner.
        let c = StoredCert::new(
            "a-b-c-d",
            "x",
            &issue(&ca, 99),
            IDENT,
            Some(i64::MAX),
            &trust,
        )
        .unwrap();
        assert_eq!(c.renew_at, c.not_before + 60 * DAY);
        // 30 days: two thirds (20 days) is sooner.
        let c = StoredCert::new("a-b-c-d", "x", &issue(&ca, 30), IDENT, None, &trust).unwrap();
        assert_eq!(c.renew_at, default_renewal(c.not_before, c.not_after));
        // An earlier time the authority suggests is kept.
        let early = now() + 100;
        let c =
            StoredCert::new("a-b-c-d", "x", &issue(&ca, 30), IDENT, Some(early), &trust).unwrap();
        assert_eq!(c.renew_at, early);
    }

    #[test]
    fn a_stored_certificate_that_fails_is_ignored_when_read() {
        let ca = ca();
        let trust = trusting(&ca);
        let dir = tempfile::tempdir().unwrap();
        let store = CertStore::new(dir.path());
        let good = StoredCert::new("a-b-c-d", "x", &issue(&ca, 60), IDENT, None, &trust).unwrap();
        store.save(&good).unwrap();
        assert!(store.load("a-b-c-d", IDENT, &trust).is_some());
        // The same file, read by a program that does not trust that
        // authority (the relay's made-up one): ignored.
        let public = CertTrust::public().unwrap();
        assert!(store.load("a-b-c-d", IDENT, &public).is_none());
        // Another name: ignored.
        assert!(store.load("e-f-g-h", IDENT, &trust).is_none());
        // A file whose renewal time was set far out is brought in.
        let mut late = good.clone();
        late.renew_at = i64::MAX / 2;
        store.save(&late).unwrap();
        let read = store.load("a-b-c-d", IDENT, &trust).unwrap();
        assert!(read.renew_at <= read.not_before + 60 * DAY);
        // A certificate that lives over the cap, put on disk by hand.
        let long = issue(&ca, 400);
        let mut f = good;
        f.chain_pem = long.chain_pem;
        f.key_pem = long.key_pem.to_string();
        store.save(&f).unwrap();
        assert!(store.load("a-b-c-d", IDENT, &trust).is_none());
    }

    #[test]
    fn the_resolver_serves_only_its_host_and_only_until_the_certificate_ends() {
        let ca = ca();
        let c =
            StoredCert::new("a-b-c-d", "x", &issue(&ca, 30), IDENT, None, &trusting(&ca)).unwrap();
        let r = Resolver::new();
        r.set_host("Secret.a-b-c-d.wanderbooks.link");
        r.set_cert(c.certified().map(|k| (k, c.not_after)));
        let host = "secret.a-b-c-d.wanderbooks.link";
        assert!(r.pick(host, c.not_after - 1).is_some());
        assert!(r.pick(host, c.not_after).is_none(), "at its end");
        assert!(
            r.pick(host, c.not_after + 3600 * 6).is_none(),
            "hours after"
        );
        assert!(r.pick("other.a-b-c-d.wanderbooks.link", 0).is_none());
    }
}
