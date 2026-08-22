package helm

import (
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/stretchr/testify/require"
)

func helmCommand(t *testing.T, args ...string) *exec.Cmd {
	t.Helper()

	_, filename, _, ok := runtime.Caller(0)
	require.True(t, ok, "failed to locate helm test helper source")

	repoRoot := filepath.Clean(filepath.Join(filepath.Dir(filename), "..", "..", ".."))
	helmPath := filepath.Join(repoRoot, "tools", "helm")
	return exec.Command(helmPath, args...)
}
