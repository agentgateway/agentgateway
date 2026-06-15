package remotehttp

import (
	"strings"
	"testing"

	"github.com/stretchr/testify/require"
)

func TestValidateJWKSBodyAcceptsValidKeyset(t *testing.T) {
	keyset, err := validateJWKSBody([]byte(`{"keys":[{"kty":"oct","k":"c2VjcmV0"}]}`), "https://idp.example/jwks", "JWKS")

	require.NoError(t, err)
	require.Len(t, keyset.Keys, 1)
}

func TestValidateJWKSBodyRejectsMalformedJSON(t *testing.T) {
	_, err := validateJWKSBody([]byte(`not-json`), "https://idp.example/jwks", "OIDC JWKS")

	require.Error(t, err)
	require.Contains(t, err.Error(), "OIDC JWKS response from https://idp.example/jwks is not a valid JWKS document")
}

func TestValidateJWKSBodyRejectsEmptyKeyset(t *testing.T) {
	_, err := validateJWKSBody([]byte(`{"keys":[]}`), "https://idp.example/jwks", "JWKS")

	require.Error(t, err)
	require.Contains(t, err.Error(), "JWKS response from https://idp.example/jwks contains no keys")
}

func TestValidateJWKSBodyRejectsNonJWKSJSON(t *testing.T) {
	_, err := validateJWKSBody([]byte(`{"error":"temporarily_unavailable"}`), "https://idp.example/jwks", "JWKS")

	require.Error(t, err)
	require.Contains(t, err.Error(), "contains no keys")
}

func TestValidateJWKSBodyErrorIncludesSourceDescription(t *testing.T) {
	_, err := validateJWKSBody([]byte(`{"keys":[]}`), "https://idp.example/oidc/jwks", "OIDC JWKS")

	require.Error(t, err)
	require.True(t, strings.Contains(err.Error(), "OIDC JWKS"))
	require.True(t, strings.Contains(err.Error(), "https://idp.example/oidc/jwks"))
}
