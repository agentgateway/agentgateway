//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/testutils/testjwt"
)

func TestRemoteJwtAuth(tt *testing.T) {
	t := New(tt)
	t.Apply(manifest("remotejwtauth", "common.yaml"))

	t.Run("RoutePolicyBackend", func(t base.Test) {
		testRemoteJwtAuthRoutePolicyBackend(t)
	})
	t.Run("RoutePolicyBackendAndTLSPolicy", func(t base.Test) {
		testRemoteJwtAuthRoutePolicyBackendAndTLSPolicy(t)
	})
	t.Run("RoutePolicySvcCACert", func(t base.Test) {
		testRemoteJwtAuthRoutePolicySvc(t, "secured-route-with-svc-ca-cert.yaml")
	})
	t.Run("RoutePolicySvc", func(t base.Test) {
		testRemoteJwtAuthRoutePolicySvc(t, "secured-route-with-svc.yaml")
	})
	t.Run("RoutePolicyWithRBAC", func(t base.Test) {
		testRemoteJwtAuthRoutePolicyWithRbac(t)
	})
	t.Run("GatewayPolicySvc", func(t base.Test) {
		testRemoteJwtAuthGatewayPolicySvc(t, "secured-gateway-policy-with-svc.yaml")
	})
	t.Run("GatewayPolicySvcCACert", func(t base.Test) {
		testRemoteJwtAuthGatewayPolicySvc(t, "secured-gateway-policy-with-svc-ca-cert.yaml")
	})
	t.Run("GatewayPolicyBackend", func(t base.Test) {
		testRemoteJwtAuthGatewayPolicyBackend(t)
	})
	t.Run("GatewayPolicyBackendWithTLSPolicy", func(t base.Test) {
		testRemoteJwtAuthGatewayPolicyBackendWithTLSPolicy(t)
	})
	t.Run("GatewayPolicyWithRBAC", func(t base.Test) {
		testRemoteJwtAuthGatewayPolicyWithRbac(t)
	})
}

func testRemoteJwtAuthRoutePolicyBackend(t base.Test) {
	applyRemoteJwtAuth(t, "insecure-route.yaml", "secured-route-with-backend.yaml")

	assertRemoteJwtRouteAccepted(t, "route-example-insecure")
	assertRemoteJwtResponse(t, "insecureroute.com", "", http.StatusOK)

	assertRemoteJwtRouteAccepted(t, "route-secure")
	assertRemoteJwtResponse(t, "secureroute.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(t, "secureroute.com", testjwt.OrgTwoJWT, http.StatusOK)
	assertRemoteJwtResponse(t, "secureroute.com", "nosuchkey", http.StatusUnauthorized)
	assertRemoteJwtResponse(t, "secureroute.com", "", http.StatusUnauthorized)
}

func testRemoteJwtAuthRoutePolicyBackendAndTLSPolicy(t base.Test) {
	applyRemoteJwtAuth(t, "secured-route-with-backend-and-ref.yaml")
	assertRemoteJwtRouteAccepted(t, "route-secure")
	assertRemoteJwtResponse(t, "secureroute.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(t, "secureroute.com", "nosuchkey", http.StatusUnauthorized)
	assertRemoteJwtResponse(t, "secureroute.com", "", http.StatusUnauthorized)
}

func testRemoteJwtAuthRoutePolicySvc(t base.Test, manifestName string) {
	applyRemoteJwtAuth(t, manifestName)
	assertRemoteJwtRouteAccepted(t, "route-secure")
	assertRemoteJwtResponse(t, "secureroute.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(t, "secureroute.com", "nosuchkey", http.StatusUnauthorized)
	assertRemoteJwtResponse(t, "secureroute.com", "", http.StatusUnauthorized)
}

func testRemoteJwtAuthRoutePolicyWithRbac(t base.Test) {
	applyRemoteJwtAuth(t, "secured-route-with-rbac.yaml")
	assertRemoteJwtRouteAccepted(t, "route-secure")
	assertRemoteJwtResponse(t, "secureroute.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(t, "secureroute.com", testjwt.OrgFourJWT, http.StatusForbidden)
}

func testRemoteJwtAuthGatewayPolicySvc(t base.Test, manifestName string) {
	applyRemoteJwtAuth(t, manifestName)
	assertRemoteJwtRouteAccepted(t, "route-secure-gw")
	assertRemoteJwtResponse(t, "securegateways.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(t, "securegateways.com", "nosuchkey", http.StatusUnauthorized)
	assertRemoteJwtResponse(t, "securegateways.com", "", http.StatusUnauthorized)
}

func testRemoteJwtAuthGatewayPolicyBackend(t base.Test) {
	applyRemoteJwtAuth(t, "secured-gateway-policy-with-backend.yaml")
	assertRemoteJwtRouteAccepted(t, "route-secure-gw")
	assertRemoteJwtResponse(t, "securegateways.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(t, "securegateways.com", testjwt.OrgTwoJWT, http.StatusOK)
	assertRemoteJwtResponse(t, "securegateways.com", "nosuchkey", http.StatusUnauthorized)
	assertRemoteJwtResponse(t, "securegateways.com", "", http.StatusUnauthorized)
}

func testRemoteJwtAuthGatewayPolicyBackendWithTLSPolicy(t base.Test) {
	applyRemoteJwtAuth(t, "secured-gateway-policy-with-backend-and-ref.yaml")
	assertRemoteJwtRouteAccepted(t, "route-secure-gw")
	assertRemoteJwtResponse(t, "securegateways.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(t, "securegateways.com", "nosuchkey", http.StatusUnauthorized)
	assertRemoteJwtResponse(t, "securegateways.com", "", http.StatusUnauthorized)
}

func testRemoteJwtAuthGatewayPolicyWithRbac(t base.Test) {
	applyRemoteJwtAuth(t, "secured-gateway-policy-with-rbac.yaml")
	assertRemoteJwtRouteAccepted(t, "route-secure-gw")
	assertRemoteJwtResponse(t, "securegateways.com", testjwt.OrgOneJWT, http.StatusOK)
	assertRemoteJwtResponse(t, "securegateways.com", testjwt.OrgFourJWT, http.StatusForbidden)
}

func applyRemoteJwtAuth(t base.Test, manifests ...string) {
	all := make([]string, 0, len(manifests))
	for _, name := range manifests {
		all = append(all, manifest("remotejwtauth", name))
	}
	t.Apply(all...)
}

func assertRemoteJwtRouteAccepted(t base.Test, route string) {
	t.HTTPRouteAccepted(route, base.Namespace)
}

func assertRemoteJwtResponse(t base.Test, host, token string, status int) {
	opts := []curl.Option{}
	if token != "" {
		opts = append(opts, curl.WithHeader("Authorization", "Bearer "+token))
	}
	t.Send(host, base.Expect(status), opts...)
}
