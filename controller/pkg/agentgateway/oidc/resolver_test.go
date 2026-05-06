package oidc

import (
	"strings"
	"testing"

	"github.com/stretchr/testify/require"
)

func TestOidcDiscoveryURL(t *testing.T) {
	tests := []struct {
		name      string
		issuer    string
		want      string
		wantError string
	}{
		{
			name:   "https issuer without trailing slash",
			issuer: "https://idp.example",
			want:   "https://idp.example/.well-known/openid-configuration",
		},
		{
			name:   "https issuer with trailing slash",
			issuer: "https://idp.example/",
			want:   "https://idp.example/.well-known/openid-configuration",
		},
		{
			name:   "https issuer with path",
			issuer: "https://idp.example/realms/main",
			want:   "https://idp.example/realms/main/.well-known/openid-configuration",
		},
		{
			name:   "https issuer with path and trailing slash",
			issuer: "https://idp.example/realms/main/",
			want:   "https://idp.example/realms/main/.well-known/openid-configuration",
		},
		{
			name:   "http issuer allowed (scheme validated by CRD CEL)",
			issuer: "http://idp.example",
			want:   "http://idp.example/.well-known/openid-configuration",
		},
		{
			name:      "rejects empty host",
			issuer:    "https:///realms/main",
			wantError: "must be absolute with a host",
		},
		{
			name:      "rejects query string",
			issuer:    "https://idp.example/?realm=main",
			wantError: "no query or fragment",
		},
		{
			name:      "rejects fragment",
			issuer:    "https://idp.example/#main",
			wantError: "no query or fragment",
		},
		{
			name:      "rejects unparseable",
			issuer:    "https://[invalid",
			wantError: "invalid issuer URL",
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got, err := OidcDiscoveryURL(tc.issuer)
			if tc.wantError != "" {
				require.Error(t, err)
				require.Contains(t, err.Error(), tc.wantError)
				return
			}
			require.NoError(t, err)
			require.Equal(t, tc.want, got)
		})
	}
}

func TestOidcDiscoveryURLAlwaysAppendsWellKnown(t *testing.T) {
	got, err := OidcDiscoveryURL("https://idp.example/realms/main")
	require.NoError(t, err)
	require.True(t, strings.HasSuffix(got, "/.well-known/openid-configuration"),
		"discovery URL should always end with the well-known path: got %q", got)
}
