package oidc

import (
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestRemoveSourceDoesNotBlockWithoutSubscriberReceiver(t *testing.T) {
	f := NewProviderFetcher(NewProviderCache())
	source := ProviderSource{
		ResourceKey: "issuer:https://issuer.example.com",
		Issuer:      "https://issuer.example.com",
		RequestURL:  "https://issuer.example.com/.well-known/openid-configuration",
		Ttl:         5 * time.Minute,
	}
	require.NoError(t, f.AddOrUpdateSource(source))
	f.SubscribeToUpdates()

	done := make(chan struct{})
	go func() {
		f.RemoveSource(source)
		close(done)
	}()

	assert.Eventually(t, func() bool {
		select {
		case <-done:
			return true
		default:
			return false
		}
	}, time.Second, 10*time.Millisecond)
}

func TestMakeProviderHTTPClientSetsTimeout(t *testing.T) {
	client := makeProviderHTTPClient(nil)
	assert.Equal(t, providerFetchTimeout, client.Timeout)
}
