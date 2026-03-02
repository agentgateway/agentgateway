//go:build e2e

package oauth2auth

import (
	"context"
	"net/http"
	"path/filepath"

	"github.com/onsi/gomega"
	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

const (
	namespace = "agentgateway-base"
)

var (
	insecureRouteManifest     = getTestFile("insecure-route.yaml")
	secureGWPolicyManifest    = getTestFile("secured-gateway-policy.yaml")
	secureRoutePolicyManifest = getTestFile("secured-route.yaml")

	setup = base.TestCase{
		Manifests: []string{
			getTestFile("common.yaml"),
		},
	}

	testCases = map[string]*base.TestCase{
		"TestRoutePolicy": {
			Manifests: []string{insecureRouteManifest, secureRoutePolicyManifest},
		},
		"TestGatewayPolicy": {
			Manifests: []string{secureGWPolicyManifest},
		},
	}
)

type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(
			ctx,
			testInst,
			setup,
			testCases,
			base.WithMinGwApiVersion(base.GwApiRequireBackendTLSPolicy),
		),
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
	s.assertResponseWithoutAuth("insecureroute.com", http.StatusOK)

	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-oauth2",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	s.assertApiChallenge("oauth2route.com")
	s.assertBrowserRedirect("oauth2route.com")
}

func (s *testingSuite) TestGatewayPolicy() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-oauth2-gw",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	s.assertApiChallenge("oauth2gateway.com")
	s.assertBrowserRedirect("oauth2gateway.com")
}

func (s *testingSuite) assertApiChallenge(hostHeader string) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: http.StatusUnauthorized,
			Headers: map[string]any{
				"WWW-Authenticate": gomega.ContainSubstring("Bearer"),
			},
		},
		curl.WithHostHeader(hostHeader),
	)
}

func (s *testingSuite) assertBrowserRedirect(hostHeader string) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: http.StatusFound,
			Headers: map[string]any{
				"Location": gomega.ContainSubstring("/protocol/openid-connect/auth"),
			},
		},
		curl.WithHostHeader(hostHeader),
		curl.WithHeader("Accept", "text/html"),
	)
}

func (s *testingSuite) assertResponseWithoutAuth(hostHeader string, expectedStatus int) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: expectedStatus},
		curl.WithHostHeader(hostHeader),
	)
}

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
