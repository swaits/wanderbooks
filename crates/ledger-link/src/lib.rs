//! The app's side of the relay (plan P15 unit B3).
//!
//! A person turns on "Allow web assistants" for a company. This crate then
//! gives the app a durable public address, `https://<secret>.<four-words>.wanderbooks.link`,
//! holds its own private key and its own certificate, keeps one outbound
//! line to the relay, and serves the sign-in and the books to Claude or
//! ChatGPT on the web through it. The relay forwards encrypted bytes and
//! can read nothing (D-80): TLS ends here.
//!
//! The crate depends on the relay crate for the wire (`proto`, `names`,
//! the client's line) and on no other `ledger-*` crate except, on Windows
//! only, `ledger-store` for the owner-only file helper.
//!
//! ```text
//! GOAL: the app's side of D-80: an address that is durable and secret, a
//!       line that comes back by itself, TLS ended with a key that never
//!       left the machine, a certificate that is ordered, kept and
//!       renewed with the authority as data, and every failure in words.
//! SOURCE: WB-link-1 (the identity and name), WB-link-2 (the line),
//!         WB-link-3 (TLS at the app and the hand to the door), WB-link-4
//!         (certificates), WB-link-5 (the states in words); D-80, D-82,
//!         D-85, D-87.
//! INVARIANTS:
//!   1. No key, secret label or token is in a log, an error, a status
//!      Debug or a test's output; the files that hold them are owner-only.
//!   2. Nothing the relay could use to read traffic leaves the machine:
//!      the relay is sent a public key, a proof and a DNS value.
//!   3. The relay protocol is not changed here; this crate only uses it.
//! SCENARIOS:
//!   1. See `tests/link.rs` and `tests/acme.rs`.
//! ```

#![forbid(unsafe_code)]

pub mod acme;
pub mod certs;
pub mod identity;
pub mod line;
pub mod link;
pub mod private;
pub mod status;
pub mod text;

pub use acme::AcmeIssuer;
pub use certs::{CertTrust, DnsProof, Issued, Issuer, OrderRequest};
pub use line::{Dial, Timing};
pub use link::{DoorTarget, Link, LinkConfig, LinkError};
pub use status::{LinkStatus, Step, Trouble};

/// The relay trust roots for the line, from the roots the program ships
/// (Mozilla's, as `webpki-root-certs`), for a relay with a public
/// certificate.
pub fn public_relay_tls() -> Result<std::sync::Arc<rustls::ClientConfig>, rustls::Error> {
    let mut roots = rustls::RootCertStore::empty();
    for der in webpki_root_certs::TLS_SERVER_ROOT_CERTS {
        // A root the parser can't read is skipped, not fatal.
        let _ = roots.add(der.clone());
    }
    relay::tls::client_config(roots)
}
