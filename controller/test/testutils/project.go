package testutils

import (
	"os/exec"
	"path/filepath"
	"strings"
)

// GitRootDirectory returns the path of the top-level directory of the working tree.
func GitRootDirectory() string {
	data, err := exec.Command("git", "rev-parse", "--show-toplevel").Output()
	if err != nil {
		panic(err)
	}
	return strings.TrimSpace(string(data))
}

// ControllerRootDirectory returns the path of the top-level directory of the controller folder.
func ControllerRootDirectory() string {
	return filepath.Join(GitRootDirectory(), "controller")
}
