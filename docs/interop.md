# ibc-attestor interop overview

For attestor service architecture, see the root `README.md`. High-level interoperability is captured in the system diagram below.

## System diagram
![IBC system diagram](system-diagram.png)

- Blue = on-chain contracts (EVM), Purple = IBC/SDK modules, Orange = off-chain infrastructure, dashed arrows = proofs/verification.

## Components and code links

**On-chain: Modules**
- Core IBC and GMP modules use ibc-go: [cosmos/ibc-go](https://github.com/cosmos/ibc-go/tree/main/modules) (core stack, ICS26 router, ICS27/ICA for GMP, callbacks middleware).
- Attestor light client for Cosmos SDK chains: [cosmos/ibc-go attestor light client](https://github.com/cosmos/ibc-go/tree/main/modules/light-clients/attestations).
- Token Factory (chain application layer) consumes IBC packets for mint/burn; it sits above the ibc-go stack on the Cosmos chain.

**On-chain: Contracts**
- IBC stack (ICS26 router, callbacks, GMP, storage) and application contracts (e.g., IFT/token logic) are in [cosmos/solidity-ibc-eureka](https://github.com/cosmos/solidity-ibc-eureka/tree/main/contracts).
- Attestor light client for EVM lives in [cosmos/solidity-ibc-eureka](https://github.com/cosmos/solidity-ibc-eureka/tree/main/packages/attestor/light-client) and the Solidity light-client interfaces in `contracts/light-clients/`.

**Off-chain: Attestation Service**
- This repo: [cosmos/ibc-attestor](https://github.com/cosmos/ibc-attestor) (Rust service in `apps/ibc-attestor/`).

**Off-chain: Proof API**
- gRPC/Proof API: `proto/ibc_attestor/ibc_attestor.proto` defines `StateAttestation`, `PacketAttestation`, and `LatestHeight`.

**Off-chain: Relayer**
- Signature aggregation plus relaying lives in [cosmos/solidity-ibc-eureka](https://github.com/cosmos/solidity-ibc-eureka/tree/main/programs/relayer) with shared proof builders in `packages/relayer/`.
- The relayer queries the AttestationService, enforces quorum, assembles proofs, and submits them to both chains.

## High-level flow
- Attestors watch the source chain, sign headers and packet commitments, and expose them via the Proof API.
- The relayer fetches and aggregates signatures, then submits proofs to the EVM light client and the Cosmos chain attestor light client.
- The on-chain IBC stacks (Solidity and ibc-go) verify the proofs, update light clients, and route packets to the Token Factory or IFT application logic.
