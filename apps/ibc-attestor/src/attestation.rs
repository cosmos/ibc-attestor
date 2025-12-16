use tracing::{debug, error};

use crate::{signer::Signer, AttestorError};

/// Sign attestation data with the provided signer
///
/// Creates an ECDSA signature over the attested_data using the signer.
/// The signature can be verified on-chain to prove the attestor signed this data.
#[tracing::instrument(skip(attested_data, signer), fields(height, data_len = attested_data.len()))]
pub async fn sign_attestation(
    height: u64,
    timestamp: Option<u64>,
    attested_data: Vec<u8>,
    signer: &impl Signer,
) -> Result<SignedAttestation, AttestorError> {
    debug!(height, timestamp, data_len = attested_data.len(), "signing attestation");

    let signature = signer.sign(&attested_data).await.map_err(|e| {
        error!(
            height,
            error = %e,
            "failed to sign attestation"
        );
        AttestorError::SignerError(e.to_string())
    })?;
    let signature_bytes = signature.as_bytes().to_vec();

    debug!(
        height,
        signature_len = signature_bytes.len(),
        signature = %hex::encode(&signature_bytes),
        "attestation signed successfully"
    );

    Ok(SignedAttestation { height, timestamp, attested_data, signature: signature_bytes })
}

/// Signed attestation containing blockchain state data and cryptographic signature
pub struct SignedAttestation {
    /// Block height being attested
    pub height: u64,
    /// Optional block timestamp (for state attestations)
    pub timestamp: Option<u64>,
    /// ABI-encoded attestation data
    pub attested_data: Vec<u8>,
    /// 65-byte ECDSA signature (r: 32, s: 32, v: 1)
    pub signature: Vec<u8>,
}
