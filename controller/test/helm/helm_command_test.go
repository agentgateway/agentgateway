package helm

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"runtime"
	"sync"
	"testing"

	"github.com/stretchr/testify/require"
)

// helmPath runs helm once so parallel tests don't all build it concurrently on first use.
var helmPath = sync.OnceValues(func() (string, error) {
	_, filename, _, _ := runtime.Caller(0)
	repoRoot := filepath.Clean(filepath.Join(filepath.Dir(filename), "..", "..", ".."))
	path := filepath.Join(repoRoot, "tools", "helm")
	if out, err := exec.Command(path, "version").CombinedOutput(); err != nil {
		return "", fmt.Errorf("%w: %s", err, out)
	}
	return path, nil
})

func helmCommand(t *testing.T, args ...string) *exec.Cmd {
	t.Helper()

	path, err := helmPath()
	require.NoError(t, err, "failed to build helm")
	return exec.Command(path, args...)
}
