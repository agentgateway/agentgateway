//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
)

func TestAgentgatewayRouting(t *testing.T) {
	agw := New(t)

	agw.Run("HTTPRoute", func() {
		testAgentgatewayHTTPRoute(agw)
	})
	agw.Run("TCPRoute", func() {
		testAgentgatewayTCPRoute(agw)
	})
}

func testAgentgatewayHTTPRoute(agw *base.BaseTestingSuite) {
	agw.Apply(manifest("routing", "agw-http-route.yaml"))

	gateway := sharedGateway(agw, "http", 1)
	gateway.Send(
		agw.T(),
		base.ExpectOK(),
		curl.WithHostHeader("www.example.com"),
		curl.WithPath("/status/200"),
	)
}

func testAgentgatewayTCPRoute(agw *base.BaseTestingSuite) {
	agw.ApplyConfig(base.TestCase{
		Manifests:       []string{manifest("routing", "agw-tcp-route.yaml")},
		MinGwApiVersion: base.GwApiRequireTcpRoutes,
	})

	gateway := sharedGateway(agw, "tcp", 1)
	gateway.Send(
		agw.T(),
		base.Expect(http.StatusOK),
		curl.WithPort(gateway.PortForRemote(9090)),
	)
}

func sharedGateway(t *base.BaseTestingSuite, listenerName string, attachedRoutes int) base.Gateway {
	t.GatewayReady("gateway", base.Namespace)
	t.TestInstallation.AssertionsT(t.T()).EventuallyGatewayListenerAttachedRoutes(
		t.Ctx,
		"gateway",
		base.Namespace,
		gwv1.SectionName(listenerName),
		int32(attachedRoutes),
	)

	name := types.NamespacedName{Name: "gateway", Namespace: base.Namespace}
	return base.Gateway{
		NamespacedName: name,
		Address:        base.ResolveGatewayAddress(t.Ctx, t.TestInstallation, name),
	}
}
