use thiserror::Error;

use crate::rpc::api::CommitmentType;

/// Cosmos adapter
pub mod cosmos;
/// EVM adapter
pub mod evm;
/// Solana Adatper
pub mod solana;

/// Errors that can occur while working with attestation adapter
#[derive(Debug, Error)]
pub enum AttestationAdapterError {
    /// Cannot build adapter
    #[error("Failed to build attestor adapter due to: {0}")]
    ConfigError(String),
    /// Bad height
    #[error("Invalid height used")]
    InvalidHeight,
    /// Requested block is not finalized
    #[error("Block not finalized")]
    BlockNotFinalized,
    /// Unable to read chain state
    #[error("Error while retrieving data: {0}")]
    RetrievalError(String),
    /// Malformed commitment
    #[error("Commitment error: {0}")]
    CommitmentError(String),
}

/// Captures builder methods needed to create an [`AttestationAdapter`]
pub trait AdapterBuilder {
    /// Config struct
    type Config: Clone;
    /// Adapter to be created
    type Adapter: AttestationAdapter;

    /// Returns the name of the adapter for logging and observability purposes.
    fn adapter_name() -> &'static str;

    /// Build the specific attestor
    fn build(config: Self::Config) -> Result<Self::Adapter, AttestationAdapterError>;
}

/// Attestation adapter methods needed to provide attestations for a given chain
#[async_trait::async_trait]
pub trait AttestationAdapter: Sync + Send + 'static {
    /// Fetch the height of the last finalized block. If there's no finalized
    /// block yet, it should return an error.
    async fn get_last_height_at_configured_finality(&self) -> Result<u64, AttestationAdapterError>;

    /// Returns a UNIX timestamp in seconds for the provided block height.
    async fn get_block_timestamp(&self, height: u64) -> Result<u64, AttestationAdapterError>;

    /// Get commitment at some block height.
    ///
    /// Note: Returns Ok(None) if commitment was not found.
    async fn get_commitment(
        &self,
        client_id: String,
        height: u64,
        sequence: u64,
        commitment_path: &[u8],
        commitment_type: CommitmentType,
    ) -> Result<Option<[u8; 32]>, AttestationAdapterError>;
}
