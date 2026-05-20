//go:build e2e

package agentgateway

import (
	"encoding/base64"
	"net/http"
	"testing"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

func TestBasicAuth(t *testing.T) {
	agw := New(t)

	agw.Run("RoutePolicy", func() {
		testBasicAuthRoutePolicy(agw)
	})
	agw.Run("GatewayPolicy", func() {
		testBasicAuthGatewayPolicy(agw)
	})
}

func testBasicAuthRoutePolicy(agw *base.BaseTestingSuite) {
	agw.Apply(
		manifest("basicauth", "insecure-route.yaml"),
		manifest("basicauth", "secured-route.yaml"),
	)

	agw.HTTPRouteAccepted("route-example-insecure", base.Namespace)
	assertBasicAuthResponse(agw, "insecureroute.com", "", http.StatusOK)

	agw.HTTPRouteAccepted("route-secure", base.Namespace)
	assertBasicAuthResponse(agw, "secureroute.com", basicAuth("alice", "alicepassword"), http.StatusOK)
	assertBasicAuthResponse(agw, "secureroute.com", basicAuth("bob", "bobpassword"), http.StatusOK)

	agw.HTTPRouteAccepted("route-secure-too", base.Namespace)
	assertBasicAuthResponse(agw, "secureroutetoo.com", basicAuth("eve", "evepassword"), http.StatusOK)
	assertBasicAuthResponse(agw, "secureroutetoo.com", basicAuth("mallory", "mallorypassword"), http.StatusOK)
	assertBasicAuthResponse(agw, "secureroute.com", basicAuth("alice", "boom"), http.StatusUnauthorized)
	assertBasicAuthResponse(agw, "secureroutetoo.com", basicAuth("eve", "boom"), http.StatusUnauthorized)
	assertBasicAuthResponse(agw, "secureroute.com", basicAuth("trent", "boom"), http.StatusUnauthorized)
	assertBasicAuthResponse(agw, "secureroute.com", "", http.StatusUnauthorized)
}

func testBasicAuthGatewayPolicy(agw *base.BaseTestingSuite) {
	agw.Apply(manifest("basicauth", "secured-gateway-policy.yaml"))

	agw.HTTPRouteAccepted("route-secure-gw", base.Namespace)
	assertBasicAuthResponse(agw, "securegateways.com", basicAuth("alice", "alicepassword"), http.StatusOK)
	assertBasicAuthResponse(agw, "securegateways.com", basicAuth("bob", "bobpassword"), http.StatusOK)
	assertBasicAuthResponse(agw, "securegateways.com", basicAuth("alice", "boom"), http.StatusUnauthorized)
	assertBasicAuthResponse(agw, "securegateways.com", basicAuth("trent", "boom"), http.StatusUnauthorized)
	assertBasicAuthResponse(agw, "securegateways.com", "", http.StatusUnauthorized)
}

func assertBasicAuthResponse(t *base.BaseTestingSuite, host, auth string, status int) {
	opts := []curl.Option{}
	if auth != "" {
		opts = append(opts, curl.WithHeader("Authorization", "Basic "+auth))
	}
	t.Send(host, base.Expect(status), opts...)
}

func basicAuth(username, password string) string {
	return base64.StdEncoding.EncodeToString([]byte(username + ":" + password))
}
