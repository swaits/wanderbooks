//! The ACME client: an [`Issuer`] over `instant-acme` (RFC 8555, DNS-01,
//! wildcard, and RFC 9773 renewal information).
//!
//! ```text
//! GOAL: order a wildcard certificate for the app's own name from
//!       whichever authority the relay lists, proving the name through
//!       the relay's DNS and nowhere else, keeping the account and
//!       certificate keys on this machine.
//! SOURCE: WB-link-4 (certificates), D-87; RFC 8555 (ACME; DNS-01 in
//!         section 8.4; external account binding in section 7.3.4),
//!         RFC 9773 (ARI).
//! INVARIANTS:
//!   1. One account per authority, made on first use and kept in a
//!      private file (private.rs); its contact list is empty: the app
//!      gives an authority no name, address or email of the person.
//!   2. The certificate key is made by the client library for each order
//!      and returned to the caller; it is not sent anywhere.
//!   3. Only the DNS-01 challenge is used, and only its value is sent to
//!      the relay (which publishes it for the app's own name); the relay
//!      never orders.
//!   4. An authority that needs an account binding gets the credential
//!      the relay gave this app; none is invented or reused across
//!      authorities.
//!   5. Every failure comes back in the app's own words, chosen from the
//!      kind of failure; what the authority said is only a short quoted
//!      detail at the end (text.rs), never a bare code and never the
//!      authority's text on its own.
//!   6. The app sends a request only to an authority's directory on a host
//!      from [`DIRECTORY_HOSTS`], which is compiled in: the relay chooses
//!      which of them and in what order, but cannot name a new host, so it
//!      cannot make the app send requests to any server it likes. An
//!      authority on another host, over http, with a port or with a name
//!      and password in its address, is skipped and said so. A kept
//!      account is only used if the directory it was made at is on the list
//!      too. Adding a brand-new authority takes a new release of the app;
//!      that is the price of the relay not being trusted with the choice of
//!      where the app sends its requests (plan p15-app-and-relay.md 5).
//! SCENARIOS:
//!   1. Against an in-process authority (`tests/acme.rs`): account,
//!      order, proof, finalize and download work; the second order reuses
//!      the account; a renewal names the certificate it replaces; the
//!      authority's renewal window is read.
//!   2. An authority on a host that is not in the list is refused before
//!      any request; the hosts in the list are accepted; the authority's
//!      words are bounded and plain (unit tests here).
//! ```

use crate::certs::{DnsProof, Issued, Issuer, OrderRequest, leaf_der};
use crate::private;
use crate::text::detail;
use base64::Engine as _;
use futures::future::BoxFuture;
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, CertificateIdentifier, ChallengeType,
    ExternalAccountKey, HttpClient, Identifier, NewAccount, NewOrder, OrderStatus, RetryPolicy,
};
use relay::proto::Authority;
use rustls::pki_types::CertificateDer;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use zeroize::Zeroizing;

/// How to make the HTTP client an account speaks through. The default is
/// the platform's roots; tests give one that reaches an in-process
/// authority.
pub type HttpFactory = Arc<dyn Fn() -> Option<Box<dyn HttpClient>> + Send + Sync>;

/// The hosts an authority's ACME directory may be on (INVARIANT 6), each
/// from the authority's own documentation. The relay's list says which of
/// these to use; a host that is not here is never contacted.
pub const DIRECTORY_HOSTS: &[(&str, &str)] = &[
    // https://letsencrypt.org/docs/acme-protocol-updates/
    ("acme-v02.api.letsencrypt.org", "Let's Encrypt"),
    // https://letsencrypt.org/docs/staging-environment/
    (
        "acme-staging-v02.api.letsencrypt.org",
        "Let's Encrypt (staging)",
    ),
    // https://cloud.google.com/certificate-manager/docs/public-ca-tutorial
    ("dv.acme-v02.api.pki.goog", "Google Trust Services"),
    (
        "dv.acme-v02.test-api.pki.goog",
        "Google Trust Services (test)",
    ),
    // https://zerossl.com/documentation/acme/
    ("acme.zerossl.com", "ZeroSSL"),
    // https://www.ssl.com/guide/ssl-tls-certificate-issuance-and-revocation-with-acme/
    ("acme.ssl.com", "SSL.com"),
];

/// The ACME client.
pub struct AcmeIssuer {
    dir: PathBuf,
    hosts: Vec<String>,
    http: HttpFactory,
    /// How long an order may take to be ready or issued.
    patience: Duration,
}

impl AcmeIssuer {
    /// A client keeping its accounts in `dir` (the link folder).
    pub fn new(dir: &Path) -> Self {
        AcmeIssuer {
            dir: dir.join("acme"),
            hosts: DIRECTORY_HOSTS
                .iter()
                .map(|(h, _)| (*h).to_owned())
                .collect(),
            http: Arc::new(|| None),
            patience: Duration::from_secs(120),
        }
    }

    /// Accept directories on these hosts instead of [`DIRECTORY_HOSTS`]
    /// (tests: a stand-in authority on localhost). Nothing in the program
    /// calls this; the relay cannot reach it (INVARIANT 6).
    pub fn with_directory_hosts_for_tests(mut self, hosts: &[&str]) -> Self {
        self.hosts = hosts.iter().map(|h| (*h).to_ascii_lowercase()).collect();
        self
    }

    /// Speak through `http` (tests).
    pub fn with_http(mut self, http: HttpFactory) -> Self {
        self.http = http;
        self
    }

    /// Wait this long for an order (tests).
    pub fn with_patience(mut self, patience: Duration) -> Self {
        self.patience = patience;
        self
    }

    /// Whether `url` is a directory the app may contact (INVARIANT 6): https,
    /// on a listed host, with no port other than 443 and no name or
    /// password in front. It is read with the parser the ACME library
    /// reads it with, so the two agree on what the host is.
    fn directory_allowed(&self, url: &str) -> Result<(), String> {
        let refused = |why: &str| {
            format!(
                "The directory {} is {why}, so this version of Wanderbooks does not use it.",
                detail(url)
            )
        };
        if url
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
        {
            return Err(refused("not a plain address"));
        }
        let uri: http::Uri = url.parse().map_err(|_| refused("not an address"))?;
        if uri.scheme_str() != Some("https") {
            return Err(refused("not an https address"));
        }
        let authority = uri.authority().ok_or_else(|| refused("not an address"))?;
        if authority.as_str().contains('@') {
            return Err(refused("an address with a name in front"));
        }
        if authority.port_u16().is_some_and(|p| p != 443) {
            return Err(refused("on an unusual port"));
        }
        let host = authority.host().to_ascii_lowercase();
        if self.hosts.contains(&host) {
            Ok(())
        } else {
            Err(refused("on a host it does not know"))
        }
    }

    fn builder(&self) -> Result<instant_acme::AccountBuilder, String> {
        match (self.http)() {
            Some(http) => Ok(Account::builder_with_http(http)),
            None => Account::builder().map_err(|e| words(Kind::Start, &e)),
        }
    }

    fn file(&self, authority: &Authority) -> PathBuf {
        // The id is the relay's own short word; keep only what is safe in
        // a file name.
        let safe: String = authority
            .id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{safe}.json"))
    }

    /// The account at this authority: the kept one, or a new one
    /// (INVARIANTS 1 and 4).
    async fn account(&self, authority: &Authority) -> Result<Account, String> {
        self.directory_allowed(&authority.directory)?;
        let file = self.file(authority);
        if let Ok(bytes) = std::fs::read(&file) {
            let bytes = Zeroizing::new(bytes);
            // A kept account is used only for a directory on the list: what
            // is in the file is what the library will contact.
            let kept_at_a_listed_host = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .is_some_and(|v| {
                    v.get("urls").is_none_or(serde_json::Value::is_null)
                        && v.get("directory")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|d| self.directory_allowed(d).is_ok())
                });
            if kept_at_a_listed_host
                && let Ok(credentials) = serde_json::from_slice::<AccountCredentials>(&bytes)
            {
                match self.builder()?.from_credentials(credentials).await {
                    Ok(account) => return Ok(account),
                    // The authority forgot it, or cannot be reached: a new
                    // account is made below only for the first; a network
                    // failure is reported as it is.
                    Err(e) if is_network(&e) => return Err(words(Kind::Unreachable, &e)),
                    Err(_) => {}
                }
            }
        }
        let eab = match (&authority.binding, authority.needs_binding) {
            (Some(b), _) => {
                let key = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(b.hmac_key.trim_end_matches('='))
                    .map_err(|_| {
                        "the account binding for this authority is not valid.".to_string()
                    })?;
                Some(ExternalAccountKey::new(b.kid.clone(), &key))
            }
            (None, true) => {
                return Err(
                    "this authority needs an account binding the relay has not given yet.".into(),
                );
            }
            (None, false) => None,
        };
        let (account, credentials) = self
            .builder()?
            .create(
                &NewAccount {
                    contact: &[],
                    terms_of_service_agreed: true,
                    only_return_existing: false,
                },
                authority.directory.clone(),
                eab.as_ref(),
            )
            .await
            .map_err(|e| words(Kind::NewAccount, &e))?;
        private::make_dir(&self.dir)
            .map_err(|e| format!("can't keep the account ({})", e.kind()))?;
        let text = Zeroizing::new(
            serde_json::to_vec(&credentials).map_err(|_| "can't keep the account".to_string())?,
        );
        private::write(&file, &text)
            .map_err(|e| format!("can't keep the account ({})", e.kind()))?;
        Ok(account)
    }
}

fn is_network(e: &instant_acme::Error) -> bool {
    matches!(
        e,
        instant_acme::Error::Other(_) | instant_acme::Error::Http(_)
    )
}

/// What kind of thing went wrong: the app's words are chosen from this,
/// never from what the authority said (INVARIANT 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Start,
    Unreachable,
    NewAccount,
    Order,
    Proof,
    Finish,
    Issue,
    Download,
}

impl Kind {
    fn words(self) -> &'static str {
        match self {
            Kind::Start => "the app could not start talking to the authority",
            Kind::Unreachable => "the authority could not be reached",
            Kind::NewAccount => "the authority would not open an account",
            Kind::Order => "the authority refused the order",
            Kind::Proof => "the authority could not check the proof of the name",
            Kind::Finish => "the authority did not finish the order",
            Kind::Issue => "the authority refused to issue the certificate",
            Kind::Download => "the authority did not hand over the certificate",
        }
    }
}

/// A failure as the app says it (INVARIANT 5): its own words for the kind
/// of failure, and what the authority said only as a short quoted detail.
fn words(kind: Kind, e: &instant_acme::Error) -> String {
    match e {
        instant_acme::Error::Api(p) => {
            let said = detail(p.detail.as_deref().unwrap_or(""));
            let what = match p.r#type.as_deref().unwrap_or("").rsplit(':').next() {
                Some("rateLimited") => {
                    "the authority says too many certificates were asked for lately"
                }
                _ => kind.words(),
            };
            if said.is_empty() {
                format!("{what}.")
            } else {
                format!("{what}. It said {said}.")
            }
        }
        other => {
            let said = detail(&other.to_string());
            if said.is_empty() {
                format!("{}.", kind.words())
            } else {
                format!("{}. The detail was {said}.", kind.words())
            }
        }
    }
}

impl Issuer for AcmeIssuer {
    fn issue<'a>(&'a self, req: OrderRequest<'a>) -> BoxFuture<'a, Result<Issued, String>> {
        Box::pin(async move {
            let account = self.account(req.authority).await?;
            let identifiers = [Identifier::Dns(req.identifier.clone())];
            let replaces = req
                .replaces
                .as_deref()
                .and_then(|der| CertificateIdentifier::try_from(&CertificateDer::from(der)).ok());
            let mut new = NewOrder::new(&identifiers);
            if let Some(r) = replaces {
                new = new.replaces(r);
            }
            let mut order = match account.new_order(&new).await {
                Ok(o) => o,
                // An authority without renewal information refuses the
                // `replaces` hint: ordering again without it is how the
                // certificate is renewed there.
                Err(instant_acme::Error::Unsupported(_)) => account
                    .new_order(&NewOrder::new(&identifiers))
                    .await
                    .map_err(|e| words(Kind::Order, &e))?,
                Err(e) => return Err(words(Kind::Order, &e)),
            };
            let mut authorizations = order.authorizations();
            let mut published = false;
            while let Some(result) = authorizations.next().await {
                let mut authz = result.map_err(|e| words(Kind::Proof, &e))?;
                match authz.status {
                    AuthorizationStatus::Pending => {}
                    AuthorizationStatus::Valid => continue,
                    _ => return Err("the authority will not accept this name.".into()),
                }
                let mut challenge = authz
                    .challenge(ChallengeType::Dns01)
                    .ok_or("the authority offers no DNS proof for this name.")?;
                let value = challenge.key_authorization().dns_value();
                req.dns.publish(&value).await.map_err(|why| {
                    format!(
                        "the relay would not publish the proof of the name. It said {}.",
                        detail(&why)
                    )
                })?;
                published = true;
                challenge
                    .set_ready()
                    .await
                    .map_err(|e| words(Kind::Proof, &e))?;
            }
            let result = async {
                let policy = RetryPolicy::new().timeout(self.patience);
                let status = order
                    .poll_ready(&policy)
                    .await
                    .map_err(|e| words(Kind::Finish, &e))?;
                if status != OrderStatus::Ready {
                    return Err("the authority could not check the name's proof.".to_string());
                }
                let key_pem =
                    Zeroizing::new(order.finalize().await.map_err(|e| words(Kind::Issue, &e))?);
                let chain_pem = order
                    .poll_certificate(&policy)
                    .await
                    .map_err(|e| words(Kind::Download, &e))?;
                Ok(Issued { chain_pem, key_pem })
            }
            .await;
            if published {
                req.dns.clear().await;
            }
            result
        })
    }

    fn renewal_window<'a>(
        &'a self,
        authority: &'a Authority,
        chain_pem: &'a str,
    ) -> BoxFuture<'a, Option<(i64, i64)>> {
        Box::pin(async move {
            let der = leaf_der(chain_pem)?;
            let id = CertificateIdentifier::try_from(&CertificateDer::from(der)).ok()?;
            let account = self.account(authority).await.ok()?;
            let (info, _retry) = account.renewal_info(&id).await.ok()?;
            Some((
                info.suggested_window.start.unix_timestamp(),
                info.suggested_window.end.unix_timestamp(),
            ))
        })
    }
}

// Silence the unused-import lint when no trait object use needs it.
#[allow(dead_code)]
fn _uses(_: &dyn DnsProof) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_host_is_accepted_at_its_directory_and_no_other_is() {
        let dir = std::env::temp_dir();
        let issuer = AcmeIssuer::new(&dir);
        for (host, _) in DIRECTORY_HOSTS {
            assert!(
                issuer
                    .directory_allowed(&format!("https://{host}/directory"))
                    .is_ok(),
                "{host}"
            );
            assert!(
                issuer
                    .directory_allowed(&format!("https://{host}:443/directory"))
                    .is_ok(),
                "{host} on 443"
            );
            assert!(
                issuer
                    .directory_allowed(&format!("https://x.{host}/directory"))
                    .is_err(),
                "a host under {host}"
            );
            assert!(
                issuer
                    .directory_allowed(&format!("http://{host}/directory"))
                    .is_err(),
                "{host} over http"
            );
        }
        // The hosts' own documentation names these; a brand-new one is a
        // release of the app.
        let hosts: Vec<&str> = DIRECTORY_HOSTS.iter().map(|(h, _)| *h).collect();
        for want in [
            "acme-v02.api.letsencrypt.org",
            "acme-staging-v02.api.letsencrypt.org",
            "dv.acme-v02.api.pki.goog",
            "acme.zerossl.com",
            "acme.ssl.com",
        ] {
            assert!(hosts.contains(&want), "{want}");
        }
    }

    #[test]
    fn an_authoritys_problem_is_the_apps_words_and_a_short_quoted_detail() {
        let hostile = format!("a\u{7}\n\u{202e}b {}", "x".repeat(5000));
        let problem: instant_acme::Problem = serde_json::from_value(serde_json::json!({
            "type": "urn:made:up:evil",
            "detail": hostile,
        }))
        .unwrap();
        let said = words(Kind::Order, &instant_acme::Error::Api(problem));
        assert!(said.starts_with("the authority refused the order. It said \u{201c}a b "));
        assert!(said.chars().count() < 200, "{}", said.chars().count());
        assert!(!said.chars().any(|c| c.is_control() || c == '\u{202e}'));
    }
}
