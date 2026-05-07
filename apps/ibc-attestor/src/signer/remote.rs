use std::path::{Path, PathBuf};
use std::time::Duration;

use alloy_primitives::Signature;
use async_trait::async_trait;
use tonic::metadata::{Ascii, MetadataValue};
use tonic::transport::{Channel, Endpoint};
use tracing::{Instrument, info, info_span};
use url::Url;

use super::{Signer, SignerBuilder, SignerError};
use crate::proto::signer::{
    RecoverableMessage, SignRequest, signer_service_client::SignerServiceClient,
};

/// Configuration for building a remote signer
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RemoteSignerConfig {
    /// gRPC endpoint (e.g., "<http://localhost:50051>")
    pub endpoint: Url,
    /// Wallet ID to use for signing
    pub wallet_id: String,
    /// Path to a file containing a bare JWT (no JSON envelope) — the format
    /// `kubernetes.io/service-account-token`-typed Secrets are populated in.
    ///
    /// When set, the file is read asynchronously on each signing request and
    /// attached as `Authorization: Bearer <token>`.
    #[serde(default)]
    pub service_account_token_path: Option<PathBuf>,
}

/// Remote signer implementation using gRPC client
///
/// The gRPC channel is created once and shared across signing requests.
pub struct RemoteSigner {
    wallet_id: String,
    client: SignerServiceClient<Channel>,
    service_account_token_path: Option<PathBuf>,
}

impl RemoteSigner {
    /// Create a new remote signer (does not connect until first use)
    pub fn new(
        endpoint: Url,
        wallet_id: String,
        service_account_token_path: Option<PathBuf>,
    ) -> Result<Self, SignerError> {
        info!(
            endpoint = %endpoint,
            walletId = %wallet_id,
            authEnabled = service_account_token_path.is_some(),
            "remote signer configured (connection deferred until first use)"
        );

        let channel = Endpoint::from_shared(endpoint.to_string())
            .map_err(|e| SignerError::ConnectionError(e.to_string()))?
            .timeout(Duration::from_secs(30))
            .connect_lazy();

        Ok(Self {
            wallet_id,
            client: SignerServiceClient::new(channel),
            service_account_token_path,
        })
    }
}

impl SignerBuilder for RemoteSigner {
    type Config = RemoteSignerConfig;
    type Signer = Self;

    fn signer_name() -> &'static str {
        "remote"
    }

    fn build(config: Self::Config) -> Result<Self::Signer, SignerError> {
        Self::new(
            config.endpoint,
            config.wallet_id,
            config.service_account_token_path,
        )
    }
}

#[async_trait]
impl Signer for RemoteSigner {
    #[tracing::instrument(
        skip(self, message),
        fields(signer = "remote", walletId = %self.wallet_id, messageLen = message.len())
    )]
    async fn sign(&self, message: &[u8]) -> Result<Signature, SignerError> {
        let mut request = tonic::Request::new(SignRequest {
            wallet_id: self.wallet_id.clone(),
            payload: Some(
                crate::proto::signer::sign_request::Payload::RecoverableMessage(
                    RecoverableMessage {
                        message: message.to_vec(),
                    },
                ),
            ),
        });

        if let Some(path) = &self.service_account_token_path {
            let bearer = load_bearer_header(path).await?;
            request.metadata_mut().insert("authorization", bearer);
        }

        let response = self
            .client
            .clone()
            .sign(request)
            .instrument(info_span!("signer.sign_rpc"))
            .await
            .map_err(|e| SignerError::RemoteError(e.to_string()))?;

        let signature = response
            .into_inner()
            .signature
            .ok_or_else(|| SignerError::RemoteError("no signature in response".to_string()))?;

        // Extract raw signature bytes
        let signature_bytes: Vec<_> = match signature {
            crate::proto::signer::sign_response::Signature::RecoverableSignature(recoverable) => {
                [recoverable.r, recoverable.s, recoverable.v]
                    .into_iter()
                    .flatten()
                    .collect()
            }
            _ => {
                return Err(SignerError::InvalidSignature(
                    "expected resoverable signature".to_string(),
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

/// Read a Kubernetes `ServiceAccount` token from disk and parse it into a
/// `Bearer` header value.
async fn load_bearer_header(path: &Path) -> Result<MetadataValue<Ascii>, SignerError> {
    let token = tokio::fs::read_to_string(path).await.map_err(|e| {
        SignerError::ConfigError(format!(
            "read service account token at {}: {e}",
            path.display()
        ))
    })?;
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(SignerError::ConfigError(format!(
            "service account token file at {} is empty",
            path.display()
        )));
    }
    format!("Bearer {trimmed}")
        .parse()
        .map_err(|e| SignerError::ConfigError(format!("invalid token bytes: {e}")))
}
