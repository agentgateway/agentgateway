//go:build e2e

package agentgateway

import (
	"net/http"
	"testing"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

func TestCSRFGatewayPolicy(t *testing.T) {
	agw := New(t)

	agw.Apply(
		manifest("csrf", "routes.yaml"),
		manifest("csrf", "csrf-gw.yaml"),
	)

	// Requests without an Origin header are allowed.
	assertCSRF(agw, "example.com/path1", http.StatusOK)
	assertCSRF(agw, "example.com/path2", http.StatusOK)

	assertCSRF(agw, "example.com/path1", http.StatusForbidden, curl.WithHeader("Origin", "example.com"))
	assertCSRF(agw, "example.com/path1", http.StatusOK, curl.WithHeader("Origin", "example.org"))
	assertCSRF(agw, "example.com/path2", http.StatusOK, curl.WithHeader("Origin", "example.org"))
}

func assertCSRF(t *base.BaseTestingSuite, target string, expectedStatus int, opts ...curl.Option) {
	t.Send(target, base.Expect(expectedStatus), append([]curl.Option{curl.WithMethod("POST")}, opts...)...)
}
