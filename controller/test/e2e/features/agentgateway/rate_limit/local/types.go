//go:build e2e

package local

import (
	"path/filepath"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
)

const (
	namespace = "agentgateway-base"
)

var (
	// manifests
	// local rate limit traffic policies
	routeLocalRateLimitManifest         = getTestFile("route-local-rate-limit.yaml")
	gwLocalRateLimitManifest            = getTestFile("gw-local-rate-limit.yaml")
	disabledRouteLocalRateLimitManifest = getTestFile("route-local-rate-limit-disabled.yaml")
	httpRoutesManifest                  = getTestFile("httproutes.yaml")
	extensionRefManifest                = getTestFile("extensionref-rl.yaml")

	route = &gwv1.HTTPRoute{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "svc-route",
			Namespace: namespace,
		},
	}
	route2 = &gwv1.HTTPRoute{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "svc-route-2",
			Namespace: namespace,
		},
	}

	routeRateLimitTrafficPolicy = &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "route-rl-policy",
			Namespace: namespace,
		},
	}

	gwRateLimitTrafficPolicy = &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "gw-rl-policy",
			Namespace: namespace,
		},
	}
)

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
