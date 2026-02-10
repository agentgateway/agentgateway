//go:build e2e

package runtime

import (
	"path/filepath"

	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

// PathToBugReport returns the absolute path to the directory where the bug_report will be stored
// This mirrors logic in the Makefile to ensure this directory exists (see: BUG_REPORT_DIR)
func PathToBugReport() string {
	return filepath.Join(testutils.ControllerRootDirectory(), "_test", "bug_report")
}
