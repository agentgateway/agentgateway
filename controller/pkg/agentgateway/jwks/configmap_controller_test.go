package jwks

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

func TestPlanConfigMapSyncKeepsCanonicalConfigMap(t *testing.T) {
	keyset := Keyset{
		RequestKey: remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key(),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[]}`,
	}
	plan := planConfigMapSync(keyset.RequestKey, nil, DefaultJwksStorePrefix, func(requestKey remotehttp.FetchKey) (Keyset, bool) {
		if requestKey == keyset.RequestKey {
			return keyset, true
		}
		return Keyset{}, false
	})

	if assert.NotNil(t, plan.keyset) {
		assert.Equal(t, keyset, *plan.keyset)
	}
	assert.Equal(t, JwksConfigMapName(DefaultJwksStorePrefix, keyset.RequestKey), plan.upsertName)
	assert.Empty(t, plan.deleteNames)
}

func TestPlanConfigMapSyncDeletesInactiveConfigMap(t *testing.T) {
	keyset := Keyset{
		RequestKey: remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key(),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[]}`,
	}
	cmName := JwksConfigMapName(DefaultJwksStorePrefix, keyset.RequestKey)
	existingEntry := persistedEntryWithKeyset(cmName, "agentgateway-system", keyset)

	plan := planConfigMapSync(keyset.RequestKey, []PersistedEntry{existingEntry}, DefaultJwksStorePrefix, func(remotehttp.FetchKey) (Keyset, bool) {
		return Keyset{}, false
	})

	assert.Nil(t, plan.keyset)
	assert.Empty(t, plan.upsertName)
	assert.Equal(t, []string{cmName}, plan.deleteNames)
}

func TestPlanConfigMapSyncNoopsWhenConfigMapIsAlreadyGone(t *testing.T) {
	requestKey := remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key()

	plan := planConfigMapSync(requestKey, nil, DefaultJwksStorePrefix, func(remotehttp.FetchKey) (Keyset, bool) {
		return Keyset{}, false
	})

	assert.Nil(t, plan.keyset)
	assert.Empty(t, plan.upsertName)
	assert.Empty(t, plan.deleteNames)
}

func TestPlanConfigMapSyncDeletesNonCanonicalConfigMapsForActiveRequest(t *testing.T) {
	keyset := Keyset{
		RequestKey: remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key(),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[]}`,
	}
	canonicalName := JwksConfigMapName(DefaultJwksStorePrefix, keyset.RequestKey)
	legacyName := "jwks-store-legacy-name"
	plan := planConfigMapSync(
		keyset.RequestKey,
		[]PersistedEntry{
			persistedEntryWithKeyset(canonicalName, "agentgateway-system", keyset),
			persistedEntryWithKeyset(legacyName, "agentgateway-system", keyset),
		},
		DefaultJwksStorePrefix,
		func(requestKey remotehttp.FetchKey) (Keyset, bool) {
			if requestKey == keyset.RequestKey {
				return keyset, true
			}
			return Keyset{}, false
		},
	)

	if assert.NotNil(t, plan.keyset) {
		assert.Equal(t, keyset, *plan.keyset)
	}
	assert.Equal(t, canonicalName, plan.upsertName)
	assert.Equal(t, []string{legacyName}, plan.deleteNames)
}

func persistedEntryWithKeyset(name, namespace string, keyset Keyset) PersistedEntry {
	return PersistedEntry{
		NamespacedName: types.NamespacedName{
			Name:      name,
			Namespace: namespace,
		},
		Keyset: &keyset,
	}
}
