use std::path::{Path, PathBuf};
use std::time::Duration;

use alloy_primitives::Signature;
use async_trait::async_trait;
use tonic::metadata::{Ascii, MetadataValue};
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tracing::{Instrument, info, info_span, warn};
use url::Url;

use super::{Signer, SignerBuilder, SignerError};
use crate::proto::signer::{
    RecoverableMessage, SignRequest, signer_service_client::SignerServiceClient,
};

/// Configuration for building a remote signer
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RemoteSignerConfig {
    /// gRPC endpoint (e.g., "<https://remote-signer.example:50051>")
    pub endpoint: Url,
    /// Wallet ID to use for signing
    pub wallet_id: String,
    /// Allow plaintext gRPC for non-production test environments.
    #[serde(default)]
    pub allow_insecure_plaintext: bool,
    /// Path to a file containing a bare JWT (no JSON envelope) — the format
    /// `kubernetes.io/service-account-token`-typed Secrets are populated in.
    ///
    /// When set, the file is read asynchronously on each signing request and
    /// attached as `Authorization: Bearer <token>`.
    #[serde(default)]
    pub service_account_token_path: Option<PathBuf>,
}

impl RemoteSignerConfig {
    fn validate_endpoint_security(&self) -> Result<(), SignerError> {
        match self.endpoint.scheme() {
            "https" => Ok(()),
            "http" if self.allow_insecure_plaintext => {
                warn!(
                    endpoint = %self.endpoint,
                    "remote signer plaintext transport is enabled; this is intended only for tests"
                );
                Ok(())
            }
            "http" => Err(SignerError::ConfigError(
                "remote signer endpoint must use https://; set signer.allow_insecure_plaintext = true only for tests"
                    .to_string(),
            )),
            scheme => Err(SignerError::ConfigError(format!(
                "unsupported remote signer endpoint scheme `{scheme}`; expected https://"
            ))),
        }
    }
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
    ///
    /// # Errors
    /// Returns [`SignerError::ConnectionError`] if `endpoint` is not a valid
    /// gRPC URI accepted by `tonic::transport::Endpoint`.
    pub async fn new(
        endpoint: Url,
        wallet_id: String,
        allow_insecure_plaintext: bool,
        service_account_token_path: Option<PathBuf>,
    ) -> Result<Self, SignerError> {
        let config = RemoteSignerConfig {
            endpoint,
            wallet_id,
            allow_insecure_plaintext,
            service_account_token_path,
        };
        config.validate_endpoint_security()?;

        info!(
            endpoint = %config.endpoint,
            walletId = %config.wallet_id,
            authEnabled = config.service_account_token_path.is_some(),
            "connecting remote signer"
        );

        let endpoint = Endpoint::from_shared(config.endpoint.to_string())
            .map_err(|e| SignerError::ConnectionError(e.to_string()))?
            .timeout(Duration::from_secs(30));

        let endpoint = if config.endpoint.scheme() == "https" {
            endpoint
                .tls_config(ClientTlsConfig::new().with_enabled_roots())
                .map_err(|e| SignerError::ConnectionError(e.to_string()))?
        } else {
            endpoint
        };

        let channel = endpoint
            .connect()
            .await
            .map_err(|e| SignerError::ConnectionError(e.to_string()))?;

        Ok(Self {
            wallet_id: config.wallet_id,
            client: SignerServiceClient::new(channel),
            service_account_token_path: config.service_account_token_path,
        })
    }
}

#[async_trait]
impl SignerBuilder for RemoteSigner {
    type Config = RemoteSignerConfig;
    type Signer = Self;

    fn signer_name() -> &'static str {
        "remote"
    }

    async fn build(config: Self::Config) -> Result<Self::Signer, SignerError> {
        Self::new(
            config.endpoint,
            config.wallet_id,
            config.allow_insecure_plaintext,
            config.service_account_token_path,
        )
        .await
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

        let signature_bytes: [u8; R_LEN + S_LEN + V_LEN] =
            [recoverable.r, recoverable.s, recoverable.v]
                .into_iter()
                .flatten()
                .collect::<Vec<u8>>()
                .try_into()
                .map_err(|_| {
                    SignerError::InvalidSignature("invalid signature byte lengths".to_string())
                })?;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn config(endpoint: &str, allow_insecure_plaintext: bool) -> RemoteSignerConfig {
        RemoteSignerConfig {
            endpoint: endpoint.parse().expect("valid URL"),
            wallet_id: "test-wallet".to_string(),
            allow_insecure_plaintext,
            service_account_token_path: None,
        }
    }

    #[test]
    fn accepts_https_endpoint() {
        config("https://remote-signer.example:50051", false)
            .validate_endpoint_security()
            .expect("https endpoint should be accepted");
    }

    #[test]
    fn rejects_http_endpoint_by_default() {
        let err = config("http://remote-signer.example:50051", false)
            .validate_endpoint_security()
            .expect_err("http endpoint should require explicit opt-in");

        assert!(matches!(err, SignerError::ConfigError(_)));
    }

    #[test]
    fn accepts_http_endpoint_with_explicit_plaintext_opt_in() {
        config("http://remote-signer.example:50051", true)
            .validate_endpoint_security()
            .expect("explicit plaintext opt-in should be accepted");
    }

    #[test]
    fn rejects_unsupported_endpoint_scheme() {
        let err = config("ftp://remote-signer.example:50051", true)
            .validate_endpoint_security()
            .expect_err("unsupported scheme should be rejected");

        assert!(matches!(err, SignerError::ConfigError(_)));
    }
}
