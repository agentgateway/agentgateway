//go:build e2e

package jwtauth

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
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, testCases),
	}
}

func (s *testingSuite) TestRoutePolicy() {
	s.assertRouteAccepted(insecureRouteName)
	s.assertResponseWithoutAuth(insecureRouteHost, http.StatusOK)
	s.assertProtectedRoute(secureRouteName, secureRouteHost, []string{jwt1, jwt2, jwt3})
}

func (s *testingSuite) TestRoutePolicyWithRbac() {
	s.assertRouteAccepted(secureRouteName)
	s.assertResponse(secureRouteHost, jwt4, http.StatusOK)
	s.assertResponse(secureRouteHost, jwt5, http.StatusForbidden)
}

func (s *testingSuite) TestGatewayPolicy() {
	s.assertProtectedRoute(secureGatewayRouteName, secureGatewayHost, []string{jwt1, jwt2, jwt3})
}

func (s *testingSuite) TestGatewayPolicyWithRbac() {
	s.assertRouteAccepted(secureGatewayRouteName)
	s.assertResponse(secureGatewayHost, jwt4, http.StatusOK)
	s.assertResponse(secureGatewayHost, jwt5, http.StatusForbidden)
}

func (s *testingSuite) assertProtectedRoute(routeName, host string, okTokens []string) {
	s.assertRouteAccepted(routeName)
	for _, token := range okTokens {
		s.assertResponse(host, token, http.StatusOK)
	}
	s.assertResponse(host, "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth(host, http.StatusUnauthorized)
}

func (s *testingSuite) assertRouteAccepted(routeName string) {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		routeName,
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
}

func (s *testingSuite) assertResponse(hostHeader, authHeader string, expectedStatus int) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: expectedStatus},
		curl.WithHostHeader(hostHeader),
		curl.WithHeader("Authorization", "Bearer "+authHeader),
	)
}

func (s *testingSuite) assertResponseWithoutAuth(hostHeader string, expectedStatus int) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: expectedStatus},
		curl.WithHostHeader(hostHeader),
	)
}
