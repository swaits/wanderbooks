//! TLS settings for the line, both ends.
//!
//! The line is TLS 1.3 only. The relay presents its own certificate for
//! its own host; the app checks it against the roots it is given. The app
//! proves itself not with a certificate but with the signature of
//! [`crate::proto`], bound to this session's exporter.

use std::path::Path;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The relay's server settings from a chain and key.
pub fn server_config(
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<Arc<ServerConfig>, rustls::Error> {
    let mut cfg = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(chain, key)?;
    // A line is long-lived and proven by signature: no tickets to hold.
    cfg.send_tls13_tickets = 0;
    Ok(Arc::new(cfg))
}

/// The relay's server settings from PEM files.
pub fn server_config_from_files(cert: &Path, key: &Path) -> Result<Arc<ServerConfig>, String> {
    let chain = CertificateDer::pem_file_iter(cert)
        .map_err(|e| format!("{}: {e}", cert.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{}: {e}", cert.display()))?;
    let key = PrivateKeyDer::from_pem_file(key).map_err(|e| format!("{}: {e}", key.display()))?;
    server_config(chain, key).map_err(|e| e.to_string())
}

/// An app's client settings: TLS 1.3, checking the relay against `roots`.
pub fn client_config(roots: RootCertStore) -> Result<Arc<ClientConfig>, rustls::Error> {
    Ok(Arc::new(
        ClientConfig::builder_with_provider(provider())
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}
