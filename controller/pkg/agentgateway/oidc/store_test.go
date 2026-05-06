package oidc

import (
	"context"
	"net/http"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil/krttest"
)

func TestStoreLoadsPersistedProvidersBeforeServing(t *testing.T) {
	const issuer = "https://idp.example"
	target := remotehttp.FetchTarget{URL: "https://idp.example/.well-known/openid-configuration"}
	requestKey := oidcRequestKey(target, issuer)

	persistedProvider := DiscoveredProvider{
		RequestKey:            requestKey,
		IssuerURL:             issuer,
		AuthorizationEndpoint: "https://idp.example/auth",
		TokenEndpoint:         "https://idp.example/token",
		JwksURI:               "https://idp.example/jwks",
		JwksInline:            `{"keys":[]}`,
		FetchedAt:             time.Unix(100, 0).UTC(),
	}
	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      remotecache.ConfigMapName(DefaultStorePrefix, requestKey),
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultStorePrefix),
		},
	}
	require.NoError(t, SetProviderInConfigMap(cm, persistedProvider))

	krtOpts := krttest.KrtOptions(t)
	policies := krttest.NewStaticCollection(t, []*agentgateway.AgentgatewayPolicy{
		testOidcPolicy("p1", 30*time.Minute),
	}, krtOpts, "OidcPoliciesHydrate")

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		KrtOpts:              krtOpts,
	})

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](krttest.AlwaysSynced{}, []*corev1.ConfigMap{cm}),
		DefaultStorePrefix,
		"agentgateway-system",
	)

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	store := NewStore(collections.SharedRequests, persisted, DefaultStorePrefix)
	store.Fetcher.Driver.(*OidcDriver).DefaultClient = &http.Client{Transport: krttest.OfflineTransport{}}

	go func() {
		_ = store.Start(ctx)
	}()
	require.Eventually(t, store.HasSynced, krttest.EventuallyTimeout, krttest.EventuallyPoll)

	got, ok := store.ProviderByRequestKey(requestKey)
	require.True(t, ok, "persisted provider must be served from cache before any live fetch")
	require.Equal(t, persistedProvider.IssuerURL, got.IssuerURL)
	require.Equal(t, persistedProvider.JwksURI, got.JwksURI)
}

func TestStoreClearsCacheWhenLastPolicyDeleted(t *testing.T) {
	const issuer = "https://idp.example"
	target := remotehttp.FetchTarget{URL: "https://idp.example/.well-known/openid-configuration"}
	requestKey := oidcRequestKey(target, issuer)

	persistedProvider := DiscoveredProvider{
		RequestKey:            requestKey,
		IssuerURL:             issuer,
		AuthorizationEndpoint: "https://idp.example/auth",
		TokenEndpoint:         "https://idp.example/token",
		JwksURI:               "https://idp.example/jwks",
		JwksInline:            `{"keys":[]}`,
		FetchedAt:             time.Unix(100, 0).UTC(),
	}
	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      remotecache.ConfigMapName(DefaultStorePrefix, requestKey),
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultStorePrefix),
		},
	}
	require.NoError(t, SetProviderInConfigMap(cm, persistedProvider))

	krtOpts := krttest.KrtOptions(t)
	policies := krttest.NewStaticCollection(t, []*agentgateway.AgentgatewayPolicy{
		testOidcPolicy("p1", 30*time.Minute),
	}, krtOpts, "OidcPoliciesDelete")

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		KrtOpts:              krtOpts,
	})

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](krttest.AlwaysSynced{}, []*corev1.ConfigMap{cm}),
		DefaultStorePrefix,
		"agentgateway-system",
	)

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	store := NewStore(collections.SharedRequests, persisted, DefaultStorePrefix)
	store.Fetcher.Driver.(*OidcDriver).DefaultClient = &http.Client{Transport: krttest.OfflineTransport{}}

	go func() {
		_ = store.Start(ctx)
	}()
	require.Eventually(t, store.HasSynced, krttest.EventuallyTimeout, krttest.EventuallyPoll)
	require.EventuallyWithT(t, func(c *assert.CollectT) {
		assert.Equal(c, 1, store.Fetcher.RequestCountForTest())
	}, krttest.EventuallyTimeout, krttest.EventuallyPoll)

	_, ok := store.ProviderByRequestKey(requestKey)
	require.True(t, ok, "cache should be hydrated before policy deletion")

	policies.Reset(nil)

	require.EventuallyWithT(t, func(c *assert.CollectT) {
		_, ok := store.ProviderByRequestKey(requestKey)
		assert.False(c, ok)
	}, krttest.EventuallyTimeout, krttest.EventuallyPoll)
}

func TestStoreClearsOrphanCacheAtStartup(t *testing.T) {
	const issuer = "https://idp.example"
	target := remotehttp.FetchTarget{URL: "https://idp.example/.well-known/openid-configuration"}
	requestKey := oidcRequestKey(target, issuer)

	orphanProvider := DiscoveredProvider{
		RequestKey:            requestKey,
		IssuerURL:             issuer,
		AuthorizationEndpoint: "https://idp.example/auth",
		TokenEndpoint:         "https://idp.example/token",
		JwksURI:               "https://idp.example/jwks",
		JwksInline:            `{"keys":[]}`,
		FetchedAt:             time.Unix(100, 0).UTC(),
	}
	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      remotecache.ConfigMapName(DefaultStorePrefix, requestKey),
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultStorePrefix),
		},
	}
	require.NoError(t, SetProviderInConfigMap(cm, orphanProvider))

	krtOpts := krttest.KrtOptions(t)
	policies := krttest.NewStaticCollection[*agentgateway.AgentgatewayPolicy](t, nil, krtOpts, "OidcPoliciesOrphan")

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		KrtOpts:              krtOpts,
	})

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](krttest.AlwaysSynced{}, []*corev1.ConfigMap{cm}),
		DefaultStorePrefix,
		"agentgateway-system",
	)

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	store := NewStore(collections.SharedRequests, persisted, DefaultStorePrefix)
	store.Fetcher.Driver.(*OidcDriver).DefaultClient = &http.Client{Transport: krttest.OfflineTransport{}}

	go func() {
		_ = store.Start(ctx)
	}()
	require.Eventually(t, store.HasSynced, krttest.EventuallyTimeout, krttest.EventuallyPoll)

	require.EventuallyWithT(t, func(c *assert.CollectT) {
		_, ok := store.ProviderByRequestKey(requestKey)
		assert.False(c, ok, "orphan cache entry should be cleared after sync")
	}, krttest.EventuallyTimeout, krttest.EventuallyPoll)
}
