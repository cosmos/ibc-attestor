# IBC Attestor

![IBC Attestor](.github/workflows/assets/cosmos-ibc-attestor-github.svg)

## Overview

The IBC Attestor is a lightweight, blockchain-agnostic attestation service that provides cryptographically signed attestations of blockchain state for IBC cross-chain communication. IBC Attestors publish attestations on demand and are stateless: consumers of the service must send requests to the service's gRPC server to receive attestations.

### Key features

- Multi-chain support via pluggable adapter pattern (EVM, Solana, Cosmos)
- Flexible signing (local keystore or remote HSM/KMS)
- gRPC API for attestation requests

## Quickstart

```bash
# 1. Build
cargo build -p ibc-attestor                                # add --release for production builds

# 2. Generate a signing key (writes to ~/.ibc-attestor/)
./target/debug/ibc_attestor key generate

# 3. Run an EVM attestor with a local signer.
./target/debug/ibc_attestor server \
  --config apps/ibc-attestor/server.dev.toml \
  --chain-type evm \
  --signer-type local
```

### Probe the server

Health and metrics are plain HTTP:

```bash
curl -i http://localhost:8081/healthz                      # → 200 OK
curl    http://localhost:8081/metrics | head               # Prometheus output
```

The attestation API is gRPC with reflection enabled:

```bash
grpcurl -plaintext localhost:8080 list
# grpc.reflection.v1.ServerReflection
# ibc_attestor.AttestationService

grpcurl -plaintext localhost:8080 describe ibc_attestor.AttestationService
```

To build the container image instead of running natively: `make build-attestor-image` (produces `attestor-local`).

## Calling the gRPC API

Service: `ibc_attestor.AttestationService` ([proto](proto/ibc_attestor/ibc_attestor.proto)).

```proto
service AttestationService {
  rpc LatestHeight      (LatestHeightRequest)      returns (LatestHeightResponse);
  rpc StateAttestation  (StateAttestationRequest)  returns (StateAttestationResponse);
  rpc PacketAttestation (PacketAttestationRequest) returns (PacketAttestationResponse);
}
```

Both attestation responses return a single `Attestation { height, timestamp, attested_data, signature }`. `attested_data` is ABI-encoded.

`signature` is a 65-byte recoverable ECDSA signature (`r||s||v`) over `sha256(domain_tag || sha256(attested_data))`. `domain_tag` is one byte — `0x01` for `StateAttestation`,
`0x02` for `PacketAttestation` (independent of `commitmentType`) — and is not carried on the wire; verifiers reconstruct it from which response they're verifying. This prevents
state-vs-packet cross-protocol replay.

Heights above the configured finalization height are rejected.

### `LatestHeight` — what is the chain's latest finalized height?

```bash
grpcurl -plaintext -d '{}' localhost:8080 \
  ibc_attestor.AttestationService/LatestHeight
  
#{ "height": "10888039" }
```

### `StateAttestation` — attest to a block height + timestamp

```bash
grpcurl -plaintext -d '{"height": 10888039}' localhost:8080 \
  ibc_attestor.AttestationService/StateAttestation

#{
#  "attestation": {
#    "height":       "10888039",
#    "timestamp":    "1779314016",
#    "attestedData": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACmI2cAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAag4tYA==",
#    "signature":    "JTzRHBE1e8ty77aPr9knyAUCXqKKb1TGMZZou9qAyVo4gE56vq6glOH2hGgxYeYA7ojmRj/FLQ6bOK/NIgyTOhs="
#  }
#}
```

`attestedData` is the ABI-encoded `StateAttestation { uint64 height, uint64 timestamp }`. `signature` is base64-encoded `r||s||v`.

### `PacketAttestation` — attest to packet/ack/receipt commitments

```bash
grpcurl -plaintext -d '{
  "height": 10888039,
  "commitmentType": "COMMITMENT_TYPE_PACKET",
  "packets": [
    "<base64 ABI-encoded packet>"
  ]
}' localhost:8080 ibc_attestor.AttestationService/PacketAttestation

#{
#  "attestation": {
#    "height":       "10888039",
#    "attestedData":
#"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAKYjZwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
#    "signature":    "y9TgtalLccTzsL0azZPE6JjkryUvK/7BVdmThE2QNpkEfP3CJbP0s/C6MKEO+XCqGt+QtQGGS1ew/Vy6uZ/axBw="
#  }
#}
```

- `commitmentType` is one of `COMMITMENT_TYPE_PACKET`, `COMMITMENT_TYPE_ACK`, `COMMITMENT_TYPE_RECEIPT`.
- Each entry in `packets` is an ABI-encoded IBC v2 packet (bytes are base64-encoded in JSON gRPC). The attestor recomputes each commitment against on-chain state — membership
  for `PACKET`/`ACK`, non-membership for `RECEIPT` — and signs only if every packet validates.
- A request may contain at most 100 packets. Larger batches are rejected before packet decoding or chain RPC queries.

Response shape matches `StateAttestation`; `attestedData` carries `(height, packets[])` rather than `(height, timestamp)`.

## CLI

  ```
  ibc_attestor server   --config <path> --chain-type <evm|solana|cosmos> [--signer-type <local|remote>] [--keystore-password <password>|--empty-keystore-password]
  ibc_attestor key generate [--keystore <path>] [--keystore-password <password>|--empty-keystore-password]
  ibc_attestor key show     [--show-private] [--show-public] [--keystore <path>] [--keystore-password <password>|--empty-keystore-password]
  ```

## Configuration

Per-chain and per-signer settings live in a TOML file passed via `--config`. [`apps/ibc-attestor/server.dev.toml`](apps/ibc-attestor/server.dev.toml) is the EVM +
local-keystore example:

```toml
[server]
listen_addr = "0.0.0.0:8080"
health_addr = "0.0.0.0:8081"

[adapter]                                                # shape depends on --chain-type
url            = "https://ethereum-sepolia-rpc.publicnode.com"
router_address = "0xff42b3db9f1040539a3741434e4b33b352fabd80"

[signer]                                                 # shape depends on --signer-type
keystore_path  = "~/.ibc-attestor/ibc-attestor-keystore" # ~ is expanded

[tracing]                                                # optional; see docs/tracing.md
otlp_endpoint  = "http://localhost:4317"
service_name   = "ibc-attestor"
sample_rate    = 1.0
```

The `[adapter]` and `[signer]` tables are typed by `--chain-type` and `--signer-type` respectively. Local keystore passwords are prompted interactively or supplied with `IBC_ATTESTOR_KEYSTORE_PASSWORD`; they are not stored in TOML. `--keystore-password` is available for automation but can expose the secret in process listings such as `ps`, so prefer the prompt or environment variable. Empty-password keystores require `--empty-keystore-password` or an explicitly empty `IBC_ATTESTOR_KEYSTORE_PASSWORD`. See [`docs/configuration.md`](docs/configuration.md) for the full field reference.

### Finality offset (EVM only)

`[adapter].finality_offset` controls which height the attestor treats as the chain's latest finalized block. It bounds the `height` accepted by `StateAttestation` and
`PacketAttestation` — anything above is rejected with `BlockNotFinalized`.

  ```toml
  [adapter]
  # ... url, router_address, etc.
  # finality_offset = 12     # optional
  ```

| `finality_offset` | Behavior                                                                                               |
  |-------------------|--------------------------------------------------------------------------------------------------------|
| **Omitted / `None`** *(default)* | Adapter calls the RPC with the `finalized` block tag and trusts the chain's own definition of finality |
| **`Some(n)`**      | Adapter calls the RPC with `latest` and treats `latest - n` as finalized.                              |

Cosmos and Solana adapters ignore this field — they use chain-native finality (Tendermint consensus and Solana's `finalized` commitment respectively).

## Security

Within the context of IBC relaying IBC attestors are an off-chain trusted service. Trust is established with on-chain components via two mechansims:
- Securely managed secp256k1 signing keys used by attestors to create attestations. The public parts of the keys must be registered with an on-chain light client;
- Aggregating attestor signatures at relay time to satisfy the quorum the on-chain light client enforces.

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

**Reporting vulnerabilities.** See [`SECURITY.md`](SECURITY.md).

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

### Chain adapters

To add support for new kinds of chains you need to implement the `AttestationAdapter` and `AdapterBuilder` [interfaces](https://github.com/cosmos/ibc-attestor/blob/main/apps/ibc-attestor/src/adapter/mod.rs) interfaces, respectively.

- The `AttestationAdapter` is responsible for retrieving on-chain state and ensuring this state can be parsed as:
    - A valid height and timestamp for a `StateAttestation`
    - A valid 32-byte commitment path for an IBC Packet
- The `AdapterBuilder` enables per chain configurations needed by the `AttestationAdapter` implementation.

The CLI must also be extended to support any new chain types.

## Signing requirements

Currently the IBC attestor supports two signing modes: local and remote. The attestor signing algorithm is as follows:
1. Retrieve relevant chain/packet state via the chain adapter
2. Encode the data using the ABI format to facilitate EVM parsing
3. Send the encoded message to the signer which first hashes and then signs the data in ECDSA 65-byte recoverable signature (r||s||v)

Any new signer implementations **must guarantee**:
- Arbitrary ABI-encoded data is hashed before signing
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
- OTLP export support for Grafana Tempo, Jaeger, and other backends

See [Tracing Configuration](docs/tracing.md) for details on enabling OTLP trace export.
