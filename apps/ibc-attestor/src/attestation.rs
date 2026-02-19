use tracing::{debug, error};

pub use crate::attestation_payload::{AttestationPayload, AttestationType};
use crate::{AttestorError, signer::Signer};

/// Sign attestation data with domain separation.
///
/// The signature is computed over `sha256(type_tag || sha256(attested_data))`
/// to prevent cross-protocol replay between state and packet attestations.
#[tracing::instrument(skip(payload, signer), fields(height, attestation_type = ?payload.attestation_type(), data_len = payload.data().len()))]
pub async fn sign_attestation(
    height: u64,
    timestamp: Option<u64>,
    payload: AttestationPayload,
    signer: &impl Signer,
) -> Result<SignedAttestation, AttestorError> {
    debug!(height, timestamp, "signing attestation");

    let tagged_signing_input = payload.tagged_signing_input();
    let signature = signer.sign(&tagged_signing_input).await.map_err(|e| {
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

    Ok(SignedAttestation {
        height,
        timestamp,
        attested_data: payload.into_data(),
        signature: signature_bytes,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_signer_local::PrivateKeySigner;
    use ethereum_keys::recover::recover_address;

    use crate::signer::local::LocalSigner;

    fn test_signer() -> LocalSigner {
        LocalSigner::new(PrivateKeySigner::random())
    }

    #[tokio::test]
    async fn sign_attestation_produces_65_byte_signature() {
        let signer = test_signer();
        let tagged = AttestationPayload::new(b"data".to_vec(), AttestationType::State);
        let result = sign_attestation(100, Some(1_700_000_000), tagged, &signer)
            .await
            .unwrap();
        assert_eq!(result.signature.len(), 65);
    }

    #[tokio::test]
    async fn sign_attestation_preserves_original_attested_data() {
        let signer = test_signer();
        let original_data = b"original abi-encoded data".to_vec();
        let tagged = AttestationPayload::new(original_data.clone(), AttestationType::Packet);
        let result = sign_attestation(42, None, tagged, &signer).await.unwrap();
        assert_eq!(result.attested_data, original_data);
    }

    #[tokio::test]
    async fn signature_recovers_to_signer_address_with_correct_domain() {
        let pk_signer = PrivateKeySigner::random();
        let expected_address = pk_signer.address();
        let signer = LocalSigner::new(pk_signer);

        let data = b"test attestation".to_vec();
        let tagged = AttestationPayload::new(data.clone(), AttestationType::State);
        let result = sign_attestation(100, Some(123), tagged, &signer)
            .await
            .unwrap();

        let verify_msg =
            AttestationPayload::new(data, AttestationType::State).tagged_signing_input();
        let recovered = recover_address(&verify_msg, &result.signature).unwrap();
        assert_eq!(recovered, expected_address);
    }

    #[tokio::test]
    async fn signature_does_not_recover_with_wrong_domain() {
        let pk_signer = PrivateKeySigner::random();
        let expected_address = pk_signer.address();
        let signer = LocalSigner::new(pk_signer);

        let data = b"cross-domain replay test".to_vec();
        let tagged = AttestationPayload::new(data.clone(), AttestationType::State);
        let result = sign_attestation(100, Some(123), tagged, &signer)
            .await
            .unwrap();

        let wrong_msg =
            AttestationPayload::new(data, AttestationType::Packet).tagged_signing_input();
        let recovered = recover_address(&wrong_msg, &result.signature).unwrap();
        assert_ne!(recovered, expected_address);
    }

    #[tokio::test]
    async fn signature_does_not_recover_from_raw_data() {
        let pk_signer = PrivateKeySigner::random();
        let expected_address = pk_signer.address();
        let signer = LocalSigner::new(pk_signer);

        let data = b"raw data replay test".to_vec();
        let tagged = AttestationPayload::new(data.clone(), AttestationType::Packet);
        let result = sign_attestation(100, None, tagged, &signer).await.unwrap();

        let recovered = recover_address(&data, &result.signature).unwrap();
        assert_ne!(recovered, expected_address);
    }

    #[tokio::test]
    async fn state_and_packet_signatures_differ_for_same_data() {
        let signer = test_signer();
        let data = b"same data for both domains".to_vec();

        let state_result = sign_attestation(
            100,
            Some(123),
            AttestationPayload::new(data.clone(), AttestationType::State),
            &signer,
        )
        .await
        .unwrap();
        let packet_result = sign_attestation(
            100,
            None,
            AttestationPayload::new(data, AttestationType::Packet),
            &signer,
        )
        .await
        .unwrap();

        assert_ne!(state_result.signature, packet_result.signature);
    }

    #[tokio::test]
    async fn sign_attestation_is_deterministic() {
        let signer = test_signer();

        let r1 = sign_attestation(
            1,
            None,
            AttestationPayload::new(b"deterministic".to_vec(), AttestationType::State),
            &signer,
        )
        .await
        .unwrap();
        let r2 = sign_attestation(
            1,
            None,
            AttestationPayload::new(b"deterministic".to_vec(), AttestationType::State),
            &signer,
        )
        .await
        .unwrap();
        assert_eq!(r1.signature, r2.signature);
    }

    #[tokio::test]
    async fn cross_domain_replay_attack_fails() {
        let pk_signer = PrivateKeySigner::random();
        let expected_address = pk_signer.address();
        let signer = LocalSigner::new(pk_signer);

        let packet_data = b"packet attestation abi bytes".to_vec();
        let packet_signed = sign_attestation(
            100,
            None,
            AttestationPayload::new(packet_data.clone(), AttestationType::Packet),
            &signer,
        )
        .await
        .unwrap();

        // Attacker tries to use the packet signature in a state attestation context
        let state_msg =
            AttestationPayload::new(packet_data, AttestationType::State).tagged_signing_input();
        let recovered = recover_address(&state_msg, &packet_signed.signature).unwrap();

        assert_ne!(recovered, expected_address);
    }
}
