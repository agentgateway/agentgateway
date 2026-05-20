//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
)

func TestCSRFGatewayPolicy(tt *testing.T) {
	t := New(tt)

	t.Apply(
		manifest("csrf", "routes.yaml"),
		manifest("csrf", "csrf-gw.yaml"),
	)

	// Requests without an Origin header are allowed.
	assertCSRF(t, "example.com/path1", http.StatusOK)
	assertCSRF(t, "example.com/path2", http.StatusOK)

	assertCSRF(t, "example.com/path1", http.StatusForbidden, curl.WithHeader("Origin", "example.com"))
	assertCSRF(t, "example.com/path1", http.StatusOK, curl.WithHeader("Origin", "example.org"))
	assertCSRF(t, "example.com/path2", http.StatusOK, curl.WithHeader("Origin", "example.org"))
}

func assertCSRF(t base.Test, target string, expectedStatus int, opts ...curl.Option) {
	t.Send(target, base.Expect(expectedStatus), append([]curl.Option{curl.WithMethod("POST")}, opts...)...)
}
