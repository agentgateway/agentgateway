//go:build e2e

package agentgateway

import (
	"net/http"
	"testing"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	"github.com/agentgateway/agentgateway/controller/test/testutils/testjwt"
)

func TestRemoteJwtAuth(t *testing.T) {
	agw := New(t)
	agw.Apply(manifest("remotejwtauth", "common.yaml"))

	agw.Run("RoutePolicyBackend", func() {
		testRemoteJwtAuthRoutePolicyBackend(agw)
	})
	agw.Run("RoutePolicyBackendAndTLSPolicy", func() {
		testRemoteJwtAuthRoutePolicyBackendAndTLSPolicy(agw)
	})
	agw.Run("RoutePolicySvcCACert", func() {
		testRemoteJwtAuthRoutePolicySvc(agw, "secured-route-with-svc-ca-cert.yaml")
	})
	agw.Run("RoutePolicySvc", func() {
		testRemoteJwtAuthRoutePolicySvc(agw, "secured-route-with-svc.yaml")
	})
	agw.Run("RoutePolicyWithRBAC", func() {
		testRemoteJwtAuthRoutePolicyWithRbac(agw)
	})
	agw.Run("GatewayPolicySvc", func() {
		testRemoteJwtAuthGatewayPolicySvc(agw, "secured-gateway-policy-with-svc.yaml")
	})
	agw.Run("GatewayPolicySvcCACert", func() {
		testRemoteJwtAuthGatewayPolicySvc(agw, "secured-gateway-policy-with-svc-ca-cert.yaml")
	})
	agw.Run("GatewayPolicyBackend", func() {
		testRemoteJwtAuthGatewayPolicyBackend(agw)
	})
	agw.Run("GatewayPolicyBackendWithTLSPolicy", func() {
		testRemoteJwtAuthGatewayPolicyBackendWithTLSPolicy(agw)
	})
	agw.Run("GatewayPolicyWithRBAC", func() {
		testRemoteJwtAuthGatewayPolicyWithRbac(agw)
	})
}

func testRemoteJwtAuthRoutePolicyBackend(agw *base.BaseTestingSuite) {
	applyRemoteJwtAuth(agw, "insecure-route.yaml", "secured-route-with-backend.yaml")

	assertRemoteJwtRouteAccepted(agw, "route-example-insecure")
	assertRemoteJwtResponse(agw, "insecureroute.com", "", http.StatusOK)

	assertRemoteJwtRouteAccepted(agw, "route-secure")
	assertRemoteJwtResponse(agw, "secureroute.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(agw, "secureroute.com", testjwt.OrgTwoJWT, http.StatusOK)
	assertRemoteJwtResponse(agw, "secureroute.com", "nosuchkey", http.StatusUnauthorized)
	assertRemoteJwtResponse(agw, "secureroute.com", "", http.StatusUnauthorized)
}

func testRemoteJwtAuthRoutePolicyBackendAndTLSPolicy(agw *base.BaseTestingSuite) {
	applyRemoteJwtAuth(agw, "secured-route-with-backend-and-ref.yaml")
	assertRemoteJwtRouteAccepted(agw, "route-secure")
	assertRemoteJwtResponse(agw, "secureroute.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(agw, "secureroute.com", "nosuchkey", http.StatusUnauthorized)
	assertRemoteJwtResponse(agw, "secureroute.com", "", http.StatusUnauthorized)
}

func testRemoteJwtAuthRoutePolicySvc(agw *base.BaseTestingSuite, manifestName string) {
	applyRemoteJwtAuth(agw, manifestName)
	assertRemoteJwtRouteAccepted(agw, "route-secure")
	assertRemoteJwtResponse(agw, "secureroute.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(agw, "secureroute.com", "nosuchkey", http.StatusUnauthorized)
	assertRemoteJwtResponse(agw, "secureroute.com", "", http.StatusUnauthorized)
}

func testRemoteJwtAuthRoutePolicyWithRbac(agw *base.BaseTestingSuite) {
	applyRemoteJwtAuth(agw, "secured-route-with-rbac.yaml")
	assertRemoteJwtRouteAccepted(agw, "route-secure")
	assertRemoteJwtResponse(agw, "secureroute.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(agw, "secureroute.com", testjwt.OrgFourJWT, http.StatusForbidden)
}

func testRemoteJwtAuthGatewayPolicySvc(agw *base.BaseTestingSuite, manifestName string) {
	applyRemoteJwtAuth(agw, manifestName)
	assertRemoteJwtRouteAccepted(agw, "route-secure-gw")
	assertRemoteJwtResponse(agw, "securegateways.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(agw, "securegateways.com", "nosuchkey", http.StatusUnauthorized)
	assertRemoteJwtResponse(agw, "securegateways.com", "", http.StatusUnauthorized)
}

func testRemoteJwtAuthGatewayPolicyBackend(agw *base.BaseTestingSuite) {
	applyRemoteJwtAuth(agw, "secured-gateway-policy-with-backend.yaml")
	assertRemoteJwtRouteAccepted(agw, "route-secure-gw")
	assertRemoteJwtResponse(agw, "securegateways.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(agw, "securegateways.com", testjwt.OrgTwoJWT, http.StatusOK)
	assertRemoteJwtResponse(agw, "securegateways.com", "nosuchkey", http.StatusUnauthorized)
	assertRemoteJwtResponse(agw, "securegateways.com", "", http.StatusUnauthorized)
}

func testRemoteJwtAuthGatewayPolicyBackendWithTLSPolicy(agw *base.BaseTestingSuite) {
	applyRemoteJwtAuth(agw, "secured-gateway-policy-with-backend-and-ref.yaml")
	assertRemoteJwtRouteAccepted(agw, "route-secure-gw")
	assertRemoteJwtResponse(agw, "securegateways.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(agw, "securegateways.com", "nosuchkey", http.StatusUnauthorized)
	assertRemoteJwtResponse(agw, "securegateways.com", "", http.StatusUnauthorized)
}

func testRemoteJwtAuthGatewayPolicyWithRbac(agw *base.BaseTestingSuite) {
	applyRemoteJwtAuth(agw, "secured-gateway-policy-with-rbac.yaml")
	assertRemoteJwtRouteAccepted(agw, "route-secure-gw")
	assertRemoteJwtResponse(agw, "securegateways.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(agw, "securegateways.com", testjwt.OrgFourJWT, http.StatusForbidden)
}

func applyRemoteJwtAuth(t *base.BaseTestingSuite, manifests ...string) {
	all := make([]string, 0, len(manifests))
	for _, name := range manifests {
		all = append(all, manifest("remotejwtauth", name))
	}
	t.Apply(all...)
}

func assertRemoteJwtRouteAccepted(t *base.BaseTestingSuite, route string) {
	t.HTTPRouteAccepted(route, base.Namespace)
}

func assertRemoteJwtResponse(t *base.BaseTestingSuite, host, token string, status int) {
	opts := []curl.Option{}
	if token != "" {
		opts = append(opts, curl.WithHeader("Authorization", "Bearer "+token))
	}
	t.Send(host, base.Expect(status), opts...)
}
