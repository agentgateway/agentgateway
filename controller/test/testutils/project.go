package testutils

import (
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"
)

// GitRootDirectory returns the repository root, falling back to the jj workspace
// root if Git metadata is unavailable.
func GitRootDirectory() string {
	data, err := exec.Command("git", "rev-parse", "--show-toplevel").CombinedOutput()
	if err == nil {
		return strings.TrimSpace(string(data))
	}

	data, jjErr := exec.Command("jj", "workspace", "root").CombinedOutput()
	if jjErr != nil {
		panic(fmt.Errorf("failed to determine repository root using git or jj: git: %w; jj: %w", err, jjErr))
	}

	return strings.TrimSpace(string(data))
}

// ControllerRootDirectory returns the path of the top-level directory of the controller folder.
func ControllerRootDirectory() string {
	return filepath.Join(GitRootDirectory(), "controller")
}
