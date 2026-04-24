package oidc

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

func TestPlanConfigMapSyncKeepsCanonicalConfigMap(t *testing.T) {
	provider := DiscoveredProvider{
		RequestKey:            remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key(),
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksJSON:              `{"keys":[]}`,
	}
	plan := planConfigMapSync(provider.RequestKey, nil, DefaultStorePrefix, func(requestKey remotehttp.FetchKey) (DiscoveredProvider, bool) {
		if requestKey == provider.RequestKey {
			return provider, true
		}
		return DiscoveredProvider{}, false
	})

	if assert.NotNil(t, plan.provider) {
		assert.Equal(t, provider, *plan.provider)
	}
	assert.Equal(t, OidcConfigMapName(DefaultStorePrefix, provider.RequestKey), plan.upsertName)
	assert.Empty(t, plan.deleteNames)
}

func TestPlanConfigMapSyncDeletesInactiveConfigMap(t *testing.T) {
	provider := DiscoveredProvider{
		RequestKey:            remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key(),
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksJSON:              `{"keys":[]}`,
	}
	cmName := OidcConfigMapName(DefaultStorePrefix, provider.RequestKey)
	existingEntry := persistedEntryWithProvider(cmName, provider)

	plan := planConfigMapSync(provider.RequestKey, []PersistedEntry{existingEntry}, DefaultStorePrefix, func(remotehttp.FetchKey) (DiscoveredProvider, bool) {
		return DiscoveredProvider{}, false
	})

	assert.Nil(t, plan.provider)
	assert.Empty(t, plan.upsertName)
	assert.Equal(t, []string{cmName}, plan.deleteNames)
}

func TestPlanConfigMapSyncNoopsWhenConfigMapIsAlreadyGone(t *testing.T) {
	requestKey := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key()

	plan := planConfigMapSync(requestKey, nil, DefaultStorePrefix, func(remotehttp.FetchKey) (DiscoveredProvider, bool) {
		return DiscoveredProvider{}, false
	})

	assert.Nil(t, plan.provider)
	assert.Empty(t, plan.upsertName)
	assert.Empty(t, plan.deleteNames)
}

func TestPlanConfigMapSyncDeletesNonCanonicalConfigMapsForActiveRequest(t *testing.T) {
	provider := DiscoveredProvider{
		RequestKey:            remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key(),
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksJSON:              `{"keys":[]}`,
	}
	canonicalName := OidcConfigMapName(DefaultStorePrefix, provider.RequestKey)
	legacyName := "oidc-store-legacy-name"
	plan := planConfigMapSync(
		provider.RequestKey,
		[]PersistedEntry{
			persistedEntryWithProvider(canonicalName, provider),
			persistedEntryWithProvider(legacyName, provider),
		},
		DefaultStorePrefix,
		func(requestKey remotehttp.FetchKey) (DiscoveredProvider, bool) {
			if requestKey == provider.RequestKey {
				return provider, true
			}
			return DiscoveredProvider{}, false
		},
	)

	if assert.NotNil(t, plan.provider) {
		assert.Equal(t, provider, *plan.provider)
	}
	assert.Equal(t, canonicalName, plan.upsertName)
	assert.Equal(t, []string{legacyName}, plan.deleteNames)
}

func TestPlanConfigMapSyncMigratesLegacyOnlyEntriesToCanonicalName(t *testing.T) {
	provider := DiscoveredProvider{
		RequestKey:            remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key(),
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksJSON:              `{"keys":[]}`,
	}
	canonicalName := OidcConfigMapName(DefaultStorePrefix, provider.RequestKey)
	legacyName := "oidc-store-legacy-name"

	plan := planConfigMapSync(
		provider.RequestKey,
		[]PersistedEntry{
			persistedEntryWithProvider(legacyName, provider),
		},
		DefaultStorePrefix,
		func(requestKey remotehttp.FetchKey) (DiscoveredProvider, bool) {
			if requestKey == provider.RequestKey {
				return provider, true
			}
			return DiscoveredProvider{}, false
		},
	)

	if assert.NotNil(t, plan.provider) {
		assert.Equal(t, provider, *plan.provider)
	}
	assert.Equal(t, canonicalName, plan.upsertName)
	assert.Equal(t, []string{legacyName}, plan.deleteNames)
}

func TestPlanConfigMapSyncDeletesAllEntriesForInactiveRequest(t *testing.T) {
	provider := DiscoveredProvider{
		RequestKey:            remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key(),
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksJSON:              `{"keys":[]}`,
	}
	canonicalName := OidcConfigMapName(DefaultStorePrefix, provider.RequestKey)
	legacyName := "oidc-store-legacy-name"

	plan := planConfigMapSync(
		provider.RequestKey,
		[]PersistedEntry{
			persistedEntryWithProvider(canonicalName, provider),
			persistedEntryWithProvider(legacyName, provider),
		},
		DefaultStorePrefix,
		func(remotehttp.FetchKey) (DiscoveredProvider, bool) {
			return DiscoveredProvider{}, false
		},
	)

	assert.Nil(t, plan.provider)
	assert.Empty(t, plan.upsertName)
	assert.Equal(t, []string{canonicalName, legacyName}, plan.deleteNames)
}

func persistedEntryWithProvider(name string, provider DiscoveredProvider) PersistedEntry {
	return PersistedEntry{
		NamespacedName: types.NamespacedName{
			Name:      name,
			Namespace: "agentgateway-system",
		},
		Provider: &provider,
	}
}
