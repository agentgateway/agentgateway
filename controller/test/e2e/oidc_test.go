//go:build e2e

package e2e_test

import (
	"fmt"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/retry"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	ctrlclient "sigs.k8s.io/controller-runtime/pkg/client"

	agentoidc "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/oidc"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
	"github.com/agentgateway/agentgateway/controller/test/testutils/testoidc"
)

const (
	oidcSystemNamespace = "agentgateway-system"
	oidcIssuerURL       = "https://dummy-idp.default.svc.cluster.local:8443"
	oidcClientID        = testoidc.ClientID
)

// TestOidcTestdataPinsSharedClientID pins every clientID literal in the OIDC
// testdata manifests to the shared testoidc fixture; YAML cannot reference
// the Go constant, so this guard is what keeps them from drifting.
func TestOidcTestdataPinsSharedClientID(t *testing.T) {
	manifests, err := filepath.Glob(manifest("oidc", "*.yaml"))
	if err != nil || len(manifests) == 0 {
		t.Fatalf("globbing oidc testdata manifests: %v (%d found)", err, len(manifests))
	}
	found := 0
	for _, path := range manifests {
		raw, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		for line := range strings.SplitSeq(string(raw), "\n") {
			if !strings.Contains(line, "clientID:") {
				continue
			}
			found++
			if want := "clientID: " + testoidc.ClientID; strings.TrimSpace(line) != want {
				t.Errorf("%s: %q drifted from the shared fixture (want %q)", filepath.Base(path), strings.TrimSpace(line), want)
			}
		}
	}
	if found == 0 {
		t.Fatal("no clientID lines found in oidc testdata; guard is vacuous")
	}
}

func TestOidc(tt *testing.T) {
	t := New(tt)
	t.Apply(manifest("oidc", "common.yaml"))

	t.Run("RoutePolicy", func(t base.Test) {
		t.Apply(manifest("oidc", "secured-route.yaml"))
		assertOIDCFlow(t, "route-oidc", "route-policy", "oidcroute.com")
	})
	t.Run("GatewayPolicy", func(t base.Test) {
		t.Apply(manifest("oidc", "secured-gateway-policy.yaml"))
		assertOIDCFlow(t, "route-oidc-gw", "gw-policy", "oidcgateway.com")
	})
}

// assertOIDCFlow exercises the authorization-code redirect flow:
// unauthenticated → 302 to /authorize → callback exchanges code for session →
// original path with session cookie returns 200.
func assertOIDCFlow(t base.Test, routeName, policyName, host string) {
	t.HTTPRouteAccepted(routeName, base.Namespace)
	assertions.EventuallyAgwPolicyCondition(t, policyName, base.Namespace, "Accepted", metav1.ConditionTrue)
	assertions.EventuallyAgwPolicyCondition(t, policyName, base.Namespace, "Attached", metav1.ConditionTrue)
	eventuallyAssertOIDCProviderCached(t)

	originalPath := "/private?source=oidc-e2e"
	login := eventuallySendOIDCLogin(t, host, originalPath)
	defer login.Body.Close()

	loginURL, err := url.Parse(login.Header.Get("Location"))
	assert.NoError(t, err)
	assert.Equal(t, "/authorize", loginURL.Path)
	assert.Equal(t, oidcClientID, loginURL.Query().Get("client_id"))
	if !strings.Contains(loginURL.Query().Get("scope"), "openid") {
		t.Fatalf("expected openid in scope, got %q", loginURL.Query().Get("scope"))
	}
	state, nonce := loginURL.Query().Get("state"), loginURL.Query().Get("nonce")
	if state == "" || nonce == "" {
		t.Fatal("expected non-empty state and nonce")
	}

	redirectURI, err := url.Parse(loginURL.Query().Get("redirect_uri"))
	assert.NoError(t, err)
	assert.Equal(t, "/oauth/callback", redirectURI.Path)

	transactionCookie := findOIDCCookie(login.Cookies(), "agw_oidc_t_")
	if transactionCookie == nil {
		t.Fatal("expected transaction cookie")
	}

	callbackPath := fmt.Sprintf("%s?code=%s&state=%s",
		redirectURI.Path,
		url.QueryEscape(testoidc.AuthorizationCodeForNonce(nonce)),
		url.QueryEscape(state))
	callback := sendOIDCRequest(t, host, callbackPath, transactionCookie)
	defer callback.Body.Close()

	assert.Equal(t, http.StatusFound, callback.StatusCode)
	assert.Equal(t, originalPath, callback.Header.Get("Location"))

	sessionCookie := findOIDCCookie(callback.Cookies(), "agw_oidc_s_")
	if sessionCookie == nil {
		t.Fatal("expected session cookie")
	}
	if !cookieIsCleared(callback.Header.Values("Set-Cookie"), transactionCookie.Name) {
		t.Fatalf("expected cleared transaction cookie, got %v", callback.Header.Values("Set-Cookie"))
	}

	final := sendOIDCRequest(t, host, originalPath, sessionCookie)
	defer final.Body.Close()
	assert.Equal(t, http.StatusOK, final.StatusCode)
}

func eventuallyAssertOIDCProviderCached(t base.Test) {
	retry.UntilSuccessOrFail(t, func() error {
		var cms corev1.ConfigMapList
		if err := t.TestInstallation.ClusterContext.ControllerClient.List(t.Ctx, &cms,
			ctrlclient.InNamespace(oidcSystemNamespace),
			ctrlclient.MatchingLabels(remotecache.ConfigMapLabels(agentoidc.DefaultStorePrefix)),
		); err != nil {
			return err
		}
		for i := range cms.Items {
			p, err := agentoidc.ProviderFromConfigMap(&cms.Items[i])
			if err == nil && p.IssuerURL == oidcIssuerURL && p.JwksInline != "" {
				return nil
			}
		}
		return fmt.Errorf("no matching OIDC provider ConfigMap cached yet")
	}, retry.Timeout(30*time.Second))
}

// eventuallySendOIDCLogin retries the initial unauthenticated GET until the
// gateway responds 302 with an OIDC transaction cookie (filter active and
// provider config has reached the dataplane).
func eventuallySendOIDCLogin(t base.Test, host, requestURI string) *http.Response {
	var response *http.Response
	retry.UntilSuccessOrFail(t, func() error {
		resp := sendOIDCRequest(t, host, requestURI)
		if resp.StatusCode != http.StatusFound || findOIDCCookie(resp.Cookies(), "agw_oidc_t_") == nil {
			resp.Body.Close()
			return fmt.Errorf("expected 302 with oidc transaction cookie, got %d", resp.StatusCode)
		}
		response = resp
		return nil
	}, retry.Timeout(30*time.Second))
	return response
}

// sendOIDCRequest does a single non-redirect-following GET through the
// gateway. Curl-based base.Test.Send follows redirects, which would consume
// the OIDC 302 we need to inspect.
func sendOIDCRequest(t base.Test, host, requestURI string, cookies ...*http.Cookie) *http.Response {
	t.Helper()
	addr := base.BaseGateway.Address
	if _, _, err := net.SplitHostPort(addr); err != nil {
		addr = net.JoinHostPort(addr, "80")
	}
	req, err := http.NewRequestWithContext(t.Ctx, http.MethodGet, "http://"+addr+requestURI, nil)
	assert.NoError(t, err)
	req.Host = host
	for _, c := range cookies {
		req.AddCookie(c)
	}
	client := &http.Client{
		Timeout:       10 * time.Second,
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	}
	resp, err := client.Do(req)
	assert.NoError(t, err)
	return resp
}

func findOIDCCookie(cookies []*http.Cookie, prefix string) *http.Cookie {
	for _, c := range cookies {
		if strings.HasPrefix(c.Name, prefix) {
			return c
		}
	}
	return nil
}

func cookieIsCleared(setCookies []string, name string) bool {
	return slices.ContainsFunc(setCookies, func(v string) bool {
		return strings.HasPrefix(v, name+"=") && strings.Contains(v, "Max-Age=0")
	})
}
