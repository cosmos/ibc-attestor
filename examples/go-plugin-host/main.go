// Runs the Rust IBC attestor as a go-plugin and exercises it end-to-end: fetch an
// attestation, scrape metrics, and recover from a crash.
package main

import (
	"context"
	"encoding/hex"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	hclog "github.com/hashicorp/go-hclog"
	plugin "github.com/hashicorp/go-plugin"
	toml "github.com/pelletier/go-toml/v2"
	yaml "gopkg.in/yaml.v3"

	attestor "github.com/cosmos/ibc-attestor/examples/go-plugin-host/gen/attestor"
)

const (
	hostConfigPath   = "host.yaml"
	keystorePassword = "poc"
)

func main() {
	logger := hclog.New(&hclog.LoggerOptions{Name: "host", Level: hclog.Info, Output: os.Stdout})
	defer plugin.CleanupClients()

	cfg, metricsURL, err := materializeAttestorConfig(hostConfigPath)
	if err != nil {
		logger.Error("config", "err", err)
		os.Exit(1)
	}
	defer os.Remove(cfg)

	fmt.Println("\n== fetch attestation ==")
	p, svc, err := start(logger, cfg)
	if err != nil {
		logger.Error("start", "err", err)
		os.Exit(1)
	}
	if err := fetchAttestation(svc); err != nil {
		logger.Error("attestation", "err", err)
		os.Exit(1)
	}

	fmt.Println("\n== metrics ==")
	if err := printMetrics(metricsURL); err != nil {
		logger.Error("metrics", "err", err)
	}
	p.stop() // free :8081 for the recovery demo

	fmt.Println("\n== crash + recovery ==")
	if err := demonstrateRecovery(logger, cfg); err != nil {
		logger.Error("recovery", "err", err)
		os.Exit(1)
	}

	fmt.Println("\n== done ==")
}

// findAttestorBin locates the attestor binary: the ATTESTOR_BIN override, else
// beside this executable (where `make build` puts it).
func findAttestorBin() (string, error) {
	if p := os.Getenv("ATTESTOR_BIN"); p != "" {
		return p, nil
	}
	exe, err := os.Executable()
	if err != nil {
		return "", err
	}
	beside := filepath.Join(filepath.Dir(exe), "ibc_attestor")
	if _, err := os.Stat(beside); err != nil {
		return "", fmt.Errorf("attestor binary not found beside host: set ATTESTOR_BIN or run `make build`")
	}
	return beside, nil
}

// pluginProc pairs the client with its process so stop() can SIGTERM it.
type pluginProc struct {
	client *plugin.Client
	cmd    *exec.Cmd
}

// stop sends SIGTERM (the attestor's graceful path) and waits for exit before
// reaping, so normal shutdowns don't trip go-plugin's force-kill.
func (p *pluginProc) stop() {
	if p.cmd.Process != nil {
		_ = p.cmd.Process.Signal(syscall.SIGTERM)
		for i := 0; i < 50 && !p.client.Exited(); i++ {
			time.Sleep(100 * time.Millisecond)
		}
	}
	p.client.Kill()
}

// start launches the attestor in --plugin-mode and returns a connected client.
func start(logger hclog.Logger, cfg string) (*pluginProc, attestor.AttestationServiceClient, error) {
	bin, err := findAttestorBin()
	if err != nil {
		return nil, nil, err
	}
	cmd := exec.Command(bin, "server", "--plugin-mode", "--config", cfg,
		"--chain-type", "evm", "--signer-type", "local")
	cmd.Env = append(os.Environ(), "IBC_ATTESTOR_KEYSTORE_PASSWORD="+keystorePassword, "RUST_LOG=info")

	client := plugin.NewClient(&plugin.ClientConfig{
		HandshakeConfig:  Handshake,
		Plugins:          PluginMap,
		Cmd:              cmd,
		AllowedProtocols: []plugin.Protocol{plugin.ProtocolGRPC},
		Logger:           logger,
		Stderr:           os.Stderr, // surface the attestor's JSON logs
		Managed:          true,
	})
	rpc, err := client.Client()
	if err != nil {
		return nil, nil, err
	}
	raw, err := rpc.Dispense(PluginName)
	if err != nil {
		return nil, nil, err
	}
	return &pluginProc{client, cmd}, raw.(attestor.AttestationServiceClient), nil
}

func fetchAttestation(svc attestor.AttestationServiceClient) error {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	h, err := svc.LatestHeight(ctx, &attestor.LatestHeightRequest{})
	if err != nil {
		return fmt.Errorf("LatestHeight: %w", err)
	}
	resp, err := svc.StateAttestation(ctx, &attestor.StateAttestationRequest{Height: h.GetHeight()})
	if err != nil {
		return fmt.Errorf("StateAttestation: %w", err)
	}
	a := resp.GetAttestation()
	fmt.Printf("  height %d · %d-byte data · %d-byte sig 0x%s\n",
		a.GetHeight(), len(a.GetAttestedData()), len(a.GetSignature()), hex.EncodeToString(a.GetSignature()))
	return nil
}

// demonstrateRecovery starts an attestor, SIGKILLs it to simulate a crash, then
// detects the exit, relaunches, and fetches again.
func demonstrateRecovery(logger hclog.Logger, cfg string) error {
	crash, svc, err := start(logger, cfg)
	if err != nil {
		return err
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	if _, err := svc.LatestHeight(ctx, &attestor.LatestHeightRequest{}); err != nil {
		cancel()
		return fmt.Errorf("pre-crash call: %w", err)
	}
	cancel()

	logger.Warn("killing the attestor to simulate a crash")
	_ = crash.cmd.Process.Kill()
	for i := 0; i < 30 && !crash.client.Exited(); i++ {
		time.Sleep(100 * time.Millisecond)
	}
	crash.client.Kill() // reap

	recovered, svc2, err := start(logger, cfg)
	if err != nil {
		return fmt.Errorf("relaunch: %w", err)
	}
	defer recovered.stop()
	if err := fetchAttestation(svc2); err != nil {
		return fmt.Errorf("post-recovery: %w", err)
	}
	fmt.Println("  recovered")
	return nil
}

// printMetrics scrapes the attestor's Prometheus side port and prints the counters
// that moved.
func printMetrics(url string) error {
	resp, err := http.Get(url)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}
	for _, line := range strings.Split(string(body), "\n") {
		if strings.HasPrefix(line, "attestor_rpc_requests_total") ||
			strings.HasPrefix(line, "attestor_signer_signs_total") {
			fmt.Println("  " + line)
		}
	}
	return nil
}

// materializeAttestorConfig writes the opaque `attestor` section of the YAML host
// config to a temp TOML file (0600) for the subprocess, and returns that path plus
// the metrics URL derived from the attestor's health_addr. Secrets stay in env.
func materializeAttestorConfig(hostPath string) (configPath, metricsURL string, err error) {
	data, err := os.ReadFile(hostPath)
	if err != nil {
		return "", "", err
	}
	var host map[string]any
	if err := yaml.Unmarshal(data, &host); err != nil {
		return "", "", err
	}
	section, ok := host["attestor"].(map[string]any)
	if !ok {
		return "", "", fmt.Errorf("%s has no `attestor` section", hostPath)
	}
	if server, ok := section["server"].(map[string]any); ok {
		if addr, ok := server["health_addr"].(string); ok {
			metricsURL = "http://" + addr + "/metrics"
		}
	}
	rendered, err := toml.Marshal(section)
	if err != nil {
		return "", "", err
	}
	f, err := os.CreateTemp("", "attestor-*.toml")
	if err != nil {
		return "", "", err
	}
	defer f.Close()
	if _, err := f.Write(rendered); err != nil {
		return "", "", err
	}
	return f.Name(), metricsURL, nil
}
