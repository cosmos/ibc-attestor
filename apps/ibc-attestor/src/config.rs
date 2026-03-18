//! Defines the top level configuration for the attestor.
use std::{fs, net::SocketAddr, path::Path};

use serde::Deserialize;
use thiserror::Error;
use url::Url;

use crate::adapter::{
    AdapterBuilder, AdapterEnum, AttestationAdapterError,
    cosmos::{CosmosAdapterBuilder, CosmosAdapterConfig},
    evm::{EvmAdapterBuilder, EvmAdapterConfig},
    solana::{SolanaAdapterBuilder, SolanaAdapterConfig},
};
use crate::signer::{
    SignerBuilder, SignerEnum, SignerError,
    local::{LocalSigner, LocalSignerConfig},
    remote::{RemoteSigner, RemoteSignerConfig},
};

/// The type of blockchain adapter to use.
#[derive(Clone, Debug)]
pub enum ChainType {
    /// Ethereum Virtual Machine compatible chains
    Evm,
    /// Solana blockchain
    Solana,
    /// Cosmos SDK based chains
    Cosmos,
}

/// The type of signer to use.
#[derive(Clone, Debug)]
pub enum SignerType {
    /// Local signer using keystore file
    Local,
    /// Remote signer using gRPC
    Remote,
}

/// Concrete top-level configuration using enum-based dispatch.
///
/// The adapter and signer types are determined by CLI arguments, while the
/// concrete configuration values are read from the TOML file.
pub struct RuntimeConfig {
    /// The configuration for the attestor server.
    pub server: ServerConfig,
    /// The built adapter instance.
    pub adapter: AdapterEnum,
    /// The built signer instance.
    pub signer: SignerEnum,
    /// Optional tracing configuration for OpenTelemetry export.
    pub tracing: Option<TracingConfig>,
}

/// Raw TOML structure used for partial deserialization. The `adapter` and
/// `signer` sections are stored as raw TOML values so they can be
/// deserialized into the correct concrete type based on CLI arguments.
#[derive(Deserialize)]
struct RawConfig {
    server: ServerConfig,
    adapter: toml::Value,
    signer: toml::Value,
    tracing: Option<TracingConfig>,
}

impl RuntimeConfig {
    /// Load a `RuntimeConfig` from a TOML file, using the provided CLI
    /// arguments to determine which adapter and signer types to deserialize.
    ///
    /// # Errors
    /// Returns [`ConfigError`] if the file cannot be read, the TOML is
    /// invalid, or the adapter/signer sections don't match the expected type.
    pub fn from_file<P: AsRef<Path>>(
        path: P,
        chain_type: &ChainType,
        signer_type: &SignerType,
    ) -> Result<Self, ConfigError> {
        let path_ref = path.as_ref();
        let contents = fs::read_to_string(path_ref)
            .map_err(|e| ConfigError::Io(path_ref.display().to_string(), e))?;
        let raw: RawConfig = toml::from_str(&contents)?;

        let adapter = match chain_type {
            ChainType::Evm => {
                let config: EvmAdapterConfig = raw.adapter.try_into()?;
                EvmAdapterBuilder::build(config).map(AdapterEnum::Evm)
            }
            ChainType::Solana => {
                let config: SolanaAdapterConfig = raw.adapter.try_into()?;
                SolanaAdapterBuilder::build(config).map(AdapterEnum::Solana)
            }
            ChainType::Cosmos => {
                let config: CosmosAdapterConfig = raw.adapter.try_into()?;
                CosmosAdapterBuilder::build(config).map(AdapterEnum::Cosmos)
            }
        }
        .map_err(ConfigError::Adapter)?;

        let signer = match signer_type {
            SignerType::Local => {
                let config: LocalSignerConfig = raw.signer.try_into()?;
                <LocalSigner as SignerBuilder>::build(config).map(SignerEnum::Local)
            }
            SignerType::Remote => {
                let config: RemoteSignerConfig = raw.signer.try_into()?;
                <RemoteSigner as SignerBuilder>::build(config).map(SignerEnum::Remote)
            }
        }
        .map_err(ConfigError::Signer)?;

        let tracing = raw.tracing.map(TracingConfig::validate).transpose()?;

        Ok(Self {
            server: raw.server,
            adapter,
            signer,
            tracing,
        })
    }
}

/// The configuration for the relayer server.
#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    /// The address that the server should listen on.
    pub listen_addr: SocketAddr,
}

/// Configuration for OpenTelemetry tracing export.
///
/// All fields are required. If this section is present in the config,
/// trace export will be enabled to the specified OTLP endpoint.
#[derive(Clone, Debug, Deserialize)]
pub struct TracingConfig {
    /// The OTLP endpoint to export traces to (e.g., `http://tempo:4317`).
    pub otlp_endpoint: Url,
    /// The service name to use in traces.
    pub service_name: String,
    /// The sampling ratio for traces (0.0 to 1.0). Set to 1.0 to sample all traces.
    pub sample_rate: f64,
}

impl TracingConfig {
    fn validate(self) -> Result<Self, ConfigError> {
        if !self.sample_rate.is_finite() || !(0.0..=1.0).contains(&self.sample_rate) {
            return Err(ConfigError::InvalidTracingConfig(format!(
                "`tracing.sample_rate` must be a finite value in [0.0, 1.0], got {}",
                self.sample_rate
            )));
        }

        Ok(self)
    }
}

/// Errors that can occur loading the attestor config.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Missing or invalid file paths
    #[error("I/O error reading `{0}`: {1}")]
    Io(String, #[source] std::io::Error),

    /// Malformed toml
    #[error("invalid TOML in config: {0}")]
    Toml(#[from] toml::de::Error),

    /// Invalid tracing section values
    #[error("invalid tracing config: {0}")]
    InvalidTracingConfig(String),

    /// Adapter build failure
    #[error(transparent)]
    Adapter(AttestationAdapterError),

    /// Signer build failure
    #[error(transparent)]
    Signer(SignerError),
}
