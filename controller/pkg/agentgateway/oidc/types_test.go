package oidc

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"testing"
	"time"

	"github.com/stretchr/testify/require"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

func TestDiscoveredProviderJSONRoundTrip(t *testing.T) {
	requestKey := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key()

	tests := []struct {
		name     string
		provider DiscoveredProvider
	}{
		{
			name: "full provider",
			provider: DiscoveredProvider{
				RequestKey:            requestKey,
				IssuerURL:             "https://issuer.example",
				AuthorizationEndpoint: "https://issuer.example/auth",
				TokenEndpoint:         "https://issuer.example/token",
				JwksURI:               "https://issuer.example/jwks",
				JwksInline:            `{"keys":[]}`,
				TokenEndpointAuthMethodsSupported: []string{
					"client_secret_basic",
					"client_secret_post",
				},
				FetchedAt: time.Unix(1000, 0).UTC(),
			},
		},
		{
			name: "minimal provider without optional fields",
			provider: DiscoveredProvider{
				RequestKey:            requestKey,
				IssuerURL:             "https://issuer.example",
				AuthorizationEndpoint: "https://issuer.example/auth",
				TokenEndpoint:         "https://issuer.example/token",
				JwksURI:               "https://issuer.example/jwks",
				JwksInline:            `{"keys":[]}`,
				FetchedAt:             time.Unix(2000, 0).UTC(),
			},
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			b, err := json.Marshal(tc.provider)
			require.NoError(t, err)

			var got DiscoveredProvider
			require.NoError(t, json.Unmarshal(b, &got))

			require.Equal(t, tc.provider.RequestKey, got.RequestKey)
			require.Equal(t, tc.provider.IssuerURL, got.IssuerURL)
			require.Equal(t, tc.provider.AuthorizationEndpoint, got.AuthorizationEndpoint)
			require.Equal(t, tc.provider.TokenEndpoint, got.TokenEndpoint)
			require.Equal(t, tc.provider.JwksURI, got.JwksURI)
			require.Equal(t, tc.provider.JwksInline, got.JwksInline)
			require.Equal(t, tc.provider.TokenEndpointAuthMethodsSupported, got.TokenEndpointAuthMethodsSupported)
			require.Equal(t, tc.provider.FetchedAt, got.FetchedAt)
		})
	}
}

func TestOidcRequestKeyIsDomainSeparated(t *testing.T) {
	target := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}
	expectedIssuer := "https://issuer.example"

	key := oidcRequestKey(target, expectedIssuer)

	require.NotEqual(t, target.Key(), key)
	require.NotEqual(t, oldOidcRequestKeyForTest(target, expectedIssuer), key)
}

func oldOidcRequestKeyForTest(target remotehttp.FetchTarget, expectedIssuer string) remotehttp.FetchKey {
	hash := sha256.New()
	writeHashPart := func(value string) {
		_, _ = hash.Write([]byte(value))
		_, _ = hash.Write([]byte{0})
	}

	writeHashPart(target.Key().String())
	writeHashPart(expectedIssuer)

	return remotehttp.FetchKey(hex.EncodeToString(hash.Sum(nil)))
}

func TestOidcRequestKeyStable(t *testing.T) {
	target := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}
	issuer := "https://issuer.example"

	first := oidcRequestKey(target, issuer)
	second := oidcRequestKey(target, issuer)

	require.Equal(t, first, second, "same (target, issuer) must hash identically across calls")
}

func TestOidcRequestKeyDifferentiatesInputs(t *testing.T) {
	baseTarget := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}
	baseIssuer := "https://issuer.example"
	baseKey := oidcRequestKey(baseTarget, baseIssuer)

	tests := []struct {
		name   string
		target remotehttp.FetchTarget
		issuer string
	}{
		{
			name:   "different target URL",
			target: remotehttp.FetchTarget{URL: "https://other.example/.well-known/openid-configuration"},
			issuer: baseIssuer,
		},
		{
			name:   "different expected issuer",
			target: baseTarget,
			issuer: "https://other.example",
		},
		{
			name:   "issuer with trailing slash",
			target: baseTarget,
			issuer: baseIssuer + "/",
		},
		{
			name:   "issuer with different scheme",
			target: baseTarget,
			issuer: "http://issuer.example",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			other := oidcRequestKey(tc.target, tc.issuer)
			require.NotEqual(t, baseKey, other,
				"request keys must differ when (target, issuer) differs")
		})
	}
}
