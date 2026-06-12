package jwks

import (
	"testing"
	"time"

	"github.com/stretchr/testify/require"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

var testFetchedAt = time.Unix(1_700_000_000, 0).UTC()

func TestKeysetResourceName(t *testing.T) {
	k := Keyset{
		RequestKey: remotehttp.FetchKey("fetch-key-abc"),
		URL:        "https://idp.example/jwks",
		FetchedAt:  testFetchedAt,
		JwksJSON:   `{"keys":[]}`,
	}
	require.Equal(t, "fetch-key-abc", k.ResourceName())
}

func TestKeysetEquals(t *testing.T) {
	base := Keyset{
		RequestKey: remotehttp.FetchKey("https://issuer.example/jwks"),
		URL:        "https://issuer.example/jwks",
		FetchedAt:  testFetchedAt,
		JwksJSON:   `{"keys":[{"kid":"a"}]}`,
	}

	tests := []struct {
		name   string
		mutate func(*Keyset)
		equal  bool
	}{
		{"identical", func(*Keyset) {}, true},
		{"different request key", func(k *Keyset) { k.RequestKey = "other" }, false},
		{"different url", func(k *Keyset) { k.URL = "https://other.example/jwks" }, false},
		{"different jwks json", func(k *Keyset) { k.JwksJSON = `{"keys":[]}` }, false},
		{"different fetched at", func(k *Keyset) { k.FetchedAt = k.FetchedAt.Add(time.Second) }, false},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			other := base
			tc.mutate(&other)
			require.Equal(t, tc.equal, base.Equals(other))
		})
	}
}

func TestKeysetEqualsIgnoresMonotonicClock(t *testing.T) {
	// time.Now() carries a monotonic reading; a round-trip through
	// UTC() strips it. Both values represent the same instant, so
	// Equals (which uses time.Time.Equal) must return true even
	// though `==` on the struct would not.
	now := time.Now()
	stripped := now.UTC()

	base := Keyset{
		RequestKey: remotehttp.FetchKey("https://issuer.example/jwks"),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[{"kid":"a"}]}`,
	}

	a := base
	a.FetchedAt = now
	b := base
	b.FetchedAt = stripped

	require.True(t, a.Equals(b), "same instant on different clocks should be equal")
}
