//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
)

func TestLocalRateLimit(tt *testing.T) {
	t := New(tt)
	t.Apply(manifest("rate-limit", "local", "httproutes.yaml"))

	t.Run("Route", func(t base.Test) {
		testLocalRateLimitForRoute(t)
	})
	t.Run("Gateway", func(t base.Test) {
		testLocalRateLimitForGateway(t)
	})
	t.Run("RouteDisabled", func(t base.Test) {
		t.Skip("Skipping LocalRateLimit disabled at Route level on agentgateway: not supported yet")
	})
	t.Run("RouteUsingExtensionRef", func(t base.Test) {
		t.Skip("Skipping LocalRateLimit using extensionRef in HTTPRoute on agentgateway: not supported yet")
	})
}

func testLocalRateLimitForRoute(t base.Test) {
	t.Apply(
		manifest("rate-limit", "local", "route-local-rate-limit.yaml"),
	)

	t.Send("example.com/path1", base.ExpectOK())
	t.Send("example.com/path1", base.Expect(http.StatusTooManyRequests))
	t.Send("example.com/path2", base.ExpectOK())
}

func testLocalRateLimitForGateway(t base.Test) {
	t.Apply(
		manifest("rate-limit", "local", "gw-local-rate-limit.yaml"),
	)

	t.Send("example.com/path1", base.ExpectOK())
	t.Send("example.com/path1", base.Expect(http.StatusTooManyRequests))
	t.Send("example.com/path2", base.Expect(http.StatusTooManyRequests))
}
