//go:build e2e

package oauth2auth

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/url"
	"path/filepath"
	"strings"

	"github.com/onsi/gomega"
	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils/portforward"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	testmatchers "github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

const (
	namespace            = "agentgateway-base"
	browserProtectedPath = "/?source=browser"
	dummyIDPServiceName  = "dummy-idp"
	dummyIDPServiceNS    = "default"
	dummyIDPServiceHTTPS = 8443
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

type authorizeResponse struct {
	RedirectTo string `json:"redirect_to"`
}

type capturedHTTPResponse struct {
	StatusCode int
	Header     http.Header
}

func newCapturedHTTPResponse(resp *http.Response) capturedHTTPResponse {
	if resp == nil {
		return capturedHTTPResponse{}
	}
	return capturedHTTPResponse{
		StatusCode: resp.StatusCode,
		Header:     resp.Header.Clone(),
	}
}

func (r capturedHTTPResponse) Cookies() []*http.Cookie {
	return (&http.Response{Header: r.Header}).Cookies()
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
	s.assertBrowserSessionFlow("oauth2route.com")
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
	s.assertBrowserSessionFlow("oauth2gateway.com")
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

func (s *testingSuite) assertBrowserSessionFlow(hostHeader string) {
	redirectResp, _ := s.execGatewayRequest(
		hostHeader,
		browserProtectedPath,
		map[string]string{"Accept": "text/html"},
		"",
	)
	s.Require().Equal(http.StatusFound, redirectResp.StatusCode)

	authorizeLocation := redirectResp.Header.Get("Location")
	s.Require().NotEmpty(authorizeLocation)

	authorizeURL, err := url.Parse(authorizeLocation)
	s.Require().NoError(err)
	s.Require().Equal("/authorize", authorizeURL.Path)
	s.Require().Equal("https", authorizeURL.Scheme)
	s.Require().Equal("https://"+hostHeader+"/_gateway/callback", authorizeURL.Query().Get("redirect_uri"))

	state := authorizeURL.Query().Get("state")
	s.Require().NotEmpty(state)

	handshakeCookies := cookieHeaderFromResponse(redirectResp)
	s.Require().NotEmpty(handshakeCookies)

	idpResp := s.execDummyIDPAuthorize(authorizeURL)
	callbackURL, err := url.Parse(idpResp.RedirectTo)
	s.Require().NoError(err)
	s.Require().Equal(hostHeader, callbackURL.Host)
	s.Require().Equal("/_gateway/callback", callbackURL.Path)
	s.Require().Equal(state, callbackURL.Query().Get("state"))
	s.Require().NotEmpty(callbackURL.Query().Get("code"))

	callbackResp, _ := s.execGatewayRequest(hostHeader, callbackURL.RequestURI(), nil, handshakeCookies)
	s.Require().Equal(http.StatusFound, callbackResp.StatusCode)
	s.Require().Equal(browserProtectedPath, callbackResp.Header.Get("Location"))

	sessionCookies := cookieHeaderFromResponse(callbackResp)
	s.Require().NotEmpty(sessionCookies)

	finalResp, _ := s.execGatewayRequest(hostHeader, browserProtectedPath, nil, sessionCookies)
	s.Require().Equal(http.StatusOK, finalResp.StatusCode)
}

func (s *testingSuite) assertResponseWithoutAuth(hostHeader string, expectedStatus int) {
	common.BaseGateway.Send(
		s.T(),
		&testmatchers.HttpResponse{StatusCode: expectedStatus},
		curl.WithHostHeader(hostHeader),
	)
}

func (s *testingSuite) execGatewayRequest(
	hostHeader string,
	path string,
	headers map[string]string,
	cookieHeader string,
) (capturedHTTPResponse, string) {
	opts := append(
		common.GatewayAddressOptions(common.BaseGateway.ResolvedAddress()),
		curl.WithPath(path),
		curl.WithConnectionTimeout(10),
	)
	if hostHeader != "" {
		opts = append(opts, curl.WithHostHeader(hostHeader))
	}
	for key, value := range headers {
		opts = append(opts, curl.WithHeader(key, value))
	}
	if cookieHeader != "" {
		opts = append(opts, curl.WithHeader("Cookie", cookieHeader))
	}
	return s.execRequest(opts...)
}

func (s *testingSuite) execDummyIDPAuthorize(authorizeURL *url.URL) authorizeResponse {
	forwarder, err := s.TestInstallation.Actions.Kubectl().StartPortForward(
		s.Ctx,
		portforward.WithService(dummyIDPServiceName, dummyIDPServiceNS),
		portforward.WithRemotePort(dummyIDPServiceHTTPS),
	)
	s.Require().NoError(err)
	defer forwarder.Close()

	resp, body := s.execRequest(
		curl.WithHostPort(forwarder.Address()),
		curl.WithScheme("https"),
		curl.WithInsecureSkipVerify(),
		curl.WithConnectionTimeout(10),
		curl.WithPath(authorizeURL.RequestURI()),
	)
	s.Require().Equal(http.StatusOK, resp.StatusCode)

	var parsed authorizeResponse
	err = json.Unmarshal([]byte(body), &parsed)
	s.Require().NoError(err)
	s.Require().NotEmpty(parsed.RedirectTo)
	return parsed
}

func (s *testingSuite) execRequest(opts ...curl.Option) (capturedHTTPResponse, string) {
	resp, err := curl.ExecuteRequest(opts...)
	s.Require().NoError(err)
	defer resp.Body.Close()

	bodyBytes, err := io.ReadAll(resp.Body)
	s.Require().NoError(err)

	body := string(bodyBytes)
	s.T().Logf("http response status=%d location=%q body=%s", resp.StatusCode, resp.Header.Get("Location"), body)
	return newCapturedHTTPResponse(resp), body
}

func cookieHeaderFromResponse(resp capturedHTTPResponse) string {
	if resp.Header == nil {
		return ""
	}

	cookies := make([]string, 0, len(resp.Cookies()))
	for _, cookie := range resp.Cookies() {
		if cookie.Value == "" {
			continue
		}
		cookies = append(cookies, cookie.Name+"="+cookie.Value)
	}
	return strings.Join(cookies, "; ")
}

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
