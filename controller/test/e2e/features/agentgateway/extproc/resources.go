//go:build e2e

package extproc

import (
	"path/filepath"
	"time"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
)

var (
	timeout = 60 * time.Second

	extProcManifest                  = getTestFile("ext-proc-server.yaml")
	routeWithTargetReferenceManifest = getTestFile("httproute-targetref.yaml")
	gatewayTargetReferenceManifest   = getTestFile("gateway-targetref.yaml")
)

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
