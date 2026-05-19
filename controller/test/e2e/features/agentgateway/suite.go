//go:build e2e

package agentgateway

import (
	"context"
	"fmt"
	"net/http"

	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	// This suite applies TrafficPolicy to specific named sections of the HTTPRoute, and requires HTTPRoutes.spec.rules[].name to be present in the Gateway API version.
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, base.TestCase{}, testCases, base.WithMinGwApiVersion(base.GwApiRequireRouteNames)),
	}
}

func (s *testingSuite) TestAgentgatewayTCPRoute() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.Ctx,
		sharedGatewayObjectMeta.Name,
		sharedGatewayObjectMeta.Namespace,
		gwv1.GatewayConditionProgrammed,
		metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyAccepted(
		s.Ctx,
		&gwv1.Gateway{ObjectMeta: sharedGatewayObjectMeta},
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayListenerAttachedRoutes(
		s.Ctx,
		sharedGatewayObjectMeta.Name,
		sharedGatewayObjectMeta.Namespace,
		"tcp",
		1,
	)

	gateway := common.Gateway{
		NamespacedName: types.NamespacedName{
			Name:      sharedGatewayObjectMeta.Name,
			Namespace: sharedGatewayObjectMeta.Namespace,
		},
		Address: common.ResolveGatewayAddress(
			s.Ctx,
			s.TestInstallation,
			types.NamespacedName{
				Name:      sharedGatewayObjectMeta.Name,
				Namespace: sharedGatewayObjectMeta.Namespace,
			},
		),
	}
	gateway.Send(
		s.T(),
		&matchers.HttpResponse{
			StatusCode: http.StatusOK,
		},
		curl.WithPort(gateway.PortForRemote(9090)),
	)
}

func (s *testingSuite) TestAgentgatewayHTTPRoute() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.Ctx,
		sharedGatewayObjectMeta.Name,
		sharedGatewayObjectMeta.Namespace,
		gwv1.GatewayConditionProgrammed,
		metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyAccepted(
		s.Ctx,
		&gwv1.Gateway{ObjectMeta: sharedGatewayObjectMeta},
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayListenerAttachedRoutes(
		s.Ctx,
		sharedGatewayObjectMeta.Name,
		sharedGatewayObjectMeta.Namespace,
		"http",
		1,
	)

	gateway := common.Gateway{
		NamespacedName: types.NamespacedName{
			Name:      sharedGatewayObjectMeta.Name,
			Namespace: sharedGatewayObjectMeta.Namespace,
		},
		Address: common.ResolveGatewayAddress(
			s.Ctx,
			s.TestInstallation,
			types.NamespacedName{
				Name:      sharedGatewayObjectMeta.Name,
				Namespace: sharedGatewayObjectMeta.Namespace,
			},
		),
	}
	gateway.Send(
		s.T(),
		&matchers.HttpResponse{
			StatusCode: http.StatusOK,
		},
		curl.WithHostHeader("www.example.com"),
		curl.WithPath("/status/200"),
	)
}

func (s *testingSuite) TestAgentgatewayPolicyQuantityApply() {
	tests := []struct {
		name      string
		quantity  string
		wantApply bool
	}{
		{
			name:      "invalid-int",
			quantity:  "0",
			wantApply: false,
		},
		{
			name:      "invalid-string",
			quantity:  "not-a-quantity",
			wantApply: false,
		},
		{
			name:      "valid-int",
			quantity:  "1024",
			wantApply: true,
		},
		{
			name:      "valid-string",
			quantity:  "64Ki",
			wantApply: true,
		},
	}

	for _, tt := range tests {
		s.Run(tt.name, func() {
			manifest := quantityPolicyManifest(tt.name, tt.quantity)
			err := s.TestInstallation.Actions.Kubectl().Apply(s.Ctx, []byte(manifest))
			if tt.wantApply {
				s.Require().NoError(err)

				testutils.Cleanup(s.T(), func() {
					_ = s.TestInstallation.Actions.Kubectl().Delete(s.Ctx, []byte(manifest))
				})
				return
			}
			s.Require().Error(err)
		})
	}
}

func quantityPolicyManifest(name, quantity string) string {
	return fmt.Sprintf(`apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayPolicy
metadata:
  name: quantity-%s
  namespace: agentgateway-base
spec:
  targetRefs:
  - group: gateway.networking.k8s.io
    kind: Gateway
    name: gateway
  frontend:
    http:
      maxBufferSize: %s
`, name, quantity)
}
