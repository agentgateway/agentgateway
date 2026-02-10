//go:build e2e

package csrf

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
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	setupTestCase := base.TestCase{}
	testCases := map[string]*base.TestCase{
		"TestGatewayLevelCSRF": {
			Manifests: []string{routesManifest, csrfAgwPolicyManifest},
		},
	}

	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setupTestCase, testCases),
	}
}

func (s *testingSuite) SetupSuite() {
	s.BaseTestingSuite.SetupSuite()
}

func (s *testingSuite) TestGatewayLevelCSRF() {
	// Request without origin header should be allowed (agentgateway CSRF allows this)
	s.assertPreflightResponse("/path1", http.StatusOK, []curl.Option{})

	// Request without origin header should be allowed (agentgateway CSRF allows this)
	s.assertPreflightResponse("/path2", http.StatusOK, []curl.Option{})

	// Request with non-trusted origin header should be rejected
	s.assertPreflightResponse("/path1", http.StatusForbidden, []curl.Option{
		curl.WithHeader("Origin", "example.com"),
	})

	// Request with valid origin header should be allowed (configured in additionalOrigins)
	s.assertPreflightResponse("/path1", http.StatusOK, []curl.Option{
		curl.WithHeader("Origin", "example.org"),
	})

	// Request with valid origin header should be allowed (configured in additionalOrigins)
	s.assertPreflightResponse("/path2", http.StatusOK, []curl.Option{
		curl.WithHeader("Origin", "example.org"),
	})
}

// A safe http method is one that doesn't alter the state of the server (ie read only).
// A CSRF attack targets state changing requests, so the filter only acts on unsafe methods (ones that change state).
// We use POST as the unsafe method to test the filter.
func (s *testingSuite) assertPreflightResponse(path string, expectedStatus int, options []curl.Option) {
	allOptions := append([]curl.Option{
		curl.WithMethod("POST"),
		curl.WithPath(path),
		curl.WithHostHeader("example.com"),
	}, options...)

	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: expectedStatus},
		allOptions...,
	)
}
