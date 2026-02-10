//go:build e2e

package remotejwtauth

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

//
// Use `go run hack/utils/jwt/jwt-generator.go`
// to generate jwks and a jwt signed by the key in it
//

var _ e2e.NewSuiteFunc = NewTestingSuite

const (
	namespace = "agentgateway-base"
	// jwt subject is "ignore@kgateway.dev"
	// could also retrieve these jwts from  https://dummy-idp.default:8443/org-one/jwt, https://dummy-idp.default:8443/org-two/jwt
	JwtOrgOne = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjUzNTAyMzEyMTkzMDYwMzg2OTIiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6Imlnbm9yZUBrZ2F0ZXdheS5kZXYiLCJleHAiOjIwNzExNjM0MDcsIm5iZiI6MTc2MzU3OTQwNywiaWF0IjoxNzYzNTc5NDA3fQ.TsHCCdd0_629wibU4EviEi1-_UXaFUX1NuLgXCrC-tr7kqlcnUJIJC0WSab1EgXKtF8gTfwTUeQcAQNrunwngQU-K9DFcH5-2vnGeiXV3_X3SokkPq74ceRrCFEL2d7YNaGfhq_UNyvKRJsRz-pwdKK7QIPXALmWaUHn7EV7zU-CcPCKNwmt62P88qNp5HYSbgqz_WfnzIIH8LANpCC8fUqVedgTJMJ86E06pfDNUuuXe_fhjgMQXlfyDeUxIuzJunvS2qIqt4IYMzjcQbl2QI1QK3xz37tridSP_WVuuMUe2Lqo0oDjWVpxqPb5fb90W6a6khRP59Pf6qKMbQ9SQg"
	jwtOrgTwo = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjI4OTk1NjQyMzcyMTQ2ODQ5NDciLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6Imlnbm9yZUBrZ2F0ZXdheS5kZXYiLCJleHAiOjIwNzExNjM1MzIsIm5iZiI6MTc2MzU3OTUzMiwiaWF0IjoxNzYzNTc5NTMyfQ.kLazcb2o_zcVfJ7WECsQJdOaluxAJ-GdOkeuXUOJSeN8PvahjxfpftgeJjcGsp2sl-VIKXIuTLH6csHT_CBq7kI8bVKGDkk8qw3w8gem7MtiXKPMSYiYEHAoCCzsl8O-pGPF6G_PU-CfiWla8CIAjOewLzRmLeAYmwEiUYf8LQ7y6BbVDzvtxIQW3pTurHXFy0TZ6nUGqu_Xwh7uXe42WC0T-9LAI4zsGo5x_FKhlE_6N9_a7R0UIYFeRrbph_b1z47xTZ3YhZBmQmue2j1xR6hwRCnL7mOaCrxdte8SqXNUVA6vPSaiMTSkdmKyeRSzeTliDKiqAmP8eiIaqAoN5A"
	// sub "boom@kgateway.dev"
	jwtOrgFour = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjI5MjkxMDAyNTE1MzE5NjM0MCIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6ImJvb21Aa2dhdGV3YXkuZGV2IiwiZXhwIjoyMDczMTU2OTc5LCJuYmYiOjE3NjU1NzI5NzksImlhdCI6MTc2NTU3Mjk3OX0.juMOUmoChZEE_AQVZv3jwtZjytWfzN23-palLXA-DIsSa4-f-lmf3CQiwXz0n1YlSY_dt3rGO6OsDdkYn8wkYEVoQVh11crJvZ5FhpIlZlROOSp03KTW2mQ1XwGYRxffzdzBv65LrFYWK0iNQH2NKfqOzVo5xt3SLTJuxIvCE8-qnqXUWrADw3b2TIzE7SgN7xXzeRGwTpgltq4BswdkB0R5g_1xtbrcdFgT533vt3nCiumhqrBkmk4g02x3L1iSjDCnnwJX2YLHYfpUN0i7SooguTkta067lwBiOi3NOTQjRBOBlZmkoj6sz4YNQ9EwsD74pkNBW9pN-__2cVPBxw"
)

var (
	setup = base.TestCase{
		Manifests: []string{
			getTestFile("common.yaml"),
			getTestFile("service.yaml"),
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
	s.assertResponse("secureroute.com", JwtOrgOne, http.StatusOK)
	s.assertResponse("secureroute.com", jwtOrgTwo, http.StatusOK)
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
	s.assertResponse("secureroute.com", JwtOrgOne, http.StatusOK)
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
	s.assertResponse("secureroute.com", JwtOrgOne, http.StatusOK)
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
	s.assertResponse("secureroute.com", JwtOrgOne, http.StatusOK)
	// verify a jwt with unexpected subject is denied
	s.assertResponse("secureroute.com", jwtOrgFour, http.StatusForbidden)
}

func (s *testingSuite) TestGatewayPolicySvc() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-gw",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	s.assertResponse("securegateways.com", JwtOrgOne, http.StatusOK)
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
	s.assertResponse("securegateways.com", JwtOrgOne, http.StatusOK)
	s.assertResponse("securegateways.com", jwtOrgTwo, http.StatusOK)
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
	s.assertResponse("securegateways.com", JwtOrgOne, http.StatusOK)
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
	s.assertResponse("securegateways.com", JwtOrgOne, http.StatusOK)
	// verify a jwt with unexpected subject is denied
	s.assertResponse("securegateways.com", jwtOrgFour, http.StatusForbidden)
}

func (s *testingSuite) assertResponse(hostHeader, authHeader string, expectedStatus int) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: expectedStatus,
		},
		curl.WithHostHeader(hostHeader),
		curl.WithHeader("Authorization", "Bearer "+authHeader),
	)
}

func (s *testingSuite) assertResponseWithoutAuth(hostHeader string, expectedStatus int) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{
			StatusCode: expectedStatus,
		},
		curl.WithHostHeader(hostHeader),
	)
}

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
