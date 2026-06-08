//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func TestAgentgatewayRouting(tt *testing.T) {
	t := New(tt)

	t.Run("HTTPRoute", func(t base.Test) {
		testAgentgatewayHTTPRoute(t)
	})
	t.Run("TCPRoute", func(t base.Test) {
		testAgentgatewayTCPRoute(t)
	})
}

func TestTrafficPolicyInheritanceOverride(tt *testing.T) {
	t := New(tt)
	testTrafficPolicyInheritanceOverride(t)
}

func testAgentgatewayHTTPRoute(t base.Test) {
	t.Apply(manifest("routing", "agw-http-route.yaml"))

	gateway := sharedGateway(t, "http", 1)
	gateway.Send(
		t,
		base.ExpectOK(),
		curl.WithHostHeader("www.example.com"),
		curl.WithPath("/status/200"),
	)
}

func testAgentgatewayTCPRoute(t base.Test) {
	t.Apply(manifest("routing", "agw-tcp-route.yaml"))

	gateway := sharedGateway(t, "tcp", 1)
	gateway.Send(
		t,
		base.Expect(http.StatusOK),
		curl.WithPort(gateway.PortForRemote(9090)),
	)
}

func testTrafficPolicyInheritanceOverride(t base.Test) {
	t.Apply(manifest("routing", "inheritance-override.yaml"))

	t.Send("inheritance.example.com/status/200", &matchers.HttpResponse{
		StatusCode: http.StatusOK,
		Headers: map[string]any{
			"x-policy-inheritance": "gateway",
		},
	})
}

func sharedGateway(t base.Test, listenerName string, attachedRoutes int) base.Gateway {
	t.GatewayReady("gateway", base.Namespace)
	assertions.EventuallyGatewayListenerAttachedRoutes(t,
		"gateway",
		base.Namespace,
		gwv1.SectionName(listenerName),
		int32(attachedRoutes), // nolint: gosec // testing only
	)

	name := types.NamespacedName{Name: "gateway", Namespace: base.Namespace}
	return base.Gateway{
		NamespacedName: name,
		Address:        base.ResolveGatewayAddress(t, t.Ctx, t.TestInstallation, name),
	}
}
