pub mod adapter;
pub mod attestation;
pub mod config;
pub mod logging;
pub mod rpc;
pub mod signer;

mod error;

// Proto definitions
pub mod proto {
    pub mod attestor {
        tonic::include_proto!("ibc_attestor");
    }

    pub mod signer {
        tonic::include_proto!("signerservice");
    }
}

use alloy_sol_types::SolType;
pub use error::AttestorError;
use ibc_eureka_solidity_types::ics26::IICS26RouterMsgs::Packet;

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
