use std::time::Duration;

use alloy_primitives::Signature;
use async_trait::async_trait;
use tonic::transport::Endpoint;
use tracing::{Instrument, info, info_span};
use url::Url;

use super::{Signer, SignerBuilder, SignerError};
use crate::proto::signer::{
    GetWalletRequest, PubKeyType, RecoverableMessage, SignRequest,
    signer_service_client::SignerServiceClient,
};

/// Configuration for building a remote signer
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RemoteSignerConfig {
    /// gRPC endpoint (e.g., "<http://localhost:50051>")
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
    #[tracing::instrument(
        skip(self, message),
        fields(signer = "remote", walletId = %self.wallet_id, messageLen = message.len())
    )]
    async fn sign(&self, message: &[u8]) -> Result<Signature, SignerError> {
        const R_LEN: usize = 32;
        const S_LEN: usize = 32;
        const V_LEN: usize = 1;

        // Create a new client connection for this request
        let mut client = self
            .create_client()
            .instrument(info_span!("signer.connect"))
            .await?;

        // Fetch wallet information on each signing request
        let wallet_request = tonic::Request::new(GetWalletRequest {
            id: self.wallet_id.clone(),
            pubkey_type: PubKeyType::Ethereum as i32,
        });

        let wallet = client
            .get_wallet(wallet_request)
            .instrument(info_span!("signer.get_wallet"))
            .await
            .map_err(|e| SignerError::RemoteError(e.to_string()))?
            .into_inner()
            .wallet
            .ok_or_else(|| SignerError::RemoteError("wallet not found".to_string()))?;

        let request = tonic::Request::new(SignRequest {
            wallet_id: wallet.id,
            payload: Some(
                crate::proto::signer::sign_request::Payload::RecoverableMessage(
                    RecoverableMessage {
                        message: message.to_vec(),
                    },
                ),
            ),
        });

        let response = client
            .sign(request)
            .instrument(info_span!("signer.sign_rpc"))
            .await
            .map_err(|e| SignerError::RemoteError(e.to_string()))?;

        let signature = response
            .into_inner()
            .signature
            .ok_or_else(|| SignerError::RemoteError("no signature in response".to_string()))?;

        let crate::proto::signer::sign_response::Signature::RecoverableSignature(recoverable) =
            signature
        else {
            return Err(SignerError::InvalidSignature(
                "expected recoverable signature".to_string(),
            ));
        };

        if recoverable.r.len() != R_LEN
            || recoverable.s.len() != S_LEN
            || recoverable.v.len() != V_LEN
        {
            return Err(SignerError::InvalidSignature(format!(
                "expected r={R_LEN} s={S_LEN} v={V_LEN} bytes, got r={} s={} v={}",
                recoverable.r.len(),
                recoverable.s.len(),
                recoverable.v.len()
            )));
        }

        let mut signature_bytes = [0u8; R_LEN + S_LEN + V_LEN];
        signature_bytes[..R_LEN].copy_from_slice(&recoverable.r);
        signature_bytes[R_LEN..R_LEN + S_LEN].copy_from_slice(&recoverable.s);
        signature_bytes[R_LEN + S_LEN..].copy_from_slice(&recoverable.v);

        Signature::try_from(signature_bytes.as_slice())
            .map_err(|e| SignerError::InvalidSignature(e.to_string()))
    }
}
