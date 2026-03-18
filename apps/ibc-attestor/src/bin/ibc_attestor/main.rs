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
    config::{AttestorConfig, TracingConfig},
    logging::init_logging,
    rpc::{RpcError, health, server},
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

fn run_servers<B: AdapterBuilder + 'static, S: SignerBuilder + 'static>(
    config: AttestorConfig<B::Config, S::Config>,
    shutdown_tx: &broadcast::Sender<()>,
) -> Result<(JoinHandle<Result<(), RpcError>>, JoinHandle<()>), anyhow::Error> {
    let adapter = B::build(config.adapter)?;
    let signer = S::build(config.signer)?;
    let server_config = config.server;

    let grpc_shutdown_rx = shutdown_tx.subscribe();
    let health_shutdown_rx = shutdown_tx.subscribe();

    let grpc_addr = server_config.listen_addr;
    let health_addr = server_config.health_addr;

    let grpc_handle = tokio::spawn(async move {
        server::start(
            grpc_addr,
            adapter,
            B::adapter_name(),
            signer,
            S::signer_name(),
            grpc_shutdown_rx,
        )
        .await
    });

    let health_handle = tokio::spawn(async move {
        health::start(health_addr, grpc_addr, health_shutdown_rx).await;
    });

    Ok((grpc_handle, health_handle))
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let cli = AttestorCli::parse();

    match cli.command {
        Commands::Server(args) => {
            // Load tracing config first to initialize logging before parsing full config
            let tracing_config = TracingConfig::from_file(&args.config)?;
            let _tracing_guard = init_logging(tracing_config);

            // Create shutdown broadcast channel
            let (shutdown_tx, _shutdown_rx) = broadcast::channel(1);

            let (grpc_handle, health_handle) = match (args.chain_type, args.signer_type) {
                (ChainType::Evm, SignerType::Local) => {
                    let config = AttestorConfig::<EvmAdapterConfig, LocalSignerConfig>::from_file(
                        &args.config,
                    )?;
                    run_servers::<EvmAdapterBuilder, LocalSigner>(config, &shutdown_tx)?
                }
                (ChainType::Evm, SignerType::Remote) => {
                    let config = AttestorConfig::<EvmAdapterConfig, RemoteSignerConfig>::from_file(
                        &args.config,
                    )?;
                    run_servers::<EvmAdapterBuilder, RemoteSigner>(config, &shutdown_tx)?
                }
                (ChainType::Solana, SignerType::Local) => {
                    let config =
                        AttestorConfig::<SolanaAdapterConfig, LocalSignerConfig>::from_file(
                            &args.config,
                        )?;
                    run_servers::<SolanaAdapterBuilder, LocalSigner>(config, &shutdown_tx)?
                }
                (ChainType::Solana, SignerType::Remote) => {
                    let config =
                        AttestorConfig::<SolanaAdapterConfig, RemoteSignerConfig>::from_file(
                            &args.config,
                        )?;
                    run_servers::<SolanaAdapterBuilder, RemoteSigner>(config, &shutdown_tx)?
                }
                (ChainType::Cosmos, SignerType::Local) => {
                    let config =
                        AttestorConfig::<CosmosAdapterConfig, LocalSignerConfig>::from_file(
                            &args.config,
                        )?;
                    run_servers::<CosmosAdapterBuilder, LocalSigner>(config, &shutdown_tx)?
                }
                (ChainType::Cosmos, SignerType::Remote) => {
                    let config =
                        AttestorConfig::<CosmosAdapterConfig, RemoteSignerConfig>::from_file(
                            &args.config,
                        )?;
                    run_servers::<CosmosAdapterBuilder, RemoteSigner>(config, &shutdown_tx)?
                }
            };

            _ = wait_for_shutdown_signal().await;
            info!("shutdown signal received, starting graceful shutdown");
            let _ = shutdown_tx.send(());
            grpc_handle.await??;
            health_handle.await?;
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
