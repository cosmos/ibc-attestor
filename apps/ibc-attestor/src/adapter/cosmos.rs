// Only call the query methods, on the client built by `tendermint_tls`. Don't
// use `TendermintRpcExt::from_rpc_url` or `from_env`: they build their own
// reqwest 0.11 client, which verifies certificates with rustls-webpki 0.101.
use ibc_eureka_utils::rpc::TendermintRpcExt;
use serde::Deserialize;
use tendermint::block::Height;
use tendermint_rpc::{Client, HttpClient, Url};
use tracing::{debug, error, info};

use crate::{
    adapter::{
        AdapterBuilder, AttestationAdapter, AttestationAdapterError, retry::with_retry_backoff,
    },
    rpc::api::CommitmentType,
};

/// TLS for the Tendermint RPC client.
///
/// `tendermint-rpc` is built on reqwest 0.11 / rustls 0.21, which verify server
/// certificates with rustls-webpki 0.101. That release does not distrust the
/// eMudhra (emSign) G3 intermediate CAs, so the client built here hands
/// verification to rustls 0.23's `WebPkiServerVerifier` (rustls-webpki 0.103)
/// with the OS root certificates. Remove this module once `tendermint-rpc`
/// moves to a reqwest release built on rustls 0.23.
mod tendermint_tls {
    use std::{
        sync::Arc,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use rustls::{
        RootCertStore,
        client::{WebPkiServerVerifier, danger::ServerCertVerifier as _},
        pki_types::{CertificateDer, ServerName, UnixTime},
    };
    use rustls_0_21::{
        Certificate, CertificateError, ClientConfig,
        client::{ServerCertVerified, ServerCertVerifier},
    };
    use tendermint_rpc::{HttpClient, HttpClientUrl, Url};

    use crate::adapter::tls::{self, TlsError};

    /// Same request timeout `tendermint-rpc` applies to the clients it builds itself.
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
    const USER_AGENT: &str = concat!("ibc-attestor/", env!("CARGO_PKG_VERSION"));

    /// Errors that can occur while building the Tendermint RPC client.
    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        /// The TLS configuration could not be built.
        #[error(transparent)]
        Tls(#[from] TlsError),
        /// The underlying HTTP client could not be built.
        #[error("failed to build HTTP client: {0}")]
        Http(#[from] reqwest_0_11::Error),
        /// The Tendermint RPC client could not be built.
        #[error("failed to build Tendermint RPC client: {0}")]
        Rpc(#[from] tendermint_rpc::Error),
    }

    /// Builds a Tendermint RPC client whose TLS connections are verified by
    /// rustls-webpki 0.103 against the OS root certificates.
    ///
    /// Plain HTTP endpoints verify no certificates, so they don't require OS
    /// root certificates. Their client trusts no roots and refuses redirects,
    /// so it never makes an HTTPS connection that reqwest 0.11's rustls-webpki
    /// 0.101 would verify.
    pub fn http_client(url: Url) -> Result<HttpClient, Error> {
        if url.is_secure() {
            return http_client_with_roots(url, tls::native_roots()?);
        }
        tls::warn_if_plaintext(&url.clone().into());
        build(
            url,
            reqwest_0_11::Client::builder()
                .tls_built_in_root_certs(false)
                .redirect(reqwest_0_11::redirect::Policy::custom(|attempt| {
                    attempt.error("redirects from plain-HTTP endpoints are not followed")
                })),
        )
    }

    /// Builds a Tendermint RPC client whose TLS connections are verified by
    /// rustls-webpki 0.103 against `roots`.
    pub fn http_client_with_roots(url: Url, roots: RootCertStore) -> Result<HttpClient, Error> {
        let verifier = LegacyVerifierAdapter(tls::verifier(roots)?);

        let mut tls_config = ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        // reqwest 0.11 is built without HTTP/2 support.
        tls_config.alpn_protocols = vec![b"http/1.1".to_vec()];

        build(
            url,
            reqwest_0_11::Client::builder().use_preconfigured_tls(tls_config),
        )
    }

    fn build(url: Url, builder: reqwest_0_11::ClientBuilder) -> Result<HttpClient, Error> {
        let inner = builder
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .build()?;

        Ok(HttpClient::builder(HttpClientUrl::try_from(url)?)
            .client(inner)
            .build()?)
    }

    /// Exposes the rustls 0.23 `WebPkiServerVerifier` through the rustls 0.21
    /// verifier interface expected by reqwest 0.11.
    ///
    /// Only certificate chain and name verification is delegated. The TLS
    /// handshake signature checks keep rustls 0.21's defaults, which verify the
    /// handshake against the already-validated end-entity certificate's key.
    ///
    /// SCTs are ignored and stapled OCSP responses aren't checked, as with the
    /// default verifiers: neither Certificate Transparency nor revocation is
    /// enforced. The eMudhra distrust comes from rustls-webpki's denylist.
    #[derive(Debug)]
    struct LegacyVerifierAdapter(Arc<WebPkiServerVerifier>);

    impl ServerCertVerifier for LegacyVerifierAdapter {
        fn verify_server_cert(
            &self,
            end_entity: &Certificate,
            intermediates: &[Certificate],
            server_name: &rustls_0_21::ServerName,
            _scts: &mut dyn Iterator<Item = &[u8]>,
            ocsp_response: &[u8],
            now: SystemTime,
        ) -> Result<ServerCertVerified, rustls_0_21::Error> {
            let end_entity = CertificateDer::from(end_entity.0.as_slice());
            let intermediates: Vec<_> = intermediates
                .iter()
                .map(|cert| CertificateDer::from(cert.0.as_slice()))
                .collect();
            let server_name = match server_name {
                rustls_0_21::ServerName::DnsName(name) => ServerName::try_from(name.as_ref())
                    .map_err(|_| {
                        rustls_0_21::Error::InvalidCertificate(CertificateError::NotValidForName)
                    })?,
                rustls_0_21::ServerName::IpAddress(ip) => ServerName::IpAddress((*ip).into()),
                _ => {
                    return Err(rustls_0_21::Error::General(
                        "unsupported server name type".to_string(),
                    ));
                }
            };
            let now = now
                .duration_since(UNIX_EPOCH)
                .map(UnixTime::since_unix_epoch)
                .map_err(|_| rustls_0_21::Error::FailedToGetCurrentTime)?;

            self.0
                .verify_server_cert(
                    &end_entity,
                    &intermediates,
                    &server_name,
                    ocsp_response,
                    now,
                )
                .map(|_| ServerCertVerified::assertion())
                .map_err(|err| {
                    rustls_0_21::Error::InvalidCertificate(CertificateError::Other(Arc::new(err)))
                })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::adapter::test_tls::TestCa;

        fn verify(
            trusted: &TestCa,
            server_cert: &CertificateDer<'_>,
            server_name: &str,
        ) -> Result<ServerCertVerified, rustls_0_21::Error> {
            let mut roots = RootCertStore::empty();
            roots.add(trusted.der().clone()).unwrap();

            LegacyVerifierAdapter(tls::verifier(roots).unwrap()).verify_server_cert(
                &Certificate(server_cert.to_vec()),
                &[],
                &rustls_0_21::ServerName::try_from(server_name).unwrap(),
                &mut std::iter::empty(),
                &[],
                SystemTime::now(),
            )
        }

        /// The rustls 0.23 error a rejection was mapped from.
        fn rejection(result: Result<ServerCertVerified, rustls_0_21::Error>) -> rustls::Error {
            // The result isn't printed: it derives from certificate data.
            let Err(rustls_0_21::Error::InvalidCertificate(CertificateError::Other(err))) = result
            else {
                panic!("expected a rejection mapped from the rustls 0.23 verifier");
            };
            err.downcast_ref::<rustls::Error>()
                .expect("rejection must come from the rustls 0.23 verifier")
                .clone()
        }

        #[test]
        fn accepts_trusted_chain() {
            let ca = TestCa::new("Trusted CA");
            let (cert, _) = ca.issue("localhost");

            assert!(
                verify(&ca, &cert, "localhost").is_ok(),
                "certificate was rejected"
            );
        }

        #[test]
        fn rejects_unknown_issuer() {
            let trusted = TestCa::new("Trusted CA");
            let (cert, _) = TestCa::new("Untrusted CA").issue("localhost");

            assert!(
                matches!(
                    rejection(verify(&trusted, &cert, "localhost")),
                    rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer)
                ),
                "expected an UnknownIssuer rejection"
            );
        }

        #[test]
        fn rejects_wrong_hostname() {
            let ca = TestCa::new("Trusted CA");
            let (cert, _) = ca.issue("example.com");

            assert!(
                matches!(
                    rejection(verify(&ca, &cert, "localhost")),
                    rustls::Error::InvalidCertificate(
                        rustls::CertificateError::NotValidForNameContext { .. }
                    )
                ),
                "expected a NotValidForName rejection"
            );
        }

        #[test]
        fn accepts_matching_ip_address() {
            let ca = TestCa::new("Trusted CA");
            let (cert, _) = ca.issue("127.0.0.1");

            assert!(
                verify(&ca, &cert, "127.0.0.1").is_ok(),
                "certificate was rejected"
            );
        }

        #[test]
        fn rejects_wrong_ip_address() {
            let ca = TestCa::new("Trusted CA");
            let (cert, _) = ca.issue("127.0.0.1");

            assert!(
                matches!(
                    rejection(verify(&ca, &cert, "10.0.0.1")),
                    rustls::Error::InvalidCertificate(
                        rustls::CertificateError::NotValidForNameContext { .. }
                    )
                ),
                "expected a NotValidForName rejection"
            );
        }
    }
}

/// Configuration for the Cosmos blockchain client adapter.
#[derive(Clone, Debug, Deserialize)]
pub struct CosmosAdapterConfig {
    /// The URL of the Tendermint RPC endpoint.
    pub url: Url,
}

/// Builder for creating Cosmos adapter instances
pub struct CosmosAdapterBuilder;

impl AdapterBuilder for CosmosAdapterBuilder {
    type Config = CosmosAdapterConfig;
    type Adapter = CosmosAdapter;

    fn adapter_name() -> &'static str {
        "cosmos"
    }

    fn build(config: Self::Config) -> Result<Self::Adapter, AttestationAdapterError> {
        info!(
            rpcUrl = %config.url,
            "initializing Cosmos adapter"
        );

        let client = tendermint_tls::http_client(config.url.clone()).map_err(|err| {
            error!(
                rpcUrl = %config.url,
                error = %err,
                "failed to initialize Cosmos client"
            );
            AttestationAdapterError::ConfigError(format!(
                "Cosmos client couldn't be initialized: {err}"
            ))
        })?;

        info!("Cosmos adapter initialized successfully");

        Ok(CosmosAdapter { client })
    }
}

/// Cosmos adapter for interacting with Cosmos SDK based chains via Tendermint RPC
#[derive(Debug)]
pub struct CosmosAdapter {
    client: HttpClient,
}

impl CosmosAdapter {
    async fn get_packet_commitment(
        &self,
        client_id: String,
        height: u64,
        sequence: u64,
    ) -> Result<Option<Vec<u8>>, AttestationAdapterError> {
        debug!("fetching packet commitment from Cosmos chain");

        let result =
            with_retry_backoff("cosmos.get_packet_commitment.v2_packet_commitment", || {
                let client_id = client_id.clone();
                async move {
                    self.client
                        .v2_packet_commitment(client_id, sequence, height, false)
                        .await
                        .map_err(|err| {
                            // error log emitted by retry module
                            debug!(
                                error = %err,
                                "failed to fetch packet commitment from Cosmos chain"
                            );
                            AttestationAdapterError::RetrievalError(err.to_string())
                        })
                }
            })
            .await?;

        if result.commitment.is_empty() {
            debug!("packet commitment not found (empty)");
            Ok(None)
        } else {
            debug!("packet commitment retrieved");
            Ok(Some(result.commitment))
        }
    }

    async fn get_ack_commitment(
        &self,
        client_id: String,
        height: u64,
        sequence: u64,
    ) -> Result<Option<Vec<u8>>, AttestationAdapterError> {
        debug!("fetching ack commitment from Cosmos chain");

        let result = with_retry_backoff(
            "cosmos.get_ack_commitment.v2_packet_acknowledgement",
            || {
                let client_id = client_id.clone();
                async move {
                    self.client
                        .v2_packet_acknowledgement(client_id, sequence, height)
                        .await
                        .map_err(|err| {
                            // error log emitted by retry module
                            debug!(
                                error = %err,
                                "failed to fetch ack commitment from Cosmos chain"
                            );
                            AttestationAdapterError::RetrievalError(err.to_string())
                        })
                }
            },
        )
        .await?;

        if result.acknowledgement.is_empty() {
            debug!("ack commitment not found (empty)");
            Ok(None)
        } else {
            debug!("ack commitment retrieved");
            Ok(Some(result.acknowledgement))
        }
    }

    async fn get_receipt_commitment(
        &self,
        client_id: String,
        height: u64,
        sequence: u64,
    ) -> Result<Option<Vec<u8>>, AttestationAdapterError> {
        debug!("fetching receipt commitment from Cosmos chain");

        let response =
            with_retry_backoff("cosmos.get_receipt_commitment.v2_packet_receipt", || {
                let client_id = client_id.clone();
                async move {
                    self.client
                        .v2_packet_receipt(client_id, sequence, height)
                        .await
                        .map_err(|err| {
                            // error log emitted by retry module
                            debug!(
                                error = %err,
                                "failed to fetch receipt commitment from Cosmos chain"
                            );
                            AttestationAdapterError::RetrievalError(err.to_string())
                        })
                }
            })
            .await?;

        if response.received {
            // NOTE: IBC-Go uses a fixed value of [1] to indicate packet receipt
            debug!("receipt commitment exists (packet was received)");
            Ok(Some(vec![1_u8; 32]))
        } else {
            debug!("receipt commitment not found (packet not received)");
            Ok(None)
        }
    }
}

#[async_trait::async_trait]
impl AttestationAdapter for CosmosAdapter {
    #[tracing::instrument(skip(self), fields(chain = "cosmos"))]
    async fn get_last_height_at_configured_finality(&self) -> Result<u64, AttestationAdapterError> {
        debug!("fetching last finalized height from Cosmos chain");

        let block = with_retry_backoff("cosmos.get_last_height.latest_commit", || async {
            self.client.latest_commit().await.map_err(|err| {
                // error log emitted by retry module
                debug!(error = %err, "failed to fetch latest commit from Cosmos chain");
                AttestationAdapterError::RetrievalError(err.to_string())
            })
        })
        .await?;

        let height = block.signed_header.header().height.value();
        debug!(height, "retrieved last finalized height");
        Ok(height)
    }

    #[tracing::instrument(skip(self), fields(chain = "cosmos", height))]
    async fn get_block_timestamp(&self, height: u64) -> Result<u64, AttestationAdapterError> {
        debug!("fetching block timestamp from Cosmos chain");

        let height = Height::try_from(height).map_err(|_| {
            error!("invalid height for Cosmos chain");
            AttestationAdapterError::InvalidHeight
        })?;

        let block = with_retry_backoff("cosmos.get_block_timestamp.commit", || async {
            self.client.commit(height).await.map_err(|err| {
                // error log emitted by retry module
                debug!(error = %err, "failed to fetch block from Cosmos chain");
                AttestationAdapterError::RetrievalError(err.to_string())
            })
        })
        .await?;

        let timestamp = block.signed_header.header.time.unix_timestamp();
        let timestamp = u64::try_from(timestamp).map_err(|err| {
            error!(timestamp, error = %err, "failed to convert timestamp to u64");
            AttestationAdapterError::RetrievalError(err.to_string())
        })?;

        debug!(timestamp, "retrieved block timestamp");
        Ok(timestamp)
    }

    #[tracing::instrument(
        skip(self, _commitment_path),
        fields(chain = "cosmos", clientId = %client_id, height, sequence, commitmentType = ?commitment_type)
    )]
    async fn get_commitment(
        &self,
        client_id: String,
        height: u64,
        sequence: u64,
        _commitment_path: &[u8],
        commitment_type: CommitmentType,
    ) -> Result<Option<[u8; 32]>, AttestationAdapterError> {
        debug!("fetching commitment from Cosmos chain");

        // Get commitment
        let commitment = match commitment_type {
            CommitmentType::Packet => {
                self.get_packet_commitment(client_id.clone(), height, sequence)
                    .await
            }
            CommitmentType::Ack => {
                self.get_ack_commitment(client_id.clone(), height, sequence)
                    .await
            }
            CommitmentType::Receipt => {
                self.get_receipt_commitment(client_id.clone(), height, sequence)
                    .await
            }
        }?;

        // Early return if commitment is None
        let Some(commitment) = commitment else {
            debug!("commitment not found");
            return Ok(None);
        };

        let commitment: [u8; 32] = commitment.try_into().map_err(|_| {
            error!("commitment length mismatch (expected 32 bytes)");
            AttestationAdapterError::CommitmentError("Commitment length mismatch".to_string())
        })?;

        debug!("commitment retrieved successfully");
        Ok(Some(commitment))
    }
}

#[cfg(test)]
mod tests {
    use rustls::RootCertStore;
    use serde_json::json;
    use tendermint_rpc::{Client, HttpClient};

    use super::tendermint_tls::{http_client, http_client_with_roots};
    use crate::adapter::test_tls::{
        Reply, TestCa, assert_not_connected, redirect_target, serve_http,
    };

    fn client_trusting(ca: &TestCa, port: u16) -> HttpClient {
        let mut roots = RootCertStore::empty();
        roots.add(ca.der().clone()).unwrap();
        http_client_with_roots(format!("https://localhost:{port}").parse().unwrap(), roots).unwrap()
    }

    // These tests go end to end through reqwest 0.11 and `tendermint-rpc`, so
    // they also cover how `http_client_with_roots` wires the verifier and the
    // HTTP/1.1 ALPN setting into the client.

    #[tokio::test]
    async fn http_client_with_roots_connects_to_server_from_trusted_root() {
        let ca = TestCa::new("Trusted CA");
        let port = ca.serve("localhost", json!({})).await;

        assert!(
            client_trusting(&ca, port).health().await.is_ok(),
            "request to the trusted server failed"
        );
    }

    #[tokio::test]
    async fn http_client_with_roots_rejects_server_from_untrusted_root() {
        let trusted = TestCa::new("Trusted CA");
        let port = TestCa::new("Untrusted CA")
            .serve("localhost", json!({}))
            .await;

        let err = client_trusting(&trusted, port).health().await.unwrap_err();
        assert!(
            err.to_string().contains("UnknownIssuer"),
            "expected an UnknownIssuer rejection"
        );
    }

    #[tokio::test]
    async fn http_client_connects_over_plain_http() {
        let port = serve_http(Reply::Result(json!({}))).await;
        let url = format!("http://localhost:{port}").parse().unwrap();

        http_client(url).unwrap().health().await.unwrap();
    }

    #[tokio::test]
    async fn http_client_does_not_follow_redirect_to_https() {
        let (target, target_url) = redirect_target().await;
        let port = serve_http(Reply::Redirect(target_url)).await;
        let url = format!("http://localhost:{port}").parse().unwrap();

        http_client(url).unwrap().health().await.unwrap_err();
        assert_not_connected(&target).await;
    }
}
