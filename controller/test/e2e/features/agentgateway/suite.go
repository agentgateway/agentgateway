//go:build e2e

package agentgateway

import (
	"context"
	"net/http"

	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
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
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.Ctx,
		sharedGatewayObjectMeta.Name,
		sharedGatewayObjectMeta.Namespace,
		gwv1.GatewayConditionAccepted,
		metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayListenerAttachedRoutes(
		s.Ctx,
		sharedGatewayObjectMeta.Name,
		sharedGatewayObjectMeta.Namespace,
		"tcp",
		1,
	)

	gateway := common.Gateway{
		Address: s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayAddress(
			s.Ctx,
			sharedGatewayObjectMeta.Name,
			sharedGatewayObjectMeta.Namespace,
		),
	}
	gateway.Send(
		s.T(),
		&matchers.HttpResponse{
			StatusCode: http.StatusOK,
		},
		curl.WithPort(9090),
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
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.Ctx,
		sharedGatewayObjectMeta.Name,
		sharedGatewayObjectMeta.Namespace,
		gwv1.GatewayConditionAccepted,
		metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayListenerAttachedRoutes(
		s.Ctx,
		sharedGatewayObjectMeta.Name,
		sharedGatewayObjectMeta.Namespace,
		"http",
		1,
	)

	gateway := common.Gateway{
		Address: s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayAddress(
			s.Ctx,
			sharedGatewayObjectMeta.Name,
			sharedGatewayObjectMeta.Namespace,
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
