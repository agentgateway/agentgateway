//go:build e2e

package jwtauth

import (
	"context"
	"net/http"
	"path/filepath"

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

//
// Use `go run hack/utils/jwt/jwt-generator.go`
// to generate jwks and a jwt signed by the key in it
//

var _ e2e.NewSuiteFunc = NewTestingSuite

const (
	// test namespace for proxy resources
	namespace = "agentgateway-base"
	// jwt subject is "ignore@agentgateway.dev"
	jwt1 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjUzNTg0ODg0MTQ2NzkzMTE2NDQiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA4NTMxNzExNSwibmJmIjoxNzc3NzMzMTE1LCJpYXQiOjE3Nzc3MzMxMTV9.dCvD5WQYRYTcHlULa9WisRTxJYTYINbJGX_QCk9x_nA6NcDETxtYXpFe6zivWkBzkEDLby9U0JfcrdeuNc2fVWlm1VjWSzFBCdf15xQBTmqfblC1Fd_0KsW17lUA01lq-p4yomV4XGPLYWTx9TiQ2zOrQSmKkIWzWRouI-eTWBpnkP6x3cQkjXWZPgZoCRyxkOXXyJTkGP5JxlaeJb3J_v94i53ZYt9jDC2gXN5HZz7IZB-IWaZSlBbCgaAl3EJtg06npQZQtlYs-QkacmA9MZMYTTZS5xB3AaqVWltEau9zbJnkqpzVH1DmsOwvT-hiJVXZoqfGHw7vvMFrbQbK-g"
	jwt2 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjExNzA4NjQ1NDE4MzI5NzA3ODkiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA4NTMxNzExNCwibmJmIjoxNzc3NzMzMTE0LCJpYXQiOjE3Nzc3MzMxMTR9.n1nH82Kcn3uCnnFUcol5e0yNM5M9jZijjZtPtjtJQiuVRqB6nHGeFLLCEtjbpgzYjK_Saxyv91aCFHNkbin0dHJOFf9HaxdmH_DrAycZtbUp8Runj8VoZeOUtlU7qvutbi7vKRO_I11EoNOjpA4PIi9IJouEgdjKeP9eZTt4TDrfYKME8DXa-OqvrHYRqgntjg7_i_6k23qhlTO1GFCXRWNc9pmMSSFML_nt0xpUxIHJ8SifvPrujtQ3NIB4iEM9d4XTNk7-sCfHPAyk5tFFZTO_mxOiNthxbqB1jeyS_ZHGhTDEJ9ww78yqpkc4sxwT-2NEPcgSUCQ_k_PMMxpd9g"
	jwt3 = "eyJhbGciOiJSUzI1NiIsImtpZCI6Ijc5NzEzODI2NzkxNzg1NDk2MjAiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA4NTMxNzExNCwibmJmIjoxNzc3NzMzMTE0LCJpYXQiOjE3Nzc3MzMxMTR9.SPuJpi6W_UM-cUWDYw3AcIGRGIGSjjogeqWzf-_rrHZ7FsOY4566FmKaqxai0T3a6z4TYj30qIItgftQEVXrFxXVkMLLN7PoPSmiqp2T8FOmPZODOKio_IVwfOPlc99I9y0_cGsyEOsilxm1qje0gRovqUyVd3wWnsoknf3YWLbBWwNCWawteumDBAN4A7CVncDXKNNjk_uXdUwO-ah_Cwao-nLdU2GPiVGtP-V3_5ClK-khWvk8qthEuTOkZ0jeRTcMNQKHkTONALqLsnXEhZOOFjQ8d-ueTk2tYduSqJ8uiiF9Uvzz-tNVrC1-nvXcpKb0Ob3YnMH1VycK1invNA"
	jwt4 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjcwNzMwMjQzNTI5MTkzMjkwOTQiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA4NTMxNzExNSwibmJmIjoxNzc3NzMzMTE1LCJpYXQiOjE3Nzc3MzMxMTV9.BZqclslF020OmjLY8ZmLhtx-LCqwUxn1Wsdq7SeUtzZ7NI64MwH37Bxd2z9AGSVOhliBB8otcRdiWRHMhHfaKu4l9NDYpsmFWIYViiuZQd4OUPtS5d2NmRXAl4noZ5EzmtMHrTYhv1wBB8bWQGs20mimTjdcdJbmzHcEqmNMMHxX93Wk25xAn4habR8b8Z2HlxlU-MZj40gL_iPsH088e8gf-Qb4JCqrQc4_UI8EpsO4vWk42gwJGU9ZLDFDt6mWs88OWMgs0c0DB82lX5xyVZtmFyVmq1p7mW9Ez9olUg64iOBIhdnv7560Ilc6_9AwJ9zU2fcDGaBP0ZaF1vxOsg"
	// jwt subject is "boom@agentgateway.dev"
	jwt5 = "eyJhbGciOiJSUzI1NiIsImtpZCI6IjcwNzMwMjQzNTI5MTkzMjkwOTQiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJib29tQGFnZW50Z2F0ZXdheS5kZXYiLCJleHAiOjIwODUzMTcxMTUsIm5iZiI6MTc3NzczMzExNSwiaWF0IjoxNzc3NzMzMTE1fQ.MS9PaXb81m8tBEs1qtTBD6LSD8lTYJuP2ygvmrzwnwiYLb7-QbLJUwtxwCSxu6icwOU50OHQiFsyLnYnmpACvJ0Nc3co_a2q4lThUNuUyLxwxqJWRRFiFqF78hv3E3A3Nrdpuvk5qF4M8yqusPcpOd6dhAwwlSoEM8_2q5__PuNNFIx6Z37LS507rKcmYfk7kCvpBbddi5n9tyYcHpvZEckPhNdWn_E7yyEi_WrIhAq1OcgrwbS2JFrLoeUap2FrpSVvkk-dfRzR2QreTehc4WihFCPTPc0edhHeb0AW8wfsyjSQvq4DkXw_SWMdonRWqxQYqnYiDv1v49bC-ro6Xg"
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
	// against an installation of agentgateway
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
