//go:build e2e

package locality

import (
	"path/filepath"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

const (
	namespace = "agentgateway-locality"

	gatewayName = "gateway"
	serviceName = "locality-svc"
	routeName   = "locality-route"
	hostname    = "locality.test"

	// Labels on the sole kind node — see controller/test/setup/setup-kind-ci.sh.
	// The gateway's own Workload gets these via WDS, so a WorkloadEntry with
	// locality "region/zone" is what counts as "same zone" for PreferClose.
	sameRegion  = "region"
	sameZone    = "zone"
	otherZone   = "other-zone"
	otherRegion = "other-region"

	backendZoneA   = "backend-zone-a"
	backendZoneB   = "backend-zone-b"
	backendRegionB = "backend-region-b"
)

var (
	gatewayManifest      = filepath.Join(fsutils.MustGetThisDir(), "testdata", "gateway.yaml")
	backendsManifest     = filepath.Join(fsutils.MustGetThisDir(), "testdata", "backends.yaml")
	serviceRouteManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "service-route.yaml")

	setup = base.TestCase{
		Manifests: []string{gatewayManifest, backendsManifest, serviceRouteManifest},
	}

	testCases = map[string]*base.TestCase{
		"TestFailoverWorkloadRemoval": {},
		"TestFailoverHealthiness":     {},
	}
)
