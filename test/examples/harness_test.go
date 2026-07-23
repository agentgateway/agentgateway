//go:build examples

// Package examples contains functional smoke tests for the examples under
// examples/. Each test starts the gateway against an example's shipped
// config.yaml, sends real traffic through it, and asserts on the response.
//
// These are gated behind the `examples` build tag so they stay out of the
// normal `go test ./...` run; they need a built gateway binary, network access
// (npx for the MCP example), and fixed local ports. They run on a schedule via
// .github/workflows/examples.yml.
//
// Run locally:
//
//	make build UI=0
//	AGENTGATEWAY_BIN=$PWD/target/release/agentgateway go test -tags examples -v ./test/examples/...
package examples

import (
	"bytes"
	"context"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
)

// repoRoot resolves the repository root relative to this source file
// (<root>/test/examples/harness_test.go).
func repoRoot(t *testing.T) string {
	t.Helper()
	_, file, _, ok := runtime.Caller(0)
	require.True(t, ok, "runtime.Caller failed")
	return filepath.Clean(filepath.Join(filepath.Dir(file), "..", ".."))
}

// gatewayBin returns the agentgateway binary to test. CI sets AGENTGATEWAY_BIN;
// locally it falls back to the release build, skipping if none is present.
func gatewayBin(t *testing.T) string {
	t.Helper()
	if bin := os.Getenv("AGENTGATEWAY_BIN"); bin != "" {
		return bin
	}
	def := filepath.Join(repoRoot(t), "target", "release", "agentgateway")
	if _, err := os.Stat(def); err != nil {
		t.Skipf("gateway binary not found (set AGENTGATEWAY_BIN, or run 'make build'): %v", err)
	}
	return def
}

// syncBuffer is a goroutine-safe buffer for capturing gateway output while the
// process runs and we poll readiness concurrently.
type syncBuffer struct {
	mu  sync.Mutex
	buf bytes.Buffer
}

func (b *syncBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.Write(p)
}

func (b *syncBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.String()
}

// startGateway launches the gateway against configPath and blocks until its
// readiness endpoint reports ready. The process is terminated (SIGTERM, then
// killed after a grace period) via t.Cleanup, and its log is dumped on failure.
// Extra "KEY=VALUE" strings are appended to the child environment.
func startGateway(t *testing.T, configPath, readyURL string, env ...string) {
	t.Helper()
	bin := gatewayBin(t)

	ctx, cancel := context.WithCancel(context.Background())
	cmd := exec.CommandContext(ctx, bin, "-f", configPath)
	cmd.Env = append(os.Environ(), env...)
	cmd.Cancel = func() error { return cmd.Process.Signal(syscall.SIGTERM) }
	cmd.WaitDelay = 5 * time.Second

	logBuf := &syncBuffer{}
	cmd.Stdout = logBuf
	cmd.Stderr = logBuf

	require.NoError(t, cmd.Start(), "start gateway")
	t.Cleanup(func() {
		cancel()
		_ = cmd.Wait()
		if t.Failed() {
			t.Logf("gateway log for %s:\n%s", filepath.Base(configPath), logBuf.String())
		}
	})

	waitReady(t, readyURL, 90*time.Second, logBuf)
	t.Logf("gateway ready (%s)", readyURL)
}

// waitReady polls url until it returns 200 or the timeout elapses.
func waitReady(t *testing.T, url string, timeout time.Duration, logBuf *syncBuffer) {
	t.Helper()
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		resp, err := http.Get(url) //nolint:noctx // simple readiness poll
		if err == nil {
			_ = resp.Body.Close()
			if resp.StatusCode == http.StatusOK {
				return
			}
		}
		time.Sleep(500 * time.Millisecond)
	}
	t.Fatalf("gateway not ready at %s within %s\ngateway log:\n%s", url, timeout, logBuf.String())
}

// startHTTPUpstream serves 200 "ok" on a fixed address, standing in for the
// backend an example routes to. Registered for cleanup with t.
func startHTTPUpstream(t *testing.T, addr string) {
	t.Helper()
	ln, err := net.Listen("tcp", addr)
	require.NoErrorf(t, err, "listen on %s", addr)
	srv := &http.Server{
		Handler: http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
			w.WriteHeader(http.StatusOK)
			_, _ = w.Write([]byte("ok"))
		}),
		ReadHeaderTimeout: 5 * time.Second,
	}
	go func() { _ = srv.Serve(ln) }()
	t.Cleanup(func() { _ = srv.Close() })
}
