//go:build e2e

package agentgateway

import (
	"context"
	"net/http"

	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/common"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
	"github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
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
		tcpGatewayObjectMeta.Name,
		tcpGatewayObjectMeta.Namespace,
		gwv1.GatewayConditionProgrammed,
		metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.Ctx,
		tcpGatewayObjectMeta.Name,
		tcpGatewayObjectMeta.Namespace,
		gwv1.GatewayConditionAccepted,
		metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayListenerAttachedRoutes(
		s.Ctx,
		tcpGatewayObjectMeta.Name,
		tcpGatewayObjectMeta.Namespace,
		"tcp",
		1,
	)

	gateway := common.Gateway{
		Address: s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayAddress(
			s.Ctx,
			tcpGatewayObjectMeta.Name,
			tcpGatewayObjectMeta.Namespace,
		),
	}
	gateway.Send(
		s.T(),
		&matchers.HttpResponse{
			StatusCode: http.StatusOK,
		},
		curl.VerboseOutput(),
		curl.WithPort(8080),
	)
}

func (s *testingSuite) TestAgentgatewayHTTPRoute() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.Ctx,
		httpGatewayObjectMeta.Name,
		httpGatewayObjectMeta.Namespace,
		gwv1.GatewayConditionProgrammed,
		metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.Ctx,
		httpGatewayObjectMeta.Name,
		httpGatewayObjectMeta.Namespace,
		gwv1.GatewayConditionAccepted,
		metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayListenerAttachedRoutes(
		s.Ctx,
		httpGatewayObjectMeta.Name,
		httpGatewayObjectMeta.Namespace,
		"http",
		1,
	)

	gateway := common.Gateway{
		Address: s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayAddress(
			s.Ctx,
			httpGatewayObjectMeta.Name,
			httpGatewayObjectMeta.Namespace,
		),
	}
	gateway.Send(
		s.T(),
		&matchers.HttpResponse{
			StatusCode: http.StatusOK,
		},
		curl.VerboseOutput(),
		curl.WithHostHeader("www.example.com"),
		curl.WithPath("/status/200"),
		curl.WithPort(8080),
	)
}
