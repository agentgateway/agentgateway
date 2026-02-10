//go:build e2e

package global

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

// testingSuite is a suite of global rate limiting tests
type testingSuite struct {
	*base.BaseTestingSuite
}

// rlBurstTries: run a tiny burst so all checks stay in one fixed RL window.
// The external rate limiter uses clock-aligned windows (per-minute resets at :00),
// so long loops can straddle the boundary and flake.
// 3 = one to establish state, two to confirm; fewer risks a transient, more risks crossing the window.
var rlBurstTries = 3

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	setup := base.TestCase{
		Manifests: []string{
			rateLimitServerManifest,
		},
	}
	testCases := map[string]*base.TestCase{
		"TestGlobalRateLimitByRemoteAddress": {
			Manifests: []string{httpRoutesManifest, ipRateLimitManifest},
		},
		"TestGlobalRateLimitByPath": {
			Manifests: []string{httpRoutesManifest, pathRateLimitManifest},
		},
		"TestGlobalRateLimitByUserID": {
			Manifests: []string{httpRoutesManifest, userRateLimitManifest},
		},
		"TestCombinedLocalAndGlobalRateLimit": {
			Manifests: []string{httpRoutesManifest, combinedRateLimitManifest},
		},
	}

	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, testCases),
	}
}

// Test cases for global rate limit based on remote address (client IP)
func (s *testingSuite) TestGlobalRateLimitByRemoteAddress() {
	// First request should be successful
	s.assertResponse("/path1", http.StatusOK)

	// Consecutive requests should be rate limited
	s.assertConsistentResponse("/path1", http.StatusTooManyRequests)

	// Second route should also be rate limited since the rate limit is based on client IP
	s.assertConsistentResponse("/path2", http.StatusTooManyRequests)
}

// Test cases for global rate limit based on request path
func (s *testingSuite) TestGlobalRateLimitByPath() {
	// First request should be successful
	s.assertResponse("/path1", http.StatusOK)

	// Consecutive requests to the same path should be rate limited
	s.assertConsistentResponse("/path1", http.StatusTooManyRequests)

	// Second route shouldn't be rate limited since it has a different path
	s.assertConsistentResponse("/path2", http.StatusOK)
}

// Test cases for global rate limit based on user ID header
func (s *testingSuite) TestGlobalRateLimitByUserID() {
	// First request should be successful
	s.assertResponseWithHeader("/path1", "X-User-ID", "user1", http.StatusOK)

	// Consecutive requests from same user should be rate limited
	s.assertConsistentResponseWithHeader("/path1", "X-User-ID", "user1", http.StatusTooManyRequests)

	// Requests from different user shouldn't be rate limited
	s.assertResponseWithHeader("/path1", "X-User-ID", "user2", http.StatusOK)
}

// Test cases for combined local and global rate limiting
func (s *testingSuite) TestCombinedLocalAndGlobalRateLimit() {
	// First request should be successful
	s.assertResponse("/path1", http.StatusOK)

	// Consecutive requests should be rate limited
	s.assertConsistentResponse("/path1", http.StatusTooManyRequests)
}

func (s *testingSuite) assertResponse(path string, expectedStatus int) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: expectedStatus,
		},
		curl.WithPath(path),
		curl.WithHostHeader("example.com"),
	)
}

func (s *testingSuite) assertResponseWithHeader(path string, headerName string, headerValue string, expectedStatus int) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: expectedStatus,
		},
		curl.WithPath(path),
		curl.WithHostHeader("example.com"),
		curl.WithHeader(headerName, headerValue),
	)
}

// Burst a few quick checks so the test doesn't cross a rate-limit window boundary.
func (s *testingSuite) assertConsistentResponse(path string, expectedStatus int) {
	for range rlBurstTries {
		common.BaseGateway.Send(
			s.T(),
			&testmatchers.HttpResponse{StatusCode: expectedStatus},
			curl.WithPath(path),
			curl.WithHostHeader("example.com"),
		)
	}
}

// Safe burst a few quick checks so the test doesn't cross a rate-limit window boundary.
func (s *testingSuite) assertConsistentResponseWithHeader(path, headerName, headerValue string, expectedStatus int) {
	for range rlBurstTries {
		common.BaseGateway.Send(
			s.T(),
			&testmatchers.HttpResponse{StatusCode: expectedStatus},
			curl.WithPath(path),
			curl.WithHostHeader("example.com"),
			curl.WithHeader(headerName, headerValue),
		)
	}
}
