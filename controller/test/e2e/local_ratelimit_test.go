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

func TestLocalRateLimit(t *testing.T) {
	agw := New(t)
	agw.Apply(manifest("rate-limit", "local", "httproutes.yaml"))

	agw.Run("Route", func() {
		testLocalRateLimitForRoute(agw)
	})
	agw.Run("Gateway", func() {
		testLocalRateLimitForGateway(agw)
	})
	agw.Run("RouteDisabled", func() {
		agw.T().Skip("Skipping LocalRateLimit disabled at Route level on agentgateway: not supported yet")
	})
	agw.Run("RouteUsingExtensionRef", func() {
		agw.T().Skip("Skipping LocalRateLimit using extensionRef in HTTPRoute on agentgateway: not supported yet")
	})
}

func testLocalRateLimitForRoute(agw *base.BaseTestingSuite) {
	agw.Apply(
		manifest("rate-limit", "local", "route-local-rate-limit.yaml"),
	)

	agw.TestInstallation.AssertionsT(agw.T()).EventuallyObjectsExist(
		agw.Ctx,
		httpRoute("svc-route"),
		httpRoute("svc-route-2"),
		agwPolicy("route-rl-policy"),
	)

	agw.Send("example.com/path1", base.ExpectOK())
	agw.Send("example.com/path1", base.Expect(http.StatusTooManyRequests))
	agw.Send("example.com/path2", base.ExpectOK())
}

func testLocalRateLimitForGateway(agw *base.BaseTestingSuite) {
	agw.Apply(
		manifest("rate-limit", "local", "gw-local-rate-limit.yaml"),
	)

	agw.TestInstallation.AssertionsT(agw.T()).EventuallyObjectsExist(
		agw.Ctx,
		httpRoute("svc-route"),
		httpRoute("svc-route-2"),
		agwPolicy("gw-rl-policy"),
	)

	agw.Send("example.com/path1", base.ExpectOK())
	agw.Send("example.com/path1", base.Expect(http.StatusTooManyRequests))
	agw.Send("example.com/path2", base.Expect(http.StatusTooManyRequests))
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
