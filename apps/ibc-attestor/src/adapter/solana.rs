use serde::Deserialize;
use solana_commitment_config::CommitmentConfig;
use solana_ibc_types::Commitment;
use solana_rpc_client::{
    http_sender::HttpSender, nonblocking::rpc_client::RpcClient, rpc_client::RpcClientConfig,
};
use solana_sdk::pubkey::{MAX_SEED_LEN, Pubkey};
use std::{str::FromStr, time::Duration};
use tracing::{debug, error, info};

use rustls::RootCertStore;
use url::Url;

use crate::adapter::{
    AdapterBuilder, AttestationAdapter, AttestationAdapterError,
    retry::with_retry_backoff,
    tls::{self, TlsError},
};
use crate::rpc::api::CommitmentType;

/// The anchors discriminator length for the accounts data.
const ANCHOR_DISCRIMINATOR_LEN: usize = 8;

/// Commitment length
const COMMITMENT_LEN: usize = 32;

/// Same request timeout `solana-rpc-client` applies to the clients it builds itself.
const RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// Configuration for the Solana blockchain client adapter
#[derive(Clone, Debug, Deserialize)]
pub struct SolanaAdapterConfig {
    /// RPC endpoint URL for the Solana chain
    pub url: String,
    /// The router program ID (Solana program address)
    #[serde(alias = "router_address")]
    pub router_program_id: String,
}

/// Solana adapter for interacting with the Solana blockchain
pub struct SolanaAdapter {
    client: RpcClient,
    router_program_id: Pubkey,
}

/// Builder for creating Solana adapter instances
pub struct SolanaAdapterBuilder;

impl AdapterBuilder for SolanaAdapterBuilder {
    type Config = SolanaAdapterConfig;
    type Adapter = SolanaAdapter;

    fn adapter_name() -> &'static str {
        "solana"
    }

    fn build(config: Self::Config) -> Result<Self::Adapter, AttestationAdapterError> {
        info!(
            rpcUrl = %config.url,
            routerProgramId = %config.router_program_id,
            "initializing Solana adapter"
        );

        let client = rpc_client(config.url.clone()).map_err(|err| {
            error!(
                rpcUrl = %config.url,
                error = %err,
                "failed to initialize Solana client"
            );
            AttestationAdapterError::ConfigError(format!(
                "Solana client couldn't be initialized: {err}"
            ))
        })?;

        let router_program_id = Pubkey::from_str(&config.router_program_id).map_err(|err| {
            error!(
                routerProgramId = %config.router_program_id,
                error = %err,
                "invalid router program ID"
            );
            AttestationAdapterError::ConfigError(format!(
                "Invalid router program ID {}: {err}",
                config.router_program_id
            ))
        })?;

        info!(
            routerProgramId = %router_program_id,
            "Solana adapter initialized successfully"
        );

        Ok(SolanaAdapter {
            client,
            router_program_id,
        })
    }
}

/// Errors that can occur while building the Solana RPC client.
#[derive(Debug, thiserror::Error)]
enum RpcClientError {
    /// The TLS configuration could not be built.
    #[error(transparent)]
    Tls(#[from] TlsError),
    /// The underlying HTTP client could not be built.
    #[error("failed to build HTTP client: {0}")]
    Http(#[from] reqwest::Error),
}

/// Builds an RPC client like `RpcClient::new`, but verifying HTTPS connections
/// with rustls-webpki 0.103 against the OS root certificates instead of
/// reqwest's bundled webpki roots.
///
/// Plain HTTP endpoints verify no certificates, so they don't require OS root
/// certificates. Their client trusts no roots and refuses redirects, so it
/// never makes an HTTPS connection. Redirects fail as errors rather than
/// returning the `3xx` response, which `HttpSender` would panic on.
fn rpc_client(url: String) -> Result<RpcClient, RpcClientError> {
    let parsed = Url::parse(&url);
    if parsed.as_ref().is_ok_and(|url| url.scheme() == "https") {
        return rpc_client_with_roots(url, tls::native_roots()?);
    }
    if let Ok(parsed) = &parsed {
        tls::warn_if_plaintext(parsed);
    }
    build_rpc_client(
        url,
        reqwest::Client::builder()
            .tls_built_in_root_certs(false)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                attempt.error("redirects from plain-HTTP endpoints are not followed")
            })),
    )
}

/// Builds an RPC client like `RpcClient::new` whose HTTPS connections are
/// verified by rustls-webpki 0.103 against `roots`.
fn rpc_client_with_roots(url: String, roots: RootCertStore) -> Result<RpcClient, RpcClientError> {
    let tls_config = tls::client_config(tls::verifier(roots)?)?;
    build_rpc_client(
        url,
        reqwest::Client::builder().use_preconfigured_tls(tls_config),
    )
}

/// Builds an RPC client from `builder` with the same headers and timeouts as
/// `RpcClient::new`.
fn build_rpc_client(
    url: String,
    builder: reqwest::ClientBuilder,
) -> Result<RpcClient, RpcClientError> {
    let client = builder
        .default_headers(HttpSender::default_headers())
        .timeout(RPC_TIMEOUT)
        .pool_idle_timeout(RPC_TIMEOUT)
        .build()?;

    Ok(RpcClient::new_sender(
        HttpSender::new_with_client(url, client),
        RpcClientConfig::with_commitment(CommitmentConfig::default()),
    ))
}

#[async_trait::async_trait]
impl AttestationAdapter for SolanaAdapter {
    #[tracing::instrument(skip(self), fields(chain = "solana"))]
    async fn get_last_height_at_configured_finality(&self) -> Result<u64, AttestationAdapterError> {
        debug!("fetching last finalized slot from Solana chain");

        let current_finalized_slot = with_retry_backoff(
            "solana.get_last_height.get_slot_with_commitment",
            || async {
                self.client
                    .get_slot_with_commitment(CommitmentConfig::finalized())
                    .await
                    .map_err(|err| {
                        // error log emitted by retry module
                        debug!(error = %err, "failed to fetch finalized slot from Solana chain");
                        AttestationAdapterError::RetrievalError(err.to_string())
                    })
            },
        )
        .await?;

        debug!(
            slot = current_finalized_slot,
            "retrieved last finalized slot"
        );
        Ok(current_finalized_slot)
    }

    #[tracing::instrument(skip(self), fields(chain = "solana", slot))]
    async fn get_block_timestamp(&self, slot: u64) -> Result<u64, AttestationAdapterError> {
        debug!("fetching block timestamp from Solana chain");

        let block_time =
            with_retry_backoff("solana.get_block_timestamp.get_block_time", || async {
                self.client.get_block_time(slot).await.map_err(|err| {
                    // error log emitted by retry module
                    debug!(error = %err, "failed to fetch block time from Solana chain");
                    AttestationAdapterError::RetrievalError(err.to_string())
                })
            })
            .await?;

        let timestamp = u64::try_from(block_time).map_err(|err| {
            error!(blockTime = block_time, error = %err, "failed to convert block time to u64");
            AttestationAdapterError::RetrievalError(err.to_string())
        })?;

        debug!(timestamp, "retrieved block timestamp");
        Ok(timestamp)
    }

    #[tracing::instrument(
        skip(self, _commitment_path),
        fields(chain = "solana", clientId = %client_id, slot, sequence, commitmentType = ?commitment_type)
    )]
    async fn get_commitment(
        &self,
        client_id: String,
        slot: u64,
        sequence: u64,
        _commitment_path: &[u8],
        commitment_type: CommitmentType,
    ) -> Result<Option<[u8; COMMITMENT_LEN]>, AttestationAdapterError> {
        debug!("fetching commitment from Solana chain");

        validate_client_id_seed(&client_id)?;

        let (commitment_pda, _bump) = match commitment_type {
            CommitmentType::Packet => {
                Commitment::packet_commitment_pda(&client_id, sequence, self.router_program_id)
            }
            CommitmentType::Ack => {
                Commitment::packet_ack_pda(&client_id, sequence, self.router_program_id)
            }
            CommitmentType::Receipt => {
                Commitment::packet_receipt_pda(&client_id, sequence, self.router_program_id)
            }
        };

        let account = with_retry_backoff(
            "solana.get_commitment.get_account_with_commitment",
            || async {
                self.client
                    .get_account_with_commitment(&commitment_pda, CommitmentConfig::finalized())
                    .await
                    .map_err(|e| {
                        // error log emitted by retry module
                        debug!(
                            error = %e,
                            "failed to get commitment account from Solana chain"
                        );
                        AttestationAdapterError::RetrievalError(format!(
                            "Failed to get commitment account for client_id={client_id}, sequence={sequence}, slot={slot}: {e}",
                        ))
                    })
            },
        )
        .await?
        .value;

        // Early return if account is not found
        let Some(account) = account else {
            debug!("commitment account not found");
            return Ok(None);
        };

        let account_data_len = account.data.len();

        // The account data should be a 32-byte commitment value
        // Skip the 8-byte anchor discriminator
        if account_data_len < ANCHOR_DISCRIMINATOR_LEN + COMMITMENT_LEN {
            error!(
                dataLength = account_data_len,
                "invalid commitment account data length"
            );
            return Err(AttestationAdapterError::RetrievalError(format!(
                "Invalid commitment account data length: got {account_data_len} bytes, expected at least 40",
            )));
        }

        let (_discriminator, commitment) = account.data.split_at(ANCHOR_DISCRIMINATOR_LEN);
        let commitment: [u8; 32] = commitment.try_into().map_err(|_| {
            error!("commitment length mismatch after parsing");
            AttestationAdapterError::CommitmentError("Commitment length mismatch".to_string())
        })?;

        debug!("commitment retrieved successfully");
        Ok(Some(commitment))
    }
}

fn validate_client_id_seed(client_id: &str) -> Result<(), AttestationAdapterError> {
    if client_id.len() > MAX_SEED_LEN {
        return Err(AttestationAdapterError::ArgumentTooLong(
            client_id.to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::adapter::test_tls::{
        Reply, TestCa, assert_not_connected, redirect_target, serve_http,
    };

    fn test_adapter() -> SolanaAdapter {
        SolanaAdapter {
            client: RpcClient::new("http://127.0.0.1:0".to_string()),
            router_program_id: Pubkey::new_from_array([1; 32]),
        }
    }

    fn overlong_client_id() -> String {
        "a".repeat(MAX_SEED_LEN + 1)
    }

    #[tokio::test]
    async fn get_packet_commitment_rejects_overlong_client_id_before_pda_derivation() {
        let client_id = overlong_client_id();
        let result = test_adapter()
            .get_commitment(client_id.clone(), 1, 1, &[], CommitmentType::Packet)
            .await;

        let error = result.expect_err("overlong client ID must be rejected");
        assert!(matches!(
            error,
            AttestationAdapterError::ArgumentTooLong(rejected) if rejected == client_id
        ));
    }

    #[tokio::test]
    async fn get_ack_commitment_rejects_overlong_client_id_before_pda_derivation() {
        let result = test_adapter()
            .get_commitment(overlong_client_id(), 1, 1, &[], CommitmentType::Ack)
            .await;

        assert!(matches!(
            result,
            Err(AttestationAdapterError::ArgumentTooLong(_))
        ));
    }

    #[tokio::test]
    async fn get_receipt_commitment_rejects_overlong_client_id_before_pda_derivation() {
        let result = test_adapter()
            .get_commitment(overlong_client_id(), 1, 1, &[], CommitmentType::Receipt)
            .await;

        assert!(matches!(
            result,
            Err(AttestationAdapterError::ArgumentTooLong(_))
        ));
    }

    #[test]
    fn validate_client_id_seed_accepts_max_seed_length() {
        let client_id = "a".repeat(MAX_SEED_LEN);

        assert!(validate_client_id_seed(&client_id).is_ok());
    }

    fn rpc_client_trusting(ca: &TestCa, port: u16) -> RpcClient {
        let mut roots = RootCertStore::empty();
        roots.add(ca.der().clone()).unwrap();
        rpc_client_with_roots(format!("https://localhost:{port}"), roots).unwrap()
    }

    #[tokio::test]
    async fn rpc_client_accepts_certificate_from_trusted_root() {
        let ca = TestCa::new("Trusted CA");
        let port = ca.serve("localhost", json!(42)).await;

        let slot = rpc_client_trusting(&ca, port).get_slot().await;
        assert!(
            matches!(slot, Ok(42)),
            "request to the trusted server failed"
        );
    }

    #[tokio::test]
    async fn rpc_client_rejects_certificate_from_untrusted_root() {
        let trusted = TestCa::new("Trusted CA");
        let port = TestCa::new("Untrusted CA")
            .serve("localhost", json!(42))
            .await;

        let err = rpc_client_trusting(&trusted, port)
            .get_slot()
            .await
            .unwrap_err();
        assert!(
            format!("{err:?}").contains("UnknownIssuer"),
            "expected an UnknownIssuer rejection"
        );
    }

    #[tokio::test]
    async fn rpc_client_rejects_certificate_for_wrong_hostname() {
        let ca = TestCa::new("Trusted CA");
        let port = ca.serve("example.com", json!(42)).await;

        let err = rpc_client_trusting(&ca, port).get_slot().await.unwrap_err();
        assert!(
            format!("{err:?}").contains("NotValidForName"),
            "expected a NotValidForName rejection"
        );
    }

    #[tokio::test]
    async fn rpc_client_connects_over_plain_http() {
        let port = serve_http(Reply::Result(json!(42))).await;

        let client = rpc_client(format!("http://localhost:{port}")).unwrap();
        assert_eq!(client.get_slot().await.unwrap(), 42);
    }

    #[tokio::test]
    async fn rpc_client_does_not_follow_redirect_to_https() {
        let (target, target_url) = redirect_target().await;
        let port = serve_http(Reply::Redirect(target_url)).await;

        let client = rpc_client(format!("http://localhost:{port}")).unwrap();
        client.get_slot().await.unwrap_err();
        assert_not_connected(&target).await;
    }
}
