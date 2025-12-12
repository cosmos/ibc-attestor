//! Defines the client interface for the attestor server.
use clap::{Parser, ValueEnum};

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

/// The type of signer to use
#[derive(Clone, Debug, ValueEnum)]
pub enum SignerType {
    /// Local signer using keystore file
    Local,
    /// Remote signer using gRPC
    Remote,
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
    use super::{ChainType, Parser, SignerType};

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
    }
}

/// The arguments for the key subcommand.
pub mod key {
    use std::path::PathBuf;

    use super::Parser;

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
    }
}
