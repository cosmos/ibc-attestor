use alloy::sol_types::Error as AbiError;
use std::fmt::Debug;
use thiserror::Error;
use tonic::{Code, Status};

use crate::adapter::AttestationAdapterError;
use crate::signer::SignerError;

/// Errors that can occur while working with attestor
#[derive(Debug, Error)]
pub enum AttestorError {
    #[error("Block is not finalized")]
    BlockNotFinalized,

    #[error("Packet commitment found but invalid due to: {reason}")]
    InvalidCommitment { reason: String },

    #[error("Commitment not found client_id={client_id}, sequence={sequence} at height={height}")]
    CommitmentNotFound {
        client_id: String,
        sequence: u64,
        height: u64,
    },

    #[error("Failed to sign attestation due to: {0}")]
    SignerError(String),

    #[error("Signer initialization failed: {0}")]
    SignerInitError(#[from] SignerError),

    #[error("AbiError: {0}")]
    AbiError(#[from] AbiError),

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
