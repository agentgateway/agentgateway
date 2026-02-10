//go:build e2e

package local

import (
	"context"
	"net/http"

	"github.com/stretchr/testify/suite"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/common"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
	testmatchers "github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

// testingSuite is a suite of basic routing / "happy path" tests
type testingSuite struct {
	*base.BaseTestingSuite
	agentgateway bool
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	setup := base.TestCase{
		Manifests: []string{httpRoutesManifest},
	}
	testCases := map[string]*base.TestCase{
		"TestLocalRateLimitForRoute":                  {Manifests: []string{routeLocalRateLimitManifest}},
		"TestLocalRateLimitForGateway":                {Manifests: []string{gwLocalRateLimitManifest}},
		"TestLocalRateLimitDisabledForRoute":          {Manifests: []string{disabledRouteLocalRateLimitManifest}},
		"TestLocalRateLimitForRouteUsingExtensionRef": {Manifests: []string{extensionRefManifest}},
	}

	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, testCases),
		agentgateway:     true,
	}
}

// Test cases for local rate limit on a route (/path1)
func (s *testingSuite) TestLocalRateLimitForRoute() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyObjectsExist(s.Ctx, route, route2, routeRateLimitTrafficPolicy)

	// First request should be successful
	s.assertResponse("/path1")

	// Consecutive requests should be rate limited
	s.assertConsistentResponse("/path1", http.StatusTooManyRequests)

	// Second route shouldn't be rate limited
	s.assertConsistentResponse("/path2", http.StatusOK)
}

// Test cases for local rate limit on a gateway
func (s *testingSuite) TestLocalRateLimitForGateway() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyObjectsExist(s.Ctx, route, route2, gwRateLimitTrafficPolicy)

	// First request should be successful (to any route)
	s.assertResponse("/path1")

	// Consecutive requests should be rate limited
	s.assertConsistentResponse("/path1", http.StatusTooManyRequests)

	// Also verify that the second route is rate limited
	s.assertConsistentResponse("/path2", http.StatusTooManyRequests)
}

// Test cases for local rate limit on a gateway and route (/path1) with disabled
// local rate limit
func (s *testingSuite) TestLocalRateLimitDisabledForRoute() {
	s.skipIfAgentgatewayUnsupported("LocalRateLimit disabled at Route level")
	s.TestInstallation.AssertionsT(s.T()).EventuallyObjectsExist(s.Ctx, route, route2, gwRateLimitTrafficPolicy, routeRateLimitTrafficPolicy)

	// First request should be successful (to any route)
	s.assertResponse("/path1")

	// Consecutive requests should not be rate limited (disabled for this path)
	s.assertConsistentResponse("/path1", http.StatusOK)

	// Also verify that the second route is rate limited
	s.assertConsistentResponse("/path2", http.StatusTooManyRequests)
}

// Test cases for local rate limit on a route (/path2) using extensionref in the HTTPRoute
func (s *testingSuite) TestLocalRateLimitForRouteUsingExtensionRef() {
	s.skipIfAgentgatewayUnsupported("LocalRateLimit using extensionRef in HTTPRoute")
	s.TestInstallation.AssertionsT(s.T()).EventuallyObjectsExist(s.Ctx, route, routeRateLimitTrafficPolicy)

	// First request should be successful
	s.assertResponse("/path2")

	// Consecutive requests should be rate limited
	s.assertConsistentResponse("/path2", http.StatusTooManyRequests)

	// Second route shouldn't be rate limited
	s.assertConsistentResponse("/path1", http.StatusOK)
}

func (s *testingSuite) assertResponse(path string) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: http.StatusOK,
		},
		curl.WithPath(path),
		curl.WithHostHeader("example.com"),
	)
}

func (s *testingSuite) assertConsistentResponse(path string, expectedStatus int) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: expectedStatus,
		},
		curl.WithPath(path),
		curl.WithHostHeader("example.com"),
	)
}

// skipIfAgentgatewayUnsupported skips a test when the agentgateway class
// is running and the feature isn't supported there yet.
func (s *testingSuite) skipIfAgentgatewayUnsupported(feature string) {
	if s.agentgateway {
		s.T().Helper()
		s.T().Skipf("Skipping %s on agentgateway: not supported yet", feature)
	}
}
