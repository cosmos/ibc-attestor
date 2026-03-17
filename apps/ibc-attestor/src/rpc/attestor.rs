use alloy_primitives::keccak256;
use alloy_sol_types::SolValue;
use futures::{StreamExt, stream::FuturesOrdered};
use ibc_eureka_solidity_types::ics26::IICS26RouterMsgs::Packet;
use ibc_eureka_solidity_types::msgs::IAttestationMsgs;
use tonic::{Request, Response, Status};
use tracing::{debug, error};

use super::api::attestation_service_server::AttestationService;
use crate::{
    AttestorError, Packets,
    adapter::AttestationAdapter,
    attestation::{SignedAttestation, sign_attestation},
    attestation_payload::{AttestationPayload, AttestationType},
    rpc::api::{
        Attestation, CommitmentType, LatestHeightRequest, LatestHeightResponse,
        PacketAttestationRequest, PacketAttestationResponse, StateAttestationRequest,
        StateAttestationResponse,
    },
    signer::Signer,
};

/// gRPC service implementation for attestation requests
///
/// This service provides endpoints for:
/// - Getting the latest finalized height
/// - Generating state attestations
/// - Generating packet attestations
pub struct AttestorService<A, S> {
    adapter: A,
    adapter_name: &'static str,
    signer: S,
    signer_name: &'static str,
}

impl<A, S> AttestorService<A, S> {
    pub const fn new(
        adapter: A,
        adapter_name: &'static str,
        signer: S,
        signer_name: &'static str,
    ) -> Self {
        Self {
            adapter,
            adapter_name,
            signer,
            signer_name,
        }
    }

    pub const fn adapter_name(&self) -> &'static str {
        self.adapter_name
    }

    pub const fn signer_name(&self) -> &'static str {
        self.signer_name
    }
}

#[tonic::async_trait]
impl<A, S> AttestationService for AttestorService<A, S>
where
    A: AttestationAdapter,
    S: Signer,
{
    async fn latest_height(
        &self,
        _request: Request<LatestHeightRequest>,
    ) -> Result<Response<LatestHeightResponse>, Status> {
        let height = self
            .adapter
            .get_last_height_at_configured_finality()
            .await
            .map_err(AttestorError::from)?;

        Ok(Response::new(LatestHeightResponse { height }))
    }

    async fn state_attestation(
        &self,
        request: Request<StateAttestationRequest>,
    ) -> Result<Response<StateAttestationResponse>, Status> {
        let height = request.get_ref().height;

        validate_height(&self.adapter, height).await?;

        // Create unsigned attestation
        let timestamp = self
            .adapter
            .get_block_timestamp(height)
            .await
            .map_err(AttestorError::from)?;
        let unsigned_attestation = IAttestationMsgs::StateAttestation { height, timestamp };
        let attested_data = unsigned_attestation.abi_encode();

        // Signed attestation
        let attestation = sign_attestation(
            height,
            Some(timestamp),
            AttestationPayload::new(attested_data, AttestationType::State),
            &self.signer,
        )
        .await?;

        Ok(Response::from(attestation))
    }

    async fn packet_attestation(
        &self,
        request: Request<PacketAttestationRequest>,
    ) -> Result<Response<PacketAttestationResponse>, Status> {
        let request_inner = request.into_inner();
        let height = request_inner.height;
        let packets = Packets::try_from_abi_encoded(&request_inner.packets)?;
        let commitment_type =
            CommitmentType::try_from(request_inner.commitment_type).map_err(AttestorError::from)?;

        validate_height(&self.adapter, height).await?;

        // Create unsigned attestation
        let unsigned_attestation =
            create_packets_attestation(&self.adapter, packets, height, commitment_type).await?;
        let attested_data = unsigned_attestation.abi_encode();

        // Signed attestation
        let attestation = sign_attestation(
            height,
            None,
            AttestationPayload::new(attested_data, AttestationType::Packet),
            &self.signer,
        )
        .await?;

        Ok(Response::from(attestation))
    }
}

/// Validate the block height is finalized
async fn validate_height(
    adapter: &impl AttestationAdapter,
    height: u64,
) -> Result<(), AttestorError> {
    // Check that the request is for the finalized height
    let finalized = adapter.get_last_height_at_configured_finality().await?;
    if height > finalized {
        error!(
            requestedHeight = height,
            finalizedHeight = finalized,
            "requested height is not finalized"
        );
        return Err(AttestorError::BlockNotFinalized);
    }

    debug!(finalizedHeight = finalized, "height validation passed");
    Ok(())
}

async fn create_packets_attestation(
    adapter: &impl AttestationAdapter,
    packets: Packets,
    height: u64,
    commitment_type: CommitmentType,
) -> Result<IAttestationMsgs::PacketAttestation, AttestorError> {
    let futures = packets
        .into_iter()
        .map(|packet| create_single_packet_attestation(adapter, height, packet, commitment_type))
        .collect::<FuturesOrdered<_>>();
    let validations = futures.collect::<Vec<_>>().await;

    // We handle packets only if all are valid
    let packets = validations.into_iter().collect::<Result<Vec<_>, _>>()?;

    Ok(IAttestationMsgs::PacketAttestation { height, packets })
}

/// Create unsigned packet attestation
#[tracing::instrument(
    skip(adapter, height, packet, commitment_type),
    fields(clientId = packet.sourceClient, sequence = packet.sequence)
)] // NOTE: we span here as packet attestation logs use decoded `Packet` fields
async fn create_single_packet_attestation(
    adapter: &impl AttestationAdapter,
    height: u64,
    packet: Packet,
    commitment_type: CommitmentType,
) -> Result<IAttestationMsgs::PacketCompact, AttestorError> {
    match commitment_type {
        CommitmentType::Packet => {
            handle_packet_commitment(adapter, height, packet, commitment_type).await
        }
        CommitmentType::Ack => {
            handle_ack_commitment(adapter, height, packet, commitment_type).await
        }
        CommitmentType::Receipt => {
            handle_receipt_commitment(adapter, height, packet, commitment_type).await
        }
    }
}

async fn handle_packet_commitment(
    adapter: &impl AttestationAdapter,
    height: u64,
    packet: Packet,
    commitment_type: CommitmentType,
) -> Result<IAttestationMsgs::PacketCompact, AttestorError> {
    let commitment_path = packet.commitment_path();
    let expected_commitment = packet.commitment();
    let client_id = packet.sourceClient.clone();
    let sequence = packet.sequence;

    debug!("validating packet commitment");

    // Get packet commitment from the chain
    let commitment = adapter
        .get_commitment(
            client_id.clone(),
            height,
            sequence,
            &commitment_path,
            commitment_type,
        )
        .await?;

    // Packet commitment is expected to exist
    let commitment = commitment.ok_or_else(|| {
        error!("packet commitment not found on chain");
        AttestorError::CommitmentNotFound {
            client_id: client_id.clone(),
            sequence,
            height,
        }
    })?;

    if expected_commitment == commitment {
        debug!("packet commitment validated successfully");
        Ok(IAttestationMsgs::PacketCompact {
            path: keccak256(commitment_path),
            commitment: commitment.into(),
        })
    } else {
        error!(
            expected = %hex::encode(&expected_commitment),
            actual = %hex::encode(commitment),
            "packet commitment mismatch"
        );
        Err(AttestorError::InvalidCommitment {
            reason: format!(
                "Packet commitment mismatch for client_id={} seq={}: expected 0x{}, got 0x{}",
                client_id,
                sequence,
                hex::encode(&expected_commitment),
                hex::encode(commitment)
            ),
        })
    }
}

async fn handle_ack_commitment(
    adapter: &impl AttestationAdapter,
    height: u64,
    packet: Packet,
    commitment_type: CommitmentType,
) -> Result<IAttestationMsgs::PacketCompact, AttestorError> {
    let commitment_path = packet.ack_commitment_path();
    let client_id = packet.destClient.clone();
    let sequence = packet.sequence;

    debug!(height, "validating ack commitment");

    // Get commitment from the chain
    let commitment = adapter
        .get_commitment(
            client_id.clone(),
            height,
            sequence,
            &commitment_path,
            commitment_type,
        )
        .await?;

    // Ack commitment is expected to exist
    let commitment = commitment.ok_or_else(|| {
        error!(height, "ack commitment not found on chain");
        AttestorError::CommitmentNotFound {
            client_id,
            sequence,
            height,
        }
    })?;

    Ok(IAttestationMsgs::PacketCompact {
        path: keccak256(commitment_path),
        commitment: commitment.into(),
    })
}

async fn handle_receipt_commitment(
    adapter: &impl AttestationAdapter,
    height: u64,
    packet: Packet,
    commitment_type: CommitmentType,
) -> Result<IAttestationMsgs::PacketCompact, AttestorError> {
    let commitment_path = packet.receipt_commitment_path();
    let client_id = packet.destClient.clone();
    let sequence = packet.sequence;

    debug!("validating receipt commitment (expecting zero/non-existence)");

    // Get commitment from the chain
    let commitment = adapter
        .get_commitment(
            client_id.clone(),
            height,
            sequence,
            &commitment_path,
            commitment_type,
        )
        .await?;

    // The expected commitment is empty commitment (for timeout proofs)
    // so flag when commitment exists
    if let Some(commit) = commitment {
        error!(
            actual = %hex::encode(commit),
            "receipt commitment should be zero but found non-zero value"
        );
        Err(AttestorError::InvalidCommitment {
            reason: format!(
                "Receipt commitment should be zero for client_id={} seq={}: got 0x{}",
                client_id,
                sequence,
                hex::encode(commit)
            ),
        })
    } else {
        debug!("receipt commitment validated (zero/non-existent as expected)");
        Ok(IAttestationMsgs::PacketCompact {
            path: keccak256(commitment_path),
            commitment: [0; 32].into(),
        })
    }
}

impl From<SignedAttestation> for Response<StateAttestationResponse> {
    fn from(signed: SignedAttestation) -> Self {
        let attestation = Attestation {
            height: signed.height,
            timestamp: signed.timestamp,
            attested_data: signed.attested_data,
            signature: signed.signature,
        };

        Self::new(StateAttestationResponse {
            attestation: Some(attestation),
        })
    }
}

impl From<SignedAttestation> for Response<PacketAttestationResponse> {
    fn from(signed: SignedAttestation) -> Self {
        let attestation = Attestation {
            height: signed.height,
            timestamp: signed.timestamp,
            attested_data: signed.attested_data,
            signature: signed.signature,
        };

        Self::new(PacketAttestationResponse {
            attestation: Some(attestation),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::adapter::AttestationAdapterError;
    use ibc_eureka_solidity_types::ics26::IICS26RouterMsgs::Payload;

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct CommitmentKey {
        client_id: String,
        height: u64,
        sequence: u64,
        commitment_path: Vec<u8>,
        commitment_type: CommitmentType,
    }

    struct TestAdapter {
        finalized_height: u64,
        commitments: HashMap<CommitmentKey, Option<[u8; 32]>>,
    }

    impl TestAdapter {
        fn with_finalized_height(finalized_height: u64) -> Self {
            Self {
                finalized_height,
                commitments: HashMap::new(),
            }
        }

        fn insert_commitment(
            &mut self,
            client_id: String,
            height: u64,
            sequence: u64,
            commitment_path: Vec<u8>,
            commitment_type: CommitmentType,
            commitment: Option<[u8; 32]>,
        ) {
            self.commitments.insert(
                CommitmentKey {
                    client_id,
                    height,
                    sequence,
                    commitment_path,
                    commitment_type,
                },
                commitment,
            );
        }
    }

    #[async_trait::async_trait]
    impl AttestationAdapter for TestAdapter {
        async fn get_last_height_at_configured_finality(
            &self,
        ) -> Result<u64, AttestationAdapterError> {
            Ok(self.finalized_height)
        }

        async fn get_block_timestamp(&self, _height: u64) -> Result<u64, AttestationAdapterError> {
            Ok(1_700_000_000)
        }

        async fn get_commitment(
            &self,
            client_id: String,
            height: u64,
            sequence: u64,
            commitment_path: &[u8],
            commitment_type: CommitmentType,
        ) -> Result<Option<[u8; 32]>, AttestationAdapterError> {
            let key = CommitmentKey {
                client_id,
                height,
                sequence,
                commitment_path: commitment_path.to_vec(),
                commitment_type,
            };

            Ok(self.commitments.get(&key).copied().flatten())
        }
    }

    fn test_packet(sequence: u64) -> Packet {
        Packet {
            sequence,
            sourceClient: "src-client".to_string(),
            destClient: "dst-client".to_string(),
            timeoutTimestamp: 123_456_789,
            payloads: vec![Payload {
                sourcePort: "transfer".to_string(),
                destPort: "transfer".to_string(),
                version: "ics20-1".to_string(),
                encoding: "proto3".to_string(),
                value: vec![1, 2, 3].into(),
            }],
        }
    }

    #[tokio::test]
    async fn validate_height_accepts_finalized_height() {
        let adapter = TestAdapter::with_finalized_height(10);
        let result = validate_height(&adapter, 10).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn validate_height_rejects_future_height() {
        let adapter = TestAdapter::with_finalized_height(10);
        let result = validate_height(&adapter, 11).await;
        assert!(matches!(result, Err(AttestorError::BlockNotFinalized)));
    }

    #[tokio::test]
    async fn handle_packet_commitment_succeeds_when_commitment_matches() {
        let packet = test_packet(7);
        let path = packet.commitment_path();
        let expected_commitment: [u8; 32] = packet
            .commitment()
            .try_into()
            .expect("packet commitment must be 32 bytes");

        let mut adapter = TestAdapter::with_finalized_height(100);
        adapter.insert_commitment(
            packet.sourceClient.clone(),
            50,
            packet.sequence,
            path,
            CommitmentType::Packet,
            Some(expected_commitment),
        );

        let result = handle_packet_commitment(&adapter, 50, packet, CommitmentType::Packet).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn handle_packet_commitment_errors_when_missing() {
        let packet = test_packet(8);
        let adapter = TestAdapter::with_finalized_height(100);

        let result = handle_packet_commitment(&adapter, 50, packet, CommitmentType::Packet).await;
        assert!(matches!(
            result,
            Err(AttestorError::CommitmentNotFound { .. })
        ));
    }

    #[tokio::test]
    async fn handle_packet_commitment_errors_when_mismatched() {
        let packet = test_packet(9);
        let path = packet.commitment_path();

        let different_commitment = [9; 32];

        let mut adapter = TestAdapter::with_finalized_height(100);
        adapter.insert_commitment(
            packet.sourceClient.clone(),
            50,
            packet.sequence,
            path,
            CommitmentType::Packet,
            Some(different_commitment),
        );

        let result = handle_packet_commitment(&adapter, 50, packet, CommitmentType::Packet).await;
        assert!(matches!(
            result,
            Err(AttestorError::InvalidCommitment { .. })
        ));
    }

    #[tokio::test]
    async fn handle_ack_commitment_succeeds_when_present() {
        let packet = test_packet(10);
        let path = packet.ack_commitment_path();
        let mut adapter = TestAdapter::with_finalized_height(100);
        adapter.insert_commitment(
            packet.destClient.clone(),
            50,
            packet.sequence,
            path,
            CommitmentType::Ack,
            Some([7; 32]),
        );

        let result = handle_ack_commitment(&adapter, 50, packet, CommitmentType::Ack).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn handle_ack_commitment_errors_when_missing() {
        let packet = test_packet(11);
        let adapter = TestAdapter::with_finalized_height(100);

        let result = handle_ack_commitment(&adapter, 50, packet, CommitmentType::Ack).await;
        assert!(matches!(
            result,
            Err(AttestorError::CommitmentNotFound { .. })
        ));
    }

    #[tokio::test]
    async fn handle_receipt_commitment_accepts_none_as_zero() {
        let packet = test_packet(12);
        let adapter = TestAdapter::with_finalized_height(100);

        let result = handle_receipt_commitment(&adapter, 50, packet, CommitmentType::Receipt).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn handle_receipt_commitment_rejects_non_zero_commitment() {
        let packet = test_packet(13);
        let path = packet.receipt_commitment_path();
        let mut adapter = TestAdapter::with_finalized_height(100);
        adapter.insert_commitment(
            packet.destClient.clone(),
            50,
            packet.sequence,
            path,
            CommitmentType::Receipt,
            Some([1; 32]),
        );

        let result = handle_receipt_commitment(&adapter, 50, packet, CommitmentType::Receipt).await;
        assert!(matches!(
            result,
            Err(AttestorError::InvalidCommitment { .. })
        ));
    }

    #[tokio::test]
    async fn create_packets_attestation_succeeds_when_all_packets_valid() {
        let packet_a = test_packet(20);
        let packet_b = test_packet(21);

        let mut adapter = TestAdapter::with_finalized_height(100);
        let commitment_a: [u8; 32] = packet_a
            .commitment()
            .try_into()
            .expect("packet commitment must be 32 bytes");
        let commitment_b: [u8; 32] = packet_b
            .commitment()
            .try_into()
            .expect("packet commitment must be 32 bytes");
        adapter.insert_commitment(
            packet_a.sourceClient.clone(),
            60,
            packet_a.sequence,
            packet_a.commitment_path(),
            CommitmentType::Packet,
            Some(commitment_a),
        );
        adapter.insert_commitment(
            packet_b.sourceClient.clone(),
            60,
            packet_b.sequence,
            packet_b.commitment_path(),
            CommitmentType::Packet,
            Some(commitment_b),
        );

        let encoded = vec![packet_a.abi_encode(), packet_b.abi_encode()];
        let packets = crate::Packets::try_from_abi_encoded(&encoded).expect("packets must decode");

        let result =
            create_packets_attestation(&adapter, packets, 60, CommitmentType::Packet).await;
        assert!(result.is_ok());
        assert_eq!(
            result.expect("result must be ok").packets.len(),
            2,
            "both packets should be included in the unsigned attestation"
        );
    }

    #[tokio::test]
    async fn create_packets_attestation_errors_if_any_packet_is_invalid() {
        let packet_a = test_packet(30);
        let packet_b = test_packet(31);

        let mut adapter = TestAdapter::with_finalized_height(100);

        let commitment_a: [u8; 32] = packet_a
            .commitment()
            .try_into()
            .expect("packet commitment must be 32 bytes");
        let bad_commitment_b = [0xFF; 32];

        adapter.insert_commitment(
            packet_a.sourceClient.clone(),
            70,
            packet_a.sequence,
            packet_a.commitment_path(),
            CommitmentType::Packet,
            Some(commitment_a),
        );
        adapter.insert_commitment(
            packet_b.sourceClient.clone(),
            70,
            packet_b.sequence,
            packet_b.commitment_path(),
            CommitmentType::Packet,
            Some(bad_commitment_b),
        );

        let encoded = vec![packet_a.abi_encode(), packet_b.abi_encode()];
        let packets = crate::Packets::try_from_abi_encoded(&encoded).expect("packets must decode");

        let result =
            create_packets_attestation(&adapter, packets, 70, CommitmentType::Packet).await;
        assert!(matches!(
            result,
            Err(AttestorError::InvalidCommitment { .. })
        ));
    }
}
