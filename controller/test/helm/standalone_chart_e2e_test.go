//go:build e2e

package helm

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
)

func TestStandaloneChartHelmInstall(t *testing.T) {
	if os.Getenv("AGW_STANDALONE_HELM_E2E") != "1" {
		t.Skip("set AGW_STANDALONE_HELM_E2E=1 to run the standalone Helm cluster e2e test")
	}

	chartPath := filepath.Join("..", "..", "install", "helm", "agentgateway-standalone")
	absChartPath, err := filepath.Abs(chartPath)
	require.NoError(t, err)

	releaseName := "agw-standalone-e2e"
	namespace := fmt.Sprintf("%s-%d", releaseName, time.Now().Unix())
	timeout := os.Getenv("AGW_STANDALONE_HELM_TIMEOUT")
	if timeout == "" {
		timeout = "2m"
	}
	t.Cleanup(func() {
		_ = runStandaloneE2ECommand(t, "helm", "uninstall", releaseName, "--namespace", namespace)
		_ = runStandaloneE2ECommand(t, "kubectl", "delete", "namespace", namespace, "--ignore-not-found=true")
	})

	installArgs := []string{
		"upgrade", "--install", releaseName, absChartPath,
		"--namespace", namespace,
		"--create-namespace",
		"--wait",
		"--timeout", timeout,
		"--set", "namespaceOverride=" + namespace,
		"--set", "gateway.service.type=ClusterIP",
	}

	if image := os.Getenv("AGW_STANDALONE_IMAGE"); image != "" {
		registry, repository, tag := splitStandaloneImage(t, image)
		installArgs = append(installArgs,
			"--set", "image.registry="+registry,
			"--set", "image.repository="+repository,
			"--set", "image.tag="+tag,
		)
	}

	if err := runStandaloneE2ECommand(t, "helm", installArgs...); err != nil {
		dumpStandaloneE2EDiagnostics(t, namespace, releaseName)
		require.NoError(t, err)
	}

	requireStandaloneE2EOutput(t, releaseName+"-config", "kubectl", "get", "configmap", releaseName+"-config", "--namespace", namespace, "-o", "jsonpath={.metadata.name}")
	require.NoError(t, runStandaloneE2ECommand(t, "kubectl", "rollout", "restart", "deployment/"+releaseName, "--namespace", namespace))
	require.NoError(t, runStandaloneE2ECommand(t, "kubectl", "rollout", "status", "deployment/"+releaseName, "--namespace", namespace, "--timeout", timeout))
}

func runStandaloneE2ECommand(t *testing.T, name string, args ...string) error {
	t.Helper()

	_, err := runStandaloneE2ECommandOutput(t, name, args...)
	return err
}

func runStandaloneE2ECommandOutput(t *testing.T, name string, args ...string) (string, error) {
	t.Helper()

	cmd := exec.Command(name, args...) //nolint:gosec // Test helper: name and args are controlled by test code, not user input
	var stdout bytes.Buffer
	var stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	err := cmd.Run()
	if err != nil {
		t.Logf("%s %s failed\nstdout:\n%s\nstderr:\n%s", name, strings.Join(args, " "), stdout.String(), stderr.String())
		return stdout.String(), err
	}

	t.Logf("%s %s\nstdout:\n%s\nstderr:\n%s", name, strings.Join(args, " "), stdout.String(), stderr.String())
	return stdout.String(), nil
}

func requireStandaloneE2EOutput(t *testing.T, expected string, name string, args ...string) {
	t.Helper()

	stdout, err := runStandaloneE2ECommandOutput(t, name, args...)
	require.NoError(t, err)
	require.Equal(t, expected, stdout)
}

func splitStandaloneImage(t *testing.T, image string) (string, string, string) {
	t.Helper()

	lastSlash := strings.LastIndex(image, "/")
	require.NotEqual(t, -1, lastSlash, "AGW_STANDALONE_IMAGE must include a registry and repository")

	lastColon := strings.LastIndex(image, ":")
	require.Greater(t, lastColon, lastSlash, "AGW_STANDALONE_IMAGE must include a tag, for example cr.agentgateway.dev/agentgateway:v0.1.0")

	name := image[:lastColon]
	tag := image[lastColon+1:]

	return name[:lastSlash], name[lastSlash+1:], tag
}

func dumpStandaloneE2EDiagnostics(t *testing.T, namespace string, releaseName string) {
	t.Helper()

	selector := "app.kubernetes.io/instance=" + releaseName
	_ = runStandaloneE2ECommand(t, "kubectl", "get", "all", "--namespace", namespace, "--selector", selector, "-o", "wide")
	_ = runStandaloneE2ECommand(t, "kubectl", "get", "events", "--namespace", namespace, "--sort-by=.lastTimestamp")
	_ = runStandaloneE2ECommand(t, "kubectl", "describe", "deployment", "--namespace", namespace, "--selector", selector)
	_ = runStandaloneE2ECommand(t, "kubectl", "describe", "pod", "--namespace", namespace, "--selector", selector)
	_ = runStandaloneE2ECommand(t, "kubectl", "logs", "--namespace", namespace, "--selector", selector, "--all-containers", "--tail=100")
}
