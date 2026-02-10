//go:build e2e

package extauth

import (
	"path/filepath"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
)

var (
	// Manifest files
	simpleServiceManifest          = getTestFile("service.yaml")
	extAuthManifest                = getTestFile("ext-authz-server.yaml")
	securedGatewayPolicyManifest   = getTestFile("secured-gateway-policy.yaml")
	securedRouteManifest           = getTestFile("secured-route.yaml")
	insecureRouteManifest          = getTestFile("insecure-route.yaml")
	securedRouteMissingRefManifest = getTestFile("secured-route-missing-ref.yaml")
)

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
