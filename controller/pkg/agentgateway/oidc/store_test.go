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
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

func TestStoreLoadsPersistedProvidersBeforeServing(t *testing.T) {
	const issuer = "https://idp.example"
	target := remotehttp.FetchTarget{URL: "https://idp.example/.well-known/openid-configuration"}
	requestKey := oidcRequestKey(target, issuer, false)

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

	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	policies := krt.NewStaticCollection(nil, []*agentgateway.AgentgatewayPolicy{
		testOidcPolicy("p1"),
	}, krtOpts.ToOptions("OidcPoliciesHydrate")...)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Resolver:             NewResolver(nil),
		KrtOpts:              krtOpts,
	})

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](nil, []*corev1.ConfigMap{cm}),
		"agentgateway-system",
	)

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	store := NewStore(collections.SharedRequests, persisted)
	store.Driver.DefaultClient = &http.Client{Transport: offlineTransport{}}

	go func() {
		_ = store.Start(ctx)
	}()
	require.Eventually(t, store.HasSynced, eventuallyTimeout, eventuallyPoll)

	got, ok := store.FetchedResults().Get(requestKey)
	require.True(t, ok, "persisted provider must be served from cache before any live fetch")
	require.Equal(t, persistedProvider.IssuerURL, got.IssuerURL)
	require.Equal(t, persistedProvider.JwksURI, got.JwksURI)
}

func TestStoreClearsCacheWhenLastPolicyDeleted(t *testing.T) {
	const issuer = "https://idp.example"
	target := remotehttp.FetchTarget{URL: "https://idp.example/.well-known/openid-configuration"}
	requestKey := oidcRequestKey(target, issuer, false)

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

	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	policies := krt.NewStaticCollection(nil, []*agentgateway.AgentgatewayPolicy{
		testOidcPolicy("p1"),
	}, krtOpts.ToOptions("OidcPoliciesDelete")...)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Resolver:             NewResolver(nil),
		KrtOpts:              krtOpts,
	})

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](nil, []*corev1.ConfigMap{cm}),
		"agentgateway-system",
	)

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	store := NewStore(collections.SharedRequests, persisted)
	store.Driver.DefaultClient = &http.Client{Transport: offlineTransport{}}

	go func() {
		_ = store.Start(ctx)
	}()
	require.Eventually(t, store.HasSynced, eventuallyTimeout, eventuallyPoll)
	require.EventuallyWithT(t, func(c *assert.CollectT) {
		assert.Equal(c, 1, store.Fetcher.RequestCountForTest())
	}, eventuallyTimeout, eventuallyPoll)

	_, ok := store.FetchedResults().Get(requestKey)
	require.True(t, ok, "cache should be hydrated before policy deletion")

	policies.Reset(nil)

	require.EventuallyWithT(t, func(c *assert.CollectT) {
		_, ok := store.FetchedResults().Get(requestKey)
		assert.False(c, ok)
	}, eventuallyTimeout, eventuallyPoll)
}

func TestStoreClearsOrphanCacheAtStartup(t *testing.T) {
	const issuer = "https://idp.example"
	target := remotehttp.FetchTarget{URL: "https://idp.example/.well-known/openid-configuration"}
	requestKey := oidcRequestKey(target, issuer, false)

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

	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	policies := krt.NewStaticCollection[*agentgateway.AgentgatewayPolicy](nil, nil, krtOpts.ToOptions("OidcPoliciesOrphan")...)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Resolver:             NewResolver(nil),
		KrtOpts:              krtOpts,
	})

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](nil, []*corev1.ConfigMap{cm}),
		"agentgateway-system",
	)

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	store := NewStore(collections.SharedRequests, persisted)
	store.Driver.DefaultClient = &http.Client{Transport: offlineTransport{}}

	go func() {
		_ = store.Start(ctx)
	}()
	require.Eventually(t, store.HasSynced, eventuallyTimeout, eventuallyPoll)

	require.EventuallyWithT(t, func(c *assert.CollectT) {
		_, ok := store.FetchedResults().Get(requestKey)
		assert.False(c, ok, "orphan cache entry should be cleared after sync")
	}, eventuallyTimeout, eventuallyPoll)
}
