//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
)

func TestApiKeyAuth(tt *testing.T) {
	t := New(tt)

	t.Run("RoutePolicy", func(t base.Test) {
		testApiKeyAuthRoutePolicy(t)
	})
	t.Run("GatewayPolicy", func(t base.Test) {
		testApiKeyAuthGatewayPolicy(t)
	})
}

func testApiKeyAuthRoutePolicy(t base.Test) {
	t.Apply(
		manifest("apikeyauth", "insecure-route.yaml"),
		manifest("apikeyauth", "secured-route.yaml"),
	)

	t.HTTPRouteAccepted("route-example-insecure", base.Namespace)
	assertApiKeyResponse(t, "insecureroute.com", "", http.StatusOK)

	t.HTTPRouteAccepted("route-secure", base.Namespace)
	assertApiKeyResponse(t, "secureroute.com", "k-1230", http.StatusOK)
	assertApiKeyResponse(t, "secureroute.com", "k-4560", http.StatusOK)
	assertApiKeyResponse(t, "secureroute.com", "nosuchkey", http.StatusUnauthorized)
	assertApiKeyResponse(t, "secureroute.com", "", http.StatusUnauthorized)
}

func testApiKeyAuthGatewayPolicy(t base.Test) {
	t.Apply(manifest("apikeyauth", "secured-gateway-policy.yaml"))

	t.HTTPRouteAccepted("route-secure-gw", base.Namespace)
	assertApiKeyResponse(t, "securegateways.com", "k-123", http.StatusOK)
	assertApiKeyResponse(t, "securegateways.com", "k-456", http.StatusOK)
	assertApiKeyResponse(t, "securegateways.com", "nosuchkey", http.StatusUnauthorized)
	assertApiKeyResponse(t, "securegateways.com", "", http.StatusUnauthorized)
}

func assertApiKeyResponse(t base.Test, host, key string, status int) {
	opts := []curl.Option{}
	if key != "" {
		opts = append(opts, curl.WithHeader("Authorization", "Bearer "+key))
	}
	t.Send(host, base.Expect(status), opts...)
}
