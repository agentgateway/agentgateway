//go:build e2e

package csrf

import (
	"path/filepath"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
)

var (
	// manifests
	routesManifest        = getTestFile("routes.yaml")
	csrfAgwPolicyManifest = getTestFile("csrf-gw.yaml")
)

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
