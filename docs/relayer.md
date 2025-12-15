# IBC v2 Relayer

IBC v2 Relayer is a standalone, production-ready, request-driven relayer service for the IBC v2 Protocol. The relayer supports interoperating between a Cosmos-based chain and major EVM networks (Ethereum, Base, Optimism, Arbitrum, Polygon, and more). The core relayer service has been used in production since 2023 via Skip Go, and is now modularized to be run on-prem by clients who cannot leverage the Skip Go managed service.

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


## Upcoming Features

- Solana support

## Unsupported Features

- Charging end users fees to relay IBC transactions
- Relaying IBC v1 packets
