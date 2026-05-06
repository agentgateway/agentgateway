package jwks

import (
	"testing"

	"github.com/stretchr/testify/assert"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

func TestPlanConfigMapSyncKeepsCanonicalConfigMap(t *testing.T) {
	keyset := Keyset{
		RequestKey: remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key(),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[]}`,
	}

	canonicalName := remotecache.ConfigMapName(DefaultJwksStorePrefix, keyset.RequestKey)
	plan := remotecache.PlanConfigMapSync(nil, canonicalName, true)

	assert.Equal(t, canonicalName, plan.UpsertName)
	assert.Empty(t, plan.DeleteNames)
}

func TestPlanConfigMapSyncDeletesInactiveConfigMap(t *testing.T) {
	keyset := Keyset{
		RequestKey: remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key(),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[]}`,
	}
	cmName := remotecache.ConfigMapName(DefaultJwksStorePrefix, keyset.RequestKey)

	plan := remotecache.PlanConfigMapSync([]string{cmName}, "", false)

	assert.Empty(t, plan.UpsertName)
	assert.Equal(t, []string{cmName}, plan.DeleteNames)
}

func TestPlanConfigMapSyncNoopsWhenConfigMapIsAlreadyGone(t *testing.T) {
	plan := remotecache.PlanConfigMapSync(nil, "", false)

	assert.Empty(t, plan.UpsertName)
	assert.Empty(t, plan.DeleteNames)
}

func TestPlanConfigMapSyncDeletesNonCanonicalConfigMapsForActiveRequest(t *testing.T) {
	keyset := Keyset{
		RequestKey: remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key(),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[]}`,
	}
	canonicalName := remotecache.ConfigMapName(DefaultJwksStorePrefix, keyset.RequestKey)
	legacyName := "jwks-store-legacy-name"

	plan := remotecache.PlanConfigMapSync(
		[]string{canonicalName, legacyName},
		canonicalName,
		true,
	)

	assert.Equal(t, canonicalName, plan.UpsertName)
	assert.Equal(t, []string{legacyName}, plan.DeleteNames)
}

func TestPlanConfigMapSyncMigratesLegacyOnlyEntriesToCanonicalName(t *testing.T) {
	keyset := Keyset{
		RequestKey: remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key(),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[]}`,
	}
	canonicalName := remotecache.ConfigMapName(DefaultJwksStorePrefix, keyset.RequestKey)
	legacyName := "jwks-store-legacy-name"

	plan := remotecache.PlanConfigMapSync(
		[]string{legacyName},
		canonicalName,
		true,
	)

	assert.Equal(t, canonicalName, plan.UpsertName)
	assert.Equal(t, []string{legacyName}, plan.DeleteNames)
}

func TestPlanConfigMapSyncDeletesAllEntriesForInactiveRequest(t *testing.T) {
	keyset := Keyset{
		RequestKey: remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key(),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[]}`,
	}
	canonicalName := remotecache.ConfigMapName(DefaultJwksStorePrefix, keyset.RequestKey)
	legacyName := "jwks-store-legacy-name"

	plan := remotecache.PlanConfigMapSync(
		[]string{canonicalName, legacyName},
		"",
		false,
	)

	assert.Empty(t, plan.UpsertName)
	assert.Equal(t, []string{canonicalName, legacyName}, plan.DeleteNames)
}
