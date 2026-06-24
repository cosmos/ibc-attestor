# Attestor as a go-plugin — PoC

Runs the Rust IBC attestor as a [HashiCorp go-plugin](https://github.com/hashicorp/go-plugin)
under a Go host. The host launches the attestor as a managed subprocess, negotiates a gRPC
connection, and drives it — proving that go-plugin can act as the supervisor/lifecycle manager
for the attestor.

go-plugin supports non-Go plugins over gRPC: the plugin prints a handshake line on stdout and
serves its gRPC service plus the standard gRPC health service. The attestor already speaks gRPC
(tonic `AttestationService`), so the plugin support is a small, additive `--plugin-mode` that
leaves the normal `server` path untouched.

```
Go host (hashicorp/go-plugin)
  │ spawn + handshake (magic cookie env, protocol negotiation)
  │ surfaces plugin stderr (Stderr: os.Stderr)                    [goal 1: logs]
  │ scrapes http://127.0.0.1:8081/metrics                          [goal 2: metrics]
  ▼
ibc_attestor server --plugin-mode   (Rust subprocess)
  stdout: 1|1|tcp|127.0.0.1:<ephemeral>|grpc     ← handshake line only
  stderr: JSON logs                             ← surfaced by host [goal 1]
  gRPC  : AttestationService + grpc.health.v1 (service "plugin" = SERVING)
  http  : /metrics + /healthz on 127.0.0.1:8081 (fixed)
```

## What it proves

| Goal | How the host verifies it |
|------|--------------------------|
| **1. Logs** | The attestor logs JSON to stderr; the host surfaces it (`ClientConfig.Stderr = os.Stderr`), so the attestor's structured logs appear in the host's output. |
| **2. Metrics** | The host scrapes `http://127.0.0.1:8081/metrics` and prints `attestor_rpc_requests_total` / `attestor_signer_signs_total`. |
| **3. Crash recovery** | The host SIGKILLs the attestor to simulate a crash, detects the exit (`client.Exited()`), relaunches a fresh plugin, and fetches again. |
| **4. Attestation** | The host calls `LatestHeight` then `StateAttestation` and prints the returned height, `attested_data`, and 65-byte signature. |

## Prerequisites

- Rust toolchain (to build the attestor)
- Go ≥ 1.25
- `protoc`, plus the Go plugins:
  ```
  go install google.golang.org/protobuf/cmd/protoc-gen-go@latest
  go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@latest
  ```

## Run

```bash
# from this directory
make setup     # build both binaries into ./bin, generate stubs, create a throwaway keystore
make run       # build + launch ./bin/host; exercises all four checks
```

`make build` puts the host and attestor binaries side by side in `./bin`, so the host finds the
attestor **beside its own executable** — no path config needed. Override with `ATTESTOR_BIN` for a
custom layout (`findAttestorBin` in `main.go`). `make clean` removes the binaries, stubs, and keystore.

## How plugin mode works (attestor side)

`ibc_attestor server --plugin-mode` differs from the normal server only in bootstrap:

1. **Handshake** — binds the gRPC server to an **ephemeral loopback port** (`127.0.0.1:0`,
   the OS picks a free one) and prints `1|1|tcp|127.0.0.1:<port>|grpc` on stdout, which
   go-plugin reads and dials.
2. **Health** — registers `grpc.health.v1.Health` with the well-known service name `"plugin"` set
   to `SERVING`, which go-plugin polls for liveness.
3. **Logs** — writes JSON to **stderr** (stdout is reserved for the handshake); the host surfaces
   it via `ClientConfig.Stderr`.
4. **Metrics/health HTTP** — stays on the fixed `health_addr` (`127.0.0.1:8081`) so the Prometheus
   endpoint is scrapeable at a stable address even though the gRPC socket is per-launch.

The keystore password (`IBC_ATTESTOR_KEYSTORE_PASSWORD=poc`) is passed to the plugin via its
environment by the host (go-plugin also sets its handshake magic-cookie env automatically).

## Unified config (one file the operator edits)

The host owns a single `host.yaml`. Host settings live under `host:`; the attestor's settings live
under `attestor:`, which the host treats as an **opaque pass-through** — it never models the
attestor's schema. At launch the host reads `host.yaml`, extracts the `attestor` map, re-serializes
it to a private `0600` temp **TOML** file (the attestor reads TOML, so the host's YAML and the
attestor's TOML stay independent), and starts the subprocess with `--config <tempfile>` (removed on
exit). See `materializeAttestorConfig` in `main.go`.

**Secrets are never written to the temp file**: the keystore password rides in via
`IBC_ATTESTOR_KEYSTORE_PASSWORD`, which the attestor's loader deliberately ignores in config
(`#[serde(skip)]`).

> Read-only-rootfs note: the temp file needs a writable dir (e.g. a memory-backed `emptyDir`).
> Passing the config via an env var instead would avoid the write entirely.

## Notes / limitations

- The host stops plugins with `SIGTERM` (the attestor's graceful path), so normal shutdowns are
  clean. The one `signal: killed` ERROR in a run is the host SIGKILLing the attestor to demonstrate
  crash recovery.
- The plugin's gRPC is an **ephemeral loopback TCP port**, reachable by any process in the network
  namespace. Serving over a permission-restricted Unix socket would scope it to the host user —
  left out here for simplicity.
- Uses the EVM adapter against a public Sepolia RPC and a local keystore. The signing key is a
  throwaway generated by `make setup`; signatures are real but not from a registered attestor.
