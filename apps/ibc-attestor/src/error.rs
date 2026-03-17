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

    /// Failed to decode commitment type
    #[error("MalformedCommitmentError: {0}")]
    MalformedCommitmentError(#[from] prost::UnknownEnumValue),
}

impl From<AttestorError> for Status {
    fn from(value: AttestorError) -> Self {
        match value {
            AttestorError::BlockNotFinalized => {
                Self::new(Code::FailedPrecondition, value.to_string())
            }
            AttestorError::CommitmentNotFound { .. } => {
                Self::new(Code::NotFound, value.to_string())
            }
            AttestorError::InvalidCommitment { .. } => {
                Self::new(Code::InvalidArgument, value.to_string())
            }
            AttestorError::SignerError(_) | AttestorError::SignerInitError(_) => {
                Self::new(Code::Internal, value.to_string())
            }
            _ => Self::new(Code::Internal, value.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signer::SignerError;

    #[test]
    fn block_not_finalized_maps_to_failed_precondition() {
        let status = Status::from(AttestorError::BlockNotFinalized);
        assert_eq!(status.code(), Code::FailedPrecondition);
    }

    #[test]
    fn commitment_not_found_maps_to_not_found() {
        let status = Status::from(AttestorError::CommitmentNotFound {
            client_id: "client-a".to_string(),
            sequence: 1,
            height: 10,
        });
        assert_eq!(status.code(), Code::NotFound);
    }

    #[test]
    fn invalid_commitment_maps_to_invalid_argument() {
        let status = Status::from(AttestorError::InvalidCommitment {
            reason: "mismatch".to_string(),
        });
        assert_eq!(status.code(), Code::InvalidArgument);
    }

    #[test]
    fn signer_errors_map_to_internal() {
        let status = Status::from(AttestorError::SignerError("boom".to_string()));
        assert_eq!(status.code(), Code::Internal);

        let init_status = Status::from(AttestorError::SignerInitError(SignerError::ConfigError(
            "bad cfg".to_string(),
        )));
        assert_eq!(init_status.code(), Code::Internal);
    }
}
