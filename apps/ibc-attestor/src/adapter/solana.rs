use serde::Deserialize;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_ibc_types::Commitment;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tracing::{debug, error, info};

use crate::adapter::{AdapterBuilder, AttestationAdapter, AttestationAdapterError};
use crate::rpc::api::CommitmentType;

/// The anchors discriminator length for the accounts data.
const ANCHOR_DISCRIMINATOR_LEN: usize = 8;

/// Commitment length
const COMMITMENT_LEN: usize = 32;

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

        let client = RpcClient::new(config.url.clone());

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

        Ok(SolanaAdapter { client, router_program_id })
    }
}

#[async_trait::async_trait]
impl AttestationAdapter for SolanaAdapter {
    async fn get_last_finalized_height(&self) -> Result<u64, AttestationAdapterError> {
        debug!("fetching last finalized slot from Solana chain");

        let current_finalized_slot = self
            .client
            .get_slot_with_commitment(CommitmentConfig::finalized())
            .await
            .map_err(|err| {
                error!(error = %err, "failed to fetch finalized slot from Solana chain");
                AttestationAdapterError::RetrievalError(err.to_string())
            })?;

        debug!(slot = current_finalized_slot, "retrieved last finalized slot");
        Ok(current_finalized_slot)
    }

    async fn get_block_timestamp(&self, slot: u64) -> Result<u64, AttestationAdapterError> {
        debug!("fetching block timestamp from Solana chain");

        let block_time = self.client.get_block_time(slot).await.map_err(|err| {
            error!(error = %err, "failed to fetch block time from Solana chain");
            AttestationAdapterError::RetrievalError(err.to_string())
        })?;

        let timestamp = u64::try_from(block_time).map_err(|err| {
            error!(blockTime = block_time, error = %err, "failed to convert block time to u64");
            AttestationAdapterError::RetrievalError(err.to_string())
        })?;

        debug!(timestamp, "retrieved block timestamp");
        Ok(timestamp)
    }

    async fn get_commitment(
        &self,
        client_id: String,
        slot: u64,
        sequence: u64,
        _commitment_path: &[u8],
        commitment_type: CommitmentType,
    ) -> Result<Option<[u8; COMMITMENT_LEN]>, AttestationAdapterError> {
        debug!("fetching commitment from Solana chain");

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

        let account = self
            .client
            .get_account_with_commitment(&commitment_pda, CommitmentConfig::finalized())
            .await
            .map_err(|e| {
                error!(
                    error = %e,
                    "failed to get commitment account from Solana chain"
                );
                AttestationAdapterError::RetrievalError(format!(
                    "Failed to get commitment account for client_id={}, sequence={}, slot={}: {}",
                    client_id, sequence, slot, e
                ))
            })?
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
            error!(dataLength = account_data_len, "invalid commitment account data length");
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
