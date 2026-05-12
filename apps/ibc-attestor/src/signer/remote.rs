use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use alloy_primitives::Signature;
use async_trait::async_trait;
use tonic::Status;
use tonic::metadata::{Ascii, MetadataValue};
use tonic::service::{Interceptor, interceptor::InterceptedService};
use tonic::transport::{Channel, Endpoint};
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
    /// Path to a file containing a bare JWT (no JSON envelope) — the format
    /// `kubernetes.io/service-account-token`-typed Secrets are populated in.
    ///
    /// When set, the file is read once at startup and attached as
    /// `Authorization: Bearer <token>` on every gRPC call to the signer.
    #[serde(default)]
    pub service_account_token_path: Option<PathBuf>,
}

/// Remote signer implementation using gRPC client
///
/// This signer connects to a remote signing service via gRPC to perform
/// cryptographic signing operations. The connection is created on-demand
/// for each signing request.
pub struct RemoteSigner {
    endpoint: Url,
    wallet_id: String,
    bearer_header: Option<MetadataValue<Ascii>>,
}

impl RemoteSigner {
    /// Create a new remote signer (does not connect until first use).
    ///
    /// If `service_account_token_path` is set, the file is read and parsed
    /// into a `Bearer` header value at construction time. Subsequent calls
    /// reuse the cached header.
    ///
    /// # Errors
    /// Returns [`SignerError::ConfigError`] if the token file cannot be read
    /// or contains bytes that aren't valid HTTP header characters.
    pub fn new(
        endpoint: Url,
        wallet_id: String,
        service_account_token_path: Option<PathBuf>,
    ) -> Result<Self, SignerError> {
        let bearer_header = service_account_token_path
            .map(|path| -> Result<MetadataValue<Ascii>, SignerError> {
                let token = fs::read_to_string(&path).map_err(|e| {
                    SignerError::ConfigError(format!(
                        "read service account token at {}: {e}",
                        path.display()
                    ))
                })?;
                format!("Bearer {}", token.trim())
                    .parse()
                    .map_err(|e| SignerError::ConfigError(format!("invalid token bytes: {e}")))
            })
            .transpose()?;

        info!(
            endpoint = %endpoint,
            walletId = %wallet_id,
            authEnabled = bearer_header.is_some(),
            "remote signer configured (connection deferred until first use)"
        );

        Ok(Self {
            endpoint,
            wallet_id,
            bearer_header,
        })
    }

    /// Create a new gRPC client connection
    async fn create_client(
        &self,
    ) -> Result<SignerServiceClient<InterceptedService<Channel, AuthInterceptor>>, SignerError>
    {
        let channel = Endpoint::from_shared(self.endpoint.to_string())
            .map_err(|e| SignerError::ConnectionError(e.to_string()))?
            .timeout(Duration::from_secs(30))
            .connect()
            .await
            .map_err(|e| SignerError::ConnectionError(e.to_string()))?;

        let interceptor = AuthInterceptor {
            bearer_header: self.bearer_header.clone(),
        };
        Ok(SignerServiceClient::with_interceptor(channel, interceptor))
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

/// gRPC interceptor that attaches a cached `Authorization: Bearer` header on
/// every outgoing request. When the configured header is `None` requests pass
/// through unchanged.
#[derive(Clone)]
pub struct AuthInterceptor {
    bearer_header: Option<MetadataValue<Ascii>>,
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, mut request: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
        if let Some(header) = &self.bearer_header {
            request
                .metadata_mut()
                .insert("authorization", header.clone());
        }
        Ok(request)
    }
}
