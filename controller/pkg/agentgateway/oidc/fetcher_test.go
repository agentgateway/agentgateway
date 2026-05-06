package oidc

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/stretchr/testify/require"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

func TestFetcherFetchesAndValidatesDiscovery(t *testing.T) {
	ctx := t.Context()
	// Minimal valid JWKS so the schema validator accepts the body. Uses an
	// `oct` key to avoid generating real RSA material in test fixtures.
	const jwksJSON = `{"keys":[{"kty":"oct","k":"AAECAwQFBgc"}]}`

	var backendURL string
	backend := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		if r.URL.Path == "/jwks" {
			_, _ = fmt.Fprint(w, jwksJSON)
			return
		}
		_ = json.NewEncoder(w).Encode(discoveryDocument{
			Issuer:                backendURL,
			AuthorizationEndpoint: backendURL + "/auth",
			TokenEndpoint:         backendURL + "/token",
			JwksURI:               backendURL + "/jwks",
		})
	}))
	defer backend.Close()
	backendURL = backend.URL

	cache := NewCache()
	f := NewFetcher(cache)
	f.Driver.(*OidcDriver).DefaultClient = backend.Client()

	source := SharedOidcRequest{
		RequestKey:     remotehttp.FetchTarget{URL: backendURL}.Key(),
		ExpectedIssuer: backendURL,
		Target:         remotehttp.FetchTarget{URL: backendURL},
		TTL:            time.Hour,
	}

	f.AddOrUpdate(source)
	go f.MaybeFetch(ctx)

	require.Eventually(t, func() bool {
		_, ok := cache.Get(source.RequestKey)
		return ok
	}, time.Second, 10*time.Millisecond)

	provider, _ := cache.Get(source.RequestKey)
	require.Equal(t, backendURL, provider.IssuerURL)
	require.Equal(t, jwksJSON, provider.JwksInline)
}

func TestOidcDriverRejectsInvalidJWKSBody(t *testing.T) {
	tests := []struct {
		name            string
		jwksBody        string
		wantErrContains string
	}{
		{
			name:            "non-jwks 200 OK is rejected, not persisted",
			jwksBody:        `{"status":"error"}`,
			wantErrContains: "contains no keys",
		},
		{
			name:            "explicitly empty key set is rejected",
			jwksBody:        `{"keys":[]}`,
			wantErrContains: "contains no keys",
		},
		{
			name:            "malformed key fails strict JWKS parse",
			jwksBody:        `{"keys":[{"kty":"RSA","n":"!"}]}`,
			wantErrContains: "not a valid JWKS document",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			var backendURL string
			backend := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.Header().Set("Content-Type", "application/json")
				if r.URL.Path == "/jwks" {
					_, _ = fmt.Fprint(w, tc.jwksBody)
					return
				}
				_ = json.NewEncoder(w).Encode(discoveryDocument{
					Issuer:                backendURL,
					AuthorizationEndpoint: backendURL + "/auth",
					TokenEndpoint:         backendURL + "/token",
					JwksURI:               backendURL + "/jwks",
				})
			}))
			defer backend.Close()
			backendURL = backend.URL

			driver := &OidcDriver{DefaultClient: backend.Client()}
			source := SharedOidcRequest{
				RequestKey:     remotehttp.FetchTarget{URL: backendURL}.Key(),
				ExpectedIssuer: backendURL,
				Target:         remotehttp.FetchTarget{URL: backendURL},
				TTL:            time.Hour,
			}

			_, err := driver.Fetch(t.Context(), source)
			require.ErrorContains(t, err, tc.wantErrContains)
		})
	}
}

func TestValidateDiscoveryDocument(t *testing.T) {
	tests := []struct {
		name           string
		doc            discoveryDocument
		expectedIssuer string
		wantErr        bool
	}{
		{
			name: "valid",
			doc: discoveryDocument{
				Issuer:                "https://issuer",
				AuthorizationEndpoint: "https://issuer/auth",
				TokenEndpoint:         "https://issuer/token",
				JwksURI:               "https://issuer/jwks",
			},
			expectedIssuer: "https://issuer",
			wantErr:        false,
		},
		{
			name: "issuer mismatch",
			doc: discoveryDocument{
				Issuer: "https://wrong",
			},
			expectedIssuer: "https://right",
			wantErr:        true,
		},
		{
			name: "non-https auth endpoint",
			doc: discoveryDocument{
				Issuer:                "https://issuer",
				AuthorizationEndpoint: "http://issuer/auth",
			},
			expectedIssuer: "https://issuer",
			wantErr:        true,
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			err := validateDiscoveryDocument(tc.doc, tc.expectedIssuer)
			if tc.wantErr {
				require.Error(t, err)
			} else {
				require.NoError(t, err)
			}
		})
	}
}
