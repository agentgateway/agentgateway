package plugins_test

import (
	"strings"
	"testing"

	"istio.io/istio/pkg/ptr"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/identity/oidc"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
)

func TestTranslateAgentgatewayPolicyRejectsStoredProviderMetadataWithInvalidEndpoint(t *testing.T) {
	oauth2 := &agentgateway.OAuth2{
		ClientID: "agw-client",
		Issuer:   ptr.Of(agentgateway.LongString("https://issuer.example.com")),
		ClientSecret: agentgateway.OAuth2ClientSecret{
			Inline: ptr.Of("super-secret"),
		},
		RedirectURI: agentgateway.LongString("https://gateway.example.com/oauth2/callback"),
	}
	policy := testGatewayOAuth2PolicyForConfig("oauth2-invalid-stored-provider", oauth2)

	status, output := testTranslateGatewayOAuth2PolicyWithInputs(
		t,
		policy,
		testOIDCProviderConfigMap(
			t,
			policy.Namespace,
			string(*oauth2.Issuer),
			nil,
			oidcProvider("https://issuer.example.com", "http://idp.example.com/authorize", "https://issuer.example.com/token", "https://issuer.example.com/jwks"),
		),
		testJWKSConfigMap(t, "https://issuer.example.com/jwks", testOIDCJWKSInline),
	)
	if len(output) != 0 {
		t.Fatalf("expected no translated policies, got %d", len(output))
	}
	accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
	if accepted.Reason != string(shared.PolicyReasonInvalid) {
		t.Fatalf("expected invalid reason, got %q", accepted.Reason)
	}
	if got, want := accepted.Message, "oauth2 authorizationEndpoint must use https (or http on loopback hosts), include a host, must not contain a fragment, and must not include userinfo"; got != want {
		t.Fatalf("unexpected error: got %q want %q", got, want)
	}
}

func TestTranslateAgentgatewayPolicyRejectsPolicyWhenStoredProviderIsMissing(t *testing.T) {
	policy := testGatewayOAuth2PolicyForConfig("oauth2-missing-stored-provider", testOAuth2DiscoveryConfig())

	ctx := testutils.BuildMockPolicyContext(t, []any{testGateway(), policy})
	status, output := plugins.TranslateAgentgatewayPolicy(ctx.Krt, policy, ctx.Collections)
	if len(output) != 0 {
		t.Fatalf("expected no translated policies, got %d", len(output))
	}
	accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
	if accepted.Reason != string(shared.PolicyReasonInvalid) {
		t.Fatalf("expected invalid reason, got %q", accepted.Reason)
	}
	if !strings.Contains(accepted.Message, "failed resolving oauth2 provider metadata") {
		t.Fatalf("unexpected error, expected resolver failure prefix, got %q", accepted.Message)
	}
	if !strings.Contains(accepted.Message, "oidc provider ConfigMap") {
		t.Fatalf("unexpected error, expected missing provider cause, got %q", accepted.Message)
	}
}

func oidcProvider(issuer, authorizationEndpoint, tokenEndpoint, jwksURI string) oidc.StoredProvider {
	return oidc.StoredProvider{
		Issuer:                issuer,
		AuthorizationEndpoint: authorizationEndpoint,
		TokenEndpoint:         tokenEndpoint,
		JwksURI:               jwksURI,
	}
}
