use sha2::{Digest, Sha256};

/// Distinguishes attestation types in the signing scheme to prevent cross-protocol replay.
///
/// The domain tag byte is prepended before the inner hash, producing a 33-byte message:
/// `domain_tag || sha256(data)`. The signer hashes this again, so the final signature covers
/// `sha256(domain_tag || sha256(data))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AttestationType {
    /// State attestations (height + timestamp)
    State = 0x01,
    /// Packet attestations (height + packets)
    Packet = 0x02,
}

impl AttestationType {
    /// Returns the single-byte wire representation.
    #[must_use]
    pub const fn as_byte(self) -> u8 {
        self as u8
    }
}

/// ABI-encoded attestation data bound to a specific [`AttestationType`].
///
/// Bundling type and data ensures that a caller can never forget to supply
/// the type tag or accidentally swap it.
pub struct AttestationPayload {
    attestation_type: AttestationType,
    data: Vec<u8>,
}

impl AttestationPayload {
    /// Bind ABI-encoded `data` to the given [`AttestationType`].
    #[must_use]
    pub fn new(data: Vec<u8>, attestation_type: AttestationType) -> Self {
        Self {
            attestation_type,
            data,
        }
    }

    /// Construct the 33-byte tagged message: `type_tag || sha256(data)`.
    #[must_use]
    pub fn tagged_signing_input(&self) -> Vec<u8> {
        let inner_hash = Sha256::digest(&self.data);
        let mut tagged = Vec::with_capacity(33);
        tagged.push(self.attestation_type.as_byte());
        tagged.extend_from_slice(&inner_hash);
        tagged
    }

    /// Returns a reference to the raw ABI-encoded data.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Consumes `self` and returns the raw ABI-encoded data.
    #[must_use]
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// Returns the [`AttestationType`] this payload is bound to.
    #[must_use]
    pub const fn attestation_type(&self) -> AttestationType {
        self.attestation_type
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;

    #[test]
    fn as_byte_returns_expected_values() {
        assert_eq!(AttestationType::State.as_byte(), 0x01);
        assert_eq!(AttestationType::Packet.as_byte(), 0x02);
    }

    #[test]
    fn tagged_signing_input_is_33_bytes() {
        let tagged =
            AttestationPayload::new(b"some attestation data".to_vec(), AttestationType::State);
        let msg = tagged.tagged_signing_input();
        assert_eq!(msg.len(), 33);
        assert_eq!(msg[0], AttestationType::State.as_byte());
    }

    #[test]
    fn tagged_signing_input_starts_with_domain_byte() {
        assert_eq!(
            AttestationPayload::new(b"data".to_vec(), AttestationType::State)
                .tagged_signing_input()[0],
            0x01
        );
        assert_eq!(
            AttestationPayload::new(b"data".to_vec(), AttestationType::Packet)
                .tagged_signing_input()[0],
            0x02
        );
    }

    #[test]
    fn tagged_signing_input_suffix_is_sha256_of_data() {
        let data = b"test data";
        let expected_hash = Sha256::digest(data);
        let msg =
            AttestationPayload::new(data.to_vec(), AttestationType::State).tagged_signing_input();
        assert_eq!(&msg[1..], expected_hash.as_slice());
    }

    #[test]
    fn different_domains_produce_different_tagged_signing_inputs() {
        let data = b"identical data".to_vec();
        let state_msg =
            AttestationPayload::new(data.clone(), AttestationType::State).tagged_signing_input();
        let packet_msg =
            AttestationPayload::new(data, AttestationType::Packet).tagged_signing_input();
        assert_ne!(state_msg, packet_msg);
    }

    #[test]
    fn same_domain_same_data_is_deterministic() {
        let a = AttestationPayload::new(b"deterministic check".to_vec(), AttestationType::State)
            .tagged_signing_input();
        let b = AttestationPayload::new(b"deterministic check".to_vec(), AttestationType::State)
            .tagged_signing_input();
        assert_eq!(a, b);
    }

    #[test]
    fn different_data_produces_different_tagged_signing_inputs() {
        let a = AttestationPayload::new(b"data-a".to_vec(), AttestationType::State)
            .tagged_signing_input();
        let b = AttestationPayload::new(b"data-b".to_vec(), AttestationType::State)
            .tagged_signing_input();
        assert_ne!(a, b);
    }

    #[test]
    fn domain_accessor_returns_correct_variant() {
        assert_eq!(
            AttestationPayload::new(vec![], AttestationType::State).attestation_type(),
            AttestationType::State
        );
        assert_eq!(
            AttestationPayload::new(vec![], AttestationType::Packet).attestation_type(),
            AttestationType::Packet
        );
    }

    #[test]
    fn data_accessor_returns_original_bytes() {
        let raw = b"original bytes".to_vec();
        let tagged = AttestationPayload::new(raw.clone(), AttestationType::State);
        assert_eq!(tagged.data(), raw.as_slice());
        assert_eq!(tagged.into_data(), raw);
    }

    #[test]
    fn domain_tagged_digest_matches_manual_computation() {
        let data = b"verify digest";
        let tagged = AttestationPayload::new(data.to_vec(), AttestationType::State);
        let msg = tagged.tagged_signing_input();

        // The signer will compute sha256(msg), which should equal
        // sha256(0x01 || sha256(data))
        let inner = Sha256::digest(data);
        let mut expected_input = vec![AttestationType::State.as_byte()];
        expected_input.extend_from_slice(&inner);
        let expected_digest = B256::from_slice(&Sha256::digest(&expected_input));

        let actual_digest = B256::from_slice(&Sha256::digest(&msg));
        assert_eq!(actual_digest, expected_digest);
    }
}
