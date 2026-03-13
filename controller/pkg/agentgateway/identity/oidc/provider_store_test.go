package oidc

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestStoreRetainsSharedSourceUntilLastOwnerIsRemoved(t *testing.T) {
	resourceKey, err := CanonicalSourceKey("https://issuer.example.com", "default", nil)
	require.NoError(t, err)

	store := &Store{
		storePrefix:  DefaultStorePrefix,
		cache:        NewProviderCache(),
		fetcher:      NewProviderFetcher(NewProviderCache()),
		cmNameToKey:  make(map[string]string),
		sourceOwners: make(map[string]map[string]ProviderSource),
	}

	sourceA := ProviderSource{
		OwnerKey:    "default/oauth2-a",
		ResourceKey: resourceKey,
		Issuer:      "https://issuer.example.com",
		RequestURL:  "https://issuer.example.com/.well-known/openid-configuration",
	}
	sourceB := ProviderSource{
		OwnerKey:    "default/oauth2-b",
		ResourceKey: resourceKey,
		Issuer:      "https://issuer.example.com",
		RequestURL:  "https://issuer.example.com/.well-known/openid-configuration",
	}

	require.NoError(t, store.applySourceChange(sourceA))
	require.NoError(t, store.applySourceChange(sourceB))
	require.Contains(t, store.fetcher.sources, resourceKey)
	require.Len(t, store.sourceOwners[resourceKey], 2)
	require.Equal(t, resourceKey, store.cmNameToKey[ProviderConfigMapName(DefaultStorePrefix, resourceKey)])

	sourceA.Deleted = true
	require.NoError(t, store.applySourceChange(sourceA))
	require.Contains(t, store.fetcher.sources, resourceKey)
	require.Len(t, store.sourceOwners[resourceKey], 1)
	require.Equal(t, resourceKey, store.cmNameToKey[ProviderConfigMapName(DefaultStorePrefix, resourceKey)])

	sourceB.Deleted = true
	require.NoError(t, store.applySourceChange(sourceB))
	require.NotContains(t, store.fetcher.sources, resourceKey)
	require.NotContains(t, store.sourceOwners, resourceKey)
	require.NotContains(t, store.cmNameToKey, ProviderConfigMapName(DefaultStorePrefix, resourceKey))
}
