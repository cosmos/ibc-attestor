package main

import (
	"context"

	plugin "github.com/hashicorp/go-plugin"
	"google.golang.org/grpc"

	attestor "github.com/cosmos/ibc-attestor/examples/go-plugin-host/gen/attestor"
)

const PluginName = "attestor"

// Must match the constants the Rust plugin checks in --plugin-mode.
var Handshake = plugin.HandshakeConfig{
	ProtocolVersion:  1,
	MagicCookieKey:   "IBC_ATTESTOR_PLUGIN",
	MagicCookieValue: "ibc-attestor-poc",
}

// AttestorPlugin dispenses the gRPC client; the server lives in the Rust process.
type AttestorPlugin struct{ plugin.NetRPCUnsupportedPlugin }

func (*AttestorPlugin) GRPCServer(*plugin.GRPCBroker, *grpc.Server) error { return nil }

func (*AttestorPlugin) GRPCClient(_ context.Context, _ *plugin.GRPCBroker, c *grpc.ClientConn) (any, error) {
	return attestor.NewAttestationServiceClient(c), nil
}

var PluginMap = map[string]plugin.Plugin{PluginName: &AttestorPlugin{}}
