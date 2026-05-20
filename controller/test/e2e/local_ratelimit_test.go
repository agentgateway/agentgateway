//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
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

	t.TestInstallation.AssertionsT(t).EventuallyObjectsExist(
		t.Ctx,
		httpRoute("svc-route"),
		httpRoute("svc-route-2"),
		agwPolicy("route-rl-policy"),
	)

	t.Send("example.com/path1", base.ExpectOK())
	t.Send("example.com/path1", base.Expect(http.StatusTooManyRequests))
	t.Send("example.com/path2", base.ExpectOK())
}

func testLocalRateLimitForGateway(t base.Test) {
	t.Apply(
		manifest("rate-limit", "local", "gw-local-rate-limit.yaml"),
	)

	t.TestInstallation.AssertionsT(t).EventuallyObjectsExist(
		t.Ctx,
		httpRoute("svc-route"),
		httpRoute("svc-route-2"),
		agwPolicy("gw-rl-policy"),
	)

	t.Send("example.com/path1", base.ExpectOK())
	t.Send("example.com/path1", base.Expect(http.StatusTooManyRequests))
	t.Send("example.com/path2", base.Expect(http.StatusTooManyRequests))
}

func httpRoute(name string) *gwv1.HTTPRoute {
	return &gwv1.HTTPRoute{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: base.Namespace,
		},
	}
}

func agwPolicy(name string) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: base.Namespace,
		},
	}
}
