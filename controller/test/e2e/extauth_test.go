//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/onsi/gomega"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func TestExtAuth(tt *testing.T) {
	t := New(tt)
	t.Apply(extAuthManifest("service.yaml"))

	t.Run("GatewayPolicy", func(t base.Test) {
		testExtAuthGatewayPolicy(t)
	})
	t.Run("RoutePolicy", func(t base.Test) {
		testExtAuthRoutePolicy(t)
	})
	t.Run("BackendTargetedPolicy", func(t base.Test) {
		testExtAuthBackendTargetedPolicy(t)
	})
	t.Run("ConditionalPolicy", func(t base.Test) {
		testExtAuthConditionalPolicy(t)
	})
	t.Run("PolicyMissingBackendRef", func(t base.Test) {
		testExtAuthPolicyMissingBackendRef(t)
	})
}

func testExtAuthGatewayPolicy(t base.Test) {
	t.Apply(
		extAuthManifest("secured-gateway-policy.yaml"),
		extAuthManifest("insecure-route.yaml"),
	)

	runExtAuthCases(t, []extAuthCase{
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

func testExtAuthRoutePolicy(t base.Test) {
	t.Apply(
		extAuthManifest("secured-route.yaml"),
		extAuthManifest("insecure-route.yaml"),
	)

	runExtAuthCases(t, []extAuthCase{
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

func testExtAuthBackendTargetedPolicy(t base.Test) {
	t.Apply(
		extAuthManifest("backend-targeted-route.yaml"),
	)

	runExtAuthCases(t, []extAuthCase{
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

func testExtAuthConditionalPolicy(t base.Test) {
	t.Apply(
		extAuthManifest("conditional-route.yaml"),
	)

	runExtAuthCases(t, []extAuthCase{
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

func testExtAuthPolicyMissingBackendRef(t base.Test) {
	t.Apply(
		extAuthManifest("secured-route-missing-ref.yaml"),
	)

	runExtAuthCases(t, []extAuthCase{
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

func runExtAuthCases(t base.Test, cases []extAuthCase) {
	t.Helper()
	for _, tc := range cases {
		t.Run(tc.name, func(t base.Test) {
			opts := []curl.Option{}
			for k, v := range tc.headers {
				opts = append(opts, curl.WithHeader(k, v))
			}
			t.Send(tc.target, &testmatchers.HttpResponse{
				StatusCode: tc.status,
				Body:       gomega.ContainSubstring(tc.body),
			}, opts...)
		})
	}
}
