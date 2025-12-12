FROM rust:1.88-bullseye AS build

RUN apt-get update && apt-get install -y

# Default version on apt does not support optional fields
ENV PROTOC_VERSION=3.15.8
RUN curl -L -o /tmp/protoc.zip https://github.com/protocolbuffers/protobuf/releases/download/v30.2/protoc-30.2-linux-x86_64.zip \
 && unzip /tmp/protoc.zip -d /opt/protoc \
 && mv /opt/protoc/bin/protoc /usr/local/bin/protoc \
 && mv /opt/protoc/include /usr/local/include/protoc \
 && rm -rf /tmp/protoc.zip /opt/protoc

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY apps apps
COPY libs libs
COPY proto proto

RUN cargo build --bin ibc_attestor --release --locked

FROM debian:bullseye-slim

ARG PORT

RUN apt-get update && apt-get install -y \
    ca-certificates

WORKDIR /usr/local/bin
COPY --from=build /app/target/release/ibc_attestor /usr/local/bin/ibc_attestor

# The chain type must be specified when running the container
# Mounts should be used to provide the configuration file
# Example: docker run ... ibc_attestor server --config /tmp/attestor.toml --chain-type evm
ENTRYPOINT ["ibc_attestor"]
