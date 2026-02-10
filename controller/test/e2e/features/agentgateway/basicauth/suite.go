//go:build e2e

package basicauth

import (
	"context"
	"encoding/base64"
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

	setup = base.TestCase{
		Manifests: []string{},
	}

	testCases = map[string]*base.TestCase{
		"TestRoutePolicy": {
			Manifests: []string{insecureRouteManifest, secureRoutePolicyManifest},
		},
		"TestGatewayPolicy": {
			Manifests: []string{secureGwPolicyManifest},
		},
	}
)

// testingSuite is a suite of global rate limiting tests
type testingSuite struct {
	*base.BaseTestingSuite

	// testInstallation contains all the metadata/utilities necessary to execute a series of tests
	// against an installation of kgateway
	testInstallation *e2e.TestInstallation
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, testCases),
		testInstallation: testInst,
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
	// test unprotected route works
	s.assertResponseWithoutAuth("insecureroute.com", http.StatusOK)

	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// test inline username/password store
	s.assertResponse("secureroute.com", base64.StdEncoding.EncodeToString(([]byte)("alice:alicepassword")), http.StatusOK)
	s.assertResponse("secureroute.com", base64.StdEncoding.EncodeToString(([]byte)("bob:bobpassword")), http.StatusOK)

	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-too",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// test secret-based username/password store
	s.assertResponse("secureroutetoo.com", base64.StdEncoding.EncodeToString(([]byte)("eve:evepassword")), http.StatusOK)
	s.assertResponse("secureroutetoo.com", base64.StdEncoding.EncodeToString(([]byte)("mallory:mallorypassword")), http.StatusOK)
	// test invalid username/password combinations
	s.assertResponse("secureroute.com", base64.StdEncoding.EncodeToString(([]byte)("alice:boom")), http.StatusUnauthorized)
	s.assertResponse("secureroutetoo.com", base64.StdEncoding.EncodeToString(([]byte)("eve:boom")), http.StatusUnauthorized)
	s.assertResponse("secureroute.com", base64.StdEncoding.EncodeToString(([]byte)("trent:boom")), http.StatusUnauthorized)
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
	// test inline user/password store
	s.assertResponse("securegateways.com", base64.StdEncoding.EncodeToString(([]byte)("alice:alicepassword")), http.StatusOK)
	s.assertResponse("securegateways.com", base64.StdEncoding.EncodeToString(([]byte)("bob:bobpassword")), http.StatusOK)

	// test invalid username/password combinations
	s.assertResponse("securegateways.com", base64.StdEncoding.EncodeToString(([]byte)("alice:boom")), http.StatusUnauthorized)
	s.assertResponse("securegateways.com", base64.StdEncoding.EncodeToString(([]byte)("trent:boom")), http.StatusUnauthorized)
	s.assertResponseWithoutAuth("securegateways.com", http.StatusUnauthorized)
}

func (s *testingSuite) assertResponse(hostHeader, authHeader string, expectedStatus int) {
	common.BaseGateway.Send(s.T(),
		&testmatchers.HttpResponse{StatusCode: expectedStatus},
		curl.WithHostHeader(hostHeader),
		curl.WithHeader("Authorization", "Basic "+authHeader))
}

func (s *testingSuite) assertResponseWithoutAuth(hostHeader string, expectedStatus int) {
	common.BaseGateway.Send(s.T(),
		&testmatchers.HttpResponse{StatusCode: expectedStatus},
		curl.WithHostHeader(hostHeader))
}

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
