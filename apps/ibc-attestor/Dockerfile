# Base toolchain image: Rust + protoc + cargo-chef. Stable layer, cached across builds.
FROM rust:1.91-bookworm AS chef

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl unzip \
    && rm -rf /var/lib/apt/lists/*

# Default version on apt does not support optional fields
ENV PROTOC_VERSION=3.15.8
ARG TARGETARCH
RUN case "${TARGETARCH}" in \
        "amd64")  PROTOC_ARCH="x86_64"  ;; \
        "arm64")  PROTOC_ARCH="aarch_64"  ;; \
        *)        echo "Unsupported architecture: ${TARGETARCH}" && exit 1 ;; \
    esac \
 && curl -L -o /tmp/protoc.zip https://github.com/protocolbuffers/protobuf/releases/download/v30.2/protoc-30.2-linux-${PROTOC_ARCH}.zip \
 && unzip /tmp/protoc.zip -d /opt/protoc \
 && mv /opt/protoc/bin/protoc /usr/local/bin/protoc \
 && mv /opt/protoc/include /usr/local/include/protoc \
 && rm -rf /tmp/protoc.zip /opt/protoc

RUN cargo install cargo-chef --locked
WORKDIR /app

# Plan stage: distil the workspace into a dependency recipe. recipe.json only
# changes when Cargo manifests or the lockfile change, not on app source edits.
FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY apps apps
COPY libs libs
COPY proto proto
RUN cargo chef prepare --recipe-path recipe.json

# Build stage: compile dependencies from the recipe first. This layer is cached
# and reused as long as dependencies are unchanged, so source-only changes skip
# straight to the final (fast) application compile.
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Note: no --locked here. cargo-chef rebuilds a skeleton workspace and re-serializes
# the captured Cargo.lock, which cargo then flags as needing an update, so --locked
# panics. Dependency versions stay pinned via the lock captured in the recipe, and the
# final `cargo build --locked` below strictly enforces the real Cargo.lock.
RUN cargo chef cook --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY apps apps
COPY libs libs
COPY proto proto
RUN cargo build --bin ibc_attestor --release --locked

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 65532 -g nogroup nonroot

ARG BUILD_DATE
ARG VCS_REF
ARG VERSION
ARG COMMIT
LABEL org.opencontainers.image.title="ibc-attestor" \
      org.opencontainers.image.description="Cosmos IBC Attestor" \
      org.opencontainers.image.source="https://github.com/cosmos/ibc-attestor" \
      org.opencontainers.image.revision=$VCS_REF \
      org.opencontainers.image.created=$BUILD_DATE \
      org.opencontainers.image.version=$VERSION

COPY --from=builder --chown=nonroot:nogroup /app/target/release/ibc_attestor /bin/ibc_attestor

ENV PLATFORM_COMMIT=${COMMIT}
EXPOSE 8090 9000
USER nonroot:nogroup
# The chain type must be specified when running the container
# Mounts should be used to provide the configuration file
# Example: docker run ... ibc_attestor server --config /tmp/attestor.toml --chain-type evm
ENTRYPOINT ["ibc_attestor"]

