package main

import (
	"crypto/x509"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestBuildDummyIDPServerCertificate(t *testing.T) {
	tlsCert, err := buildDummyIDPServerCertificate()
	require.NoError(t, err)
	require.NotEmpty(t, tlsCert.Certificate)

	leaf, err := x509.ParseCertificate(tlsCert.Certificate[0])
	require.NoError(t, err)

	assert.False(t, leaf.IsCA, "expected generated server certificate to not be a CA")
	assert.Equal(t, "dummy-idp.default", leaf.Subject.CommonName)
	assert.Len(t, leaf.DNSNames, 2)
}

func TestHandleDiscoveryDocumentAlwaysReturnsOIDCMetadata(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name     string
		path     string
		jwksPath string
	}{
		{
			name:     "oauth authorization server metadata",
			path:     "/.well-known/oauth-authorization-server",
			jwksPath: "/.well-known/jwks.json",
		},
		{
			name:     "openid configuration metadata",
			path:     "/.well-known/openid-configuration",
			jwksPath: "/.well-known/oidc-jwks.json",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			req := httptest.NewRequest(http.MethodGet, "https://dummy-idp.default:8443"+tt.path, nil)
			rec := httptest.NewRecorder()

			handleDiscoveryDocument(rec, req, tt.jwksPath)

			var discovery map[string]any
			require.NoError(t, json.Unmarshal(rec.Body.Bytes(), &discovery))
			assert.Equal(t, http.StatusOK, rec.Code)
			assert.Equal(t, "https://dummy-idp.default:8443", discovery["issuer"])
			assert.Equal(t, "https://dummy-idp.default:8443"+tt.jwksPath, discovery["jwks_uri"])
			assert.Contains(t, discovery, "subject_types_supported")
			assert.Contains(t, discovery, "id_token_signing_alg_values_supported")
		})
	}
}
