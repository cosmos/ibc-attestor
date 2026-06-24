# Attestor as a go-plugin — PoC

Runs the Rust IBC attestor as a [HashiCorp go-plugin](https://github.com/hashicorp/go-plugin):
a Go host launches it as a managed subprocess, dials its gRPC, and drives it. The plugin support
is an additive `ibc_attestor server --plugin-mode` — the normal `server` path is untouched.

## Run

```bash
make setup   # build both binaries into ./bin, generate stubs, create a throwaway keystore
make run     # launch ./bin/host
```

Needs Rust, Go ≥ 1.25, and `protoc` with the Go plugins:

```bash
go install google.golang.org/protobuf/cmd/protoc-gen-go@latest
go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@latest
```

## What it shows

The host launches the attestor and verifies four things work under go-plugin supervision:

1. **Attestation** — calls `LatestHeight` + `StateAttestation`, prints the signed result.
2. **Logs** — the attestor's JSON stderr is surfaced on the host (`ClientConfig.Stderr`).
3. **Metrics** — scrapes `http://127.0.0.1:8081/metrics`.
4. **Crash recovery** — SIGKILLs the attestor, detects the exit, relaunches, fetches again.

## How it works

`--plugin-mode` makes the attestor speak go-plugin's protocol: it binds gRPC to an ephemeral
loopback port and prints the handshake line (`1|1|tcp|127.0.0.1:<port>|grpc`) on stdout for the
host to dial, serves the `grpc.health.v1.Health` service go-plugin polls, and logs to stderr
(stdout is reserved for the handshake). The host owns one `host.yaml`; its opaque `attestor:`
section is written to a temp TOML the subprocess reads, and the keystore password goes via env.

The host finds the attestor binary beside its own executable (where `make build` puts it), or via
`ATTESTOR_BIN`.

## Notes

- The gRPC is a loopback TCP port, reachable by any local process; a permission-restricted Unix
  socket would scope it to the host user — left out for simplicity.
- Uses the EVM adapter against a public Sepolia RPC and a throwaway keystore from `make setup`;
  signatures are real but not from a registered attestor.
