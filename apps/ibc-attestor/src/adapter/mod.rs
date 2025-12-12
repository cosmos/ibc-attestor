use thiserror::Error;

use crate::rpc::api::CommitmentType;

pub mod cosmos;
pub mod evm;
pub mod solana;

/// Errors that can occur while working with attestation adapter
#[derive(Debug, Error)]
pub enum AttestationAdapterError {
    #[error("Failed to build attestor adapter due to: {0}")]
    ConfigError(String),
    #[error("Invalid height used")]
    InvalidHeight,
    #[error("Block not finalized")]
    BlockNotFinalized,
    #[error("Error while retrieving data: {0}")]
    RetrievalError(String),
    #[error("Commitment error: {0}")]
    CommitmentError(String),
}

pub trait AdapterBuilder {
    type Config: Clone;
    type Adapter: AttestationAdapter;

    /// Returns the name of the adapter for logging and observability purposes.
    fn adapter_name() -> &'static str;

    /// Build the specific attestor
    fn build(config: Self::Config) -> Result<Self::Adapter, AttestationAdapterError>;
}

#[async_trait::async_trait]
pub trait AttestationAdapter: Sync + Send + 'static {
    /// Fetch the height of the last finalized block. If there's no finalized
    /// block yet, it should return an error.
    async fn get_last_finalized_height(&self) -> Result<u64, AttestationAdapterError>;

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
