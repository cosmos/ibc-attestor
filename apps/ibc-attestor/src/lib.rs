#![deny(clippy::nursery, clippy::pedantic, warnings, missing_docs)]

//! IBC Attestor Library
//!
//! This library provides the core functionality for the IBC attestor service,
//! which generates cryptographic attestations of blockchain state for IBC operations.

/// Blockchain adapter implementations for different chain types
pub mod adapter;
/// Attestation signing and data structures
pub mod attestation;
/// Attestation payload and type separation for signatures
pub mod attestation_payload;
/// Configuration structures and loading
pub mod config;
/// Logging and observability setup
pub mod logging;
/// gRPC server and service implementations
pub mod rpc;
/// Signer implementations for local and remote signing
pub mod signer;

mod error;

/// Attestor and IBC proto definitions
pub mod proto {
    /// Attestor protos
    #[allow(clippy::nursery, clippy::pedantic, warnings, missing_docs)]
    pub mod attestor {
        tonic::include_proto!("ibc_attestor");
    }

    /// Remote signer protos
    #[allow(clippy::nursery, clippy::pedantic, warnings, missing_docs)]
    pub mod signer {
        tonic::include_proto!("signerservice");
    }
}

use alloy_sol_types::SolType;
pub use error::AttestorError;
use ibc_eureka_solidity_types::ics26::IICS26RouterMsgs::Packet;

/// A collection of IBC packets for batch attestation
pub struct Packets(Vec<Packet>);

impl Packets {
    fn try_from_abi_encoded(encoded: Vec<Vec<u8>>) -> Result<Self, AttestorError> {
        let packets = encoded
            .iter()
            .map(|p| Packet::abi_decode(p).map_err(AttestorError::AbiError))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self(packets))
    }

    /// Returns the number of packets
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if there are no packets
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl IntoIterator for Packets {
    type Item = Packet;
    type IntoIter = std::vec::IntoIter<Packet>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Packets {
    type Item = &'a Packet;
    type IntoIter = std::slice::Iter<'a, Packet>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
