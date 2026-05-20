//go:build e2e

package e2e_test

const (
	localityNamespace = "agentgateway-locality"

	localityGatewayName = "gateway"
	localityServiceName = "locality-svc"
	localityRouteName   = "locality-route"
	localityHostname    = "locality.test"

	// Labels on the sole kind node — see controller/test/localitySetup/localitySetup-kind-ci.sh.
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
	gatewayManifest      = manifest("locality", "gateway.yaml")
	backendsManifest     = manifest("locality", "backends.yaml")
	serviceRouteManifest = manifest("locality", "service-route.yaml")

	localitySetup = []string{gatewayManifest, backendsManifest, serviceRouteManifest}
)
