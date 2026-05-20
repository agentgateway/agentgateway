//go:build e2e

package agentgateway

import (
	"net/http"
	"testing"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

func TestApiKeyAuth(t *testing.T) {
	agw := New(t)

	agw.Run("RoutePolicy", func() {
		testApiKeyAuthRoutePolicy(agw)
	})
	agw.Run("GatewayPolicy", func() {
		testApiKeyAuthGatewayPolicy(agw)
	})
}

func testApiKeyAuthRoutePolicy(agw *base.BaseTestingSuite) {
	agw.Apply(
		manifest("apikeyauth", "insecure-route.yaml"),
		manifest("apikeyauth", "secured-route.yaml"),
	)

	agw.HTTPRouteAccepted("route-example-insecure", base.Namespace)
	assertApiKeyResponse(agw, "insecureroute.com", "", http.StatusOK)

	agw.HTTPRouteAccepted("route-secure", base.Namespace)
	assertApiKeyResponse(agw, "secureroute.com", "k-1230", http.StatusOK)
	assertApiKeyResponse(agw, "secureroute.com", "k-4560", http.StatusOK)
	assertApiKeyResponse(agw, "secureroute.com", "nosuchkey", http.StatusUnauthorized)
	assertApiKeyResponse(agw, "secureroute.com", "", http.StatusUnauthorized)
}

func testApiKeyAuthGatewayPolicy(agw *base.BaseTestingSuite) {
	agw.Apply(manifest("apikeyauth", "secured-gateway-policy.yaml"))

	agw.HTTPRouteAccepted("route-secure-gw", base.Namespace)
	assertApiKeyResponse(agw, "securegateways.com", "k-123", http.StatusOK)
	assertApiKeyResponse(agw, "securegateways.com", "k-456", http.StatusOK)
	assertApiKeyResponse(agw, "securegateways.com", "nosuchkey", http.StatusUnauthorized)
	assertApiKeyResponse(agw, "securegateways.com", "", http.StatusUnauthorized)
}

func assertApiKeyResponse(t *base.BaseTestingSuite, host, key string, status int) {
	opts := []curl.Option{}
	if key != "" {
		opts = append(opts, curl.WithHeader("Authorization", "Bearer "+key))
	}
	t.Send(host, base.Expect(status), opts...)
}
