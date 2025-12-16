# IBC v2 Relayer

IBC v2 Relayer is a standalone, production-ready, request-driven relayer service for the IBC v2 Protocol. The relayer supports interoperating between a Cosmos-based chain and major EVM networks (Ethereum, Base, Optimism, Arbitrum, Polygon, and more). The core relayer service has been used in production since 2023 via Skip Go, and is now modularized to be run on-prem by clients who cannot leverage the Skip Go managed service.

## Relaying Sequence
![Relaying Sequence](relaying-sequence.png)

## Supported Features

- Compatible with all major EVM chains (Ethereum, Base, Optimism, Arbitrum, Polygon, and more)
- Request-driven design for configurable, on-demand relaying
- Failure retry support
  - Re-orgs
  - Out-of-gas
  - Inadequate gas price
  - Tx network propagation fails to reach leader
  - Invalid by the network, but valid by the submitting node
- Transaction Tracking API
- Remote signing support
- Concurrent packet intake and processing
- Configurable packet delivery latency via batching
- Ability to blacklist addresses (ex: OFAC)
- Transaction cost tracking

## Design
![Design](relayer-design.png)
The relayer has three main components - the gRPC server which clients use to interact with the relayer, a postgres db, and the core relayer. The gRPC server populates the db with packets, which the core relayer monitors and updates as it progresses in relaying those packets.

![Relaying Pipeline](relaying-pipeline.png)
The relayer is designed as a pipeline which is composed of a set of asynchronously running processors. Transfers pass through the processors sequentially. Some pipeline steps process transfers individually while others process transfers in [batches](https://etherscan.io/tx/0x49c7d94cd2d28cadfdeccc546edc67548b31f2fa4d3126495453832ece919c8d).

## API Interface
The relayer serves a gRPC server which clients use to specify what packets to relay and track packet relaying progress.

```proto
service RelayerApiService {
    // Relay is used to specify a source tx hash for packets the relayer should relay.
    // The relayer will identify all packets created by the transaction and attempt to relay them all.
    rpc Relay(RelayRequest) returns (RelayResponse) {}

    // The status endpoint is used to track the progress of packet relaying.
    // It takes a transaction hash and returns the status of any relevant packets the relayer is aware of.
    // The transaction must first have been passed to the relay endpoint.
    rpc Status(StatusRequest) returns (StatusResponse) {}
}

message StatusRequest {
    string tx_hash = 1;
    string chain_id = 2;
}

enum TransferState {
    TRANSFER_STATE_UNKNOWN = 0;
    TRANSFER_STATE_PENDING = 1;
    TRANSFER_STATE_COMPLETE = 2;
    TRANSFER_STATE_FAILED = 3;
}

message TransactionInfo {
    string tx_hash = 1;
    string chain_id = 2;
}

message PacketStatus {
    TransferState state = 1;
    uint64 sequence_number = 2;
    string source_client_id = 3;
    TransactionInfo send_tx = 4;
    TransactionInfo recv_tx = 5;
    TransactionInfo ack_tx = 6;
    TransactionInfo timeout_tx = 7;
}

message StatusResponse {
    repeated PacketStatus packet_statuses = 1;
}

message RelayRequest {
    string tx_hash = 1;
    string chain_id = 2;
}

message RelayResponse {}
```

## Observability

| Type      | Name                                   | Description |
|-----------|----------------------------------------|-------------|
| Metric    | Relayer api request count               | Paginated by method and response code |
| Metric    | Relayer api request latency             | Paginated by method |
| Metric    | Transfer count                          | Paginated by source, destination chain, and transfer state |
| Metric    | Relayer gas balance                     | Paginated by chain and gas token |
| Metric    | Relayer gas balance state               | A gauge where each value represents a gas balance state. 0 = ok, 1 = warning, 2 = critical. The thresholds that define each state are defined in the relayer configuration. Paginated by chain |
| Metric    | External request count                  | Paginated by endpoint, method and response code |
| Metric    | External request latency                | Paginated by endpoint and method |
| Metric    | Transactions submitted counter          | Paginated by node response success status and chain |
| Metric    | Transaction retry counter               | Paginated by source and destination chain |
| Metric    | Transactions confirmed counter          | Paginated by execution success and chain |
| Metric    | Transaction gas cost counter            | Paginated by chain |
| Metric    | Relay latency                           | Time between send tx and ack/timeout tx. Paginated by source and destination chain |
| Metric    | Detected client update required counter | Paginated by chain |
| Metric    | Client updated counter                  | Paginated by chain |
| Metric    | Excessive relay latency counter         | Incremented anytime a transfer is pending for longer than some configured threshold. Paginated by source and destination chain |
| Alert     | Excessive relay latency                 | Should alert whenever the excessive relay latency counter increases |
| Alert     | Excessive gas usage                     | Should alert whenever the gas cost counter increases faster than some threshold |
| Alert     | Low gas balance                         | Should alert whenever the relayer gas balance state metric is in the warning or critical state |


## Configuring the Relayer

The relayer is configured via a yaml file. The following are examples of functionality that can be configured.

### Adding a New Chain
```yaml
chains:
  ethereum_testnet:
    chain_name: 'sepolia'
    chain_id: '11155111'
    type: 'evm'
    environment: 'testnet'
    evm:
      rpc: https://eth-spolia.g.alchemy.com/v2/abcdef
      contracts:
        ics_26_router_address: 0x3fcBB8b5d85FB5F77603e11536b5E90FeE37e6c0
        ics_20_transfer_address: 0x3a4e076D1c5EBfC813993c497Bb284598121b515
```
Adding a new chain involves adding a new entry under the chains list. The entry will configure information like chain id,
chain type, node rpcs, and contract addresses.

### Configuring Batch Sizes

```yaml
chains:
  ethereum_testnet:
    ibc_v2:
      ack_batch_size: 100
      ack_batch-timeout: 10s
      recv_batch_size: 100
      timeout_batch_size: 100
```
In the above snippet, `ack_batch_size: 100` indicates that the relayer should allow batching of up to 100 ack packets.
`ack_batch-timeout: 10s` indicates that the relayer will wait 10 seconds for acknowledgements to accumulate in the current batch before it relays the batch.

### Configuring Gas Threshold Metrics

```yaml
chains:
  ethereum_testnet:
    signer_gas_alert_thresholds:
      ibc_v2:
        warning_threshold: 1000000000000000000 # 1 eth
        critical_threshold: 500000000000000000 # 0.5 eth
```
If the operator would like to be alerted when the relaying wallet is running low on gas, they can use the `signer_gas_alert_thresholds` config to specify thresholds at which the relayer exports a metric indicating the gas balance is at a warning level and critical level.


## Upcoming Features

- Solana support

## Unsupported Features

- Charging end users fees to relay IBC transactions
- Relaying IBC v1 packets
