use alloy::{consensus::BlockHeader, eips::BlockId};
use alloy_primitives::{Address, keccak256};
use alloy_provider::{Provider, RootProvider};
use tracing::{debug, error, info};

use ibc_eureka_solidity_types::ics26::router::routerInstance;
use serde::Deserialize;
use url::Url;

use crate::{
    adapter::{AdapterBuilder, AttestationAdapter, AttestationAdapterError},
    rpc::api::CommitmentType,
};

/// Configuration for connecting to an EVM-compatible blockchain.
#[derive(Clone, Debug, Deserialize)]
pub struct EvmAdapterConfig {
    /// RPC endpoint URL for the EVM chain.
    ///
    /// This should be a valid HTTP or HTTPS URL pointing to an EVM JSON-RPC endpoint.
    pub url: Url,

    /// The Ethereum address of the IBC router contract.
    pub router_address: Address,

    /// Is used for specifying which block height is finalized. If it is set.
    /// Then we take `latest` block height and subtract the finality offset. If
    /// it's None then we use `finalized` block and its height.
    pub finality_offset: Option<u64>,
}

/// Builder for creating EVM adapter instances
pub struct EvmAdapterBuilder;

impl AdapterBuilder for EvmAdapterBuilder {
    type Config = EvmAdapterConfig;
    type Adapter = EvmAdapter;

    fn adapter_name() -> &'static str {
        "evm"
    }

    fn build(config: Self::Config) -> Result<Self::Adapter, AttestationAdapterError> {
        info!(
            rpcUrl = %config.url,
            routerAddress = %config.router_address,
            finalityOffset = ?config.finality_offset,
            "initializing EVM adapter"
        );

        let client = RootProvider::new_http(config.url.clone());
        let router = routerInstance::new(config.router_address, client.clone());

        info!(
            routerAddress = %config.router_address,
            "EVM adapter initialized successfully"
        );

        Ok(EvmAdapter {
            config,
            client,
            router,
        })
    }
}

/// EVM adapter for interacting with Ethereum Virtual Machine compatible chains
#[derive(Debug)]
pub struct EvmAdapter {
    config: EvmAdapterConfig,
    client: RootProvider,
    router: routerInstance<RootProvider>,
}

#[async_trait::async_trait]
impl AttestationAdapter for EvmAdapter {
    async fn get_last_height_at_configured_finality(&self) -> Result<u64, AttestationAdapterError> {
        debug!("fetching last finalized height from EVM chain");

        let block_id = match self.config.finality_offset {
            Some(_) => BlockId::latest(),
            None => BlockId::finalized(),
        };

        let block = self.client.get_block(block_id).await.map_err(|err| {
            error!(error = %err, "failed to fetch block from EVM chain");
            AttestationAdapterError::RetrievalError(err.to_string())
        })?;

        let block = block.ok_or_else(|| {
            error!("block not found (finalized block does not exist)");
            AttestationAdapterError::BlockNotFinalized
        })?;

        let finalized_height = match self.config.finality_offset {
            Some(offset) => {
                let latest = block.number();
                let finalized = latest.saturating_sub(offset);
                debug!(
                    latestHeight = latest,
                    finalityOffset = offset,
                    finalizedHeight = finalized,
                    "calculated finalized height using offset"
                );
                finalized
            }
            None => {
                debug!(
                    finalizedHeight = block.number(),
                    "using finalized block tag"
                );
                block.number()
            }
        };

        debug!(
            finalizedHeight = finalized_height,
            "retrieved last finalized height"
        );
        Ok(finalized_height)
    }

    async fn get_block_timestamp(&self, height: u64) -> Result<u64, AttestationAdapterError> {
        debug!("fetching block timestamp from EVM chain");

        let block = self
            .client
            .get_block(BlockId::number(height))
            .await
            .map_err(|err| {
                error!(error = %err, "failed to fetch block from EVM chain");
                AttestationAdapterError::RetrievalError(err.to_string())
            })?;

        let block = block.ok_or_else(|| {
            error!("block not found at specified height");
            AttestationAdapterError::BlockNotFinalized
        })?;

        let timestamp = block.header.timestamp();
        debug!(timestamp, "retrieved block timestamp");
        Ok(timestamp)
    }

    async fn get_commitment(
        &self,
        _client_id: String,
        height: u64,
        _sequence: u64,
        commitment_path: &[u8],
        _commitment_type: CommitmentType,
    ) -> Result<Option<[u8; 32]>, AttestationAdapterError> {
        let hashed_path = keccak256(commitment_path);

        debug!(
            pathHash = %hex::encode(hashed_path),
            "fetching commitment from EVM router contract"
        );

        let commitment = self
            .router
            .getCommitment(hashed_path)
            .block(BlockId::number(height))
            .call()
            .await
            .map_err(|e| {
                error!(
                    pathHash = %hex::encode(hashed_path),
                    error = %e,
                    "failed to call getCommitment on EVM router contract"
                );
                AttestationAdapterError::RetrievalError(e.to_string())
            })?;

        // Array of 0s means not found
        if !commitment.is_zero() {
            debug!(
                commitment = %hex::encode(commitment),
                "commitment found"
            );
            Ok(Some(commitment.into()))
        } else {
            debug!("commitment not found (zero bytes)");
            Ok(None)
        }
    }
}
