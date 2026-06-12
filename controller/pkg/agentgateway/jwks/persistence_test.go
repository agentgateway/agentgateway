package jwks

import (
	"context"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

func TestJwksFromConfigMapAcceptsLegacyPayload(t *testing.T) {
	cm := &corev1.ConfigMap{
		Data: map[string]string{
			configMapKey: `{"https://issuer.example/jwks":"{\"keys\":[]}"}`,
		},
	}

	keyset, err := JwksFromConfigMap(cm)

	assert.NoError(t, err)
	assert.Equal(t, "https://issuer.example/jwks", keyset.URL)
	assert.Equal(t, remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key(), keyset.RequestKey)
}

func TestJwksFromConfigMapRejectsMultiEntryLegacyPayload(t *testing.T) {
	cm := &corev1.ConfigMap{
		Data: map[string]string{
			configMapKey: `{"https://a.example/jwks":"{\"keys\":[]}","https://b.example/jwks":"{\"keys\":[]}"}`,
		},
	}

	_, err := JwksFromConfigMap(cm)

	assert.Error(t, err)
	assert.Contains(t, err.Error(), "expected 1 entry, got 2")
}

func TestJwksFromConfigMapRejectsEmptyLegacyPayload(t *testing.T) {
	cm := &corev1.ConfigMap{
		Data: map[string]string{
			configMapKey: `{}`,
		},
	}

	_, err := JwksFromConfigMap(cm)

	assert.Error(t, err)
	assert.Contains(t, err.Error(), "expected 1 entry, got 0")
}

func TestSetAndReadConfigMapRoundTrip(t *testing.T) {
	original := Keyset{
		RequestKey: remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key(),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[]}`,
	}
	cm := &corev1.ConfigMap{}

	assert.NoError(t, SetJwksInConfigMap(cm, original))

	got, err := JwksFromConfigMap(cm)

	assert.NoError(t, err)
	assert.Equal(t, original.RequestKey, got.RequestKey)
	assert.Equal(t, original.URL, got.URL)
	assert.Equal(t, original.JwksJSON, got.JwksJSON)
}

func TestLoadAllPrefersCanonicalAcrossDuplicates(t *testing.T) {
	requestKey := remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key()
	canonicalName := remotecache.ConfigMapName(DefaultJwksStorePrefix, requestKey)

	canonical := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      canonicalName,
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultJwksStorePrefix),
		},
	}
	assert.NoError(t, SetJwksInConfigMap(canonical, Keyset{
		RequestKey: requestKey,
		URL:        "https://issuer.example/jwks",
		FetchedAt:  time.Unix(100, 0).UTC(),
		JwksJSON:   `{"keys":[{"kid":"canonical"}]}`,
	}))

	legacy := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "jwks-store-legacy-name",
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultJwksStorePrefix),
		},
	}
	assert.NoError(t, SetJwksInConfigMap(legacy, Keyset{
		RequestKey: requestKey,
		URL:        "https://issuer.example/jwks",
		FetchedAt:  time.Unix(100, 0).UTC(),
		JwksJSON:   `{"keys":[{"kid":"legacy"}]}`,
	}))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{legacy, canonical}, krt.WithName("jwks/PersistedKeysetsCanonicalTieConfigMaps")),
		"agentgateway-system",
	)
	keysets, err := persisted.LoadAll(context.Background())

	assert.NoError(t, err)
	if assert.Len(t, keysets, 1) {
		assert.Equal(t, `{"keys":[{"kid":"canonical"}]}`, keysets[0].JwksJSON)
	}
}

func TestLoadAllPrefersFreshestAcrossDuplicates(t *testing.T) {
	requestKey := remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key()
	canonicalName := remotecache.ConfigMapName(DefaultJwksStorePrefix, requestKey)

	canonical := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      canonicalName,
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultJwksStorePrefix),
		},
	}
	assert.NoError(t, SetJwksInConfigMap(canonical, Keyset{
		RequestKey: requestKey,
		URL:        "https://issuer.example/jwks",
		FetchedAt:  time.Unix(100, 0).UTC(),
		JwksJSON:   `{"keys":[{"kid":"stale-canonical"}]}`,
	}))

	legacy := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "jwks-store-legacy-name",
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultJwksStorePrefix),
		},
	}
	assert.NoError(t, SetJwksInConfigMap(legacy, Keyset{
		RequestKey: requestKey,
		URL:        "https://issuer.example/jwks",
		FetchedAt:  time.Unix(200, 0).UTC(),
		JwksJSON:   `{"keys":[{"kid":"fresh-legacy"}]}`,
	}))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{canonical, legacy}, krt.WithName("jwks/PersistedKeysetsFreshestConfigMaps")),
		"agentgateway-system",
	)
	keysets, err := persisted.LoadAll(context.Background())

	assert.NoError(t, err)
	if assert.Len(t, keysets, 1) {
		assert.Equal(t, `{"keys":[{"kid":"fresh-legacy"}]}`, keysets[0].JwksJSON)
	}
}

func TestJwksFromConfigMapReturnsErrorForMalformedPayload(t *testing.T) {
	cm := &corev1.ConfigMap{
		Data: map[string]string{
			configMapKey: "not-json",
		},
	}

	_, err := JwksFromConfigMap(cm)

	assert.Error(t, err)
}
