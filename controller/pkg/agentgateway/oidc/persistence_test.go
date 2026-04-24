package oidc

import (
	"context"
	"strings"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

func TestProviderFromConfigMapRejectsMalformedPayload(t *testing.T) {
	cm := &corev1.ConfigMap{
		Data: map[string]string{
			oidcConfigMapKey: "not-json",
		},
	}

	_, err := ProviderFromConfigMap(cm)

	assert.Error(t, err)
}

func TestSetAndReadConfigMapRoundTrip(t *testing.T) {
	original := DiscoveredProvider{
		RequestKey:            remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key(),
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksJSON:              `{"keys":[]}`,
	}
	cm := &corev1.ConfigMap{}

	assert.NoError(t, SetProviderInConfigMap(cm, original))
	assert.NotContains(t, cm.Data[oidcConfigMapKey], `"version"`, "persisted oidc payloads should stay versionless")

	got, err := ProviderFromConfigMap(cm)

	assert.NoError(t, err)
	assert.Equal(t, original, got)
}

func TestPersistedEntriesLoadPrefersNewestProviderAcrossDuplicates(t *testing.T) {
	requestKey := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key()
	canonical := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      OidcConfigMapName(DefaultStorePrefix, requestKey),
			Namespace: "agentgateway-system",
			Labels:    OidcStoreConfigMapLabel(DefaultStorePrefix),
		},
	}
	assert.NoError(t, SetProviderInConfigMap(canonical, DiscoveredProvider{
		RequestKey:            requestKey,
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksJSON:              `{"keys":[{"kid":"canonical"}]}`,
		FetchedAt:             time.Unix(100, 0).UTC(),
	}))

	legacy := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "oidc-store-legacy-name",
			Namespace: "agentgateway-system",
			Labels:    OidcStoreConfigMapLabel(DefaultStorePrefix),
		},
	}
	assert.NoError(t, SetProviderInConfigMap(legacy, DiscoveredProvider{
		RequestKey:            requestKey,
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksJSON:              `{"keys":[{"kid":"legacy"}]}`,
		FetchedAt:             time.Unix(200, 0).UTC(),
	}))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{legacy, canonical}),
		DefaultStorePrefix,
		"agentgateway-system",
	)
	reader := newPersistedProviderReader(persisted)

	providers, err := reader.LoadPersistedProviders(context.Background())

	assert.NoError(t, err)
	if assert.Len(t, providers, 1) {
		assert.Equal(t, `{"keys":[{"kid":"legacy"}]}`, providers[0].JwksJSON)
		assert.Equal(t, time.Unix(200, 0).UTC(), providers[0].FetchedAt)
	}
}

func TestLoadPersistedProvidersPrefersCanonicalEntryWhenFetchedAtTies(t *testing.T) {
	requestKey := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key()
	canonicalName := OidcConfigMapName(DefaultStorePrefix, requestKey)

	canonical := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      canonicalName,
			Namespace: "agentgateway-system",
			Labels:    OidcStoreConfigMapLabel(DefaultStorePrefix),
		},
	}
	assert.NoError(t, SetProviderInConfigMap(canonical, DiscoveredProvider{
		RequestKey:            requestKey,
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksJSON:              `{"keys":[{"kid":"canonical"}]}`,
		FetchedAt:             time.Unix(100, 0).UTC(),
	}))

	legacy := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "oidc-store-legacy-name",
			Namespace: "agentgateway-system",
			Labels:    OidcStoreConfigMapLabel(DefaultStorePrefix),
		},
	}
	assert.NoError(t, SetProviderInConfigMap(legacy, DiscoveredProvider{
		RequestKey:            requestKey,
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksJSON:              `{"keys":[{"kid":"legacy"}]}`,
		FetchedAt:             time.Unix(100, 0).UTC(),
	}))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{legacy, canonical}),
		DefaultStorePrefix,
		"agentgateway-system",
	)
	reader := newPersistedProviderReader(persisted)

	providers, err := reader.LoadPersistedProviders(context.Background())

	assert.NoError(t, err)
	if assert.Len(t, providers, 1) {
		assert.Equal(t, `{"keys":[{"kid":"canonical"}]}`, providers[0].JwksJSON)
	}
}

func TestLoadPersistedProvidersUsesDeterministicNameTieBreakForNonCanonicalDuplicates(t *testing.T) {
	requestKey := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key()

	earlierByName := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "oidc-store-a",
			Namespace: "agentgateway-system",
			Labels:    OidcStoreConfigMapLabel(DefaultStorePrefix),
		},
	}
	assert.NoError(t, SetProviderInConfigMap(earlierByName, DiscoveredProvider{
		RequestKey:            requestKey,
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksJSON:              `{"keys":[{"kid":"a"}]}`,
		FetchedAt:             time.Unix(100, 0).UTC(),
	}))

	laterByName := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "oidc-store-b",
			Namespace: "agentgateway-system",
			Labels:    OidcStoreConfigMapLabel(DefaultStorePrefix),
		},
	}
	assert.NoError(t, SetProviderInConfigMap(laterByName, DiscoveredProvider{
		RequestKey:            requestKey,
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksJSON:              `{"keys":[{"kid":"b"}]}`,
		FetchedAt:             time.Unix(100, 0).UTC(),
	}))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{laterByName, earlierByName}),
		DefaultStorePrefix,
		"agentgateway-system",
	)
	reader := newPersistedProviderReader(persisted)

	providers, err := reader.LoadPersistedProviders(context.Background())

	assert.NoError(t, err)
	if assert.Len(t, providers, 1) {
		assert.Equal(t, `{"keys":[{"kid":"a"}]}`, providers[0].JwksJSON)
	}
}

func TestPersistedEntriesNormalizeStoredRequestKeyFromIssuerURL(t *testing.T) {
	stop := test.NewStop(t)
	issuerURL := "https://issuer.example"
	discoveryURL := "https://issuer.example/.well-known/openid-configuration"
	currentRequestKey := testOidcRequestKey(discoveryURL)

	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      OidcConfigMapName(DefaultStorePrefix, currentRequestKey),
			Namespace: "agentgateway-system",
			Labels:    OidcStoreConfigMapLabel(DefaultStorePrefix),
		},
	}
	assert.NoError(t, SetProviderInConfigMap(cm, DiscoveredProvider{
		RequestKey:            remotehttp.FetchKey("stale-request-key"),
		IssuerURL:             issuerURL,
		AuthorizationEndpoint: issuerURL + "/auth",
		TokenEndpoint:         issuerURL + "/token",
		JwksURI:               issuerURL + "/jwks",
		JwksJSON:              `{"keys":[]}`,
	}))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{cm}),
		DefaultStorePrefix,
		"agentgateway-system",
	)
	persisted.entries.WaitUntilSynced(stop)
	cache := newProviderCache(persisted)

	provider, ok := cache.Get(krt.TestingDummyContext{}, currentRequestKey)

	assert.True(t, ok)
	assert.Equal(t, currentRequestKey, provider.RequestKey)
}

func TestRequestKeyFromConfigMapReturnsErrorForMalformedPayload(t *testing.T) {
	cm := &corev1.ConfigMap{
		Data: map[string]string{
			oidcConfigMapKey: "not-json",
		},
	}

	_, err := RequestKeyFromConfigMap(cm)

	assert.Error(t, err)
}

func TestSetProviderInConfigMapOmitsVersionField(t *testing.T) {
	cm := &corev1.ConfigMap{}

	err := SetProviderInConfigMap(cm, DiscoveredProvider{
		RequestKey: remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key(),
		IssuerURL:  "https://issuer.example",
	})

	assert.NoError(t, err)
	assert.False(t, strings.Contains(cm.Data[oidcConfigMapKey], `"version"`))
}
