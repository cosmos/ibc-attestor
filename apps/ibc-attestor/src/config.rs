//! Defines the top level configuration for the attestor.
use std::{fs, net::SocketAddr, path::Path};

use serde::Deserialize;
use thiserror::Error;

/// The top level configuration for the attestor.
#[derive(Clone, Debug, Deserialize)]
pub struct AttestorConfig<A, S> {
    /// The configuration for the attestor server.
    pub server: ServerConfig,
    /// Signer configuration (generic over signer type) See:
    /// - [crate::signer::local::LocalSignerConfig] for local config options
    /// - [crate::signer::remote::RemoteSignerConfig] for remote config options
    pub signer: S,
    /// Adapter specific configuration
    pub adapter: A,
}

impl<A, S> AttestorConfig<A, S>
where
    A: for<'de> Deserialize<'de>,
    S: for<'de> Deserialize<'de>,
{
    /// Load an `AttestorConfig` from a TOML file on disk.
    ///
    /// Accepts any `P: AsRef<Path>` (e.g. &str, String, Path, PathBuf).
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let path_ref = path.as_ref();
        let contents = fs::read_to_string(path_ref)
            .map_err(|e| ConfigError::Io(path_ref.display().to_string(), e))?;
        let cfg = toml::from_str(&contents)?;
        Ok(cfg)
    }
}

/// The configuration for the relayer server.
#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    /// The address that the server should listen on.
    pub listen_addr: SocketAddr,
    /// The address that the health check server should listen on.
    /// Defaults to port 8081 on the same host as listen_addr if not specified.
    #[serde(default)]
    pub health_addr: Option<SocketAddr>,
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
}
