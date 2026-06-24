#!/usr/bin/env bash
# Generate Go gRPC stubs for the attestor proto into ./gen/attestorpb.
# Requires protoc, protoc-gen-go and protoc-gen-go-grpc on PATH:
#   go install google.golang.org/protobuf/cmd/protoc-gen-go@latest
#   go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@latest
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
export PATH="$(go env GOPATH)/bin:$PATH"

MODPREFIX="github.com/cosmos/ibc-attestor/examples/go-plugin-host/gen"
PKG="$MODPREFIX/attestor"

rm -rf "$HERE/gen"
mkdir -p "$HERE/gen"

protoc -I "$ROOT/proto" \
  --go_out="$HERE/gen" --go_opt=module="$MODPREFIX" \
  --go_opt=Mibc_attestor/ibc_attestor.proto="$PKG" \
  --go_opt=Mibc_attestor/attestation.proto="$PKG" \
  --go-grpc_out="$HERE/gen" --go-grpc_opt=module="$MODPREFIX" \
  --go-grpc_opt=Mibc_attestor/ibc_attestor.proto="$PKG" \
  --go-grpc_opt=Mibc_attestor/attestation.proto="$PKG" \
  ibc_attestor/attestation.proto ibc_attestor/ibc_attestor.proto

echo "generated:"
ls -1 "$HERE/gen/attestor"
