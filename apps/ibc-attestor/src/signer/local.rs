use std::path::PathBuf;

use alloy_primitives::Signature;
use alloy_signer_local::PrivateKeySigner;
use async_trait::async_trait;
use ethereum_keys::{signature::sign as sync_sign, signer_local::read_from_keystore};
use tracing::info;

use super::{Signer, SignerBuilder, SignerError};

/// Default keystore name
pub const DEFAULT_KEYSTORE_NAME: &str = "ibc-attestor-keystore";

/// Configuration for building a local signer
#[derive(Clone, Debug, serde::Deserialize)]
pub struct LocalSignerConfig {
    /// Path to keystore file or directory
    pub keystore_path: PathBuf,
}

/// Local signer implementation using PrivateKeySigner
///
/// Wraps the existing synchronous signing logic in an async interface
pub struct LocalSigner {
    inner: PrivateKeySigner,
}

impl LocalSigner {
    /// Creates a new instance of [`LocalSigner`]
    pub fn new(signer: PrivateKeySigner) -> Self {
        Self { inner: signer }
    }
}

impl SignerBuilder for LocalSigner {
    type Config = LocalSignerConfig;
    type Signer = Self;

    fn signer_name() -> &'static str {
        "local"
    }

    fn build(config: Self::Config) -> Result<Self::Signer, SignerError> {
        let keystore_path_with_file = if config.keystore_path.is_dir() {
            config.keystore_path.join(DEFAULT_KEYSTORE_NAME)
        } else {
            config.keystore_path
        };

        let with_expanded_home = if keystore_path_with_file.starts_with("~/") {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .map_err(|_| {
                    SignerError::ConfigError(
                        "unable to determine home directory from environment".to_string(),
                    )
                })?;
            keystore_path_with_file
                .to_string_lossy()
                .replace("~", &home)
        } else {
            keystore_path_with_file.to_string_lossy().to_string()
        };

        info!(keystorePath = %with_expanded_home, "initalizing local signer");

        let private_key_signer = read_from_keystore(PathBuf::from(with_expanded_home.clone()))
            .map_err(|e| SignerError::ConfigError(e.to_string()))?;

        info!(
            keystorePath = %with_expanded_home,
            "local signer initialized successfully"
        );

        Ok(Self::new(private_key_signer))
    }
}

#[async_trait]
impl Signer for LocalSigner {
    async fn sign(&self, message: &[u8]) -> Result<Signature, SignerError> {
        // Call the existing sync signing function
        sync_sign(&self.inner, message).map_err(|e| SignerError::LocalError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_local_signer_sign() {
        let private_key_signer = PrivateKeySigner::random();
        let signer = LocalSigner::new(private_key_signer.clone());
        let message = b"test message";

        let signature = signer.sign(message).await.unwrap();
        assert_eq!(signature.as_bytes().len(), 65);
    }

    #[tokio::test]
    async fn test_local_signer_deterministic() {
        let private_key_signer = PrivateKeySigner::random();
        let signer = LocalSigner::new(private_key_signer);
        let message = b"test message";

        let sig1 = signer.sign(message).await.unwrap();
        let sig2 = signer.sign(message).await.unwrap();
        assert_eq!(sig1, sig2);
    }
}
