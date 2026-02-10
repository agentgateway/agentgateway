//go:build e2e

package rbac

import (
	"context"

	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/common"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

// testingSuite is a suite of tests for rbac functionality
type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, testCases),
	}
}

// TestRBACHeaderAuthorization tests header based rbac
func (s *testingSuite) TestRBACHeaderAuthorization() {
	// Verify HTTPRoute is accepted before running the test
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(s.Ctx, "httpbin-route", namespace, gwv1.RouteConditionAccepted, metav1.ConditionTrue)

	// TODO(npolshak): re-enable once sectionName is supported for TrafficPolicy in agentgateway once https://github.com/agentgateway/agentgateway/pull/323 is pulled in
	// missing header, no rbac on route, should succeed
	//s.T().Log("The /status route has no rbac")
	//common.BaseGateway.Send(
	//	s.T(),
	//	expectStatus200Success,
	//	curl.WithHostHeader("httpbin"),
	//	curl.WithPath("/status/200"),
	//)

	// missing header, should fail
	s.T().Log("The /get route has an rbac policy applied at the route level, should fail when the header is missing")
	common.BaseGateway.Send(
		s.T(),
		expectRBACDenied,
		curl.WithHostHeader("httpbin"),
		curl.WithPath("/get"),
	)
	// has header, should succeed
	s.T().Log("The /get route has an rbac policy applied at the route level, should succeed when the header is present")
	common.BaseGateway.Send(
		s.T(),
		expectStatus200Success,
		curl.WithHostHeader("httpbin"),
		curl.WithPath("/get"),
		curl.WithHeader("x-my-header", "cool-beans"),
	)
}
