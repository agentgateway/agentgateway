//go:build e2e

package remotejwtauth

import (
	"context"
	"net/http"

	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
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

func (s *testingSuite) TestRoutePolicyBackend() {
	s.assertRouteAccepted(insecureRouteName)
	s.assertResponseWithoutAuth(insecureRouteHost, http.StatusOK)
	s.assertProtectedRoute(secureRouteName, secureRouteHost, []string{jwtOrgOne, jwtOrgTwo})
}

func (s *testingSuite) TestRoutePolicyBackendAndTlsPolicy() {
	s.assertProtectedRoute(secureRouteName, secureRouteHost, []string{jwtOrgOne})
}

func (s *testingSuite) TestRoutePolicySvcCaCert() {
	s.TestRoutePolicySvc()
}

func (s *testingSuite) TestRoutePolicySvc() {
	s.assertProtectedRoute(secureRouteName, secureRouteHost, []string{jwtOrgOne})
}

func (s *testingSuite) TestRoutePolicyWithRbac() {
	s.assertRouteAccepted(secureRouteName)
	s.assertResponse(secureRouteHost, jwtOrgOne, http.StatusOK)
	s.assertResponse(secureRouteHost, jwtOrgFour, http.StatusForbidden)
}

func (s *testingSuite) TestGatewayPolicySvc() {
	s.assertProtectedRoute(secureGatewayRouteName, secureGatewayHost, []string{jwtOrgOne})
}

func (s *testingSuite) TestGatewayPolicySvcCaCert() {
	s.TestGatewayPolicySvc()
}

func (s *testingSuite) TestGatewayPolicyBackend() {
	s.assertProtectedRoute(secureGatewayRouteName, secureGatewayHost, []string{jwtOrgOne, jwtOrgTwo})
}

func (s *testingSuite) TestGatewayPolicyBackendWithTlsPolicy() {
	s.assertProtectedRoute(secureGatewayRouteName, secureGatewayHost, []string{jwtOrgOne})
}

func (s *testingSuite) TestGatewayPolicyWithRbac() {
	s.assertRouteAccepted(secureGatewayRouteName)
	s.assertResponse(secureGatewayHost, jwtOrgOne, http.StatusOK)
	s.assertResponse(secureGatewayHost, jwtOrgFour, http.StatusForbidden)
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
	gateway := s.gateway()
	gateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: expectedStatus},
		curl.WithHostHeader(hostHeader),
		curl.WithHeader("Authorization", "Bearer "+authHeader),
	)
}

func (s *testingSuite) assertResponseWithoutAuth(hostHeader string, expectedStatus int) {
	gateway := s.gateway()
	gateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: expectedStatus},
		curl.WithHostHeader(hostHeader),
	)
}

func (s *testingSuite) gateway() common.Gateway {
	name := types.NamespacedName{
		Namespace: namespace,
		Name:      "gateway",
	}
	return common.Gateway{
		NamespacedName: name,
		Address:        common.ResolveGatewayAddress(s.Ctx, s.TestInstallation, name),
	}
}
