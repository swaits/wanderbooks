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
//! SCENARIOS:
//!   1. An order with no stored certificate stores one and serves it; a
//!      second start reads it and orders nothing.
//!   2. The renewal time passes: a second order is made and replaces the
//!      first, naming the first as the one it replaces.
//!   3. The first authority fails and the second issues.
//!   4. A hello for another host name gets no certificate.
//! ```

use crate::private;
use futures::future::BoxFuture;
use relay::proto::Authority;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
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
    /// valid now and have a key that matches (INVARIANT 2).
    pub fn new(
        name: &str,
        authority: &str,
        issued: &Issued,
        identifier: &str,
        renew_at: Option<i64>,
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
        certified(&issued.chain_pem, &issued.key_pem)
            .ok_or("the certificate and its key do not go together")?;
        let renew_at = renew_at.unwrap_or_else(|| default_renewal(not_before, not_after));
        Ok(StoredCert {
            v: 1,
            name: name.to_owned(),
            authority: authority.to_owned(),
            chain_pem: issued.chain_pem.clone(),
            key_pem: issued.key_pem.to_string(),
            not_before,
            not_after,
            renew_at: renew_at.min(not_after - (not_after - not_before) / 10),
        })
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
    /// (INVARIANT 2). A file that does not read is as if there were none.
    pub fn load(&self, name: &str) -> Option<StoredCert> {
        let bytes = std::fs::read(&self.path).ok()?;
        let c: StoredCert = serde_json::from_slice(&bytes).ok()?;
        (c.v == 1 && c.name == name && c.certified().is_some()).then_some(c)
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
/// name and no other (INVARIANT 6).
pub struct Resolver {
    host: RwLock<String>,
    key: RwLock<Option<Arc<CertifiedKey>>>,
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

    /// The certificate to serve, or none.
    pub fn set_key(&self, key: Option<Arc<CertifiedKey>>) {
        *self.key.write().unwrap_or_else(|e| e.into_inner()) = key;
    }

    /// Whether a certificate is being served.
    pub fn has_key(&self) -> bool {
        self.key.read().unwrap_or_else(|e| e.into_inner()).is_some()
    }
}

impl ResolvesServerCert for Resolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let sni = hello.server_name()?.to_ascii_lowercase();
        let host = self.host.read().unwrap_or_else(|e| e.into_inner());
        if host.is_empty() || sni != *host {
            return None;
        }
        drop(host);
        self.key.read().unwrap_or_else(|e| e.into_inner()).clone()
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

    #[test]
    fn two_thirds_of_a_life() {
        assert_eq!(default_renewal(0, 90), 60);
        assert_eq!(default_renewal(100, 106), 104);
    }
}
