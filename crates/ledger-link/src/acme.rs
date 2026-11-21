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
//!   5. Every failure comes back in words that name what failed (the
//!      authority's own `detail` when it gave one), never a bare code.
//! SCENARIOS:
//!   1. Against an in-process authority (`tests/acme.rs`): account,
//!      order, proof, finalize and download work; the second order reuses
//!      the account; a renewal names the certificate it replaces; the
//!      authority's renewal window is read.
//! ```

use crate::certs::{DnsProof, Issued, Issuer, OrderRequest, leaf_der};
use crate::private;
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

/// The ACME client.
pub struct AcmeIssuer {
    dir: PathBuf,
    http: HttpFactory,
    /// How long an order may take to be ready or issued.
    patience: Duration,
}

impl AcmeIssuer {
    /// A client keeping its accounts in `dir` (the link folder).
    pub fn new(dir: &Path) -> Self {
        AcmeIssuer {
            dir: dir.join("acme"),
            http: Arc::new(|| None),
            patience: Duration::from_secs(120),
        }
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

    fn builder(&self) -> Result<instant_acme::AccountBuilder, String> {
        match (self.http)() {
            Some(http) => Ok(Account::builder_with_http(http)),
            None => Account::builder().map_err(|e| words("couldn't start", &e)),
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
        let file = self.file(authority);
        if let Ok(bytes) = std::fs::read(&file) {
            let bytes = Zeroizing::new(bytes);
            if let Ok(credentials) = serde_json::from_slice::<AccountCredentials>(&bytes) {
                match self.builder()?.from_credentials(credentials).await {
                    Ok(account) => return Ok(account),
                    // The authority forgot it, or cannot be reached: a new
                    // account is made below only for the first; a network
                    // failure is reported as it is.
                    Err(e) if is_network(&e) => return Err(words("couldn't be reached", &e)),
                    Err(_) => {}
                }
            }
        }
        let eab = match (&authority.binding, authority.needs_binding) {
            (Some(b), _) => {
                let key = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(b.hmac_key.trim_end_matches('='))
                    .map_err(|_| {
                        "the account binding for this authority is not valid".to_string()
                    })?;
                Some(ExternalAccountKey::new(b.kid.clone(), &key))
            }
            (None, true) => {
                return Err(
                    "this authority needs an account binding the relay has not given yet".into(),
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
            .map_err(|e| words("refused a new account", &e))?;
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

/// The authority's own words, as short as they can be (INVARIANT 5).
fn words(what: &str, e: &instant_acme::Error) -> String {
    match e {
        instant_acme::Error::Api(p) => {
            let kind = p
                .r#type
                .as_deref()
                .unwrap_or("")
                .rsplit(':')
                .next()
                .unwrap_or("");
            let detail = p.detail.as_deref().unwrap_or("");
            match kind {
                "rateLimited" => format!(
                    "the authority says too many certificates were asked for lately ({detail})"
                ),
                "" => format!("the authority {what}: {detail}"),
                _ => format!("the authority {what} ({kind}): {detail}"),
            }
        }
        _ => format!("the authority {what}: {e}"),
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
                    .map_err(|e| words("refused the order", &e))?,
                Err(e) => return Err(words("refused the order", &e)),
            };
            let mut authorizations = order.authorizations();
            let mut published = false;
            while let Some(result) = authorizations.next().await {
                let mut authz = result.map_err(|e| words("failed on the proof of the name", &e))?;
                match authz.status {
                    AuthorizationStatus::Pending => {}
                    AuthorizationStatus::Valid => continue,
                    _ => return Err("the authority will not accept this name".into()),
                }
                let mut challenge = authz
                    .challenge(ChallengeType::Dns01)
                    .ok_or("the authority offers no DNS proof for this name")?;
                let value = challenge.key_authorization().dns_value();
                req.dns.publish(&value).await.map_err(|why| {
                    format!("the relay would not publish the proof of the name ({why})")
                })?;
                published = true;
                challenge
                    .set_ready()
                    .await
                    .map_err(|e| words("failed on the proof of the name", &e))?;
            }
            let result = async {
                let policy = RetryPolicy::new().timeout(self.patience);
                let status = order
                    .poll_ready(&policy)
                    .await
                    .map_err(|e| words("did not finish the order", &e))?;
                if status != OrderStatus::Ready {
                    return Err("the authority could not check the name's proof".to_string());
                }
                let key_pem = Zeroizing::new(
                    order
                        .finalize()
                        .await
                        .map_err(|e| words("refused to issue", &e))?,
                );
                let chain_pem = order
                    .poll_certificate(&policy)
                    .await
                    .map_err(|e| words("did not hand over the certificate", &e))?;
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
