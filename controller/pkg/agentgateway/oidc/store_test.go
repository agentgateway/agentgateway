package oidc

import (
	"context"
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

// alwaysSynced satisfies the krt.Synced interface for test collections.
type alwaysSynced struct{}

func (alwaysSynced) WaitUntilSynced(stop <-chan struct{}) bool {
	return true
}

func (alwaysSynced) HasSynced() bool {
	return true
}

func TestSharedOidcRequestsCollapseMinTTLAcrossOwners(t *testing.T) {
	krtOpts := testKrtOptions(t)
	policies := krt.NewStaticCollection(alwaysSynced{}, []*agentgateway.AgentgatewayPolicy{
		testOidcPolicy("one", "https://issuer.example", 10*time.Minute),
		testOidcPolicy("two", "https://issuer.example", 5*time.Minute),
	})

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Resolver: oidcResolverFunc(func(owner RemoteOidcOwner) (*ResolvedOidcRequest, error) {
			return resolvedOidcRequest(owner, "https://issuer.example/.well-known/openid-configuration"), nil
		}),
		KrtOpts: krtOpts,
	})

	requests := awaitSharedOidcRequests(t, collections.SharedRequests, 1)
	assert.Equal(t, testOidcRequestKey("https://issuer.example/.well-known/openid-configuration"), requests[0].RequestKey)
	assert.Equal(t, 5*time.Minute, requests[0].TTL)
}

func TestSharedOidcRequestsRetargetOwnerAcrossRequestKeys(t *testing.T) {
	krtOpts := testKrtOptions(t)
	policies := dynamicOidcPolicies(t, []*agentgateway.AgentgatewayPolicy{
		testOidcPolicy("moving", "https://issuer-a.example", 5*time.Minute),
		testOidcPolicy("staying", "https://issuer-b.example", 10*time.Minute),
	}, krtOpts)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Resolver: oidcResolverFunc(func(owner RemoteOidcOwner) (*ResolvedOidcRequest, error) {
			discoveryURL := owner.Config.IssuerURL + "/.well-known/openid-configuration"
			return resolvedOidcRequest(owner, discoveryURL), nil
		}),
		KrtOpts: krtOpts,
	})

	requests := awaitSharedOidcRequests(t, collections.SharedRequests, 2)
	assert.Equal(t, 5*time.Minute, oidcRequestsByKey(requests)[testOidcRequestKey("https://issuer-a.example/.well-known/openid-configuration")].TTL)
	assert.Equal(t, 10*time.Minute, oidcRequestsByKey(requests)[testOidcRequestKey("https://issuer-b.example/.well-known/openid-configuration")].TTL)
}

func TestSharedOidcRequestsRemoveLastOwnerDeletesRequest(t *testing.T) {
	krtOpts := testKrtOptions(t)
	policies := dynamicOidcPolicies(t, []*agentgateway.AgentgatewayPolicy{
		testOidcPolicy("one", "https://issuer.example", 5*time.Minute),
	}, krtOpts)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Resolver: oidcResolverFunc(func(owner RemoteOidcOwner) (*ResolvedOidcRequest, error) {
			return resolvedOidcRequest(owner, "https://issuer.example/.well-known/openid-configuration"), nil
		}),
		KrtOpts: krtOpts,
	})

	awaitSharedOidcRequests(t, collections.SharedRequests, 1)

	policies.Reset(nil)

	awaitSharedOidcRequests(t, collections.SharedRequests, 0)
}

func TestStoreTracksSharedRequestCollectionLifecycle(t *testing.T) {
	krtOpts := testKrtOptions(t)
	requests := dynamicSharedOidcRequests(t, []SharedOidcRequest{
		testSharedOidcRequest("https://issuer-a.example/.well-known/openid-configuration", 5*time.Minute),
	}, krtOpts)

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, nil),
		DefaultStorePrefix,
		"agentgateway-system",
	)
	store := NewStore(requests, persisted, DefaultStorePrefix)
	store.oidcFetcher.defaultClient = offlineStubOidcClient{}
	go func() {
		_ = store.Start(ctx)
	}()

	assert.Eventually(t, store.HasSynced, testEventuallyTimeout, testEventuallyPoll)
	state := awaitOidcFetchState(t, store.oidcFetcher, testOidcRequestKey("https://issuer-a.example/.well-known/openid-configuration"))
	assert.Equal(t, 5*time.Minute, state.source.TTL)

	updatedRequests := []SharedOidcRequest{
		testSharedOidcRequest("https://issuer-b.example/.well-known/openid-configuration", 10*time.Minute),
	}
	requests.Reset(updatedRequests)

	awaitNoOidcFetchState(t, store.oidcFetcher, testOidcRequestKey("https://issuer-a.example/.well-known/openid-configuration"))
	newState := awaitOidcFetchState(t, store.oidcFetcher, testOidcRequestKey("https://issuer-b.example/.well-known/openid-configuration"))
	assert.Equal(t, 10*time.Minute, newState.source.TTL)

	requests.Reset(nil)

	awaitNoOidcFetchState(t, store.oidcFetcher, testOidcRequestKey("https://issuer-b.example/.well-known/openid-configuration"))
}

func TestStoreLoadsPersistedProvidersBeforeServing(t *testing.T) {
	target := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}
	requestKey := oidcRequestKey(target, "https://issuer.example")
	provider := DiscoveredProvider{
		RequestKey:    requestKey,
		IssuerURL:     "https://issuer.example",
		TokenEndpoint: "https://issuer.example/token",
		JwksURI:       "https://issuer.example/jwks",
		JwksJSON:      sampleJWKS,
	}

	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "oidc-store-legacy-name",
			Namespace: "agentgateway-system",
			Labels:    OidcStoreConfigMapLabel(DefaultStorePrefix),
		},
	}
	assert.NoError(t, SetProviderInConfigMap(cm, provider))

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	requests := krt.NewStaticCollection[SharedOidcRequest](alwaysSynced{}, []SharedOidcRequest{
		testSharedOidcRequest(target.URL, 5*time.Minute),
	})
	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{cm}),
		DefaultStorePrefix,
		"agentgateway-system",
	)
	store := NewStore(requests, persisted, DefaultStorePrefix)
	store.oidcFetcher.defaultClient = offlineStubOidcClient{}
	go func() {
		_ = store.Start(ctx)
	}()

	assert.Eventually(t, store.HasSynced, testEventuallyTimeout, testEventuallyPoll)
	actual, ok := store.ProviderByRequestKey(provider.RequestKey)
	assert.True(t, ok)
	assert.Equal(t, provider, actual)
}

// TestStoreClearsCacheWhenLastPolicyDeleted verifies cache eviction when the
// owning policy is deleted.
func TestStoreClearsCacheWhenLastPolicyDeleted(t *testing.T) {
	krtOpts := testKrtOptions(t)
	uri := "https://issuer.example/.well-known/openid-configuration"
	requestKey := testOidcRequestKey(uri)

	policies := dynamicOidcPolicies(t, []*agentgateway.AgentgatewayPolicy{
		testOidcPolicy("one", "https://issuer.example", 5*time.Minute),
	}, krtOpts)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Resolver: oidcResolverFunc(func(owner RemoteOidcOwner) (*ResolvedOidcRequest, error) {
			return resolvedOidcRequest(owner, uri), nil
		}),
		KrtOpts: krtOpts,
	})

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, nil),
		DefaultStorePrefix,
		"agentgateway-system",
	)
	store := NewStore(collections.SharedRequests, persisted, DefaultStorePrefix)
	store.oidcFetcher.defaultClient = offlineStubOidcClient{}
	go func() {
		_ = store.Start(ctx)
	}()

	assert.Eventually(t, store.HasSynced, testEventuallyTimeout, testEventuallyPoll)
	awaitOidcFetchState(t, store.oidcFetcher, requestKey)

	seedOidcCacheForTest(store.oidcCache, requestKey)
	_, ok := store.ProviderByRequestKey(requestKey)
	assert.True(t, ok, "cache should be populated before policy deletion")

	policies.Reset(nil)

	awaitNoOidcFetchState(t, store.oidcFetcher, requestKey)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		_, ok := store.ProviderByRequestKey(requestKey)
		assert.False(c, ok, "cache should be cleared when last policy is deleted")
	}, testEventuallyTimeout, testEventuallyPoll)
}

// TestStoreClearsOrphanCacheAtStartup verifies that startup orphan sweep works.
// Per #1618: if a ConfigMap exists with no matching policy, the cache entry
// must be evicted after startup sync.
func TestStoreClearsOrphanCacheAtStartup(t *testing.T) {
	krtOpts := testKrtOptions(t)
	uri := "https://issuer.example/.well-known/openid-configuration"
	requestKey := testOidcRequestKey(uri)

	persistedProvider := DiscoveredProvider{
		RequestKey:    requestKey,
		IssuerURL:     "https://issuer.example",
		TokenEndpoint: "https://issuer.example/token",
		JwksURI:       "https://issuer.example/jwks",
		JwksJSON:      sampleJWKS,
	}
	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      OidcConfigMapName(DefaultStorePrefix, requestKey),
			Namespace: "agentgateway-system",
			Labels:    OidcStoreConfigMapLabel(DefaultStorePrefix),
		},
	}
	assert.NoError(t, SetProviderInConfigMap(cm, persistedProvider))

	// No policies exist.
	policies := dynamicOidcPolicies(t, nil, krtOpts)
	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Resolver: oidcResolverFunc(func(owner RemoteOidcOwner) (*ResolvedOidcRequest, error) {
			return resolvedOidcRequest(owner, uri), nil
		}),
		KrtOpts: krtOpts,
	})

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{cm}),
		DefaultStorePrefix,
		"agentgateway-system",
	)
	store := NewStore(collections.SharedRequests, persisted, DefaultStorePrefix)
	store.oidcFetcher.defaultClient = offlineStubOidcClient{}
	go func() {
		_ = store.Start(ctx)
	}()

	assert.Eventually(t, store.HasSynced, testEventuallyTimeout, testEventuallyPoll)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		_, ok := store.ProviderByRequestKey(requestKey)
		assert.False(c, ok, "orphan cache entry should be cleared after sync")
	}, testEventuallyTimeout, testEventuallyPoll)
}

func TestStoreHasSyncedReflectsReadyState(t *testing.T) {
	store := &Store{
		ready: make(chan struct{}),
	}

	assert.False(t, store.HasSynced())

	close(store.ready)

	assert.True(t, store.HasSynced())
}

// offlineStubOidcClient fails every fetch so Store tests don't depend on DNS.
type offlineStubOidcClient struct{}

func (offlineStubOidcClient) FetchDiscovery(_ context.Context, _ remotehttp.FetchTarget) (discoveryDocument, error) {
	return discoveryDocument{}, errOfflineStub
}

func (offlineStubOidcClient) FetchJwks(_ context.Context, _ string) (string, error) {
	return "", errOfflineStub
}

var errOfflineStub = fmt.Errorf("offline stub")

type oidcResolverFunc func(owner RemoteOidcOwner) (*ResolvedOidcRequest, error)

func (f oidcResolverFunc) ResolveOwner(_ krt.HandlerContext, owner RemoteOidcOwner) (*ResolvedOidcRequest, error) {
	return f(owner)
}

func testKrtOptions(t *testing.T) krtutil.KrtOptions {
	t.Helper()
	return krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
}

func testOidcPolicy(name, issuer string, ttl time.Duration) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      name,
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: make([]shared.LocalPolicyTargetReferenceWithSectionName, 1),
			Traffic: &agentgateway.Traffic{
				OIDC: &agentgateway.OIDC{
					IssuerURL:       issuer,
					ClientID:        "test-client",
					RedirectURI:     "https://app.example/callback",
					RefreshInterval: &metav1.Duration{Duration: ttl},
				},
			},
		},
	}
}

func dynamicOidcPolicies(
	t *testing.T,
	initial []*agentgateway.AgentgatewayPolicy,
	krtOpts krtutil.KrtOptions,
) krt.StaticCollection[*agentgateway.AgentgatewayPolicy] {
	t.Helper()
	return krt.NewStaticCollection(alwaysSynced{}, initial, krtOpts.ToOptions("OidcPolicies")...)
}

func dynamicSharedOidcRequests(
	t *testing.T,
	initial []SharedOidcRequest,
	krtOpts krtutil.KrtOptions,
) krt.StaticCollection[SharedOidcRequest] {
	t.Helper()
	return krt.NewStaticCollection(alwaysSynced{}, initial, krtOpts.ToOptions("SharedOidcRequestsInput")...)
}

func resolvedOidcRequest(owner RemoteOidcOwner, discoveryURL string) *ResolvedOidcRequest {
	target := remotehttp.FetchTarget{URL: discoveryURL}
	return &ResolvedOidcRequest{
		OwnerID:        owner.ID,
		ExpectedIssuer: owner.Config.IssuerURL,
		Target: remotehttp.ResolvedTarget{
			Key:    target.Key(),
			Target: target,
		},
		TTL: owner.TTL,
	}
}

func testSharedOidcRequest(discoveryURL string, ttl time.Duration) SharedOidcRequest {
	target := remotehttp.FetchTarget{URL: discoveryURL}
	expectedIssuer := strings.TrimSuffix(discoveryURL, "/.well-known/openid-configuration")
	return SharedOidcRequest{
		RequestKey:     oidcRequestKey(target, expectedIssuer),
		ExpectedIssuer: expectedIssuer,
		Target:         target,
		TTL:            ttl,
	}
}

func testOidcRequestKey(discoveryURL string) remotehttp.FetchKey {
	target := remotehttp.FetchTarget{URL: discoveryURL}
	expectedIssuer := strings.TrimSuffix(discoveryURL, "/.well-known/openid-configuration")
	return oidcRequestKey(target, expectedIssuer)
}

func oidcRequestsByKey(requests []SharedOidcRequest) map[remotehttp.FetchKey]SharedOidcRequest {
	out := make(map[remotehttp.FetchKey]SharedOidcRequest, len(requests))
	for _, request := range requests {
		out[request.RequestKey] = request
	}
	return out
}

func awaitOidcFetchState(t *testing.T, f *Fetcher, requestKey remotehttp.FetchKey) fetchState {
	t.Helper()

	var state fetchState
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		var ok bool
		state, ok = f.lookup(requestKey)
		assert.True(c, ok)
	}, testEventuallyTimeout, testEventuallyPoll)

	return state
}

func awaitNoOidcFetchState(t *testing.T, f *Fetcher, requestKey remotehttp.FetchKey) {
	t.Helper()

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		_, ok := f.lookup(requestKey)
		assert.False(c, ok)
	}, testEventuallyTimeout, testEventuallyPoll)
}

func awaitSharedOidcRequests(t *testing.T, requests krt.Collection[SharedOidcRequest], expectedLen int) []SharedOidcRequest {
	t.Helper()

	var shared []SharedOidcRequest
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		shared = requests.List()
		assert.Len(c, shared, expectedLen)
	}, testEventuallyTimeout, testEventuallyPoll)

	return shared
}

var _ = oidcRequestsByKey // suppress unused warning
