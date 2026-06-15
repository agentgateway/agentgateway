package oidc

import (
	"crypto/sha256"
	"crypto/tls"
	"encoding/hex"
	"encoding/json"
	"slices"
	"testing"
	"time"

	"github.com/stretchr/testify/require"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
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

// TestOidcRequestSpecEqualsDistinguishesEveryComparedField guards the embedded
// spec: every compared field must break equality, while the +noKrtEquals
// *tls.Config fields must not. A regression here silently breaks KRT cache
// invalidation, so this is the lockstep check for OidcSource/SharedOidcRequest.
func TestOidcRequestSpecEqualsDistinguishesEveryComparedField(t *testing.T) {
	shared := &tls.Config{ServerName: "shared"} //nolint:gosec // test data
	base := func() OidcSource {
		return OidcSource{
			OwnerKey: remotecache.OwnerID{Kind: remotecache.OwnerKindPolicy, Namespace: "ns", Name: "n", Path: "p"},
			oidcRequestSpec: oidcRequestSpec{
				RequestKey:     "rk",
				ExpectedIssuer: "https://idp.example",
				Target:         remotehttp.FetchTarget{URL: "https://idp.example/.well-known"},
				ViaBackendRef:  true,
				TLSConfig:      shared,
				ProxyTLSConfig: shared,
				TTL:            5 * time.Minute,
			},
		}
	}

	require.True(t, base().Equals(base()), "identical sources must be equal")

	mutators := map[string]func(*OidcSource){
		"OwnerKey":       func(s *OidcSource) { s.OwnerKey.Name = "other" },
		"RequestKey":     func(s *OidcSource) { s.RequestKey = "other" },
		"ExpectedIssuer": func(s *OidcSource) { s.ExpectedIssuer = "https://other" },
		"Target":         func(s *OidcSource) { s.Target = remotehttp.FetchTarget{URL: "https://other"} },
		"ViaBackendRef":  func(s *OidcSource) { s.ViaBackendRef = false },
		"TTL":            func(s *OidcSource) { s.TTL = time.Hour },
	}
	for name, mutate := range mutators {
		got := base()
		mutate(&got)
		require.Falsef(t, base().Equals(got), "mutating %s must break equality", name)
	}

	// *tls.Config fields are +noKrtEquals: differences must NOT affect equality.
	diffTLS := base()
	diffTLS.TLSConfig = &tls.Config{ServerName: "different"}      //nolint:gosec // test data
	diffTLS.ProxyTLSConfig = &tls.Config{ServerName: "different"} //nolint:gosec // test data
	require.True(t, base().Equals(diffTLS), "tls.Config differences must not affect equality")

	// SharedOidcRequest shares the spec: same contract, minus OwnerKey.
	sharedReq := SharedOidcRequest{base().oidcRequestSpec}
	other := sharedReq
	other.RequestKey = "other"
	require.False(t, sharedReq.Equals(other))
	require.True(t, sharedReq.Equals(SharedOidcRequest{base().oidcRequestSpec}))
}

func TestOidcRequestKeyIsDomainSeparated(t *testing.T) {
	target := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}
	expectedIssuer := "https://issuer.example"

	key := oidcRequestKey(target, expectedIssuer, false)

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

	first := oidcRequestKey(target, issuer, false)
	second := oidcRequestKey(target, issuer, false)

	require.Equal(t, first, second, "same (target, issuer) must hash identically across calls")
}

var testFetchedAt = time.Unix(1_700_000_000, 0).UTC()

func TestDiscoveredProviderResourceName(t *testing.T) {
	p := DiscoveredProvider{
		RequestKey:            remotehttp.FetchKey("fetch-key-abc"),
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksInline:            `{"keys":[]}`,
		FetchedAt:             testFetchedAt,
	}
	require.Equal(t, "fetch-key-abc", p.ResourceName())
}

func TestDiscoveredProviderEquals(t *testing.T) {
	base := DiscoveredProvider{
		RequestKey:                        remotehttp.FetchKey("fetch-key-abc"),
		IssuerURL:                         "https://issuer.example",
		AuthorizationEndpoint:             "https://issuer.example/auth",
		TokenEndpoint:                     "https://issuer.example/token",
		JwksURI:                           "https://issuer.example/jwks",
		JwksInline:                        `{"keys":[{"kid":"a"}]}`,
		TokenEndpointAuthMethodsSupported: []string{"client_secret_basic", "client_secret_post"},
		FetchedAt:                         testFetchedAt,
	}

	tests := []struct {
		name   string
		mutate func(*DiscoveredProvider)
		equal  bool
	}{
		{"identical", func(*DiscoveredProvider) {}, true},
		{"different request key", func(p *DiscoveredProvider) { p.RequestKey = "other" }, false},
		{"different issuer url", func(p *DiscoveredProvider) { p.IssuerURL = "https://other.example" }, false},
		{"different authorization endpoint", func(p *DiscoveredProvider) {
			p.AuthorizationEndpoint = "https://other.example/auth"
		}, false},
		{"different token endpoint", func(p *DiscoveredProvider) {
			p.TokenEndpoint = "https://other.example/token"
		}, false},
		{"different jwks uri", func(p *DiscoveredProvider) { p.JwksURI = "https://other.example/jwks" }, false},
		{"different jwks inline", func(p *DiscoveredProvider) { p.JwksInline = `{"keys":[]}` }, false},
		{"different fetched at", func(p *DiscoveredProvider) { p.FetchedAt = p.FetchedAt.Add(time.Second) }, false},
		{"different token endpoint auth methods supported", func(p *DiscoveredProvider) {
			p.TokenEndpointAuthMethodsSupported = []string{"private_key_jwt"}
		}, false},
		// slices.Equal is order-sensitive: a permutation of the same elements
		// must compare unequal so a re-ordered IdP response still propagates as
		// a change event.
		{"same auth methods different order", func(p *DiscoveredProvider) {
			p.TokenEndpointAuthMethodsSupported = []string{"client_secret_post", "client_secret_basic"}
		}, false},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			other := base
			// Deep-copy the slice so per-case mutations don't leak across cases.
			other.TokenEndpointAuthMethodsSupported = slices.Clone(base.TokenEndpointAuthMethodsSupported)
			tc.mutate(&other)
			require.Equal(t, tc.equal, base.Equals(other))
		})
	}
}

func TestDiscoveredProviderEqualsTreatsNilAndEmptyAuthMethodsAsEqual(t *testing.T) {
	a := DiscoveredProvider{
		RequestKey:                        "fk",
		TokenEndpointAuthMethodsSupported: nil,
	}
	b := DiscoveredProvider{
		RequestKey:                        "fk",
		TokenEndpointAuthMethodsSupported: []string{},
	}
	require.True(t, a.Equals(b), "nil and empty slice should be equal so KRT does not churn when an IdP fluctuates between omitting and emitting an empty array")
}

func TestDiscoveredProviderEqualsIgnoresMonotonicClock(t *testing.T) {
	// time.Now() carries a monotonic reading; a round-trip through
	// UTC() strips it. Both values represent the same instant, so
	// Equals (which uses time.Time.Equal) must return true even
	// though `==` on the struct would not.
	now := time.Now()
	stripped := now.UTC()

	base := DiscoveredProvider{
		RequestKey:            remotehttp.FetchKey("fetch-key-abc"),
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksInline:            `{"keys":[{"kid":"a"}]}`,
	}

	a := base
	a.FetchedAt = now
	b := base
	b.FetchedAt = stripped

	require.True(t, a.Equals(b), "same instant on different clocks should be equal")
}

func TestOidcRequestKeyDifferentiatesInputs(t *testing.T) {
	baseTarget := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}
	baseIssuer := "https://issuer.example"
	baseKey := oidcRequestKey(baseTarget, baseIssuer, false)

	tests := []struct {
		name          string
		target        remotehttp.FetchTarget
		issuer        string
		viaBackendRef bool
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
		{
			name:          "via provider backend",
			target:        baseTarget,
			issuer:        baseIssuer,
			viaBackendRef: true,
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			other := oidcRequestKey(tc.target, tc.issuer, tc.viaBackendRef)
			require.NotEqual(t, baseKey, other,
				"request keys must differ when sharing inputs differ")
		})
	}
}
