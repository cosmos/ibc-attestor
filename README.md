# IBC Attestor Architecture

## Overview

The IBC Attestor is a lightweight, blockchain-agnostic attestation service that provides cryptographically signed attestations of blockchain state for IBC cross-chain communication. IBC Attestors publish attestations on demand and are stateless: consumers of the service must send requests to the service's gRPC server to receive attestations.

For the broader interop picture and component links, see [this overview](https://github.com/cosmos/ibc-attestor/blob/main/docs/deployment-overview.md).

### Key features

- Multi-chain support via pluggable adapter pattern (EVM, Solana, Cosmos)
- Flexible signing (local keystore or remote HSM/KMS)
- gRPC API for attestation requests
- Concurrent packet validation

### Attestation structure

IBC attestors support two kinds of attestations:
- State attestations: A given chain has a block of height x where the block was produced at time y
- Packet attestations: A given chain's state explicitly does or does not contain Packet commitment z
    - **Note**: Receipt packets should _not_ exist in the context of attestors as receipt packets are used for timeouts and non-membership proofs

The attestation gRPC server does differentiate between these two types even though under the hood they both use the same proto type: [Attestation](https://github.com/cosmos/ibc-attestor/blob/main/proto/ibc_attestor/attestation.proto). The reason we differentiate is because the `attested_data` field for each of these attestation kinds contains different data:
- State attestations hold the height and timestamp of a block
- Packet attestations contain the packets which were provided in initial request and the height at which the commitments were found.


### Security model and trust assumptions

Within the context of IBC relaying IBC attestors are an off-chain trusted service. Trust is established with on-chain components via two mechansims:
- Securely managed secp256k1 signing keys used by attestors to create attestations. The public parts of the keys must be registered with an on-chain light client;
- An aggregation layer during relaying that asserts a configurable m-of-n signatures attest to the same state.

At the level of individual attestor instances we make the following trust assumptions:
- RPC endpoints provide accurate data
- Private key is kept secure

Attestor instances can make the following security guarantees:
- Packet commitments must be valid before signing:
    - Packet: Must match computed value
    - Ack: Must exist on chain
    - Receipt: Must be absent (zero)
- Signatures are cryptographically sound and recoverable
- Any heights in gRPC queries cannot be greater than the configured finalization height

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

## Operating an attestor instance

### CLI and configuration

When running this program you need to specify:
- What kind of chain (`--chain-type`) will be attested to
- How the signer key (`--signer-type`) will be provided 

Each chain and signer type has its own configuration parameters which are caputured under separate sections in the configuration toml. Here is [an example](https://github.com/cosmos/ibc-attestor/blob/main/apps/ibc-attestor/server.dev.toml) of how to configure an EVM attestor.

### Chain adapters

To add support for new kinds of chains you need to implement the `AttestationAdapter` and `AdapterBuilder` [interfaces](https://github.com/cosmos/ibc-attestor/blob/main/apps/ibc-attestor/src/adapter/mod.rs) interfaces, respectively.

- The `AttestationAdapter` is responsible for retrieving on-chain state and ensuring this state can be parsed as:
    - A valid height and timestamp for a `StateAttestation`
    - A valid 32-byte commitment path for an IBC Packet
- The `AdapterBuilder` enables per chain configurations needed by the `AttestationAdapter` implementation.

The CLI must also be extended to support any new chain types.

### Signing requirements

Currently the IBC attestor supports two signing modes: local and remote. The remote signer is based on the Cosmos Labs remote signer which uses AWS KMS for key rotation.

The attestor signing algorithm is a follows:
1. Retrieve relevant chain/packet state via the chain adapter
2. Encode the data using the ABI format to facilitate EVM parsing
3. Send the encoded message to the signer which first hashes and then signs the data in ECDSA 65-byte recoverable signature (r||s||v)

Any new signer implementations **must guarantee**:
- Arbitrary ABI-encoded data is hash before signing
- The signature is in the ECDSA 65-byte recoverable signature (r||s||v)

## Observability

The IBC attestor uses a logging middleware to time requests, set trace IDs and to add structured fieds to traces. Currently these fields include:
- Adapter kind
- Signer kind
- Requested height (where applicable)
- Number of packets (where applicable)
- Packet commitment kind (where applicable)

### Logging

- Logs are emitted in JSON format
- Errors must be logged at occurence to simplify line number tracing
- Info logs should be reserved for middleware and startup operations
- Debug logs should capture adapter and attestation creation operations

### Tracing
- OpenTelemetry-compatible spans
- Minimal request time tracking
