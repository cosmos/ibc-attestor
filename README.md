# ibc-attestor

Off-chain attestors that watch a source chain, sign state/packet data with an m-of-n key set, and expose those signatures to relayers so IBC light clients on other chains can be updated quickly.

## What the attestor service does
- Runs per-chain adapters to query finalized headers and packet commitments, then signs ABI-encoded payloads with secp256k1 keys (Ethereum-style recoverable signatures).
- Serves the signed data over gRPC (`proto/ibc_attestor/ibc_attestor.proto`) so aggregators/relayers can fetch state and packet attestations.
- Works with on-chain attestor light clients (Cosmos SDK and Solidity) that verify the aggregated signatures and advance IBC clients.
- More implementation details live in `ARCHITECTURE.md` and the binary entrypoint in `apps/ibc-attestor/`.

## System diagram
![WF IBC system diagram](docs/system-diagram.png)

- Blue = on-chain contracts (EVM), Purple = IBC/SDK modules, Orange = off-chain infrastructure, dashed arrows = proofs/verification.

## Components and code links

**Attestor service (off-chain)**
- This repo: https://github.com/cosmos/ibc-attestor (Rust service in `apps/ibc-attestor/`).
- gRPC/Proof API: `proto/ibc_attestor/ibc_attestor.proto` defines `StateAttestation`, `PacketAttestation`, and `LatestHeight`.

**Relayer and aggregation (off-chain)**
- Signature aggregation plus relaying lives in https://github.com/cosmos/solidity-ibc-eureka/tree/main/programs/relayer with shared proof builders in `packages/relayer/`.
- The relayer queries the AttestationService, enforces quorum, assembles proofs, and submits them to both chains.

**Ethereum / EVM side**
- IBC stack (ICS26 router, callbacks, GMP, storage) and application contracts (e.g., IFT/token logic) are in https://github.com/cosmos/solidity-ibc-eureka/tree/main/contracts.
- Attestor light client for EVM lives in https://github.com/cosmos/solidity-ibc-eureka/tree/main/packages/attestor/light-client and the Solidity light-client interfaces in `contracts/light-clients/`.

**WF chain (Cosmos SDK) side**
- Core IBC and GMP modules use ibc-go: https://github.com/cosmos/ibc-go/tree/main/modules (core stack, ICS26 router, ICS27/ICA for GMP, callbacks middleware).
- Attestor light client for Cosmos SDK chains: https://github.com/cosmos/ibc-go/tree/main/modules/light-clients/attestations.
- Token Factory (chain application layer) consumes IBC packets for mint/burn; it sits above the ibc-go stack on the WF chain.

## High-level flow
- Attestors watch the source chain, sign headers and packet commitments, and expose them via the Proof API.
- The relayer fetches and aggregates signatures, then submits proofs to the EVM light client and the WF chain attestor light client.
- The on-chain IBC stacks (Solidity and ibc-go) verify the proofs, update light clients, and route packets to the Token Factory or IFT application logic.
