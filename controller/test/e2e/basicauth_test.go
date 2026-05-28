//go:build e2e

package e2e_test

import (
	"encoding/base64"
	"net/http"
	"testing"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
)

func TestBasicAuth(tt *testing.T) {
	t := New(tt)

	t.Run("RoutePolicy", func(t base.Test) {
		testBasicAuthRoutePolicy(t)
	})
	t.Run("GatewayPolicy", func(t base.Test) {
		testBasicAuthGatewayPolicy(t)
	})
}

func testBasicAuthRoutePolicy(t base.Test) {
	t.Apply(
		manifest("basicauth", "insecure-route.yaml"),
		manifest("basicauth", "secured-route.yaml"),
	)

	t.HTTPRouteAccepted("route-example-insecure", base.Namespace)
	assertBasicAuthResponse(t, "insecureroute.com", "", http.StatusOK)

	t.HTTPRouteAccepted("route-secure", base.Namespace)
	assertBasicAuthResponse(t, "secureroute.com", basicAuth("alice", "alicepassword"), http.StatusOK)
	assertBasicAuthResponse(t, "secureroute.com", basicAuth("bob", "bobpassword"), http.StatusOK)

	t.HTTPRouteAccepted("route-secure-too", base.Namespace)
	assertBasicAuthResponse(t, "secureroutetoo.com", basicAuth("eve", "evepassword"), http.StatusOK)
	assertBasicAuthResponse(t, "secureroutetoo.com", basicAuth("mallory", "mallorypassword"), http.StatusOK)
	assertBasicAuthResponse(t, "secureroute.com", basicAuth("alice", "boom"), http.StatusUnauthorized)
	assertBasicAuthResponse(t, "secureroutetoo.com", basicAuth("eve", "boom"), http.StatusUnauthorized)
	assertBasicAuthResponse(t, "secureroute.com", basicAuth("trent", "boom"), http.StatusUnauthorized)
	assertBasicAuthResponse(t, "secureroute.com", "", http.StatusUnauthorized)
}

func testBasicAuthGatewayPolicy(t base.Test) {
	t.Apply(manifest("basicauth", "secured-gateway-policy.yaml"))

	t.HTTPRouteAccepted("route-secure-gw", base.Namespace)
	assertBasicAuthResponse(t, "securegateways.com", basicAuth("alice", "alicepassword"), http.StatusOK)
	assertBasicAuthResponse(t, "securegateways.com", basicAuth("bob", "bobpassword"), http.StatusOK)
	assertBasicAuthResponse(t, "securegateways.com", basicAuth("alice", "boom"), http.StatusUnauthorized)
	assertBasicAuthResponse(t, "securegateways.com", basicAuth("trent", "boom"), http.StatusUnauthorized)
	assertBasicAuthResponse(t, "securegateways.com", "", http.StatusUnauthorized)
}

func assertBasicAuthResponse(t base.Test, host, auth string, status int) {
	opts := []curl.Option{}
	if auth != "" {
		opts = append(opts, curl.WithHeader("Authorization", "Basic "+auth))
	}
	t.Send(host, base.Expect(status), opts...)
}

func basicAuth(username, password string) string {
	return base64.StdEncoding.EncodeToString([]byte(username + ":" + password))
}
