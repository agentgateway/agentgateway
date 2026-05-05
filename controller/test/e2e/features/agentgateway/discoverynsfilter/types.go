//go:build e2e

package discoverynsfilter

import (
	"path/filepath"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

type testingSuite struct {
	*base.BaseTestingSuite
}

const (
	// DiscoveryLabel is the label key used to enable namespace discovery.
	DiscoveryLabel = "agentgateway.dev/discovery"

	nsSelected   = "discoveryns-selected"
	nsUnselected = "discoveryns-unselected"
)

var (
	_ e2e.NewSuiteFunc = NewTestingSuite

	setupManifest           = filepath.Join(fsutils.MustGetThisDir(), "testdata", "setup.yaml")
	routeSelectedManifest   = filepath.Join(fsutils.MustGetThisDir(), "testdata", "route-selected.yaml")
	routeUnselectedManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "route-unselected.yaml")

	setup = base.TestCase{
		Manifests: []string{setupManifest},
	}
)
