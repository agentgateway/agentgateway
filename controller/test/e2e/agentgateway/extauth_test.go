//go:build e2e

package agentgateway

import (
	"net/http"
	"testing"

	"github.com/onsi/gomega"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func TestExtAuth(t *testing.T) {
	agw := New(t)
	agw.Apply(extAuthManifest("service.yaml"))

	agw.Run("GatewayPolicy", func() {
		testExtAuthGatewayPolicy(agw)
	})
	agw.Run("RoutePolicy", func() {
		testExtAuthRoutePolicy(agw)
	})
	agw.Run("BackendTargetedPolicy", func() {
		testExtAuthBackendTargetedPolicy(agw)
	})
	agw.Run("ConditionalPolicy", func() {
		testExtAuthConditionalPolicy(agw)
	})
	agw.Run("PolicyMissingBackendRef", func() {
		testExtAuthPolicyMissingBackendRef(agw)
	})
}

func testExtAuthGatewayPolicy(agw *base.BaseTestingSuite) {
	agw.Apply(
		extAuthManifest("secured-gateway-policy.yaml"),
		extAuthManifest("insecure-route.yaml"),
	)

	runExtAuthCases(agw, []extAuthCase{
		{
			name:    "request allowed with allow header",
			target:  "example.com",
			headers: map[string]string{"x-ext-authz": "allow"},
			status:  http.StatusOK,
			body:    "X-Ext-Authz-Check-Result",
		},
		{
			name:   "request denied without allow header",
			target: "example.com",
			status: http.StatusForbidden,
		},
		{
			name:    "request denied with deny header",
			target:  "example.com",
			headers: map[string]string{"x-ext-authz": "deny"},
			status:  http.StatusForbidden,
		},
	})
}

func testExtAuthRoutePolicy(agw *base.BaseTestingSuite) {
	agw.Apply(
		extAuthManifest("secured-route.yaml"),
		extAuthManifest("insecure-route.yaml"),
	)

	runExtAuthCases(agw, []extAuthCase{
		{
			name:   "request allowed by default",
			target: "example.com",
			status: http.StatusOK,
		},
		{
			name:    "request allowed with allow header on secured route",
			target:  "secureroute.com",
			headers: map[string]string{"x-ext-authz": "allow"},
			status:  http.StatusOK,
			body:    "X-Ext-Authz-Check-Result",
		},
		{
			name:   "request denied without header on secured route",
			target: "secureroute.com",
			status: http.StatusForbidden,
		},
	})
}

func testExtAuthBackendTargetedPolicy(agw *base.BaseTestingSuite) {
	agw.Apply(
		extAuthManifest("backend-targeted-route.yaml"),
	)

	runExtAuthCases(agw, []extAuthCase{
		{
			name:   "request allowed on backend without ext auth",
			target: "backendextauth.com/open",
			status: http.StatusOK,
		},
		{
			name:   "request denied on backend with ext auth without allow header",
			target: "backendextauth.com/secure",
			status: http.StatusForbidden,
		},
		{
			name:    "request allowed on backend with ext auth with allow header",
			target:  "backendextauth.com/secure",
			headers: map[string]string{"x-ext-authz": "allow"},
			status:  http.StatusOK,
			body:    "X-Ext-Authz-Check-Result",
		},
	})
}

func testExtAuthConditionalPolicy(agw *base.BaseTestingSuite) {
	agw.Apply(
		extAuthManifest("conditional-route.yaml"),
	)

	runExtAuthCases(agw, []extAuthCase{
		{
			name:    "request allowed by matching conditional policy",
			target:  "conditionalextauth.com/secure",
			headers: map[string]string{"x-ext-authz": "allow"},
			status:  http.StatusOK,
			body:    "X-Ext-Authz-Check-Result",
		},
		{
			name:   "request denied by matching conditional policy",
			target: "conditionalextauth.com/secure",
			status: http.StatusForbidden,
		},
		{
			name:    "request allowed by fallback conditional policy",
			target:  "conditionalextauth.com/fallback",
			headers: map[string]string{"x-ext-authz": "allow"},
			status:  http.StatusOK,
			body:    "X-Ext-Authz-Check-Result",
		},
		{
			name:   "request denied by fallback conditional policy",
			target: "conditionalextauth.com/fallback",
			status: http.StatusForbidden,
		},
	})
}

func testExtAuthPolicyMissingBackendRef(agw *base.BaseTestingSuite) {
	agw.Apply(
		extAuthManifest("secured-route-missing-ref.yaml"),
	)

	runExtAuthCases(agw, []extAuthCase{
		{
			name:   "request denied for invalid extauth policy due to missing backendRef",
			target: "secureroute.com",
			status: http.StatusForbidden,
		},
	})
}

type extAuthCase struct {
	name    string
	target  string
	headers map[string]string
	status  int
	body    string
}

func extAuthManifest(name string) string {
	return manifest("extauth", name)
}

func runExtAuthCases(agw *base.BaseTestingSuite, cases []extAuthCase) {
	agw.T().Helper()
	for _, tc := range cases {
		tc := tc
		agw.Run(tc.name, func() {
			opts := []curl.Option{}
			for k, v := range tc.headers {
				opts = append(opts, curl.WithHeader(k, v))
			}
			agw.Send(tc.target, &testmatchers.HttpResponse{
				StatusCode: tc.status,
				Body:       gomega.ContainSubstring(tc.body),
			}, opts...)
		})
	}
}
