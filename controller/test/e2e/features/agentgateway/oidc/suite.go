//go:build e2e

package oidc

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"path/filepath"
	"strings"
	"time"

	"github.com/stretchr/testify/suite"
	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/retry"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	ctrlclient "sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	agentoidc "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/oidc"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	"github.com/agentgateway/agentgateway/controller/test/e2e/common"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

const (
	namespace       = "agentgateway-base"
	systemNamespace = "agentgateway-system"
	// issuerURL matches what the dummy IdP serves at its HTTP endpoint.
	// HTTP is used because the controller has no trust path to the dummy IdP's
	// self-signed TLS cert; the IdP serves both HTTP (8081) and HTTPS (8443).
	issuerURL   = "http://dummy-idp.default:8081"
	routeHost   = "oidcroute.com"
	gatewayHost = "oidcgateway.com"
	clientID    = "mcp_gi3APARn2_uHv2oxfJJqq2yZBDV4OyNo"
)

var (
	setup = base.TestCase{
		Manifests: []string{
			getTestFile("common.yaml"),
		},
	}

	testCases = map[string]*base.TestCase{
		"TestRoutePolicy": {
			Manifests: []string{secureRoutePolicyManifest},
		},
		"TestGatewayPolicy": {
			Manifests: []string{secureGatewayPolicyManifest},
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
	secureRoutePolicyManifest   = getTestFile("secured-route.yaml")
	secureGatewayPolicyManifest = getTestFile("secured-gateway-policy.yaml")
)

func (s *testingSuite) TestRoutePolicy() {
	s.assertOIDCFlow("route-oidc", "route-policy", routeHost)
}

func (s *testingSuite) TestGatewayPolicy() {
	s.assertOIDCFlow("route-oidc-gw", "gw-policy", gatewayHost)
}

// assertOIDCFlow exercises the full OIDC authorization-code redirect flow against
// the given gateway host. It verifies:
//  1. The controller has cached the provider discovery document.
//  2. An unauthenticated request is redirected to the IdP /authorize endpoint.
//  3. The IdP callback (/oauth/callback) exchanges the code and sets a session cookie.
//  4. A subsequent request with the session cookie reaches the backend (200 OK).
func (s *testingSuite) assertOIDCFlow(routeName, policyName, host string) {
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx,
		routeName,
		namespace,
		gwv1.RouteConditionAccepted,
		metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyAgwPolicyCondition(
		s.Ctx,
		policyName,
		namespace,
		"Accepted",
		metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyAgwPolicyCondition(
		s.Ctx,
		policyName,
		namespace,
		"Attached",
		metav1.ConditionTrue,
	)

	s.eventuallyAssertProviderConfigCached()

	originalPath := "/private?source=oidc-e2e"
	login := s.eventuallySendGatewayRequest(host, originalPath)
	defer login.Body.Close()

	assert.Equal(s.T(), http.StatusFound, login.StatusCode)
	loginLocation := login.Header.Get("Location")
	assert.Equal(s.T(), true, loginLocation != "")

	loginURL, err := url.Parse(loginLocation)
	assert.NoError(s.T(), err)
	assert.Equal(s.T(), "/authorize", loginURL.Path)
	assert.Equal(s.T(), clientID, loginURL.Query().Get("client_id"))
	assert.Equal(s.T(), true, strings.Contains(loginURL.Query().Get("scope"), "openid"))

	state := loginURL.Query().Get("state")
	nonce := loginURL.Query().Get("nonce")
	assert.Equal(s.T(), true, state != "")
	assert.Equal(s.T(), true, nonce != "")

	redirectURIParsed, err := url.Parse(loginURL.Query().Get("redirect_uri"))
	assert.NoError(s.T(), err)
	assert.Equal(s.T(), "/oauth/callback", redirectURIParsed.Path)

	transactionCookie := findCookieByPrefix(login.Cookies(), "agw_oidc_t_")
	if transactionCookie == nil {
		s.T().Fatal("expected transaction cookie")
	}

	callbackPath := fmt.Sprintf(
		"%s?code=%s&state=%s",
		redirectURIParsed.Path,
		url.QueryEscape(authorizationCodeForNonce(nonce)),
		url.QueryEscape(state),
	)
	callback := s.sendGatewayRequest(host, callbackPath, transactionCookie)
	defer callback.Body.Close()

	assert.NoError(s.T(), err)
	assert.Equal(s.T(), http.StatusFound, callback.StatusCode)
	assert.Equal(s.T(), originalPath, callback.Header.Get("Location"))

	sessionCookie := findCookieByPrefix(callback.Cookies(), "agw_oidc_s_")
	if sessionCookie == nil {
		s.T().Fatal("expected session cookie")
	}
	assert.Equal(s.T(), true, hasClearedCookie(callback.Header.Values("Set-Cookie"), transactionCookie.Name))

	final := s.sendGatewayRequest(host, originalPath, sessionCookie)
	defer final.Body.Close()

	assert.Equal(s.T(), http.StatusOK, final.StatusCode)
}

// eventuallyAssertProviderConfigCached polls the system namespace for a ConfigMap
// containing the cached OIDC discovery document for our test issuer.
func (s *testingSuite) eventuallyAssertProviderConfigCached() {
	retry.UntilSuccessOrFail(s.T(), func() error {
		var cms corev1.ConfigMapList
		if err := s.TestInstallation.ClusterContext.Client.List(
			s.Ctx,
			&cms,
			ctrlclient.InNamespace(systemNamespace),
			ctrlclient.MatchingLabels(agentoidc.OidcStoreConfigMapLabel(agentoidc.DefaultStorePrefix)),
		); err != nil {
			return err
		}
		for i := range cms.Items {
			provider, err := agentoidc.ProviderFromConfigMap(&cms.Items[i])
			if err != nil {
				continue
			}
			if provider.IssuerURL == issuerURL &&
				provider.AuthorizationEndpoint != "" &&
				provider.TokenEndpoint != "" &&
				provider.JwksJSON != "" {
				return nil
			}
		}
		return fmt.Errorf("no matching OIDC provider ConfigMap cached yet")
	}, retry.Timeout(30*time.Second))
}

// eventuallySendGatewayRequest retries until the gateway issues a 302 redirect
// with an OIDC transaction cookie — i.e. the OIDC filter is active and the
// provider config has been pushed to the dataplane.
func (s *testingSuite) eventuallySendGatewayRequest(hostHeader, requestURI string) *http.Response {
	var response *http.Response
	retry.UntilSuccessOrFail(s.T(), func() error {
		resp := s.sendGatewayRequest(hostHeader, requestURI)
		if resp.StatusCode != http.StatusFound {
			resp.Body.Close()
			return fmt.Errorf("expected 302, got %d", resp.StatusCode)
		}
		if findCookieByPrefix(resp.Cookies(), "agw_oidc_t_") == nil {
			resp.Body.Close()
			return fmt.Errorf("missing oidc transaction cookie")
		}
		response = resp
		return nil
	}, retry.Timeout(30*time.Second))
	return response
}

// sendGatewayRequest sends a single non-redirecting HTTP GET to the gateway
// with the given Host header, optional cookies, and returns the raw response.
func (s *testingSuite) sendGatewayRequest(hostHeader, requestURI string, cookies ...*http.Cookie) *http.Response {
	addr := common.BaseGateway.ResolvedAddress()
	if _, _, err := net.SplitHostPort(addr); err != nil {
		addr = net.JoinHostPort(addr, "80")
	}

	req, err := http.NewRequestWithContext(s.Ctx, http.MethodGet, "http://"+addr+requestURI, nil)
	assert.NoError(s.T(), err)
	req.Host = hostHeader
	for _, cookie := range cookies {
		req.AddCookie(cookie)
	}

	client := &http.Client{
		Timeout: 10 * time.Second,
		CheckRedirect: func(*http.Request, []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}
	resp, err := client.Do(req)
	assert.NoError(s.T(), err)
	return resp
}

func findCookieByPrefix(cookies []*http.Cookie, prefix string) *http.Cookie {
	for _, cookie := range cookies {
		if strings.HasPrefix(cookie.Name, prefix) {
			return cookie
		}
	}
	return nil
}

func hasClearedCookie(setCookies []string, cookieName string) bool {
	for _, value := range setCookies {
		if strings.HasPrefix(value, cookieName+"=") && strings.Contains(value, "Max-Age=0") {
			return true
		}
	}
	return false
}

// authorizationCodeForNonce produces the authorization code that the dummy IdP
// will accept for the given nonce.  The dummy IdP's /token handler validates
// that the code is "fixed_auth_code_123.<nonce>" when a nonce is present.
func authorizationCodeForNonce(nonce string) string {
	if nonce == "" {
		return "fixed_auth_code_123"
	}
	return "fixed_auth_code_123." + nonce
}

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}
