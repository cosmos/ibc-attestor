//! TLS configuration shared by the adapters' RPC clients.
//!
//! Server certificates are verified by rustls-webpki 0.103 (through rustls
//! 0.23's `WebPkiServerVerifier`) against the operating system's root
//! certificates. rustls-webpki 0.103.16 also rejects chains through the
//! distrusted eMudhra (emSign) G3 intermediate CAs.

use std::sync::Arc;

use rustls::{
    ClientConfig, RootCertStore,
    client::{VerifierBuilderError, WebPkiServerVerifier},
    crypto::{CryptoProvider, ring},
};
use tracing::warn;
use url::{Host, Url};

/// Errors that can occur while building TLS configuration.
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    /// The OS trust store has no usable root certificates.
    #[error("no usable root certificates found in the OS trust store")]
    NoRootCertificates,
    /// The certificate verifier could not be built.
    #[error("failed to build certificate verifier: {0}")]
    Verifier(#[from] VerifierBuilderError),
    /// The rustls client configuration could not be built.
    #[error("failed to build TLS client configuration: {0}")]
    Config(#[from] rustls::Error),
}

/// Loads the OS root certificates, logging any that fail to load.
///
/// Fails if none are usable, so clients never start with an empty trust store.
pub fn native_roots() -> Result<RootCertStore, TlsError> {
    let native = rustls_native_certs::load_native_certs();
    for err in &native.errors {
        warn!(error = %err, "failed to load native root certificates");
    }

    let mut roots = RootCertStore::empty();
    let (_, invalid) = roots.add_parsable_certificates(native.certs);
    if invalid > 0 {
        warn!(invalid, "skipped unparsable native root certificates");
    }
    if roots.is_empty() {
        return Err(TlsError::NoRootCertificates);
    }
    Ok(roots)
}

/// Builds the rustls-webpki 0.103 certificate verifier trusting `roots`.
pub fn verifier(roots: RootCertStore) -> Result<Arc<WebPkiServerVerifier>, TlsError> {
    Ok(WebPkiServerVerifier::builder_with_provider(Arc::new(roots), crypto_provider()).build()?)
}

/// Builds a rustls 0.23 client configuration for HTTP/1.1 that verifies
/// server certificates with `verifier`.
pub fn client_config(verifier: Arc<WebPkiServerVerifier>) -> Result<ClientConfig, TlsError> {
    let mut config = ClientConfig::builder_with_provider(crypto_provider())
        .with_safe_default_protocol_versions()?
        .with_webpki_verifier(verifier)
        .with_no_client_auth();
    // The reqwest builds in use are compiled without HTTP/2 support.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

fn crypto_provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

/// Warns when `url` sends RPC traffic over plain HTTP to another machine.
///
/// Plain HTTP verifies no certificates, so anyone on the network path can read
/// or alter responses. Only the host is logged: RPC URLs often carry API keys.
pub fn warn_if_plaintext(url: &Url) {
    if is_plaintext_to_remote_host(url) {
        warn!(
            rpcHost = url.host_str().unwrap_or_default(),
            "RPC endpoint uses plain HTTP to a remote host; responses can be read or \
             altered in transit. Use https:// unless the network path is trusted."
        );
    }
}

fn is_plaintext_to_remote_host(url: &Url) -> bool {
    if url.scheme() != "http" {
        return false;
    }
    match url.host() {
        Some(Host::Domain(domain)) => !domain.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => !ip.is_loopback(),
        Some(Host::Ipv6(ip)) => !ip.is_loopback(),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_to_remote_host_is_detected() {
        for url in ["http://node.example:26657", "http://10.0.0.1:8545"] {
            assert!(is_plaintext_to_remote_host(&url.parse().unwrap()), "{url}");
        }
    }

    #[test]
    fn https_and_local_plaintext_are_not_flagged() {
        for url in [
            "https://node.example",
            "http://localhost:26657",
            "http://127.0.0.1:8545",
            "http://[::1]:8899",
        ] {
            assert!(!is_plaintext_to_remote_host(&url.parse().unwrap()), "{url}");
        }
    }
}
