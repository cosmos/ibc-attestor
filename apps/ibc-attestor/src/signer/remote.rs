use std::time::Duration;

use alloy_primitives::{Address, Signature};
use async_trait::async_trait;
use tonic::transport::Endpoint;
use tracing::{debug, info};
use url::Url;

use super::{Signer, SignerBuilder, SignerError};
use crate::proto::signer::{
    GetWalletRequest, PubKeyType, RawMessage, SignRequest,
    signer_service_client::SignerServiceClient,
};

/// Configuration for building a remote signer
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RemoteSignerConfig {
    /// gRPC endpoint (e.g., "http://localhost:50051")
    pub endpoint: Url,
    /// Wallet ID to use for signing
    pub wallet_id: String,
}

/// Remote signer implementation using gRPC client
///
/// This signer connects to a remote signing service via gRPC to perform
/// cryptographic signing operations. The connection is created on-demand
/// for each signing request.
pub struct RemoteSigner {
    endpoint: Url,
    wallet_id: String,
}

impl RemoteSigner {
    /// Create a new remote signer (does not connect until first use)
    pub fn new(endpoint: Url, wallet_id: String) -> Self {
        info!(
            endpoint = %endpoint,
            walletId = %wallet_id,
            "remote signer configured (connection deferred until first use)"
        );

        Self {
            endpoint,
            wallet_id,
        }
    }

    /// Create a new gRPC client connection
    async fn create_client(
        &self,
    ) -> Result<SignerServiceClient<tonic::transport::Channel>, SignerError> {
        let channel = Endpoint::from_shared(self.endpoint.to_string())
            .map_err(|e| SignerError::ConnectionError(e.to_string()))?
            .timeout(Duration::from_secs(30))
            .connect()
            .await
            .map_err(|e| SignerError::ConnectionError(e.to_string()))?;

        Ok(SignerServiceClient::new(channel))
    }
}

impl SignerBuilder for RemoteSigner {
    type Config = RemoteSignerConfig;
    type Signer = Self;

    fn signer_name() -> &'static str {
        "remote"
    }

    fn build(config: Self::Config) -> Result<Self::Signer, SignerError> {
        Ok(Self::new(config.endpoint, config.wallet_id))
    }
}

#[async_trait]
impl Signer for RemoteSigner {
    async fn sign(&self, message: &[u8]) -> Result<Signature, SignerError> {
        // Create a new client connection for this request
        let mut client = self.create_client().await?;

        // Fetch wallet information on each signing request
        let wallet_request = tonic::Request::new(GetWalletRequest {
            id: self.wallet_id.clone(),
            pubkey_type: PubKeyType::Ethereum as i32,
        });

        let wallet_response = client
            .get_wallet(wallet_request)
            .await
            .map_err(|e| SignerError::RemoteError(e.to_string()))?;

        let wallet = wallet_response
            .into_inner()
            .wallet
            .ok_or_else(|| SignerError::RemoteError("wallet not found".to_string()))?;

        let address = Address::from_raw_public_key(&wallet.pubkey);
        debug!(
            message_len = message.len(),
            wallet_id = %self.wallet_id,
            address = %address,
            "signing with remote signer"
        );

        let request = tonic::Request::new(SignRequest {
            wallet_id: self.wallet_id.clone(),
            payload: Some(crate::proto::signer::sign_request::Payload::RawMessage(
                RawMessage {
                    message: message.to_vec(),
                },
            )),
        });

        let response = client
            .sign(request)
            .await
            .map_err(|e| SignerError::RemoteError(e.to_string()))?;

        let signature = response
            .into_inner()
            .signature
            .ok_or_else(|| SignerError::RemoteError("no signature in response".to_string()))?;

        // Extract raw signature bytes
        let signature_bytes = match signature {
            crate::proto::signer::sign_response::Signature::RawSignature(raw) => raw.signature,
            _ => {
                return Err(SignerError::InvalidSignature(
                    "expected raw signature".to_string(),
                ));
            }
        };

        // Convert to 65-byte Signature
        if signature_bytes.len() != 65 {
            return Err(SignerError::InvalidSignature(format!(
                "expected 65 bytes, got {}",
                signature_bytes.len()
            )));
        }

        Signature::try_from(signature_bytes.as_slice())
            .map_err(|e| SignerError::InvalidSignature(e.to_string()))
    }
}
