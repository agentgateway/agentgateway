package plugins

import (
	"errors"
	"strings"
	"testing"

	"istio.io/istio/pkg/ptr"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

type testOIDCResolver struct {
	resolved *resolvedOIDCProvider
	err      error
}

func (s testOIDCResolver) Resolve(
	_ PolicyCtx,
	_,
	_,
	_ string,
	_ *gwv1.BackendObjectReference,
) (*resolvedOIDCProvider, error) {
	return s.resolved, s.err
}

func testOIDCResolverForResult(resolved *resolvedOIDCProvider, err error) oidcResolver {
	return testOIDCResolver{resolved: resolved, err: err}
}

func TestProcessOAuth2PolicyRejectsResolverMetadataWithInvalidEndpoint(t *testing.T) {
	resolver := testOIDCResolverForResult(&resolvedOIDCProvider{
		AuthorizationEndpoint:             "http://idp.example.com/authorize",
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

	_, err := processOAuth2Policy(
		PolicyCtx{OIDCResolver: resolver},
		oauth2,
		ptr.Of(agentgateway.PolicyPhasePreRouting),
		"traffic/default/oauth2",
		types.NamespacedName{Namespace: "default", Name: "oauth2-policy"},
		nil,
	)
	if err == nil {
		t.Fatalf("expected resolver metadata with non-loopback http endpoint to fail")
	}
	if got, want := err.Error(), "oauth2 authorizationEndpoint must use https (or http on loopback hosts), include a host, must not contain a fragment, and must not include userinfo"; got != want {
		t.Fatalf("unexpected error: got %q want %q", got, want)
	}
}

func TestProcessOAuth2PolicyRejectsPolicyWhenResolverFails(t *testing.T) {
	resolver := testOIDCResolverForResult(nil, errors.New("discovery unavailable"))

	oauth2 := &agentgateway.OAuth2{
		ClientID: "agw-client",
		Issuer:   ptr.Of(agentgateway.LongString("https://issuer.example.com")),
		ClientSecret: agentgateway.OAuth2ClientSecret{
			Inline: ptr.Of("super-secret"),
		},
		RedirectURI: agentgateway.LongString("https://gateway.example.com/oauth2/callback"),
	}

	policies, err := processOAuth2Policy(
		PolicyCtx{OIDCResolver: resolver},
		oauth2,
		ptr.Of(agentgateway.PolicyPhasePreRouting),
		"traffic/default/oauth2",
		types.NamespacedName{Namespace: "default", Name: "oauth2-policy"},
		nil,
	)
	if err == nil {
		t.Fatalf("expected discovery error to be returned")
	}
	if !strings.Contains(err.Error(), "failed resolving oauth2 provider metadata") {
		t.Fatalf("unexpected error, expected resolver failure prefix, got %q", err.Error())
	}
	if !strings.Contains(err.Error(), "discovery unavailable") {
		t.Fatalf("unexpected error, expected resolver failure cause, got %q", err.Error())
	}
	if len(policies) != 0 {
		t.Fatalf("expected no policy when provider discovery fails, got %d", len(policies))
	}
}
