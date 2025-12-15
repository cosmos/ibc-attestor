# IBC Attestor Architecture

## Overview

The IBC Attestor is a lightweight, blockchain-agnostic attestation service that provides cryptographically signed attestations of blockchain state for IBC cross-chain communication. It monitors blockchain networks and produces signed attestations that can be verified by on-chain light clients.

For the broader interop picture and component links, see `docs/interop.md`.

**Key Features:**
- Multi-chain support via pluggable adapter pattern (EVM, Solana, Cosmos)
- Flexible signing (local keystore or remote HSM/KMS)
- gRPC API for attestation requests
- Concurrent packet validation

## System Context

```
┌─────────────┐      ┌─────────────┐      ┌─────────────┐
│  Attestor   │─────▶│ Proof API   │─────▶│Light Client │
│  Service    │      │             │      │             │
└─────────────┘      └─────────────┘      └─────────────┘
      │                     │                     │
 Signs state &       Collects m-of-n       Verifies sigs
 packet data        signatures with        & updates IBC
                   quorum validation           state
```

The attestor is one node in an m-of-n multi-signature system. Multiple independent attestors provide signatures that are aggregated and verified by on-chain light clients.

## Architecture

### Component Structure

```
┌────────────────────────────────────────┐
│           Attestor Binary              │
│  ┌──────────────────────────────────┐  │
│  │         gRPC Server              │  │
│  │  - AttestationService            │  │
│  │  - Reflection API                │  │
│  │  - Logging & Tracing             │  │
│  └────────┬──────────────┬──────────┘  │
│           │              │             │
│  ┌────────▼─────┐  ┌──── ▼─────────┐   │
│  │ Attestation  │  │    Signer     │   │
│  │    Logic     │  │ - Local       │   │
│  │ - State      │  │ - Remote      │   │
│  │ - Packet     │  └───────────────┘   │
│  └────────┬─────┘                      │
│  ┌────────▼─────┐                      │
│  │   Adapter    │                      │
│  │ - EVM        │                      │
│  │ - Solana     │                      │
│  │ - Cosmos     │                      │
│  └──────────────┘                      │
└────────────────────────────────────────┘
```

## Running an attestor instance

When running this program you need to specify:
- What kind of chain (`--chain-type`) will be attested to
- How the signer key (`--signer-type`) will be provided 

Each chain and signer type has its own configuration parameters which are caputured under separate sections in the configuration toml.

## Technical Details

### Signing specs

The signing algorithm is a follows:
1. Retrieve relevant chain/packet state via the chain adapter
2. Encode the data using the ABI format to facilitate EVM parsing
3. Send the encoded message to the signer which first hashes and then signs the data in ECDSA 65-byte recoverable signature (r||s||v)

**Note**: Any additional KMS services that are to be integrated in the future need to conform to step three of the algorithm.

### Safety Guarantees

**Height Validation:**
- Only attests to finalized blocks
- Rejects requests for non-finalized heights

**Commitment Validation:**
- Packet: Must match computed value
- Ack: Must exist on chain
- Receipt: Must be absent (zero)


## Security Model

### Trust Assumptions
- Attestor honestly reports chain state
- RPC endpoints provide accurate data
- Private key is kept secure
- Only finalized blocks are attested

### Protected Against
- Block reorgs (finality requirement)
- Invalid commitments (validation before signing)
- Unauthorized attestations (cryptographic signatures)

### Not Protected Against
- Compromised private key
- Malicious RPC node responses
- Network DoS attacks

## Observability

### Logging
- INFO: Normal operations
- DEBUG: Detailed execution flow
- ERROR: Failures with full context
- JSON format

### Tracing
- OpenTelemetry-compatible spans
- Minimal request time tracking
