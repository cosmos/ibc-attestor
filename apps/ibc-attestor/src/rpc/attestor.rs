use alloy_primitives::keccak256;
use alloy_sol_types::SolValue;
use futures::{StreamExt, stream::FuturesUnordered};
use ibc_eureka_solidity_types::ics26::IICS26RouterMsgs::Packet;
use ibc_eureka_solidity_types::msgs::IAttestationMsgs;
use tonic::{Request, Response, Status};
use tracing::{debug, error};

use super::api::attestation_service_server::AttestationService;
use crate::{
    AttestorError, Packets,
    adapter::AttestationAdapter,
    attestation::{SignedAttestation, sign_attestation},
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
    pub fn new(
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

        Response::new(StateAttestationResponse {
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

        Response::new(PacketAttestationResponse {
            attestation: Some(attestation),
        })
    }
}
