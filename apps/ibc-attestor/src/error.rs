use alloy::sol_types::Error as AbiError;
use std::fmt::Debug;
use thiserror::Error;
use tonic::{Code, Status};

use crate::adapter::AttestationAdapterError;
use crate::signer::SignerError;

/// Errors that can occur while working with attestor
#[derive(Debug, Error)]
pub enum AttestorError {
    /// Requested block is not finalized
    #[error("Block is not finalized")]
    BlockNotFinalized,

    /// Malformed commitment
    #[error("Packet commitment found but invalid due to: {reason}")]
    InvalidCommitment {
        /// Why commitment is bad
        reason: String,
    },

    /// Missing commitment
    #[error("Commitment not found client_id={client_id}, sequence={sequence} at height={height}")]
    CommitmentNotFound {
        /// Client Id
        client_id: String,
        /// Sequence ID
        sequence: u64,
        /// Block height
        height: u64,
    },

    /// Failed to sign data
    #[error("Failed to sign attestation due to: {0}")]
    SignerError(String),

    /// Failed to initialize signer
    #[error("Signer initialization failed: {0}")]
    SignerInitError(#[from] SignerError),

    /// Failed to de/encode as ABI
    #[error("AbiError: {0}")]
    AbiError(#[from] AbiError),

    /// Failed to retrieve data from adapter
    #[error("AdapterError: {0}")]
    AdapterError(#[from] AttestationAdapterError),
}

impl From<AttestorError> for Status {
    fn from(value: AttestorError) -> Self {
        match value {
            AttestorError::BlockNotFinalized => {
                Status::new(Code::FailedPrecondition, value.to_string())
            }
            AttestorError::CommitmentNotFound { .. } => {
                Status::new(Code::NotFound, value.to_string())
            }
            AttestorError::InvalidCommitment { .. } => {
                Status::new(Code::InvalidArgument, value.to_string())
            }
            AttestorError::SignerError(_) | AttestorError::SignerInitError(_) => {
                Status::new(Code::Internal, value.to_string())
            }
            _ => Status::new(Code::Internal, value.to_string()),
        }
    }
}
