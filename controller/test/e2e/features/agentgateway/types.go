//go:build e2e

package agentgateway

import (
	"path/filepath"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

var (
	// Basic HTTPRoute test resources that target the shared e2e Gateway.
	httpRouteManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "agw-http-route.yaml")
	// Basic TCPRoute test resources that target the shared e2e Gateway.
	tcpRouteManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "agw-tcp-route.yaml")

	// Shared Gateway created once for the full e2e run.
	sharedGatewayObjectMeta = metav1.ObjectMeta{
		Name:      "gateway",
		Namespace: "agentgateway-base",
	}

	testCases = map[string]*base.TestCase{
		"TestAgentgatewayHTTPRoute": {
			Manifests: []string{httpRouteManifest},
		},
		"TestAgentgatewayTCPRoute": {
			Manifests:       []string{tcpRouteManifest},
			MinGwApiVersion: base.GwApiRequireTcpRoutes, // TCPRoutes are experimental only
		},
	}
)
