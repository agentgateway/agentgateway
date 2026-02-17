//go:build e2e

package global

import (
	"path/filepath"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
)

var (
	httpRoutesManifest        = getTestFile("routes.yaml")
	ipRateLimitManifest       = getTestFile("ip-rate-limit.yaml")
	pathRateLimitManifest     = getTestFile("path-rate-limit.yaml")
	userRateLimitManifest     = getTestFile("user-rate-limit.yaml")
	combinedRateLimitManifest = getTestFile("combined-rate-limit.yaml")
	rateLimitServerManifest   = getTestFile("rate-limit-server.yaml")
)

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
