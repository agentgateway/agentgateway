package plugins

import (
	"testing"

	"istio.io/istio/pkg/ptr"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
)

func TestProcessOAuth2PolicyRejectsUnsupportedProviderBackendRefKind(t *testing.T) {
	oauth2 := &agentgateway.OAuth2{
		Issuer:       "https://issuer.example.com",
		ClientID:     "agw-client",
		ClientSecret: ptr.Of("super-secret"),
		RedirectURI:  ptr.Of(agentgateway.LongString("https://gateway.example.com/oauth2/callback")),
		ProviderBackendRef: &gwv1.BackendObjectReference{
			Group: ptr.Of(gwv1.Group(wellknown.InferencePoolGVK.Group)),
			Kind:  ptr.Of(gwv1.Kind(wellknown.InferencePoolGVK.Kind)),
			Name:  "test-pool",
			Port:  ptr.Of(gwv1.PortNumber(8443)),
		},
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

func TestProcessJWTAuthenticationPolicyRejectsUnsupportedOIDCProviderBackendRefKind(t *testing.T) {
	jwt := &agentgateway.JWTAuthentication{
		Mode: agentgateway.JWTAuthenticationModeStrict,
		Providers: []agentgateway.JWTProvider{
			{
				Issuer: "https://issuer.example.com",
				JWKS: agentgateway.JWKS{
					OIDC: &agentgateway.OIDCJWKS{
						ProviderBackendRef: &gwv1.BackendObjectReference{
							Group: ptr.Of(gwv1.Group(wellknown.InferencePoolGVK.Group)),
							Kind:  ptr.Of(gwv1.Kind(wellknown.InferencePoolGVK.Kind)),
							Name:  "test-pool",
							Port:  ptr.Of(gwv1.PortNumber(8443)),
						},
					},
				},
			},
		},
	}

	policies, err := processJWTAuthenticationPolicy(
		PolicyCtx{},
		jwt,
		ptr.Of(agentgateway.PolicyPhasePreRouting),
		"traffic/default/jwt",
		types.NamespacedName{Namespace: "default", Name: "jwt-policy"},
		nil,
	)
	if err == nil {
		t.Fatalf("expected error for unsupported jwt oidc provider backend kind")
	}
	if got, want := err.Error(), "jwt oidc provider backend ref only supports Service and AgentgatewayBackend kinds"; got != want {
		t.Fatalf("unexpected error: got %q want %q", got, want)
	}
	_ = policies
}

func TestProcessJWTAuthenticationPolicyTranslatesOIDCJwksSource(t *testing.T) {
	jwt := &agentgateway.JWTAuthentication{
		Mode: agentgateway.JWTAuthenticationModeStrict,
		Providers: []agentgateway.JWTProvider{
			{
				Issuer:    "https://issuer.example.com",
				Audiences: []string{"agw-client"},
				JWKS: agentgateway.JWKS{
					OIDC: &agentgateway.OIDCJWKS{},
				},
			},
		},
	}

	policies, err := processJWTAuthenticationPolicy(
		PolicyCtx{},
		jwt,
		ptr.Of(agentgateway.PolicyPhasePreRouting),
		"traffic/default/jwt",
		types.NamespacedName{Namespace: "default", Name: "jwt-policy"},
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
	jwtSpec := traffic.GetJwt()
	if jwtSpec == nil {
		t.Fatalf("expected jwt policy spec")
	}
	if len(jwtSpec.Providers) != 1 {
		t.Fatalf("expected 1 jwt provider, got %d", len(jwtSpec.Providers))
	}
	oidc := jwtSpec.Providers[0].GetOidc()
	if oidc == nil {
		t.Fatalf("expected oidc jwks source")
	}
	if oidc.GetProviderBackend() != nil {
		t.Fatalf("expected no provider backend by default")
	}
	if _, ok := jwtSpec.Providers[0].GetJwksSource().(*api.TrafficPolicySpec_JWTProvider_Oidc); !ok {
		t.Fatalf("expected jwt provider oneof to be oidc")
	}
}
