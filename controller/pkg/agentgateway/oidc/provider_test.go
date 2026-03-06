package oidc

import (
	"strings"
	"testing"
)

func TestValidateDiscoveryIssuer(t *testing.T) {
	tests := []struct {
		name             string
		configuredIssuer string
		discoveredIssuer string
		wantErrSubstring string
	}{
		{
			name:             "accepts exact match",
			configuredIssuer: "https://issuer.example.com",
			discoveredIssuer: "https://issuer.example.com",
		},
		{
			name:             "accepts trailing slash mismatch",
			configuredIssuer: "https://issuer.example.com/",
			discoveredIssuer: "https://issuer.example.com",
		},
		{
			name:             "rejects missing issuer",
			configuredIssuer: "https://issuer.example.com",
			discoveredIssuer: "",
			wantErrSubstring: "issuer is missing in discovery metadata",
		},
		{
			name:             "rejects mismatched issuer",
			configuredIssuer: "https://issuer.example.com",
			discoveredIssuer: "https://other.example.com",
			wantErrSubstring: "issuer mismatch",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := ValidateDiscoveryIssuer(tt.configuredIssuer, tt.discoveredIssuer)
			if tt.wantErrSubstring == "" {
				if err != nil {
					t.Fatalf("unexpected error: %v", err)
				}
				return
			}
			if err == nil {
				t.Fatalf("expected error containing %q", tt.wantErrSubstring)
			}
			if got := err.Error(); got == "" || !strings.Contains(got, tt.wantErrSubstring) {
				t.Fatalf("unexpected error %q, expected substring %q", got, tt.wantErrSubstring)
			}
		})
	}
}
