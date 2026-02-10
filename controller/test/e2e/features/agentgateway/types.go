//go:build e2e

package agentgateway

import (
	"path/filepath"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/defaults"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
)

var (
	// kgateway managed deployment for the agentgateway with basic HTTPRoute
	httpRouteManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "agw-http-route.yaml")
	// kgateway managed deployment for the agentgateway with basic TCPRoute
	tcpRouteManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "agw-tcp-route.yaml")

	// Core infrastructure objects that we need to track
	httpGatewayObjectMeta = metav1.ObjectMeta{
		Name:      "http-gw-for-test",
		Namespace: "default",
	}
	tcpGatewayObjectMeta = metav1.ObjectMeta{
		Name:      "tcp-gw-for-test",
		Namespace: "default",
	}

	testCases = map[string]*base.TestCase{
		"TestAgentgatewayHTTPRoute": {
			Manifests: []string{defaults.HttpbinManifest, httpRouteManifest},
		},
		"TestAgentgatewayTCPRoute": {
			Manifests:       []string{tcpRouteManifest},
			MinGwApiVersion: base.GwApiRequireTcpRoutes, // TCPRoutes are experimental only
		},
	}
)
