use ibc_eureka_utils::rpc::TendermintRpcExt;
use serde::Deserialize;
use tendermint::block::Height;
use tendermint_rpc::{Client, HttpClient, Url};
use tracing::{debug, error, info};

use crate::{
    adapter::{AdapterBuilder, AttestationAdapter, AttestationAdapterError},
    rpc::api::CommitmentType,
};

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

        let client = HttpClient::new(config.url.clone()).map_err(|err| {
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

        let result = self
            .client
            .v2_packet_commitment(client_id.clone(), sequence, height, false)
            .await
            .map_err(|err| {
                error!(
                    error = %err,
                    "failed to fetch packet commitment from Cosmos chain"
                );
                AttestationAdapterError::RetrievalError(err.to_string())
            })?;

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

        let result = self
            .client
            .v2_packet_acknowledgement(client_id.clone(), sequence, height)
            .await
            .map_err(|err| {
                error!(
                    error = %err,
                    "failed to fetch ack commitment from Cosmos chain"
                );
                AttestationAdapterError::RetrievalError(err.to_string())
            })?;

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

        let response = self
            .client
            .v2_packet_receipt(client_id.clone(), sequence, height)
            .await
            .map_err(|err| {
                error!(
                    error = %err,
                    "failed to fetch receipt commitment from Cosmos chain"
                );
                AttestationAdapterError::RetrievalError(err.to_string())
            })?;

        // Packet was received
        if response.received {
            error!("packet was already received, cannot timeout");
            Err(AttestationAdapterError::CommitmentError(format!(
                "Packet seq={sequence} was already received, cannot timeout",
            )))
        } else {
            debug!("receipt commitment not found (packet not received)");
            Ok(None)
        }
    }
}

#[async_trait::async_trait]
impl AttestationAdapter for CosmosAdapter {
    async fn get_last_height_at_configured_finality(&self) -> Result<u64, AttestationAdapterError> {
        debug!("fetching last finalized height from Cosmos chain");

        let block = self.client.latest_commit().await.map_err(|err| {
            error!(error = %err, "failed to fetch latest commit from Cosmos chain");
            AttestationAdapterError::RetrievalError(err.to_string())
        })?;

        let height = block.signed_header.header().height.value();
        debug!(height, "retrieved last finalized height");
        Ok(height)
    }

    async fn get_block_timestamp(&self, height: u64) -> Result<u64, AttestationAdapterError> {
        debug!("fetching block timestamp from Cosmos chain");

        let height = Height::try_from(height).map_err(|_| {
            error!("invalid height for Cosmos chain");
            AttestationAdapterError::InvalidHeight
        })?;

        let block = self.client.commit(height).await.map_err(|err| {
            error!( error = %err, "failed to fetch block from Cosmos chain");
            AttestationAdapterError::RetrievalError(err.to_string())
        })?;

        let timestamp = block.signed_header.header.time.unix_timestamp();
        let timestamp = u64::try_from(timestamp).map_err(|err| {
            error!(timestamp, error = %err, "failed to convert timestamp to u64");
            AttestationAdapterError::RetrievalError(err.to_string())
        })?;

        debug!(timestamp, "retrieved block timestamp");
        Ok(timestamp)
    }

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
