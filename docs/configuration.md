# Configuration reference

The attestor reads a single TOML file passed via `--config`. The file has four top-level tables:

| Table       | Required | Shape determined by              | Purpose                                     |
|-------------|----------|----------------------------------|---------------------------------------------|
| `[server]`  | yes      | fixed                            | gRPC and HTTP listener addresses            |
| `[adapter]` | yes      | `--chain-type` (`evm`/`cosmos`/`solana`) | Chain RPC endpoint and chain-specific knobs |
| `[signer]`  | yes      | `--signer-type` (`local`/`remote`) | Where the secp256k1 key lives               |
| `[tracing]` | no       | fixed                            | OpenTelemetry export (off by default)       |

`[adapter]` and `[signer]` are deserialized into the concrete struct selected by the matching CLI flag. Putting a field that doesn't belong to that adapter/signer type produces a TOML deserialization error at startup.

A working EVM + local-signer example lives at [`apps/ibc-attestor/server.dev.toml`](../apps/ibc-attestor/server.dev.toml).

---

## `[server]`

Defined by `ServerConfig` in `apps/ibc-attestor/src/config.rs`.

| Field         | Type         | Required | Description |
|---------------|--------------|----------|-------------|
| `listen_addr` | `SocketAddr` | yes      | Address the gRPC server binds to (e.g. `0.0.0.0:8080`). |
| `health_addr` | `SocketAddr` | yes      | Address the HTTP server binds to. Exposes `GET /healthz` (returns `200` once the gRPC server is up) and `GET /metrics` (Prometheus format, labelled by `adapter` and `signer`). |

```toml
[server]
listen_addr = "0.0.0.0:8080"
health_addr = "0.0.0.0:8081"
```

---

## `[adapter]`

The shape of this table is selected at startup by `--chain-type`. Each chain has its own fields.

`url` accepts `http://` and `https://`. For `https://` endpoints, the server certificate is checked against the root certificates of the OS the attestor runs on, which in the Docker image means the image's CA bundle. See [TLS and trusted root certificates](#tls-and-trusted-root-certificates).

### EVM (`--chain-type evm`)

Defined by `EvmAdapterConfig` in `apps/ibc-attestor/src/adapter/evm.rs`.

| Field             | Type            | Required | Description |
|-------------------|-----------------|----------|-------------|
| `url`             | `Url`           | yes      | HTTP(S) JSON-RPC endpoint for the chain. |
| `router_address`  | `Address`       | yes      | ICS-26 router contract address, hex-encoded with `0x` prefix. |
| `finality_offset` | `u64` (optional) | no       | If unset, the adapter uses the RPC's `finalized` block tag. If set to `n`, the adapter uses `latest - n` instead. See [Finality offset](#finality-offset-evm-only). |

```toml
[adapter]
url            = "https://ethereum-sepolia-rpc.publicnode.com"
router_address = "0xff42b3db9f1040539a3741434e4b33b352fabd80"
# finality_offset = 12       # optional
```

### Cosmos (`--chain-type cosmos`)

Defined by `CosmosAdapterConfig` in `apps/ibc-attestor/src/adapter/cosmos.rs`.

| Field | Type  | Required | Description |
|-------|-------|----------|-------------|
| `url` | `Url` | yes      | Tendermint RPC endpoint (e.g. `https://rpc.cosmos.network:443`). The adapter uses `latest_commit()` — Tendermint BFT gives instant finality on every committed block, so there is no offset to configure. |

```toml
[adapter]
url = "https://rpc.cosmos.network:443"
```

### Solana (`--chain-type solana`)

Defined by `SolanaAdapterConfig` in `apps/ibc-attestor/src/adapter/solana.rs`.

| Field                | Type     | Required | Description |
|----------------------|----------|----------|-------------|
| `url`                | `String` | yes      | Solana JSON-RPC endpoint. |
| `router_program_id`  | `String` | yes      | Base58 program ID of the IBC router on Solana. Also accepted as `router_address` (serde alias). |

The adapter queries with `CommitmentConfig::finalized()` — Solana's native finality level — so no offset is configurable.

```toml
[adapter]
url               = "https://api.devnet.solana.com"
router_program_id = "..."
```

### TLS and trusted root certificates

The attestor doesn't ship its own list of trusted certificate authorities. For `https://` RPC endpoints, all adapters (EVM, Cosmos and Solana) trust the root certificates in the **operating system's trust store**. The remote signer's `https://` endpoint uses the same store. Certificates are checked by rustls-webpki 0.103.16 or later, except for the EVM adapter on macOS and Windows, which hands certificate checks to the operating system.

Where the roots come from depends on how the attestor runs:

| Runtime | Trusted roots |
|---------|---------------|
| Docker image (`apps/ibc-attestor/Dockerfile`) | Debian's `ca-certificates` package installed in the image (`/etc/ssl/certs`). Updating the trusted roots means rebuilding the image. |
| Native binary on Linux | The host's system CA bundle, e.g. `/etc/ssl/certs` from the distribution's `ca-certificates` package. |
| Native binary on macOS | The system Keychain. |

When running the attestor as a native binary, keep the host's CA certificates updated to the latest release. Certificate authorities are added to and removed from the root stores over time, and the attestor trusts whatever the host has installed. On Linux, update the `ca-certificates` package regularly (e.g. `apt-get update && apt-get install --only-upgrade ca-certificates` on Debian/Ubuntu). On macOS, root certificate updates come with system updates.

On every platform, the `SSL_CERT_FILE` (a PEM file) and `SSL_CERT_DIR` (a colon-separated list of directories) environment variables **replace** the system store for the Cosmos and Solana adapters and the remote signer, and for the EVM adapter on Linux. If either is set, only those certificates are trusted.

To trust a private CA, for example for an internal RPC node:

- **Docker:** build a derived image that adds the CA. The image runs as `nonroot`, so switch to `root` to install it:

  ```dockerfile
  FROM ghcr.io/cosmos/ibc-attestor:<version>
  USER root
  COPY my-ca.crt /usr/local/share/ca-certificates/my-ca.crt
  RUN update-ca-certificates
  USER nonroot:nogroup
  ```

  Alternatively, mount a PEM bundle and point `SSL_CERT_FILE` at it. That bundle must contain the public CAs too, because it replaces the image's store.
- **Native:** add the CA to the host's trust store.

If an adapter's `url` uses `https://` and no usable root certificates are found, the Cosmos and Solana adapters fail at startup with `no usable root certificates found in the OS trust store`. Certificates that fail to load or parse are logged as warnings.

`http://` endpoints verify no certificates and need no root certificates. The attestor logs a warning when an `http://` endpoint points at a non-local host, because responses can be read or changed in transit. The Cosmos and Solana adapters don't follow redirects from `http://` endpoints, so configure the URL the node actually serves on, e.g. its `https://` URL when the node redirects there.

---

## `[signer]`

The shape of this table is selected at startup by `--signer-type`.

### Local (`--signer-type local`)

Defined by `LocalSignerConfig` in `apps/ibc-attestor/src/signer/local.rs`. The signing key is generated separately with `ibc_attestor key generate`.

The local keystore password is not stored in TOML. Password source precedence is `--keystore-password <password>`, `--empty-keystore-password`, `IBC_ATTESTOR_KEYSTORE_PASSWORD`, then the interactive prompt. `--keystore-password` can expose the secret in process listings such as `ps`, so prefer the interactive prompt or `IBC_ATTESTOR_KEYSTORE_PASSWORD` when possible. Empty-password keystores require `--empty-keystore-password` or an explicitly empty `IBC_ATTESTOR_KEYSTORE_PASSWORD`. If no password source is available, startup/key loading fails instead of trying an empty password implicitly.

| Field           | Type     | Required | Description |
|-----------------|----------|----------|-------------|
| `keystore_path` | `PathBuf` | yes     | Path to the keystore file. A leading `~/` is expanded to `$HOME`. |

```toml
[signer]
keystore_path = "~/.ibc-attestor/ibc-attestor-keystore"
```

### Remote (`--signer-type remote`)

Defined by `RemoteSignerConfig` in `apps/ibc-attestor/src/signer/remote.rs`.

| Field                         | Type             | Required | Description |
|-------------------------------|------------------|----------|-------------|
| `endpoint`                    | `Url`            | yes      | gRPC endpoint of the remote signer service (e.g. `https://remote-signer.example:50051`). Must use `https://` unless `allow_insecure_plaintext` is enabled for tests. |
| `wallet_id`                   | `String`         | yes      | Identifier of the wallet to use for signing on the remote service. |
| `allow_insecure_plaintext`    | `bool`           | no       | Allows `http://` plaintext gRPC. Defaults to `false`; intended only for non-production test environments. |
| `service_account_token_path`  | `PathBuf` (optional) | no   | Path to a file containing a bare JWT (no JSON envelope) — the same format Kubernetes populates for `kubernetes.io/service-account-token`-typed Secrets. When set, the token is read on each signing request and sent as `Authorization: Bearer <token>`. |

```toml
[signer]
endpoint  = "https://remote-signer.example:50051"
wallet_id = "ibc-attestor-prod"
# service_account_token_path = "/var/run/secrets/kubernetes.io/serviceaccount/token"
```

---

## `[tracing]` (optional)


| Field           | Type     | Required when section present | Description |
|-----------------|----------|-------------------------------|-------------|
| `otlp_endpoint` | `Url`    | yes                           | OTLP gRPC endpoint (e.g. `http://tempo:4317`). |
| `service_name`  | `String` | yes                           | Service name attached to all spans. |
| `sample_rate`   | `f64`    | yes                           | Sampling ratio in `[0.0, 1.0]`. `1.0` = sample every trace; values in `[0.0, 1.0)` use `TraceIdRatioBased`. Out-of-range or non-finite values are rejected at startup. |

The section is all-or-nothing: omit it entirely to disable export, or supply all three fields. Even without OTLP export, JSON logs always include `trace_id` and `span_id` for correlation.

```toml
[tracing]
otlp_endpoint = "http://localhost:4317"
service_name  = "ibc-attestor"
sample_rate   = 1.0
```
