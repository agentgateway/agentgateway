//go:build e2e

package apikeyauth

import (
	"context"
	"net/http"
	"path/filepath"

	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/common"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
	testmatchers "github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

const (
	// test namespace for proxy resources
	namespace = "agentgateway-base"
)

var (
	insecureRouteManifest     = getTestFile("insecure-route.yaml")
	secureGwPolicyManifest    = getTestFile("secured-gateway-policy.yaml")
	secureRoutePolicyManifest = getTestFile("secured-route.yaml")

	setup = base.TestCase{}

	testCases = map[string]*base.TestCase{
		"TestRoutePolicy": {
			Manifests: []string{insecureRouteManifest, secureRoutePolicyManifest},
		},
		"TestGatewayPolicy": {
			Manifests: []string{secureGwPolicyManifest},
		},
	}
)

type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, testCases),
	}
}

func (s *testingSuite) TestRoutePolicy() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-example-insecure",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify insecure route works
	s.assertResponseWithoutAuth("insecureroute.com", http.StatusOK)

	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify key without metadata works
	s.assertResponse("secureroute.com", "k-1230", http.StatusOK)
	// verify key with metadata works
	s.assertResponse("secureroute.com", "k-4560", http.StatusOK)
	// verify invalid keys are rejected
	s.assertResponse("secureroute.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("secureroute.com", http.StatusUnauthorized)
}

func (s *testingSuite) TestGatewayPolicy() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-gw",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify key without metadata works
	s.assertResponse("securegateways.com", "k-123", http.StatusOK)
	// verify key with metadata works
	s.assertResponse("securegateways.com", "k-456", http.StatusOK)
	// verify invalid keys are rejected
	s.assertResponse("securegateways.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("securegateways.com", http.StatusUnauthorized)
}

func (s *testingSuite) assertResponse(hostHeader, authHeader string, expectedStatus int) {
	common.BaseGateway.Send(s.T(),
		&testmatchers.HttpResponse{StatusCode: expectedStatus},
		curl.WithHostHeader(hostHeader),
		curl.WithHeader("Authorization", "Bearer "+authHeader))
}

func (s *testingSuite) assertResponseWithoutAuth(hostHeader string, expectedStatus int) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: expectedStatus},
		curl.WithHostHeader(hostHeader))
}

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
