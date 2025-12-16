# Health Check Service

The IBC Attestor now includes a gRPC health check service that can be used by Kubernetes and other orchestration systems to verify that the attestor is running and ready to serve requests.

## Features

- **Standard gRPC Health Checking Protocol**: Implements the [gRPC Health Checking Protocol](https://github.com/grpc/grpc/blob/master/doc/health-checking.md)
- **Separate Port**: Runs on a separate port from the main attestation service to avoid interference
- **Configurable**: The health check port can be configured via the server configuration file
- **Service Discovery**: Includes gRPC reflection for easy service discovery

## Configuration

Add the `health_addr` field to your server configuration file:

```toml
[server]
listen_addr = "0.0.0.0:8080"
health_addr = "0.0.0.0:8081"  # Optional, defaults to port 8081
```

If `health_addr` is not specified, the health service will default to port 8081 on the same host as the main server.

## Usage

### Using grpcurl

Check the health status:

```bash
grpcurl -plaintext localhost:8081 grpc.health.v1.Health/Check
```

Example response:
```json
{
  "status": "SERVING"
}
```

### Using Kubernetes Probes

Add a readiness probe to your Kubernetes deployment:

```yaml
readinessProbe:
  exec:
    command:
    - grpcurl
    - -plaintext
    - localhost:8081
    - grpc.health.v1.Health/Check
  initialDelaySeconds: 5
  periodSeconds: 10
```

Or use a gRPC health check directly (if your Kubernetes version supports it):

```yaml
readinessProbe:
  grpc:
    port: 8081
    service: grpc.health.v1.Health
  initialDelaySeconds: 5
  periodSeconds: 10
```

## Implementation Details

- The health service starts **after** the main attestation gRPC server is initialized
- Both services handle graceful shutdown together
- The health service always returns `SERVING` status if it's responding
- The `Watch` RPC method is not implemented (returns `UNIMPLEMENTED`)

## Service Definition

The health service implements the following proto definition:

```protobuf
service Health {
  rpc Check(HealthCheckRequest) returns (HealthCheckResponse);
  rpc Watch(HealthCheckRequest) returns (stream HealthCheckResponse);
}
```

## Files Added/Modified

- `proto/health/health.proto` - Standard gRPC health check protocol definition
- `apps/ibc-attestor/src/rpc/health.rs` - Health service implementation
- `apps/ibc-attestor/src/rpc/health_server.rs` - Health server startup logic
- `apps/ibc-attestor/src/rpc/mod.rs` - Module exports
- `apps/ibc-attestor/src/config.rs` - Added `health_addr` configuration option
- `apps/ibc-attestor/src/bin/ibc_attestor/main.rs` - Updated to start health server
- `apps/ibc-attestor/build.rs` - Added health proto compilation
- `apps/ibc-attestor/server.dev.toml` - Example configuration with health_addr
