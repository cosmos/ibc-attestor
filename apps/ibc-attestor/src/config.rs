//! Defines the top level configuration for the attestor.
use std::{fs, net::SocketAddr, path::Path};

use serde::Deserialize;
use thiserror::Error;
use url::Url;

/// The top level configuration for the attestor.
#[derive(Clone, Debug, Deserialize)]
pub struct AttestorConfig<A, S> {
    /// The configuration for the attestor server.
    pub server: ServerConfig,
    /// Signer configuration (generic over signer type) See:
    /// - [`crate::signer::local::LocalSignerConfig`] for local config options
    /// - [`crate::signer::remote::RemoteSignerConfig`] for remote config options
    pub signer: S,
    /// Adapter specific configuration
    pub adapter: A,
    /// Optional tracing configuration for OpenTelemetry export.
    /// If provided, all fields are required and trace export will be enabled.
    pub tracing: Option<TracingConfig>,
}

impl<A, S> AttestorConfig<A, S>
where
    A: for<'de> Deserialize<'de>,
    S: for<'de> Deserialize<'de>,
{
    /// Load an `AttestorConfig` from a TOML file on disk.
    ///
    /// Accepts any `P: AsRef<Path>` (e.g. &str, String, Path, `PathBuf`).
    ///
    /// # Errors
    /// Returns [`ConfigError::Io`] if the file cannot be read, or
    /// [`ConfigError::Parse`] if the TOML is invalid.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let path_ref = path.as_ref();
        let contents = fs::read_to_string(path_ref)
            .map_err(|e| ConfigError::Io(path_ref.display().to_string(), e))?;
        let mut cfg: Self = toml::from_str(&contents)?;
        cfg.tracing = cfg
            .tracing
            .map(TracingConfig::validate)
            .transpose()?;
        Ok(cfg)
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

#[derive(Deserialize)]
struct PartialConfig {
    tracing: Option<TracingConfig>,
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

    /// Load just the tracing configuration from a TOML file.
    ///
    /// This allows initializing the tracer before parsing the full config,
    /// which may require type parameters for adapter/signer configurations.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Io`] if the file cannot be read, or
    /// [`ConfigError::Toml`] if the file contains invalid TOML.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Option<Self>, ConfigError> {
        let path_ref = path.as_ref();
        let contents = fs::read_to_string(path_ref)
            .map_err(|e| ConfigError::Io(path_ref.display().to_string(), e))?;
        let partial: PartialConfig = toml::from_str(&contents)?;
        partial.tracing.map(Self::validate).transpose()
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
}
