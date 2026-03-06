package plugins_test

import (
	"strings"
	"testing"

	"istio.io/istio/pkg/ptr"
	k8stypes "k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
)

func TestTranslateAgentgatewayPolicyRejectsUnsupportedOAuth2BackendRefKind(t *testing.T) {
	oauth2 := testOAuth2DiscoveryConfig()
	oauth2.Issuer = ptr.Of(agentgateway.LongString("https://issuer.example.com"))
	oauth2.BackendRef = &gwv1.BackendObjectReference{
		Group: ptr.Of(gwv1.Group(wellknown.InferencePoolGVK.Group)),
		Kind:  ptr.Of(gwv1.Kind(wellknown.InferencePoolGVK.Kind)),
		Name:  "test-pool",
		Port:  ptr.Of(gwv1.PortNumber(8443)),
	}
	policy := testGatewayOAuth2PolicyForConfig("oauth2-unsupported-backend-kind", oauth2)
	status, output := testTranslateGatewayOAuth2Policy(t, policy)
	_ = output
	accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
	if accepted.Reason != string(shared.PolicyReasonInvalid) {
		t.Fatalf("expected invalid reason, got %q", accepted.Reason)
	}
	if !strings.Contains(accepted.Message, "oauth2 provider backend ref only supports Service and AgentgatewayBackend kinds") {
		t.Fatalf("unexpected accepted message: %q", accepted.Message)
	}
}

func TestTranslateAgentgatewayPolicyTranslatesExplicitOAuth2Provider(t *testing.T) {
	policy := testGatewayOAuth2PolicyForConfig("oauth2-explicit", testOAuth2ExplicitConfig())
	status, output := testTranslateGatewayOAuth2Policy(t, policy)
	if len(output) != 1 {
		t.Fatalf("expected 1 translated policy, got %d", len(output))
	}
	accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
	if accepted.Reason != string(shared.PolicyReasonValid) {
		t.Fatalf("expected valid reason, got %q", accepted.Reason)
	}

	spec := output[0].Policy.GetTraffic().GetOauth2()
	if spec == nil {
		t.Fatalf("expected translated oauth2 spec")
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
}

func TestTranslateAgentgatewayPolicyPropagatesAllowInsecureRedirectURI(t *testing.T) {
	oauth2 := testOAuth2ExplicitConfig()
	oauth2.AllowInsecureRedirectURI = true
	oauth2.RedirectURI = agentgateway.LongString("http://app.example.com/oauth2/callback")
	policy := testGatewayOAuth2PolicyForConfig("oauth2-allow-insecure", oauth2)
	status, output := testTranslateGatewayOAuth2Policy(t, policy)
	if len(output) != 1 {
		t.Fatalf("expected 1 translated policy, got %d", len(output))
	}
	accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
	if accepted.Reason != string(shared.PolicyReasonValid) {
		t.Fatalf("expected valid reason, got %q", accepted.Reason)
	}

	spec := output[0].Policy.GetTraffic().GetOauth2()
	if spec == nil {
		t.Fatalf("expected translated oauth2 spec")
	}
	if !spec.GetAllowInsecureRedirectUri() {
		t.Fatalf("expected allowInsecureRedirectUri to be true")
	}
}

func TestTranslateAgentgatewayPolicyValidatesOAuth2Inputs(t *testing.T) {
	tests := []struct {
		name            string
		mutate          func(*agentgateway.OAuth2)
		wantErrContains string
		wantValid       bool
	}{
		{
			name: "rejects non loopback http redirect uri by default",
			mutate: func(oauth2 *agentgateway.OAuth2) {
				oauth2.RedirectURI = agentgateway.LongString("http://app.example.com/oauth2/callback")
			},
			wantErrContains: "oauth2 redirectUri must use https (or http on loopback hosts unless allowInsecureRedirectUri is true)",
		},
		{
			name: "rejects host cookie on insecure redirect uri",
			mutate: func(oauth2 *agentgateway.OAuth2) {
				oauth2.RedirectURI = agentgateway.LongString("http://127.0.0.1:3000/oauth2/callback")
				oauth2.CookieName = ptr.Of(agentgateway.ShortString("__Host-ag-session"))
			},
			wantErrContains: "__Host- cookie names require https redirectUri",
		},
		{
			name: "rejects invalid post logout redirect uri",
			mutate: func(oauth2 *agentgateway.OAuth2) {
				oauth2.PostLogoutRedirectURI = ptr.Of(agentgateway.LongString("http://app.example.com/signed-out"))
			},
			wantErrContains: "oauth2 postLogoutRedirectUri must use https (or http on loopback hosts unless allowInsecureRedirectUri is true)",
		},
		{
			name: "rejects zero refreshable cookie max age",
			mutate: func(oauth2 *agentgateway.OAuth2) {
				oauth2.RefreshableCookieMaxAgeSeconds = ptr.Of(uint64(0))
			},
			wantErrContains: "oauth2 refreshableCookieMaxAgeSeconds must be > 0",
		},
		{
			name: "rejects excessive refreshable cookie max age",
			mutate: func(oauth2 *agentgateway.OAuth2) {
				oauth2.RefreshableCookieMaxAgeSeconds = ptr.Of(uint64(30*24*60*60 + 1))
			},
			wantErrContains: "oauth2 refreshableCookieMaxAgeSeconds must be <=",
		},
		{
			name: "rejects non loopback http authorization endpoint",
			mutate: func(oauth2 *agentgateway.OAuth2) {
				oauth2.AuthorizationEndpoint = ptr.Of(agentgateway.LongString("http://idp.example.com/oauth/authorize"))
			},
			wantErrContains: "oauth2 authorizationEndpoint must use https (or http on loopback hosts)",
		},
		{
			name: "accepts loopback http explicit endpoints",
			mutate: func(oauth2 *agentgateway.OAuth2) {
				oauth2.AuthorizationEndpoint = ptr.Of(agentgateway.LongString("http://127.0.0.1:8080/oauth/authorize"))
				oauth2.TokenEndpoint = ptr.Of(agentgateway.LongString("http://127.0.0.1:8080/oauth/token"))
				oauth2.EndSessionEndpoint = ptr.Of(agentgateway.LongString("http://127.0.0.1:8080/logout"))
			},
			wantValid: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			oauth2 := testOAuth2ExplicitConfig()
			tt.mutate(oauth2)
			policy := testGatewayOAuth2PolicyForConfig("oauth2-validation-"+strings.ReplaceAll(tt.name, " ", "-"), oauth2)

			if !tt.wantValid {
				assertInvalidOAuth2Policy(t, policy, tt.wantErrContains)
				return
			}

			status, output := testTranslateGatewayOAuth2Policy(t, policy)
			if len(output) != 1 {
				t.Fatalf("expected 1 translated policy, got %d", len(output))
			}
			accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
			if accepted.Reason != string(shared.PolicyReasonValid) {
				t.Fatalf("expected valid reason, got %q", accepted.Reason)
			}
		})
	}
}

func testTranslateGatewayOAuth2Policy(t *testing.T, policy *agentgateway.AgentgatewayPolicy) (*gwv1.PolicyStatus, []plugins.AgwPolicy) {
	t.Helper()
	return testTranslateGatewayOAuth2PolicyWithInputs(t, policy)
}

func testTranslateGatewayOAuth2PolicyWithInputs(t *testing.T, policy *agentgateway.AgentgatewayPolicy, extraInputs ...any) (*gwv1.PolicyStatus, []plugins.AgwPolicy) {
	t.Helper()
	inputs := []any{testGateway(), policy}
	inputs = append(inputs, extraInputs...)
	ctx := testutils.BuildMockPolicyContext(t, inputs)
	return plugins.TranslateAgentgatewayPolicy(ctx.Krt, policy, ctx.Collections)
}

func testGatewayOAuth2PolicyForConfig(name string, oauth2 *agentgateway.OAuth2) *agentgateway.AgentgatewayPolicy {
	policy := testGatewayOAuth2Policy(name, k8stypes.UID("uid-"+name))
	policy.Spec.Traffic.OAuth2 = oauth2
	return policy
}

func testOAuth2ExplicitConfig() *agentgateway.OAuth2 {
	return &agentgateway.OAuth2{
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
}

func testOAuth2DiscoveryConfig() *agentgateway.OAuth2 {
	return &agentgateway.OAuth2{
		ClientID: "agw-client",
		Issuer:   ptr.Of(agentgateway.LongString("https://issuer.example.com")),
		ClientSecret: agentgateway.OAuth2ClientSecret{
			Inline: ptr.Of("super-secret"),
		},
		RedirectURI: agentgateway.LongString("https://gateway.example.com/oauth2/callback"),
	}
}

func assertInvalidOAuth2Policy(t *testing.T, policy *agentgateway.AgentgatewayPolicy, wantErrContains string) {
	t.Helper()
	status, output := testTranslateGatewayOAuth2Policy(t, policy)
	if len(output) != 0 {
		t.Fatalf("expected no translated policies, got %d", len(output))
	}
	accepted := findAcceptedCondition(t, status.Ancestors[0].Conditions)
	if accepted.Reason != string(shared.PolicyReasonInvalid) {
		t.Fatalf("expected invalid reason, got %q", accepted.Reason)
	}
	if !strings.Contains(accepted.Message, wantErrContains) {
		t.Fatalf("unexpected accepted message: %q", accepted.Message)
	}
}
