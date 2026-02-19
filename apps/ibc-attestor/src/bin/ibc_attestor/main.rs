use std::{env, fs, path::PathBuf};

use alloy_signer_local::PrivateKeySigner;
use clap::Parser;
use ethereum_keys::signer_local::{read_from_keystore, write_to_keystore};
use ibc_attestor::{
    adapter::{
        AdapterBuilder,
        cosmos::{CosmosAdapterBuilder, CosmosAdapterConfig},
        evm::{EvmAdapterBuilder, EvmAdapterConfig},
        solana::{SolanaAdapterBuilder, SolanaAdapterConfig},
    },
    config::AttestorConfig,
    logging::init_logging,
    rpc::{RpcError, server},
    signer::{
        SignerBuilder,
        local::{DEFAULT_KEYSTORE_NAME, LocalSigner, LocalSignerConfig},
        remote::{RemoteSigner, RemoteSignerConfig},
    },
};

use tokio::{
    signal::unix::{SignalKind, signal},
    sync::broadcast,
    task::JoinHandle,
};
use tracing::info;

use crate::cli::{AttestorCli, ChainType, Commands, SignerType, key::KeyCommands};

mod cli;

/// Default attestor dir
///
/// # Errors
///
/// Returns an error if the home directory cannot be determined
fn default_attestor_dir() -> Result<PathBuf, anyhow::Error> {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map_err(|_| anyhow::anyhow!("unable to determine home directory from environment"))?;
    Ok(PathBuf::from(home).join(".ibc-attestor"))
}

fn run_server_with_adapter_and_signer<B: AdapterBuilder, S: SignerBuilder>(
    config: AttestorConfig<B::Config, S::Config>,
    shutdown_rx: broadcast::Receiver<()>,
) -> Result<JoinHandle<Result<(), RpcError>>, anyhow::Error> {
    let adapter = B::build(config.adapter)?;
    let signer = S::build(config.signer)?;

    Ok(tokio::spawn(async move {
        // Start rpc server
        server::start(
            config.server.listen_addr,
            adapter,
            B::adapter_name(),
            signer,
            S::signer_name(),
            shutdown_rx,
        )
        .await
    }))
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let cli = AttestorCli::parse();

    match cli.command {
        Commands::Server(args) => {
            // Initialize logging
            init_logging();

            // Create shutdown broadcast channel
            let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

            let rpc_handle = match (args.chain_type, args.signer_type) {
                (ChainType::Evm, SignerType::Local) => {
                    let config = AttestorConfig::<EvmAdapterConfig, LocalSignerConfig>::from_file(
                        args.config,
                    )?;
                    run_server_with_adapter_and_signer::<EvmAdapterBuilder, LocalSigner>(
                        config,
                        shutdown_rx,
                    )?
                }
                (ChainType::Evm, SignerType::Remote) => {
                    let config = AttestorConfig::<EvmAdapterConfig, RemoteSignerConfig>::from_file(
                        args.config,
                    )?;
                    run_server_with_adapter_and_signer::<EvmAdapterBuilder, RemoteSigner>(
                        config,
                        shutdown_rx,
                    )?
                }
                (ChainType::Solana, SignerType::Local) => {
                    let config =
                        AttestorConfig::<SolanaAdapterConfig, LocalSignerConfig>::from_file(
                            args.config,
                        )?;
                    run_server_with_adapter_and_signer::<SolanaAdapterBuilder, LocalSigner>(
                        config,
                        shutdown_rx,
                    )?
                }
                (ChainType::Solana, SignerType::Remote) => {
                    let config =
                        AttestorConfig::<SolanaAdapterConfig, RemoteSignerConfig>::from_file(
                            args.config,
                        )?;
                    run_server_with_adapter_and_signer::<SolanaAdapterBuilder, RemoteSigner>(
                        config,
                        shutdown_rx,
                    )?
                }
                (ChainType::Cosmos, SignerType::Local) => {
                    let config =
                        AttestorConfig::<CosmosAdapterConfig, LocalSignerConfig>::from_file(
                            args.config,
                        )?;
                    run_server_with_adapter_and_signer::<CosmosAdapterBuilder, LocalSigner>(
                        config,
                        shutdown_rx,
                    )?
                }
                (ChainType::Cosmos, SignerType::Remote) => {
                    let config =
                        AttestorConfig::<CosmosAdapterConfig, RemoteSignerConfig>::from_file(
                            args.config,
                        )?;
                    run_server_with_adapter_and_signer::<CosmosAdapterBuilder, RemoteSigner>(
                        config,
                        shutdown_rx,
                    )?
                }
            };

            _ = wait_for_shutdown_signal().await;
            info!("shutdown signal received, starting graceful shutdown");
            let _ = shutdown_tx.send(());
            rpc_handle.await??;
        }
        Commands::Key(cmd) => {
            match cmd {
                KeyCommands::Generate(args) => {
                    let attestor_dir = match args.keystore {
                        Some(path) => path,
                        None => default_attestor_dir()?,
                    };
                    let keystore_path = attestor_dir.join(DEFAULT_KEYSTORE_NAME);

                    if !attestor_dir.exists() {
                        fs::create_dir_all(&attestor_dir)?;
                    }

                    if keystore_path.exists() {
                        return Err(anyhow::anyhow!(
                            "key pair already found at {keystore_path:?}; aborting"
                        ));
                    }

                    let signer = PrivateKeySigner::random();
                    write_to_keystore(&attestor_dir, DEFAULT_KEYSTORE_NAME, signer)
                        .map_err(|e| anyhow::anyhow!("unable to generate key {e}"))?;
                    println!("key successfully saved to {keystore_path:?}",);
                    Ok::<(), anyhow::Error>(())
                }
                KeyCommands::Show(args) => {
                    let attestor_dir = match args.keystore {
                        Some(path) => path,
                        None => default_attestor_dir()?,
                    };
                    let keystore_path = attestor_dir.join(DEFAULT_KEYSTORE_NAME);

                    let mut printed_any = false;

                    if args.show_private {
                        let signer = read_from_keystore(keystore_path.clone())?;
                        print!("{}", hex::encode(signer.credential().to_bytes()));
                        printed_any = true;
                    }

                    // Separate by newline
                    if printed_any {
                        println!("\n");
                    }

                    if args.show_public {
                        let signer = read_from_keystore(keystore_path)?;
                        let addr = signer.address();
                        print!("{}", hex::encode(addr.as_slice()));
                    }

                    Ok::<(), anyhow::Error>(())
                }
            }?
        }
    }
    Ok(())
}

/// Wait for a shutdown signal (SIGTERM or SIGINT).
///
/// # Panics
///
/// Panics if unable to register signal handlers, which indicates a critical system error.
async fn wait_for_shutdown_signal() {
    let mut signal_terminate = signal(SignalKind::terminate())
        .expect("failed to register SIGTERM handler - this is a critical system error");
    let mut signal_interrupt = signal(SignalKind::interrupt())
        .expect("failed to register SIGINT handler - this is a critical system error");

    tokio::select! {
        _ = signal_terminate.recv() => info!("received SIGTERM signal"),
        _ = signal_interrupt.recv() => info!("received SIGINT signal (Ctrl+C)"),
    };
}
