package plugins

import (
	"errors"
	"strings"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	oidcpkg "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/oidc"
)

// stubOIDCLookup is a test-only fake for oidcpkg.Lookup.
type stubOIDCLookup struct {
	provider *oidcpkg.DiscoveredProvider
	err      error
}

func (s stubOIDCLookup) ResolveForOwner(krt.HandlerContext, oidcpkg.RemoteOidcOwner) (*oidcpkg.DiscoveredProvider, error) {
	return s.provider, s.err
}

// makeOIDC returns a public-client OIDC config (no ClientSecret). The plugin
// tests below that use this fixture intentionally exercise the public-client
// path, which requires no Kubernetes Secret and therefore no harness setup.
// Confidential-client behavior (Secret resolution, Basic/Post auth) is
// exercised in TestResolveOIDCClientSecretReadsDataKey and in agent_xds tests.
func makeOIDC() *agentgateway.OIDC {
	return &agentgateway.OIDC{
		IssuerURL:   "https://idp.example.com",
		ClientID:    "my-client",
		RedirectURI: "https://app.example.com/callback",
		Scopes:      []string{"openid", "email"},
	}
}

func makeDiscoveredProvider() *oidcpkg.DiscoveredProvider {
	return &oidcpkg.DiscoveredProvider{
		IssuerURL:                         "https://idp.example.com",
		AuthorizationEndpoint:             "https://idp.example.com/authorize",
		TokenEndpoint:                     "https://idp.example.com/token",
		JwksJSON:                          `{"keys":[]}`,
		TokenEndpointAuthMethodsSupported: []string{"none"},
	}
}

func TestProcessOIDCPolicyHappyPath(t *testing.T) {
	oidcCfg := makeOIDC()
	provider := makeDiscoveredProvider()
	policyKey := types.NamespacedName{Namespace: "default", Name: "my-policy"}

	result, err := processOIDCPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		oidcCfg,
		nil,
		"traffic/default/my-policy",
		policyKey,
		stubOIDCLookup{provider: provider},
	)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result == nil {
		t.Fatal("expected non-nil policy")
	}

	oidcSpec := result.GetTraffic().GetOidc()
	if oidcSpec == nil {
		t.Fatal("expected oidc spec in policy")
	}
	if oidcSpec.Issuer != provider.IssuerURL {
		t.Errorf("issuer: got %q, want %q", oidcSpec.Issuer, provider.IssuerURL)
	}
	if oidcSpec.AuthorizationEndpoint != provider.AuthorizationEndpoint {
		t.Errorf("authorization_endpoint: got %q, want %q", oidcSpec.AuthorizationEndpoint, provider.AuthorizationEndpoint)
	}
	if oidcSpec.TokenEndpoint != provider.TokenEndpoint {
		t.Errorf("token_endpoint: got %q, want %q", oidcSpec.TokenEndpoint, provider.TokenEndpoint)
	}
	if oidcSpec.JwksInline != provider.JwksJSON {
		t.Errorf("jwks_inline: got %q, want %q", oidcSpec.JwksInline, provider.JwksJSON)
	}
	if oidcSpec.ClientId != oidcCfg.ClientID {
		t.Errorf("client_id: got %q, want %q", oidcSpec.ClientId, oidcCfg.ClientID)
	}
	if oidcSpec.RedirectUri != oidcCfg.RedirectURI {
		t.Errorf("redirect_uri: got %q, want %q", oidcSpec.RedirectUri, oidcCfg.RedirectURI)
	}
	if len(oidcSpec.Scopes) != 2 || oidcSpec.Scopes[0] != "openid" || oidcSpec.Scopes[1] != "email" {
		t.Errorf("scopes: got %v, want [openid email]", oidcSpec.Scopes)
	}
	// Public-client fixture: no ClientSecret and the IdP advertises 'none',
	// so the plugin must emit NONE and leave client_secret empty on the wire.
	if oidcSpec.TokenEndpointAuth != api.TrafficPolicySpec_OIDC_NONE {
		t.Errorf("token_endpoint_auth: got %v, want %v", oidcSpec.TokenEndpointAuth, api.TrafficPolicySpec_OIDC_NONE)
	}
	if oidcSpec.ClientSecret != "" {
		t.Errorf("client_secret must be empty for a public client, got %q", oidcSpec.ClientSecret)
	}
}

func TestProcessOIDCPolicyPolicyIDForAgentgatewayPolicy(t *testing.T) {
	policyKey := types.NamespacedName{Namespace: "prod", Name: "auth-policy"}

	result, err := processOIDCPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		makeOIDC(),
		nil,
		"traffic/prod/auth-policy",
		policyKey,
		stubOIDCLookup{provider: makeDiscoveredProvider()},
	)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	want := "policy/prod/auth-policy"
	got := result.GetTraffic().GetOidc().GetPolicyId()
	if got != want {
		t.Errorf("policy_id: got %q, want %q", got, want)
	}
}

func TestProcessOIDCPolicyKeyUsesOIDCSuffix(t *testing.T) {
	policyKey := types.NamespacedName{Namespace: "default", Name: "my-policy"}

	result, err := processOIDCPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		makeOIDC(),
		nil,
		"traffic/default/my-policy",
		policyKey,
		stubOIDCLookup{provider: makeDiscoveredProvider()},
	)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	wantKey := "traffic/default/my-policy" + oidcPolicySuffix
	if result.Key != wantKey {
		t.Errorf("policy key: got %q, want %q", result.Key, wantKey)
	}
}

func TestProcessOIDCPolicyPublicClientRequiresNoneAdvertised(t *testing.T) {
	// Public client (no clientSecret) against an IdP that advertises only
	// confidential methods must fail with a user-facing error directing the
	// operator to register the client as public.
	oidcCfg := makeOIDC()
	provider := makeDiscoveredProvider()
	provider.TokenEndpointAuthMethodsSupported = []string{"client_secret_post"}

	_, err := processOIDCPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		oidcCfg,
		nil,
		"traffic/default/my-policy",
		types.NamespacedName{Namespace: "default", Name: "my-policy"},
		stubOIDCLookup{provider: provider},
	)
	if err == nil {
		t.Fatal("expected an error when no clientSecret and IdP does not advertise 'none'")
	}
	if !contains(err.Error(), "register the client as public") {
		t.Errorf("expected user-facing guidance in error, got: %v", err)
	}
}

func TestProcessOIDCPolicyExplicitNoneOverridesDiscovery(t *testing.T) {
	// User-supplied tokenEndpointAuthMethod=None wins even when the IdP
	// advertises other methods, because the user has explicitly opted the
	// client into public-client mode.
	oidcCfg := makeOIDC()
	override := oidcConfigTokenEndpointAuthMethodNone
	oidcCfg.TokenEndpointAuthMethod = &override
	provider := makeDiscoveredProvider()
	provider.TokenEndpointAuthMethodsSupported = []string{"client_secret_post"}

	result, err := processOIDCPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		oidcCfg,
		nil,
		"traffic/default/my-policy",
		types.NamespacedName{Namespace: "default", Name: "my-policy"},
		stubOIDCLookup{provider: provider},
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if got := result.GetTraffic().GetOidc().GetTokenEndpointAuth(); got != api.TrafficPolicySpec_OIDC_NONE {
		t.Errorf("token_endpoint_auth: got %v, want %v", got, api.TrafficPolicySpec_OIDC_NONE)
	}
}

func contains(haystack, needle string) bool {
	return strings.Contains(haystack, needle)
}

func TestProcessOIDCPolicyLookupNotYetFetchedReturnsError(t *testing.T) {
	sentinel := errors.New("oidc provider not yet fetched")

	result, err := processOIDCPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		makeOIDC(),
		nil,
		"traffic/default/my-policy",
		types.NamespacedName{Namespace: "default", Name: "my-policy"},
		stubOIDCLookup{err: sentinel},
	)

	if err == nil {
		t.Fatal("expected error when lookup returns error")
	}
	if !errors.Is(err, sentinel) {
		t.Errorf("expected sentinel error, got %v", err)
	}
	if result != nil {
		t.Fatal("expected no policy when lookup returns an error")
	}
}

func TestProcessOIDCPolicyLookupReturnsNilProviderReturnsError(t *testing.T) {
	result, err := processOIDCPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		makeOIDC(),
		nil,
		"traffic/default/my-policy",
		types.NamespacedName{Namespace: "default", Name: "my-policy"},
		stubOIDCLookup{provider: nil, err: nil},
	)

	if err == nil {
		t.Fatal("expected error when lookup returns nil provider")
	}
	if result != nil {
		t.Fatal("expected no policy when lookup returns a nil provider")
	}
}

func TestProcessOIDCPolicyNilLookupReturnsError(t *testing.T) {
	_, err := processOIDCPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		makeOIDC(),
		nil,
		"traffic/default/my-policy",
		types.NamespacedName{Namespace: "default", Name: "my-policy"},
		nil,
	)

	if err == nil {
		t.Fatal("expected error when oidc lookup is nil")
	}
}

func TestProcessOIDCPolicyResultHasTrafficKind(t *testing.T) {
	result, err := processOIDCPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		makeOIDC(),
		nil,
		"traffic/default/my-policy",
		types.NamespacedName{Namespace: "default", Name: "my-policy"},
		stubOIDCLookup{provider: makeDiscoveredProvider()},
	)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	// GetTraffic() returns nil if the kind is not Traffic.
	if result.GetTraffic() == nil {
		t.Error("expected Traffic kind on policy, got nil")
	}
	// GetOidc() must be populated.
	if result.GetTraffic().GetOidc() == nil {
		t.Error("expected oidc spec inside Traffic policy")
	}
}

func TestProcessOIDCPolicyPreservesConfiguredPhase(t *testing.T) {
	preRouting := agentgateway.PolicyPhase("PreRouting")

	result, err := processOIDCPolicy(
		PolicyCtx{Krt: krt.TestingDummyContext{}},
		makeOIDC(),
		&preRouting,
		"traffic/default/my-policy",
		types.NamespacedName{Namespace: "default", Name: "my-policy"},
		stubOIDCLookup{provider: makeDiscoveredProvider()},
	)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if got := result.GetTraffic().GetPhase(); got != api.TrafficPolicySpec_GATEWAY {
		t.Fatalf("phase: got %v, want %v", got, api.TrafficPolicySpec_GATEWAY)
	}
}
