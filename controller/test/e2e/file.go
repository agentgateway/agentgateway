//go:build e2e

package e2e

import (
	"path/filepath"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
)

var (
	BaseValuesManifestPath = ManifestPath("agent-gateway-integration.yaml")
)

// ManifestPath returns the absolute path to a manifest file.
// These are all stored in the manifests directory.
func ManifestPath(pathParts ...string) string {
	manifestPathParts := append([]string{
		fsutils.MustGetThisDir(),
		"manifests",
	}, pathParts...)
	return filepath.Join(manifestPathParts...)
}
