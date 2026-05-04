package jwks

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test"
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

func TestPersistedEntriesLoadPrefersNewestKeysetAcrossDuplicates(t *testing.T) {
	requestKey := remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key()
	canonical := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      remotecache.ConfigMapName(DefaultJwksStorePrefix, requestKey),
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
		FetchedAt:  time.Unix(200, 0).UTC(),
		JwksJSON:   `{"keys":[{"kid":"legacy"}]}`,
	}))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{legacy, canonical}),
		DefaultJwksStorePrefix,
		"agentgateway-system",
	)
	keysets, err := persisted.LoadAll(context.Background())

	assert.NoError(t, err)
	if assert.Len(t, keysets, 1) {
		assert.Equal(t, `{"keys":[{"kid":"legacy"}]}`, keysets[0].JwksJSON)
		assert.Equal(t, time.Unix(200, 0).UTC(), keysets[0].FetchedAt)
	}
}

func TestLoadAllPrefersCanonicalEntryWhenFetchedAtTies(t *testing.T) {
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
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{legacy, canonical}),
		DefaultJwksStorePrefix,
		"agentgateway-system",
	)
	keysets, err := persisted.LoadAll(context.Background())

	assert.NoError(t, err)
	if assert.Len(t, keysets, 1) {
		assert.Equal(t, `{"keys":[{"kid":"canonical"}]}`, keysets[0].JwksJSON)
	}
}

func TestLoadAllUsesDeterministicNameTieBreakForNonCanonicalDuplicates(t *testing.T) {
	requestKey := remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key()

	olderByName := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "jwks-store-a",
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultJwksStorePrefix),
		},
	}
	assert.NoError(t, SetJwksInConfigMap(olderByName, Keyset{
		RequestKey: requestKey,
		URL:        "https://issuer.example/jwks",
		FetchedAt:  time.Unix(100, 0).UTC(),
		JwksJSON:   `{"keys":[{"kid":"a"}]}`,
	}))

	laterByName := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "jwks-store-b",
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultJwksStorePrefix),
		},
	}
	assert.NoError(t, SetJwksInConfigMap(laterByName, Keyset{
		RequestKey: requestKey,
		URL:        "https://issuer.example/jwks",
		FetchedAt:  time.Unix(100, 0).UTC(),
		JwksJSON:   `{"keys":[{"kid":"b"}]}`,
	}))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{laterByName, olderByName}),
		DefaultJwksStorePrefix,
		"agentgateway-system",
	)
	keysets, err := persisted.LoadAll(context.Background())

	assert.NoError(t, err)
	if assert.Len(t, keysets, 1) {
		assert.Equal(t, `{"keys":[{"kid":"a"}]}`, keysets[0].JwksJSON)
	}
}

func TestPersistedEntriesNormalizeLegacyStoredRequestKey(t *testing.T) {
	stop := test.NewStop(t)
	url := "https://issuer.example/jwks"
	currentRequestKey := remotehttp.FetchTarget{URL: url}.Key()
	legacyStoredRequestKey := hashString(string(currentRequestKey))
	legacyName := remotecache.ConfigMapName(DefaultJwksStorePrefix, currentRequestKey)

	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      legacyName,
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultJwksStorePrefix),
		},
	}
	assert.NoError(t, SetJwksInConfigMap(cm, Keyset{
		RequestKey: remotehttp.FetchKey(legacyStoredRequestKey),
		URL:        url,
		JwksJSON:   `{"keys":[]}`,
	}))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{cm}),
		DefaultJwksStorePrefix,
		"agentgateway-system",
	)
	persisted.Collection().WaitUntilSynced(stop)

	keyset, ok := persisted.CanonicalGet(krt.TestingDummyContext{}, currentRequestKey)

	assert.True(t, ok)
	assert.Equal(t, currentRequestKey, keyset.RequestKey)
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

func TestNormalizePersistedKeysetDoesNotRewriteWhenURLIsEmpty(t *testing.T) {
	keyset := Keyset{
		RequestKey: remotehttp.FetchKey("legacy-request-key"),
		URL:        "",
		JwksJSON:   `{"keys":[]}`,
	}

	got := normalizePersistedKeyset(DefaultJwksStorePrefix, "jwks-store-anything", keyset)

	assert.Equal(t, keyset, got)
}

func hashString(value string) string {
	sum := sha256.Sum256([]byte(value))
	return hex.EncodeToString(sum[:])
}
