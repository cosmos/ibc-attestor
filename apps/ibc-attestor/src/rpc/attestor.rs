use alloy_primitives::keccak256;
use alloy_sol_types::SolValue;
use futures::{stream::FuturesUnordered, StreamExt};
use ibc_eureka_solidity_types::ics26::IICS26RouterMsgs::Packet;
use ibc_eureka_solidity_types::msgs::IAttestationMsgs;
use tonic::{Request, Response, Status};
use tracing::{debug, error};

use super::api::attestation_service_server::AttestationService;
use crate::{
    adapter::AttestationAdapter,
    attestation::{sign_attestation, SignedAttestation},
    rpc::api::{
        Attestation, CommitmentType, LatestHeightRequest, LatestHeightResponse,
        PacketAttestationRequest, PacketAttestationResponse, StateAttestationRequest,
        StateAttestationResponse,
    },
    signer::Signer,
    AttestorError, Packets,
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
    pub fn new(
        adapter: A,
        adapter_name: &'static str,
        signer: S,
        signer_name: &'static str,
    ) -> Self {
        Self { adapter, adapter_name, signer, signer_name }
    }

    pub fn adapter_name(&self) -> &'static str {
        self.adapter_name
    }

    pub fn signer_name(&self) -> &'static str {
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
        let height = self.adapter.get_last_finalized_height().await.map_err(AttestorError::from)?;

        Ok(Response::new(LatestHeightResponse { height }))
    }

    async fn state_attestation(
        &self,
        request: Request<StateAttestationRequest>,
    ) -> Result<Response<StateAttestationResponse>, Status> {
        let height = request.get_ref().height;

        validate_height(&self.adapter, height).await?;

        // Create unsigned attestation
        let timestamp =
            self.adapter.get_block_timestamp(height).await.map_err(AttestorError::from)?;
        let unsigned_attestation = IAttestationMsgs::StateAttestation { height, timestamp };
        let attested_data = unsigned_attestation.abi_encode();

        // Signed attestation
        let attestation =
            sign_attestation(height, Some(timestamp), attested_data, &self.signer).await?;

        Ok(Response::from(attestation))
    }

    async fn packet_attestation(
        &self,
        request: Request<PacketAttestationRequest>,
    ) -> Result<Response<PacketAttestationResponse>, Status> {
        let request_inner = request.into_inner();
        let height = request_inner.height;
        let packets = Packets::try_from_abi_encoded(request_inner.packets)?;
        let commitment_type = CommitmentType::try_from(request_inner.commitment_type)
            .unwrap_or(CommitmentType::Packet);

        validate_height(&self.adapter, height).await?;

        // Create unsigned attestation
        let unsigned_attestation =
            create_packets_attestation(&self.adapter, packets, height, commitment_type).await?;
        let attested_data = unsigned_attestation.abi_encode();

        // Signed attestation
        let attestation = sign_attestation(height, None, attested_data, &self.signer).await?;

        Ok(Response::from(attestation))
    }
}

/// Validate the block height is finalized
async fn validate_height(
    adapter: &impl AttestationAdapter,
    height: u64,
) -> Result<(), AttestorError> {
    // Check that the request is for the finalized height
    let finalized = adapter.get_last_finalized_height().await?;
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
        .collect::<FuturesUnordered<_>>();
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
    let expected_path = packet.commitment();
    let client_id = packet.sourceClient.clone();
    let sequence = packet.sequence;

    debug!("validating packet commitment");

    // Get packet commitment from the chain
    let commitment = adapter
        .get_commitment(client_id.clone(), height, sequence, &commitment_path, commitment_type)
        .await?;

    // Packet commitment is expected to exist
    let commitment = commitment.ok_or_else(|| {
        error!("packet commitment not found on chain");
        AttestorError::CommitmentNotFound { client_id: client_id.clone(), sequence, height }
    })?;

    if expected_path == commitment {
        debug!("packet commitment validated successfully");
        Ok(IAttestationMsgs::PacketCompact {
            path: keccak256(commitment_path),
            commitment: commitment.into(),
        })
    } else {
        error!(
            expected = %hex::encode(&expected_path),
            actual = %hex::encode(commitment),
            "packet commitment mismatch"
        );
        Err(AttestorError::InvalidCommitment {
            reason: format!(
                "Packet commitment mismatch for client_id={} seq={}: expected 0x{}, got 0x{}",
                client_id,
                sequence,
                hex::encode(&expected_path),
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
        .get_commitment(client_id.clone(), height, sequence, &commitment_path, commitment_type)
        .await?;

    // Ack commitment is expected to exist
    let commitment = commitment.ok_or_else(|| {
        error!(height, "ack commitment not found on chain");
        AttestorError::CommitmentNotFound { client_id, sequence, height }
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
        .get_commitment(client_id.clone(), height, sequence, &commitment_path, commitment_type)
        .await?;

    // If commitment is `None` we set it to empty commitment
    let commitment = commitment.unwrap_or([0; 32]);

    // The expected commitment is empty commitment (for timeout proofs)
    if commitment == [0; 32] {
        debug!("receipt commitment validated (zero/non-existent as expected)");
        Ok(IAttestationMsgs::PacketCompact {
            path: keccak256(commitment_path),
            commitment: commitment.into(),
        })
    } else {
        error!(
            actual = %hex::encode(commitment),
            "receipt commitment should be zero but found non-zero value"
        );
        Err(AttestorError::InvalidCommitment {
            reason: format!(
                "Receipt commitment should be zero for client_id={} seq={}: got 0x{}",
                client_id,
                sequence,
                hex::encode(commitment)
            ),
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

        Response::new(StateAttestationResponse { attestation: Some(attestation) })
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

        Response::new(PacketAttestationResponse { attestation: Some(attestation) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{AttestationAdapter, AttestationAdapterError};
    use crate::rpc::api::CommitmentType;
    use crate::signer::{Signer, SignerError};
    use alloy_primitives::{keccak256, Signature};
    use alloy_sol_types::SolValue;
    use ibc_eureka_solidity_types::ics26::IICS26RouterMsgs::Packet;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tonic::Code;

    /// Mock adapter for testing with configurable commitment responses
    #[derive(Clone)]
    struct MockAdapter {
        finalized_height: u64,
        block_timestamps: Arc<Mutex<HashMap<u64, u64>>>,
        commitments: Arc<Mutex<HashMap<CommitmentKey, Option<[u8; 32]>>>>,
    }

    #[derive(Debug, Clone, Hash, Eq, PartialEq)]
    struct CommitmentKey {
        client_id: String,
        height: u64,
        sequence: u64,
        commitment_type: i32,
    }

    impl MockAdapter {
        fn new(finalized_height: u64) -> Self {
            Self {
                finalized_height,
                block_timestamps: Arc::new(Mutex::new(HashMap::new())),
                commitments: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        fn set_block_timestamp(&self, height: u64, timestamp: u64) {
            self.block_timestamps.lock().unwrap().insert(height, timestamp);
        }

        fn set_commitment(
            &self,
            client_id: String,
            height: u64,
            sequence: u64,
            commitment_type: CommitmentType,
            commitment: Option<[u8; 32]>,
        ) {
            let key = CommitmentKey {
                client_id,
                height,
                sequence,
                commitment_type: commitment_type as i32,
            };
            self.commitments.lock().unwrap().insert(key, commitment);
        }
    }

    #[async_trait::async_trait]
    impl AttestationAdapter for MockAdapter {
        async fn get_last_finalized_height(&self) -> Result<u64, AttestationAdapterError> {
            Ok(self.finalized_height)
        }

        async fn get_block_timestamp(&self, height: u64) -> Result<u64, AttestationAdapterError> {
            self.block_timestamps
                .lock()
                .unwrap()
                .get(&height)
                .copied()
                .ok_or_else(|| {
                    AttestationAdapterError::RetrievalError(format!(
                        "Timestamp not found for height {}",
                        height
                    ))
                })
        }

        async fn get_commitment(
            &self,
            client_id: String,
            height: u64,
            sequence: u64,
            _commitment_path: &[u8],
            commitment_type: CommitmentType,
        ) -> Result<Option<[u8; 32]>, AttestationAdapterError> {
            let key = CommitmentKey {
                client_id,
                height,
                sequence,
                commitment_type: commitment_type as i32,
            };
            Ok(self.commitments.lock().unwrap().get(&key).copied().flatten())
        }
    }

    /// Mock signer that returns a dummy signature
    struct MockSigner;

    #[async_trait::async_trait]
    impl Signer for MockSigner {
        async fn sign(&self, _message: &[u8]) -> Result<Signature, SignerError> {
            // Return a dummy signature (65 bytes: r=32, s=32, v=1)
            // Using from_scalars_and_parity which is the correct method
            let r = alloy_primitives::FixedBytes::<32>::from([0x11u8; 32]);
            let s = alloy_primitives::FixedBytes::<32>::from([0x22u8; 32]);
            Ok(Signature::from_scalars_and_parity(r, s, false))
        }
    }

    /// Helper to create a test packet
    fn create_test_packet(
        source_client: &str,
        dest_client: &str,
        sequence: u64,
    ) -> Packet {
        Packet {
            sourceClient: source_client.to_string(),
            destClient: dest_client.to_string(),
            sequence,
            timeoutTimestamp: 1_000_000_u64,
            payloads: vec![],
        }
    }

    #[tokio::test]
    async fn test_latest_height() {
        let adapter = MockAdapter::new(100);
        let signer = MockSigner;
        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let request = Request::new(LatestHeightRequest {});
        let response = service.latest_height(request).await.unwrap();

        assert_eq!(response.get_ref().height, 100);
    }

    #[tokio::test]
    async fn test_state_attestation_success() {
        let adapter = MockAdapter::new(100);
        adapter.set_block_timestamp(100, 1234567890);
        let signer = MockSigner;
        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let request = Request::new(StateAttestationRequest { height: 100 });
        let response = service.state_attestation(request).await.unwrap();

        let attestation = response.get_ref().attestation.as_ref().unwrap();
        assert_eq!(attestation.height, 100);
        assert_eq!(attestation.timestamp, Some(1234567890));
        assert!(!attestation.signature.is_empty());
    }

    #[tokio::test]
    async fn test_state_attestation_block_not_finalized() {
        let adapter = MockAdapter::new(100);
        let signer = MockSigner;
        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let request = Request::new(StateAttestationRequest { height: 101 });
        let result = service.state_attestation(request).await;

        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), Code::FailedPrecondition);
        assert!(status.message().contains("not finalized"));
    }

    #[tokio::test]
    async fn test_packet_commitment_valid() {
        let adapter = MockAdapter::new(100);
        let signer = MockSigner;

        let packet = create_test_packet("client-1", "client-2", 1);
        let expected_commitment_vec = packet.commitment();
        let expected_commitment: [u8; 32] = expected_commitment_vec.try_into().unwrap();

        // Set up the mock to return the expected commitment
        adapter.set_commitment(
            "client-1".to_string(),
            100,
            1,
            CommitmentType::Packet,
            Some(expected_commitment),
        );

        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let packets_encoded = vec![packet.abi_encode()];
        let request = Request::new(PacketAttestationRequest {
            packets: packets_encoded,
            height: 100,
            commitment_type: CommitmentType::Packet as i32,
        });

        let response = service.packet_attestation(request).await.unwrap();
        let attestation = response.get_ref().attestation.as_ref().unwrap();

        assert_eq!(attestation.height, 100);
        assert!(!attestation.signature.is_empty());
    }

    #[tokio::test]
    async fn test_packet_commitment_mismatch() {
        let adapter = MockAdapter::new(100);
        let signer = MockSigner;

        let packet = create_test_packet("client-1", "client-2", 1);
        let wrong_commitment = [0xffu8; 32];

        // Set up the mock to return a wrong commitment
        adapter.set_commitment(
            "client-1".to_string(),
            100,
            1,
            CommitmentType::Packet,
            Some(wrong_commitment),
        );

        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let packets_encoded = vec![packet.abi_encode()];
        let request = Request::new(PacketAttestationRequest {
            packets: packets_encoded,
            height: 100,
            commitment_type: CommitmentType::Packet as i32,
        });

        let result = service.packet_attestation(request).await;

        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(status.message().contains("mismatch"));
    }

    #[tokio::test]
    async fn test_packet_commitment_not_found() {
        let adapter = MockAdapter::new(100);
        let signer = MockSigner;

        let packet = create_test_packet("client-1", "client-2", 1);

        // Don't set any commitment - it will be None
        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let packets_encoded = vec![packet.abi_encode()];
        let request = Request::new(PacketAttestationRequest {
            packets: packets_encoded,
            height: 100,
            commitment_type: CommitmentType::Packet as i32,
        });

        let result = service.packet_attestation(request).await;

        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), Code::NotFound);
        assert!(status.message().contains("not found"));
    }

    #[tokio::test]
    async fn test_ack_commitment_valid() {
        let adapter = MockAdapter::new(100);
        let signer = MockSigner;

        let packet = create_test_packet("client-1", "client-2", 1);
        let ack_commitment = keccak256(b"test-ack").0;

        // Set up the mock to return an ack commitment
        adapter.set_commitment(
            "client-2".to_string(), // Note: destClient for ack
            100,
            1,
            CommitmentType::Ack,
            Some(ack_commitment),
        );

        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let packets_encoded = vec![packet.abi_encode()];
        let request = Request::new(PacketAttestationRequest {
            packets: packets_encoded,
            height: 100,
            commitment_type: CommitmentType::Ack as i32,
        });

        let response = service.packet_attestation(request).await.unwrap();
        let attestation = response.get_ref().attestation.as_ref().unwrap();

        assert_eq!(attestation.height, 100);
        assert!(!attestation.signature.is_empty());
    }

    #[tokio::test]
    async fn test_ack_commitment_not_found() {
        let adapter = MockAdapter::new(100);
        let signer = MockSigner;

        let packet = create_test_packet("client-1", "client-2", 1);

        // Don't set any ack commitment
        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let packets_encoded = vec![packet.abi_encode()];
        let request = Request::new(PacketAttestationRequest {
            packets: packets_encoded,
            height: 100,
            commitment_type: CommitmentType::Ack as i32,
        });

        let result = service.packet_attestation(request).await;

        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), Code::NotFound);
    }

    #[tokio::test]
    async fn test_receipt_commitment_zero_valid() {
        let adapter = MockAdapter::new(100);
        let signer = MockSigner;

        let packet = create_test_packet("client-1", "client-2", 1);

        // Set up the mock to return None (which becomes zero commitment)
        adapter.set_commitment(
            "client-2".to_string(), // Note: destClient for receipt
            100,
            1,
            CommitmentType::Receipt,
            None,
        );

        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let packets_encoded = vec![packet.abi_encode()];
        let request = Request::new(PacketAttestationRequest {
            packets: packets_encoded,
            height: 100,
            commitment_type: CommitmentType::Receipt as i32,
        });

        let response = service.packet_attestation(request).await.unwrap();
        let attestation = response.get_ref().attestation.as_ref().unwrap();

        assert_eq!(attestation.height, 100);
        assert!(!attestation.signature.is_empty());
    }

    #[tokio::test]
    async fn test_receipt_commitment_non_zero_invalid() {
        let adapter = MockAdapter::new(100);
        let signer = MockSigner;

        let packet = create_test_packet("client-1", "client-2", 1);
        let non_zero_commitment = [0x01u8; 32];

        // Set up the mock to return a non-zero commitment (should fail)
        adapter.set_commitment(
            "client-2".to_string(),
            100,
            1,
            CommitmentType::Receipt,
            Some(non_zero_commitment),
        );

        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let packets_encoded = vec![packet.abi_encode()];
        let request = Request::new(PacketAttestationRequest {
            packets: packets_encoded,
            height: 100,
            commitment_type: CommitmentType::Receipt as i32,
        });

        let result = service.packet_attestation(request).await;

        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(status.message().contains("should be zero"));
    }

    #[tokio::test]
    async fn test_multiple_packets_all_valid() {
        let adapter = MockAdapter::new(100);
        let signer = MockSigner;

        let packet1 = create_test_packet("client-1", "client-2", 1);
        let packet2 = create_test_packet("client-1", "client-2", 2);
        let packet3 = create_test_packet("client-3", "client-4", 1);

        let commitment1: [u8; 32] = packet1.commitment().try_into().unwrap();
        let commitment2: [u8; 32] = packet2.commitment().try_into().unwrap();
        let commitment3: [u8; 32] = packet3.commitment().try_into().unwrap();

        adapter.set_commitment(
            "client-1".to_string(),
            100,
            1,
            CommitmentType::Packet,
            Some(commitment1),
        );
        adapter.set_commitment(
            "client-1".to_string(),
            100,
            2,
            CommitmentType::Packet,
            Some(commitment2),
        );
        adapter.set_commitment(
            "client-3".to_string(),
            100,
            1,
            CommitmentType::Packet,
            Some(commitment3),
        );

        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let packets_encoded =
            vec![packet1.abi_encode(), packet2.abi_encode(), packet3.abi_encode()];
        let request = Request::new(PacketAttestationRequest {
            packets: packets_encoded,
            height: 100,
            commitment_type: CommitmentType::Packet as i32,
        });

        let response = service.packet_attestation(request).await.unwrap();
        let attestation = response.get_ref().attestation.as_ref().unwrap();

        assert_eq!(attestation.height, 100);
        assert!(!attestation.signature.is_empty());
    }

    #[tokio::test]
    async fn test_multiple_packets_one_invalid() {
        let adapter = MockAdapter::new(100);
        let signer = MockSigner;

        let packet1 = create_test_packet("client-1", "client-2", 1);
        let packet2 = create_test_packet("client-1", "client-2", 2);

        let commitment1: [u8; 32] = packet1.commitment().try_into().unwrap();
        let wrong_commitment = [0xffu8; 32];

        adapter.set_commitment(
            "client-1".to_string(),
            100,
            1,
            CommitmentType::Packet,
            Some(commitment1),
        );
        adapter.set_commitment(
            "client-1".to_string(),
            100,
            2,
            CommitmentType::Packet,
            Some(wrong_commitment), // Wrong commitment for packet2
        );

        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let packets_encoded = vec![packet1.abi_encode(), packet2.abi_encode()];
        let request = Request::new(PacketAttestationRequest {
            packets: packets_encoded,
            height: 100,
            commitment_type: CommitmentType::Packet as i32,
        });

        let result = service.packet_attestation(request).await;

        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_packet_attestation_height_not_finalized() {
        let adapter = MockAdapter::new(100);
        let signer = MockSigner;

        let packet = create_test_packet("client-1", "client-2", 1);

        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let packets_encoded = vec![packet.abi_encode()];
        let request = Request::new(PacketAttestationRequest {
            packets: packets_encoded,
            height: 101, // Beyond finalized height
            commitment_type: CommitmentType::Packet as i32,
        });

        let result = service.packet_attestation(request).await;

        assert!(result.is_err());
        let status = result.unwrap_err();
        assert_eq!(status.code(), Code::FailedPrecondition);
        assert!(status.message().contains("not finalized"));
    }

    #[tokio::test]
    async fn test_mixed_commitment_types_across_packets() {
        // This test ensures that commitment type is applied consistently
        let adapter = MockAdapter::new(100);
        let signer = MockSigner;

        let packet1 = create_test_packet("client-1", "client-2", 1);
        let packet2 = create_test_packet("client-1", "client-2", 2);

        // Set packet commitments for both
        let commitment1: [u8; 32] = packet1.commitment().try_into().unwrap();
        let commitment2: [u8; 32] = packet2.commitment().try_into().unwrap();

        adapter.set_commitment(
            "client-1".to_string(),
            100,
            1,
            CommitmentType::Packet,
            Some(commitment1),
        );
        adapter.set_commitment(
            "client-1".to_string(),
            100,
            2,
            CommitmentType::Packet,
            Some(commitment2),
        );

        let service = AttestorService::new(adapter, "mock", signer, "mock");

        let packets_encoded = vec![packet1.abi_encode(), packet2.abi_encode()];
        let request = Request::new(PacketAttestationRequest {
            packets: packets_encoded,
            height: 100,
            commitment_type: CommitmentType::Packet as i32,
        });

        // Should succeed as all packets use the same commitment type
        let response = service.packet_attestation(request).await.unwrap();
        assert!(response.get_ref().attestation.is_some());
    }
}
