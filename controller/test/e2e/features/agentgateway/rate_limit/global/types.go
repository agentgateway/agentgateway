//go:build e2e

package global

import (
	"path/filepath"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
)

const (
	// test namespace for shared gateway resources
	namespace = "agentgateway-base"
	// test namespace for ratelimit resources
	extensionsNamespace = "kgateway-test-extensions"
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
