//go:build e2e

package remotejwtauth

import (
	"context"
	"net/http"
	"path/filepath"

	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
	"github.com/agentgateway/agentgateway/controller/test/testutils/testjwt"
)

//
// Use `go run hack/utils/jwt/jwt-generator.go`
// to generate jwks and a jwt signed by the key in it
//

var _ e2e.NewSuiteFunc = NewTestingSuite

const namespace = "agentgateway-base"

var (
	setup = base.TestCase{
		Manifests: []string{
			getTestFile("common.yaml"),
		},
	}

	testCases = map[string]*base.TestCase{
		"TestRoutePolicySvc": {
			Manifests: []string{secureRoutePolicyManifestSvc},
		},
		"TestRoutePolicySvcCaCert": {
			Manifests: []string{secureRoutePolicyManifestSvcCaCert},
		},
		"TestRoutePolicyBackend": {
			Manifests: []string{insecureRouteManifest, secureRoutePolicyManifestBackend},
		},
		"TestRoutePolicyBackendAndTlsPolicy": {
			Manifests: []string{secureRoutePolicyManifestBackendAndTlsPolicy},
		},
		"TestRoutePolicyWithRbac": {
			Manifests: []string{secureRoutePolicyWithRbacManifest},
		},
		"TestGatewayPolicySvc": {
			Manifests: []string{secureGWPolicyManifestSvc},
		},
		"TestGatewayPolicySvcCaCert": {
			Manifests: []string{secureGWPolicyManifestSvcCaCert},
		},
		"TestGatewayPolicyBackend": {
			Manifests: []string{secureGWPolicyManifestBackend},
		},
		"TestGatewayPolicyBackendWithTlsPolicy": {
			Manifests: []string{secureGWPolicyManifestBackendAndTlsPolicy},
		},
		"TestGatewayPolicyWithRbac": {
			Manifests: []string{secureGWPolicyWithRbacManifest},
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

var (
	insecureRouteManifest                        = getTestFile("insecure-route.yaml")
	secureGWPolicyManifestBackend                = getTestFile("secured-gateway-policy-with-backend.yaml")
	secureGWPolicyManifestBackendAndTlsPolicy    = getTestFile("secured-gateway-policy-with-backend-and-ref.yaml")
	secureGWPolicyManifestSvc                    = getTestFile("secured-gateway-policy-with-svc.yaml")
	secureGWPolicyManifestSvcCaCert              = getTestFile("secured-gateway-policy-with-svc-ca-cert.yaml")
	secureGWPolicyWithRbacManifest               = getTestFile("secured-gateway-policy-with-rbac.yaml")
	secureRoutePolicyManifestBackend             = getTestFile("secured-route-with-backend.yaml")
	secureRoutePolicyManifestBackendAndTlsPolicy = getTestFile("secured-route-with-backend-and-ref.yaml")
	secureRoutePolicyManifestSvc                 = getTestFile("secured-route-with-svc.yaml")
	secureRoutePolicyManifestSvcCaCert           = getTestFile("secured-route-with-svc-ca-cert.yaml")
	secureRoutePolicyWithRbacManifest            = getTestFile("secured-route-with-rbac.yaml")
)

func (s *testingSuite) TestRoutePolicyBackend() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-example-insecure",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify unprotected route works
	s.assertResponseWithoutAuth("insecureroute.com", http.StatusOK)

	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify a provider with a single key in jwks works
	s.assertResponse("secureroute.com", testjwt.OrgOneJWT, http.StatusOK)
	s.assertResponse("secureroute.com", testjwt.OrgTwoJWT, http.StatusOK)
	// verify invalid/missing tokens are caught
	s.assertResponse("secureroute.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("secureroute.com", http.StatusUnauthorized)
}

func (s *testingSuite) TestRoutePolicyBackendAndTlsPolicy() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify a provider with a single key in jwks works
	s.assertResponse("secureroute.com", testjwt.OrgOneJWT, http.StatusOK)
	// verify invalid/missing tokens are caught
	s.assertResponse("secureroute.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("secureroute.com", http.StatusUnauthorized)
}

func (s *testingSuite) TestRoutePolicySvcCaCert() {
	s.TestRoutePolicySvc()
}

func (s *testingSuite) TestRoutePolicySvc() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify a provider with a single key in jwks works
	s.assertResponse("secureroute.com", testjwt.OrgOneJWT, http.StatusOK)
	// verify invalid/missing tokens are caught
	s.assertResponse("secureroute.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("secureroute.com", http.StatusUnauthorized)
}

func (s *testingSuite) TestRoutePolicyWithRbac() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify a jwt with expected subject works
	s.assertResponse("secureroute.com", testjwt.OrgOneJWT, http.StatusOK)
	// verify a jwt with unexpected subject is denied
	s.assertResponse("secureroute.com", testjwt.OrgFourJWT, http.StatusForbidden)
}

func (s *testingSuite) TestGatewayPolicySvc() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-gw",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	s.assertResponse("securegateways.com", testjwt.OrgOneJWT, http.StatusOK)
	// verify invalid/missing tokens are caught
	s.assertResponse("securegateways.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("securegateways.com", http.StatusUnauthorized)
}

func (s *testingSuite) TestGatewayPolicySvcCaCert() {
	s.TestGatewayPolicySvc()
}

func (s *testingSuite) TestGatewayPolicyBackend() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-gw",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	s.assertResponse("securegateways.com", testjwt.OrgOneJWT, http.StatusOK)
	s.assertResponse("securegateways.com", testjwt.OrgTwoJWT, http.StatusOK)
	// verify invalid/missing tokens are caught
	s.assertResponse("securegateways.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("securegateways.com", http.StatusUnauthorized)
}

func (s *testingSuite) TestGatewayPolicyBackendWithTlsPolicy() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-gw",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	s.assertResponse("securegateways.com", testjwt.OrgOneJWT, http.StatusOK)
	// verify invalid/missing tokens are caught
	s.assertResponse("securegateways.com", "nosuchkey", http.StatusUnauthorized)
	s.assertResponseWithoutAuth("securegateways.com", http.StatusUnauthorized)
}

func (s *testingSuite) TestGatewayPolicyWithRbac() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-gw",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify a jwt with expected subject works
	s.assertResponse("securegateways.com", testjwt.OrgOneJWT, http.StatusOK)
	// verify a jwt with unexpected subject is denied
	s.assertResponse("securegateways.com", testjwt.OrgFourJWT, http.StatusForbidden)
}

func (s *testingSuite) assertResponse(hostHeader, authHeader string, expectedStatus int) {
	gw := s.gateway()
	gw.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: expectedStatus,
		},
		curl.WithHostHeader(hostHeader),
		curl.WithHeader("Authorization", "Bearer "+authHeader),
	)
}

func (s *testingSuite) assertResponseWithoutAuth(hostHeader string, expectedStatus int) {
	gw := s.gateway()
	gw.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: expectedStatus,
		},
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

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
