# IBC Attestor Deployment Guide

## Overview

The IBC Attestor is a stateless gRPC service that produces cryptographic attestations of blockchain state for use in IBC v2 cross-chain communication. It connects to a chain via an adapter (EVM, Cosmos, or Solana) and signs state at requested heights using either a local keystore or a remote signing service.

Each deployed attestor instance handles a single chain type. To attest multiple chains, deploy one instance per chain.

---

## Components

### 1. IBC Attestor (this service)

The core attestation service. Exposes a gRPC API on port `8090` (configurable) and a metrics endpoint on port `9000`.

**Image**: `ghcr.io/cosmos/ibc-attestor:latest`
**Binary**: `ibc_attestor`

### 2. Signer Service (external dependency)

The attestor requires a secp256k1 signing key. Two deployment modes are supported:

- **Local signer** — key is stored in an encrypted keystore file on disk, read directly by the attestor process
- **Remote signer** — the attestor delegates signing to an external gRPC signer service (e.g. `platform-signer`) over the network

For production deployments, the remote signer with KMS-backed keys is recommended. For simple or development deployments, a local keystore is sufficient.

### 3. Chain RPC Endpoint (external dependency)

Each attestor instance requires a live RPC endpoint for the chain it is attesting:

| Chain Type | Required Endpoint |
|------------|------------------|
| EVM | JSON-RPC HTTP endpoint (e.g. Alchemy, Infura, or self-hosted `geth`) |
| Cosmos | Tendermint RPC HTTP endpoint |
| Solana | Solana JSON-RPC HTTP endpoint |

---

## Configuration

The attestor is configured via a TOML file passed with `--config`. The chain type and signer mode are passed as CLI flags.

### Full configuration reference

```toml
[server]
# Address and port the gRPC server listens on.
# Default used in tests: 0.0.0.0:8080
# Dockerfile EXPOSE: 8090
listen_addr = "0.0.0.0:8090"

[adapter]
# RPC endpoint of the chain being attested.
# EVM: HTTP or HTTPS JSON-RPC URL
# Cosmos: Tendermint RPC URL (http or https)
# Solana: Solana RPC URL
url = "https://your-rpc-endpoint"

# EVM only: address of the deployed ICS-26 router contract on the chain.
router_address = "0x..."

# EVM only (optional): number of blocks to subtract from `latest` to
# determine the finalized height. If omitted, the `finalized` block tag
# is used directly (requires the RPC to support it).
# Use 0 for local/test networks where `finalized` may lag indefinitely.
finality_offset = 64

[signer]
# --- Local signer (--signer-type local) ---
# Path to the keystore file or directory. Supports ~ expansion.
keystore_path = "~/.ibc-attestor/ibc-attestor-keystore"

# --- Remote signer (--signer-type remote) ---
# gRPC endpoint of the remote signer service.
endpoint = "http://signer-service:9006"
# Wallet ID to request from the signer. Leave empty for singleton signers.
wallet_id = ""
```

> **Note:** Only the fields relevant to the chosen `--signer-type` need to be present. The `[adapter]` fields that apply depend on the `--chain-type`.

### EVM example (`attestor-evm.toml`)

```toml
[server]
listen_addr = "0.0.0.0:8090"

[adapter]
url = "https://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY"
router_address = "0xff42b3db9f1040539a3741434e4b33b352fabd80"
finality_offset = 64

[signer]
endpoint = "http://signer-service:9006"
wallet_id = "my-eth-wallet"
```

### Cosmos example (`attestor-cosmos.toml`)

```toml
[server]
listen_addr = "0.0.0.0:8090"

[adapter]
url = "https://rpc.cosmos-chain.example.com:443"

[signer]
endpoint = "http://signer-service:9006"
wallet_id = "my-cosmos-wallet"
```

### Local signer example (`attestor-local.toml`)

```toml
[server]
listen_addr = "0.0.0.0:8090"

[adapter]
url = "https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY"
router_address = "0xff42b3db9f1040539a3741434e4b33b352fabd80"

[signer]
keystore_path = "~/.ibc-attestor/ibc-attestor-keystore"
```

---

## CLI Reference

```
ibc_attestor server
  --config <PATH>                    Path to TOML config file (required)
  --chain-type <evm|cosmos|solana>   Chain adapter to use (required)
  --signer-type <local|remote>       Signing backend (default: local)

ibc_attestor key generate [--keystore <PATH>]
ibc_attestor key show [--keystore <PATH>] [--show-private]
```

---

## Key Management

Before running the attestor with a local signer, generate a keypair:

```bash
ibc_attestor key generate
# Writes to ~/.ibc-attestor/ibc-attestor-keystore by default

# Or specify a custom path:
ibc_attestor key generate --keystore /etc/ibc-attestor/keystore
```

Inspect the public key (and optionally the private key):

```bash
ibc_attestor key show
ibc_attestor key show --show-private
```

The **public key / Ethereum address** derived from this keypair must be registered on-chain with the IBC light client so that attestations are accepted. Coordinate this registration with your chain operator.

---

## Docker Deployment

### Building the image

```bash
docker build \
  --build-arg BUILD_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ") \
  --build-arg VCS_REF=$(git rev-parse HEAD) \
  --build-arg VERSION=$(git describe --tags) \
  -t ibc-attestor:latest \
  -f apps/ibc-attestor/Dockerfile \
  .
```

The Dockerfile performs a two-stage build:
1. **Builder** — Rust 1.88 on Debian Bookworm; compiles the binary with `cargo build --release --locked`
2. **Runtime** — `debian:bookworm-slim` with only `ca-certificates` and `libssl3`; runs as non-root user `nonroot` (uid 65532)

### Running with Docker

The config file must be mounted into the container. The chain type is passed as a CLI argument.

**With remote signer (recommended):**
```bash
docker run -d \
  --name ibc-attestor-evm \
  -p 8090:8090 \
  -p 9000:9000 \
  -v /path/to/config/dir:/mnt/config:ro \
  ibc-attestor:latest \
  server \
    --config /mnt/config/attestor.toml \
    --chain-type evm \
    --signer-type remote
```

**With local keystore:**
```bash
docker run -d \
  --name ibc-attestor-evm \
  -p 8090:8090 \
  -p 9000:9000 \
  -v /path/to/config/dir:/mnt/config:ro \
  -v /path/to/keystore/dir:/home/nonroot/.ibc-attestor:ro \
  ibc-attestor:latest \
  server \
    --config /mnt/config/attestor.toml \
    --chain-type evm \
    --signer-type local
```

### Docker Compose example

```yaml
services:
  ibc-attestor-evm:
    image: ibc-attestor:latest
    command:
      - server
      - --config
      - /mnt/config/attestor-evm.toml
      - --chain-type
      - evm
      - --signer-type
      - remote
    ports:
      - "8090:8090"
      - "9000:9000"
    volumes:
      - ./config:/mnt/config:ro
    restart: unless-stopped

  ibc-attestor-cosmos:
    image: ibc-attestor:latest
    command:
      - server
      - --config
      - /mnt/config/attestor-cosmos.toml
      - --chain-type
      - cosmos
      - --signer-type
      - remote
    ports:
      - "8091:8090"
      - "9001:9000"
    volumes:
      - ./config:/mnt/config:ro
    restart: unless-stopped
```

---

## Kubernetes Deployment

The attestor is stateless and well-suited for Kubernetes. Below is a reference manifest.

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ibc-attestor-evm
  namespace: ibc
spec:
  replicas: 1
  selector:
    matchLabels:
      app: ibc-attestor-evm
  template:
    metadata:
      labels:
        app: ibc-attestor-evm
    spec:
      securityContext:
        runAsNonRoot: true
        runAsUser: 65532
      containers:
        - name: ibc-attestor
          image: ghcr.io/cosmos/ibc-attestor:latest
          args:
            - server
            - --config
            - /mnt/config/attestor.toml
            - --chain-type
            - evm
            - --signer-type
            - remote
          ports:
            - name: grpc
              containerPort: 8090
            - name: metrics
              containerPort: 9000
          volumeMounts:
            - name: config
              mountPath: /mnt/config
              readOnly: true
          readinessProbe:
            grpc:
              port: 8090
            initialDelaySeconds: 5
            periodSeconds: 10
          livenessProbe:
            grpc:
              port: 8090
            initialDelaySeconds: 15
            periodSeconds: 30
      volumes:
        - name: config
          configMap:
            name: ibc-attestor-evm-config
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: ibc-attestor-evm-config
  namespace: ibc
data:
  attestor.toml: |
    [server]
    listen_addr = "0.0.0.0:8090"

    [adapter]
    url = "https://your-evm-rpc-endpoint"
    router_address = "0x..."
    finality_offset = 64

    [signer]
    endpoint = "http://signer-service:9006"
    wallet_id = "your-wallet-id"
---
apiVersion: v1
kind: Service
metadata:
  name: ibc-attestor-evm
  namespace: ibc
spec:
  selector:
    app: ibc-attestor-evm
  ports:
    - name: grpc
      port: 9006
      targetPort: 8090
```

---

## Ports

| Port | Protocol | Purpose |
|------|----------|---------|
| `8090` | gRPC (HTTP/2) | Attestation API — used by relayers and the fleet backend |
| `9000` | HTTP | Metrics (Prometheus) and observability |

> The `listen_addr` in your TOML config controls the gRPC port. `8090` matches the `EXPOSE` in the Dockerfile; adjust to match your config.

---

## Networking

### Signer connectivity

The attestor connects to the signer service at startup of each signing request (connections are created on-demand). The endpoint must be reachable from the attestor container:

- Same Docker network: use the container name and port (e.g. `http://signer:9006`)
- Kubernetes: use the service DNS name (e.g. `http://signer-service.ibc.svc.cluster.local:9006`)
- Cross-network: ensure firewall/security group rules permit TCP on the signer port

### Chain RPC connectivity

The attestor polls the chain RPC on each attestation request. Ensure the `adapter.url` is reachable from wherever the attestor runs. For EVM chains with `finality_offset` unset, the RPC must support the `finalized` block tag (standard on most clients; may not be available on test networks).

### DNS

When deploying alongside Kurtosis-managed chains or other service-mesh-based infrastructure, ensure the attestor container's DNS resolves the RPC hostnames correctly. Custom DNS servers can be set via Docker's `--dns` flag or Kubernetes `dnsConfig`.

---

## Observability

The attestor emits structured JSON logs via `tracing`. Log level is controlled by the `RUST_LOG` environment variable:

```bash
RUST_LOG=info   # default recommended level
RUST_LOG=debug  # verbose, includes per-request details
RUST_LOG=warn   # only warnings and errors
```

OpenTelemetry tracing is built in. Configure an OTEL exporter via the standard `OTEL_EXPORTER_*` environment variables if you want distributed traces.

Prometheus metrics are exposed on port `9000`.

---

## Health Checking

The attestor is considered ready when it can successfully serve a `StateAttestation` gRPC request. There is no separate health endpoint; use gRPC health probes or poll `StateAttestation` with a low height (e.g. `height=1`) to verify liveness.

A 30-second startup timeout is typical — the attestor needs the chain RPC and signer to be reachable before it can respond.

---

## Multi-Chain Deployments

Deploy one attestor instance per chain. Each instance gets its own config file specifying the chain's RPC URL and chain type. They can share a single signer service, differentiating keys by `wallet_id`.

```
attestor-evm    --chain-type evm    → EVM RPC
attestor-cosmos --chain-type cosmos → Tendermint RPC
attestor-solana --chain-type solana → Solana RPC
     ↓                  ↓                  ↓
              signer-service:9006
```

---

## On-Chain Registration

The Ethereum address derived from the attestor's signing key must be registered with the IBC light client on-chain before attestations will be accepted. After deploying:

1. Retrieve the public key/address:
   ```bash
   ibc_attestor key show
   # or via the container:
   docker run --rm -v /path/to/keystore:/home/nonroot/.ibc-attestor ibc-attestor:latest key show
   ```

2. Register the address with the IBC router contract or light client configuration as required by your chain's deployment.

For remote signers, retrieve the wallet's Ethereum address from the signer service's `GetWallet` RPC before deployment.

---

## Upgrade Path

The attestor is stateless — upgrades are a simple image replacement with no data migration required. Rolling restarts in Kubernetes are safe provided the signer service remains available during the rollout.
