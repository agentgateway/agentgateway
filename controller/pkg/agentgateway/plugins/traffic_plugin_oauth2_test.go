package plugins

import (
	"testing"

	"istio.io/istio/pkg/ptr"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
)

func TestProcessOAuth2PolicyRejectsUnsupportedBackendRefKind(t *testing.T) {
	oauth2 := &agentgateway.OAuth2{
		ClientID: "agw-client",
		Issuer:   ptr.Of(agentgateway.LongString("https://issuer.example.com")),
		BackendRef: &gwv1.BackendObjectReference{
			Group: ptr.Of(gwv1.Group(wellknown.InferencePoolGVK.Group)),
			Kind:  ptr.Of(gwv1.Kind(wellknown.InferencePoolGVK.Kind)),
			Name:  "test-pool",
			Port:  ptr.Of(gwv1.PortNumber(8443)),
		},
		ClientSecret: agentgateway.OAuth2ClientSecret{
			Inline: ptr.Of("super-secret"),
		},
		RedirectURI: agentgateway.LongString("https://gateway.example.com/oauth2/callback"),
	}

	policies, err := processOAuth2Policy(
		PolicyCtx{},
		oauth2,
		ptr.Of(agentgateway.PolicyPhasePreRouting),
		"traffic/default/oauth2",
		types.NamespacedName{Namespace: "default", Name: "oauth2-policy"},
		nil,
	)
	if err == nil {
		t.Fatalf("expected error for unsupported oauth2 provider backend kind")
	}
	if got, want := err.Error(), "oauth2 provider backend ref only supports Service and AgentgatewayBackend kinds"; got != want {
		t.Fatalf("unexpected error: got %q want %q", got, want)
	}
	if policies != nil {
		t.Fatalf("expected no policies when oauth2 provider backend ref is invalid")
	}
}

func TestProcessOAuth2PolicyTranslatesResolvedProviderMetadata(t *testing.T) {
	withStubOIDCResolver(t, &resolvedOIDCProvider{
		AuthorizationEndpoint:             "https://issuer.example.com/authorize",
		TokenEndpoint:                     "https://issuer.example.com/token",
		EndSessionEndpoint:                "https://issuer.example.com/logout",
		TokenEndpointAuthMethodsSupported: []string{"client_secret_post"},
		JwksInline:                        `{"keys":[{"kid":"k1"}]}`,
	}, nil)

	oauth2 := &agentgateway.OAuth2{
		ClientID: "agw-client",
		Issuer:   ptr.Of(agentgateway.LongString("https://issuer.example.com")),
		ClientSecret: agentgateway.OAuth2ClientSecret{
			Inline: ptr.Of("super-secret"),
		},
		RedirectURI: agentgateway.LongString("https://gateway.example.com/oauth2/callback"),
	}

	policies, err := processOAuth2Policy(
		PolicyCtx{},
		oauth2,
		ptr.Of(agentgateway.PolicyPhasePreRouting),
		"traffic/default/oauth2",
		types.NamespacedName{Namespace: "default", Name: "oauth2-policy"},
		nil,
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(policies) != 1 {
		t.Fatalf("expected 1 policy, got %d", len(policies))
	}

	traffic := policies[0].Policy.GetTraffic()
	if traffic == nil {
		t.Fatalf("expected traffic policy")
	}
	spec := traffic.GetOauth2()
	if spec == nil {
		t.Fatalf("expected oauth2 policy spec")
	}
	if got, want := spec.GetAuthorizationEndpoint(), "https://issuer.example.com/authorize"; got != want {
		t.Fatalf("unexpected authorization endpoint: got %q want %q", got, want)
	}
	if got, want := spec.GetTokenEndpoint(), "https://issuer.example.com/token"; got != want {
		t.Fatalf("unexpected token endpoint: got %q want %q", got, want)
	}
	if got, want := spec.GetEndSessionEndpoint(), "https://issuer.example.com/logout"; got != want {
		t.Fatalf("unexpected end session endpoint: got %q want %q", got, want)
	}
	if got, want := spec.GetJwksInline(), `{"keys":[{"kid":"k1"}]}`; got != want {
		t.Fatalf("unexpected jwks inline: got %q want %q", got, want)
	}
	if got, want := spec.GetTokenEndpointAuthMethodsSupported(), []string{"client_secret_post"}; len(got) != len(want) || got[0] != want[0] {
		t.Fatalf("unexpected token endpoint auth methods: got %v want %v", got, want)
	}
}

func TestProcessOAuth2PolicyTranslatesExplicitOAuth2Provider(t *testing.T) {
	oauth2 := &agentgateway.OAuth2{
		ClientID:              "agw-client",
		AuthorizationEndpoint: ptr.Of(agentgateway.LongString("https://provider.example.com/oauth/authorize")),
		TokenEndpoint:         ptr.Of(agentgateway.LongString("https://provider.example.com/oauth/token")),
		EndSessionEndpoint:    ptr.Of(agentgateway.LongString("https://provider.example.com/logout")),
		TokenEndpointAuthMethodsSupported: []agentgateway.ShortString{
			"client_secret_post",
		},
		ClientSecret: agentgateway.OAuth2ClientSecret{
			Inline: ptr.Of("super-secret"),
		},
		RedirectURI: agentgateway.LongString("https://gateway.example.com/oauth2/callback"),
	}

	policies, err := processOAuth2Policy(
		PolicyCtx{},
		oauth2,
		ptr.Of(agentgateway.PolicyPhasePreRouting),
		"traffic/default/oauth2",
		types.NamespacedName{Namespace: "default", Name: "oauth2-policy"},
		nil,
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(policies) != 1 {
		t.Fatalf("expected 1 policy, got %d", len(policies))
	}

	spec := policies[0].Policy.GetTraffic().GetOauth2()
	if spec == nil {
		t.Fatalf("expected oauth2 policy spec")
	}
	if got, want := spec.GetProviderId(), "https://provider.example.com/oauth/authorize"; got != want {
		t.Fatalf("unexpected provider id: got %q want %q", got, want)
	}
	if got := spec.GetOidcIssuer(); got != "" {
		t.Fatalf("did not expect oidc issuer for explicit oauth2 provider, got %q", got)
	}
	if got, want := spec.GetAuthorizationEndpoint(), "https://provider.example.com/oauth/authorize"; got != want {
		t.Fatalf("unexpected authorization endpoint: got %q want %q", got, want)
	}
	if got, want := spec.GetTokenEndpoint(), "https://provider.example.com/oauth/token"; got != want {
		t.Fatalf("unexpected token endpoint: got %q want %q", got, want)
	}
	if got, want := spec.GetEndSessionEndpoint(), "https://provider.example.com/logout"; got != want {
		t.Fatalf("unexpected end session endpoint: got %q want %q", got, want)
	}
	if got := spec.GetJwksInline(); got != "" {
		t.Fatalf("did not expect jwks inline for explicit oauth2 provider, got %q", got)
	}
}

func TestProcessOAuth2PolicyPropagatesAllowInsecureRedirectURI(t *testing.T) {
	oauth2 := &agentgateway.OAuth2{
		ClientID:                 "agw-client",
		AuthorizationEndpoint:    ptr.Of(agentgateway.LongString("https://provider.example.com/oauth/authorize")),
		TokenEndpoint:            ptr.Of(agentgateway.LongString("https://provider.example.com/oauth/token")),
		AllowInsecureRedirectURI: true,
		ClientSecret: agentgateway.OAuth2ClientSecret{
			Inline: ptr.Of("super-secret"),
		},
		RedirectURI: agentgateway.LongString("https://gateway.example.com/oauth2/callback"),
	}

	policies, err := processOAuth2Policy(
		PolicyCtx{},
		oauth2,
		ptr.Of(agentgateway.PolicyPhasePreRouting),
		"traffic/default/oauth2",
		types.NamespacedName{Namespace: "default", Name: "oauth2-policy"},
		nil,
	)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(policies) != 1 {
		t.Fatalf("expected 1 policy, got %d", len(policies))
	}

	spec := policies[0].Policy.GetTraffic().GetOauth2()
	if spec == nil {
		t.Fatalf("expected oauth2 policy spec")
	}
	if !spec.GetAllowInsecureRedirectUri() {
		t.Fatalf("expected allowInsecureRedirectUri to be true")
	}
}
