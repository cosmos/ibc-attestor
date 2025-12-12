use alloy_primitives::Signature;
use async_trait::async_trait;

pub mod local;
pub mod remote;

/// Trait for signing attestation data
///
/// This trait abstracts over local and remote signing implementations,
/// allowing the attestor to use either a local keystore or a remote
/// signing service.
#[async_trait]
pub trait Signer: Send + Sync + 'static {
    /// Sign a message and return the signature
    ///
    /// # Arguments
    /// * `message` - Raw bytes to sign (will be SHA-256 hashed)
    ///
    /// # Returns
    /// * `Signature` - 65-byte ECDSA signature (r: 32, s: 32, v: 1)
    async fn sign(&self, message: &[u8]) -> Result<Signature, SignerError>;
}

/// Trait for building signer implementations
///
/// This trait provides a generic interface for constructing signers,
/// similar to the AdapterBuilder pattern used for attestation adapters.
pub trait SignerBuilder {
    type Config: Clone + Send + 'static;
    type Signer: Signer;

    /// Returns the name of the signer for logging and observability purposes.
    fn signer_name() -> &'static str;

    /// Build the specific signer implementation
    fn build(config: Self::Config) -> Result<Self::Signer, SignerError>;
}

/// Errors that can occur during signing operations
#[derive(Debug, thiserror::Error)]
pub enum SignerError {
    #[error("Local signing error: {0}")]
    LocalError(String),

    #[error("Remote signing error: {0}")]
    RemoteError(String),

    #[error("Connection failed: {0}")]
    ConnectionError(String),

    #[error("Invalid signature format: {0}")]
    InvalidSignature(String),

    #[error("Failed to build signer due to: {0}")]
    ConfigError(String),
}
