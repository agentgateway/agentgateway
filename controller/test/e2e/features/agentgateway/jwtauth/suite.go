//go:build e2e

package jwtauth

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
	// test namespace for proxy resources
	namespace = "agentgateway-base"
	// jwt subject is "ignore@kgateway.dev"
	jwt1 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjUzMzM3ODA2ODc1NTEwMzg2NTkiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6Imlnbm9yZUBrZ2F0ZXdheS5kZXYiLCJleHAiOjIwNzA3MjY3MjgsIm5iZiI6MTc2MzE0MjcyOCwiaWF0IjoxNzYzMTQyNzI4fQ.q88gLzLe6VzRnI0VC4luX7OebX3LW6OLTOOwscGofccnipqfVAi2onHNZt08St5QZ6sTm7kaIc2jLGcr2mey9TjXS5pWiV6wgIN4vZp96-G_2GXcOdTZwWvBQzhnDRLyEKQV-3tU2LTIN_9f5TgQTgZHzXtdhP4Pa3fOSzlM_Rc0ly0sRxkI0JV6WbvhW4OZT6ZT8jbaU5iTRDIf0p1R7mJS6H9g6JMYBf_7LibhiUIosHJCJFgYMEh51JvvEHSBcJrE_Snt37QPznMuK_krtHDszeJvKNs76bSioK6MBdMn7T2GXqkCxy4I46fP4hv6kehQ5abJhXHE8Lwu5NejKg"
	jwt2 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjcxMDU3OTM5NTUwODY5Mzk2NjQiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6Imlnbm9yZUBrZ2F0ZXdheS5kZXYiLCJleHAiOjIwNzA3MjY5NTUsIm5iZiI6MTc2MzE0Mjk1NSwiaWF0IjoxNzYzMTQyOTU1fQ.HmBlsqTSC-ZW1L_pnCB_ix7zorIiyg3X_mD8DiPSaZoKHVCJ-sjmUzffxUzINs4_kzglMWYvOeVsHg1YCASn0_9gBVQ5UvZo1lDZSachuqUGReJ4Bneovjdh18T0FjMJFMy-1K8Bp1RMGlSe4EgBj1lJEA-9h-mFXJv9kC_udD8UJtk-BwJbO9OoUFAbuvaWDdMblVFGKuFSVtZthvyMfFsvgjdkuKBYjeyi9ha1cpWxdV3IOdLjOigdqVkHh9s1Agyki1aVMuleqZUlkOgxaaxzHRjxcIMt7MBB0vQZ9pmItiHMBAyc6u9j4WzaKzgZ58zant48T9vqgci6rcnLBQ"
	jwt3 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjgxNDAyMjc3NTQyNDQ3Mzk0MzEiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6Imlnbm9yZUBrZ2F0ZXdheS5kZXYiLCJleHAiOjIwNzA3MjcwMjYsIm5iZiI6MTc2MzE0MzAyNiwiaWF0IjoxNzYzMTQzMDI2fQ.QNRDPZRFxI_GP1UED3iowjTC8IkuNynvDhALAYb3Rx8kuwaExe6slWNFZwLiBDOEbPJ5-sZfp_aA6l0_KIWBigg0Fsa1eS82Ax9_3YEeFJz6i9vItY4xcXFfL4vTZtmkaNWd1wb2lPsDu6jQsfm6hPTOGk9WHRax2tR8J87sgjQODvCNeZRl3GVH4G-ciDIf-Jo81C_GmoT-UI97ZQ7v7e6GFtsMc1aSyOaiqYGxOvulpTtALy41YQtKO8S07pSGdhuJcJyz-9waZHRe-CSnWsOAsU2Ft7t0X-2rzsGKYn-iASfMNmleUHhqUOLQ1e6JheXBu5VwoPGiZfaHUVXmKA"
	jwt4 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjYxOTk3ODMwNTc3OTA0NDA3NjMiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6Imlnbm9yZUBrZ2F0ZXdheS5kZXYiLCJleHAiOjIwNzA3MzA3NzgsIm5iZiI6MTc2MzE0Njc3OCwiaWF0IjoxNzYzMTQ2Nzc4fQ.TdlC__HT1hZudEo9vyDVsgqOZ3ySYumIc4uiiieLpiG3RrRkfQcl8yQjUSy_HJixM8_i6eejIs06ewdmjqCszs2pJbnHEzrqQRQxRzYl9GXjvMReCLlNxAMl1wXaruFQxr1iNJyb_hyjmRrBhj30uiQRfp2g7gS_KC_7jHeP4VTqnJA_VrHpZJ5yhqPXMTdjS3xbj63fdjvJ6aigPloHotmLwSTCmn90eHxBGFY6r2WVp09hxRMeVPMNF_PvzjSQTR6XDi54NsZ-IFopHZrfusAmSLdng6qiBo9SyYwiphbp5sI24HunYH0SSkEPtV56frAsHmnhkkWGw9S32zPWcw"
	// jwt subject is "boom@kgateway.dev"
	jwt5 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjYxOTk3ODMwNTc3OTA0NDA3NjMiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6ImJvb21Aa2dhdGV3YXkuZGV2IiwiZXhwIjoyMDcwNzMwNzc4LCJuYmYiOjE3NjMxNDY3NzgsImlhdCI6MTc2MzE0Njc3OH0.sEF_3VCJl3Ujn3glbxEO7wspeVjlR3JtJ0PHNMowrRXeKQMO2Oj2EtD2XGluqscRd_weEp-Hn27wA11tN3IJVk0MZJR7Uclwy6rVgMjj2XdoEast5ENDV-MjIsqXzWQrc85Aq2SxzFZJnFtOO8np4N3OIiSngBVnsfXhSU139xo-EChKxlP4uWL9PDguN22ayj1p1TG8fujLz5TUVYhco_YH6pXDPO6WlX1IR9YGCfwCr6jDw6BAUX5QpJmlU_TuUqIHbJo378OvG-6d6qRkzOoil1tJ_c7Ils3txdK_7MrAWNoU3tpnqdDiPwHGT6HknVLtOmCWsNe7aESRg2AhXw"
)

var (
	setup = base.TestCase{}

	testCases = map[string]*base.TestCase{
		"TestRoutePolicy": {
			Manifests: []string{insecureRouteManifest, secureRoutePolicyManifest},
		},
		"TestRoutePolicyWithRbac": {
			Manifests: []string{secureRoutePolicyWithRbacManifest},
		},
		"TestGatewayPolicy": {
			Manifests: []string{secureGWPolicyManifest},
		},
		"TestGatewayPolicyWithRbac": {
			Manifests: []string{secureGWPolicyWithRbacManifest},
		},
	}
)

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

var (
	insecureRouteManifest             = getTestFile("insecure-route.yaml")
	secureGWPolicyManifest            = getTestFile("secured-gateway-policy.yaml")
	secureGWPolicyWithRbacManifest    = getTestFile("secured-gateway-policy-with-rbac.yaml")
	secureRoutePolicyManifest         = getTestFile("secured-route.yaml")
	secureRoutePolicyWithRbacManifest = getTestFile("secured-route-with-rbac.yaml")
)

func (s *testingSuite) TestRoutePolicy() {
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
	s.assertResponse("secureroute.com", jwt1, http.StatusOK)
	// verify a provider with multiple keys in jwks works
	s.assertResponse("secureroute.com", jwt2, http.StatusOK)
	s.assertResponse("secureroute.com", jwt3, http.StatusOK)
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
	// jwt subject matches rbac policy
	s.assertResponse("secureroute.com", jwt4, http.StatusOK)
	// jwt subject doesn't match rbac policy
	s.assertResponse("secureroute.com", jwt5, http.StatusForbidden)
}

func (s *testingSuite) TestGatewayPolicy() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		"route-secure-gw",
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	// verify a provider with a single key in jwks works
	s.assertResponse("securegateways.com", jwt1, http.StatusOK)
	// verify a provider with multiple keys in jwks works
	s.assertResponse("securegateways.com", jwt2, http.StatusOK)
	s.assertResponse("securegateways.com", jwt3, http.StatusOK)
	s.assertResponse("securegateways.com", "nosuchkey", http.StatusUnauthorized)
	// verify invalid/missing tokens are caught
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
	// jwt subject matches rbac policy
	s.assertResponse("securegateways.com", jwt4, http.StatusOK)
	// jwt subject doesn't match rbac policy
	s.assertResponse("securegateways.com", jwt5, http.StatusForbidden)
}

func (s *testingSuite) assertResponse(hostHeader, authHeader string, expectedStatus int) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: expectedStatus},
		curl.WithHostHeader(hostHeader),
		curl.WithHeader("Authorization", "Bearer "+authHeader),
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
