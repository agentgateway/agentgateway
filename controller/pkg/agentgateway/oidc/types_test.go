package oidc

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
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
				JwksJSON:              `{"keys":[]}`,
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
				JwksJSON:              `{"keys":[{"kid":"test"}]}`,
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

			assert.Equal(t, tc.provider.RequestKey, got.RequestKey)
			assert.Equal(t, tc.provider.IssuerURL, got.IssuerURL)
			assert.Equal(t, tc.provider.AuthorizationEndpoint, got.AuthorizationEndpoint)
			assert.Equal(t, tc.provider.TokenEndpoint, got.TokenEndpoint)
			assert.Equal(t, tc.provider.JwksURI, got.JwksURI)
			assert.Equal(t, tc.provider.JwksJSON, got.JwksJSON)
			assert.Equal(t, tc.provider.TokenEndpointAuthMethodsSupported, got.TokenEndpointAuthMethodsSupported)
			assert.Equal(t, tc.provider.FetchedAt, got.FetchedAt)
		})
	}
}
