//! Defines the client interface for the attestor server.
use clap::{Args, Parser, ValueEnum};
use ibc_attestor::config;

/// The type of blockchain adapter to use
#[derive(Clone, Debug, ValueEnum)]
pub enum ChainType {
    /// Ethereum Virtual Machine compatible chains
    Evm,
    /// Solana blockchain
    Solana,
    /// Cosmos SDK based chains
    Cosmos,
}

impl From<ChainType> for config::ChainType {
    fn from(ct: ChainType) -> Self {
        match ct {
            ChainType::Evm => Self::Evm,
            ChainType::Solana => Self::Solana,
            ChainType::Cosmos => Self::Cosmos,
        }
    }
}

/// The type of signer to use
#[derive(Clone, Debug, ValueEnum)]
pub enum SignerType {
    /// Local signer using keystore file
    Local,
    /// Remote signer using gRPC
    Remote,
}

impl From<SignerType> for config::SignerType {
    fn from(st: SignerType) -> Self {
        match st {
            SignerType::Local => Self::Local,
            SignerType::Remote => Self::Remote,
        }
    }
}

#[derive(Clone, Args)]
pub struct KeystorePasswordArgs {
    /// Password for the keystore. Prefer the interactive prompt or IBC_ATTESTOR_KEYSTORE_PASSWORD; this value is visible in process listings.
    #[clap(long, conflicts_with = "empty_keystore_password")]
    pub keystore_password: Option<String>,

    /// Use an empty keystore password.
    #[clap(long, conflicts_with = "keystore_password", default_value = "false")]
    pub empty_keystore_password: bool,
}

impl std::fmt::Debug for KeystorePasswordArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeystorePasswordArgs")
            .field(
                "keystore_password",
                &self.keystore_password.as_ref().map(|_| "***"),
            )
            .field("empty_keystore_password", &self.empty_keystore_password)
            .finish()
    }
}

impl KeystorePasswordArgs {
    pub const fn has_explicit_password_source(&self) -> bool {
        self.keystore_password.is_some() || self.empty_keystore_password
    }
}

#[derive(Clone, Debug, Parser)]
#[command(
    name = "ibc_attestor",
    version,
    about = "IBC Attestor - Blockchain state attestation service",
    long_about = "A service for generating cryptographic attestations of blockchain state.\nSupports key management and running attestation servers."
)]
/// The command line interface for the attestor.
pub struct AttestorCli {
    /// The subcommand to run.
    #[command(subcommand)]
    pub command: Commands,
}

/// The subcommands for the attestor.
#[derive(Clone, Debug, Parser)]
pub enum Commands {
    /// The subcommand to run the server.
    Server(server::Args),

    /// The subcommand to run key management program.
    #[command(subcommand)]
    Key(key::KeyCommands),
}

/// The arguments for the start subcommand.
pub mod server {
    use super::{ChainType, KeystorePasswordArgs, Parser, SignerType};

    /// The arguments for the server subcommand.
    #[derive(Clone, Debug, Parser)]
    pub struct Args {
        /// The configuration file for the attestor.
        #[clap(long)]
        pub config: String,

        /// The type of blockchain adapter to use.
        #[clap(long, value_enum)]
        pub chain_type: ChainType,

        /// The type of signer to use.
        #[clap(long, value_enum, default_value = "local")]
        pub signer_type: SignerType,

        /// Run as a HashiCorp go-plugin: bind the gRPC server to an ephemeral
        /// port, serve the gRPC health service, and print the go-plugin
        /// handshake line on stdout (logs are redirected to stderr).
        #[clap(long, default_value = "false")]
        pub plugin_mode: bool,

        /// Local keystore password source.
        #[command(flatten)]
        pub keystore_password: KeystorePasswordArgs,
    }
}

/// The arguments for the key subcommand.
pub mod key {
    use std::path::PathBuf;

    use super::{KeystorePasswordArgs, Parser};

    #[derive(Clone, Debug, Parser)]
    pub enum KeyCommands {
        Generate(GenerateArgs),
        Show(ShowArgs),
    }

    #[derive(Clone, Debug, Parser)]
    pub struct GenerateArgs {
        /// Custom keystore directory path. If not specified, uses ~/.ibc-attestor/
        #[clap(long)]
        pub keystore: Option<PathBuf>,

        /// Generated keystore password source.
        #[command(flatten)]
        pub keystore_password: KeystorePasswordArgs,
    }

    #[derive(Clone, Debug, Parser)]
    pub struct ShowArgs {
        #[clap(long, default_value = "false")]
        pub show_private: bool,
        #[clap(long, default_value = "true")]
        pub show_public: bool,
        /// Custom keystore directory path. If not specified, uses ~/.ibc-attestor/
        #[clap(long)]
        pub keystore: Option<PathBuf>,

        /// Keystore password source.
        #[command(flatten)]
        pub keystore_password: KeystorePasswordArgs,
    }
}
