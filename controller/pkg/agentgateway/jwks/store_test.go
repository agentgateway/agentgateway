package jwks

import (
	"context"
	"net/http"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

func TestSharedJwksRequestsCollapseMinTTLAcrossOwners(t *testing.T) {
	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	policies := krt.NewStaticCollection(nil, []*agentgateway.AgentgatewayPolicy{
		testRemotePolicy("one", "https://issuer.example/jwks", 10*time.Minute),
	}, krtOpts.ToOptions("jwks/SharedOwnerPolicies")...)
	backends := krt.NewStaticCollection(alwaysSynced{}, []*agentgateway.AgentgatewayBackend{
		testBackend("shared-backend", "https://issuer.example/jwks", 5*time.Minute),
	}, krtOpts.ToOptions("jwks/SharedOwnerBackends")...)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Backends:             backends,
		Resolver: jwksResolverFunc(func(owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
			return resolvedJwksRequest(owner, "https://issuer.example/jwks"), nil
		}),
		KrtOpts: krtOpts,
	})

	requests := await(t, collections.SharedRequests, 1)
	assert.Equal(t, remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key(), requests[0].RequestKey)
	assert.Equal(t, 5*time.Minute, requests[0].TTL)
}

func TestSharedJwksRequestsRetargetOwnerAcrossRequestKeys(t *testing.T) {
	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	policies := dynamicRemotePolicies(t, []*agentgateway.AgentgatewayPolicy{
		testRemotePolicy("moving", "https://issuer.example/a", 5*time.Minute),
		testRemotePolicy("staying", "https://issuer.example/a", 10*time.Minute),
	}, krtOpts)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Backends:             staticBackends(t, krtOpts),
		Resolver: jwksResolverFunc(func(owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
			return resolvedJwksRequest(owner, owner.Remote.JwksPath), nil
		}),
		KrtOpts: krtOpts,
	})

	requests := await(t, collections.SharedRequests, 1)
	assert.Equal(t, 5*time.Minute, requests[0].TTL)

	updatedPolicies := []*agentgateway.AgentgatewayPolicy{
		testRemotePolicy("moving", "https://issuer.example/b", 5*time.Minute),
		testRemotePolicy("staying", "https://issuer.example/a", 10*time.Minute),
	}
	policies.Reset(updatedPolicies)

	requestsByKey := jwksRequestsByKey(await(t, collections.SharedRequests, 2))
	assert.Equal(t, 10*time.Minute, requestsByKey[remotehttp.FetchTarget{URL: "https://issuer.example/a"}.Key()].TTL)
	assert.Equal(t, 5*time.Minute, requestsByKey[remotehttp.FetchTarget{URL: "https://issuer.example/b"}.Key()].TTL)
}

func TestSharedJwksRequestsRemoveLastOwnerDeletesRequest(t *testing.T) {
	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	policies := dynamicRemotePolicies(t, []*agentgateway.AgentgatewayPolicy{
		testRemotePolicy("one", "https://issuer.example/jwks", 5*time.Minute),
	}, krtOpts)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Backends:             staticBackends(t, krtOpts),
		Resolver: jwksResolverFunc(func(owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
			return resolvedJwksRequest(owner, owner.Remote.JwksPath), nil
		}),
		KrtOpts: krtOpts,
	})

	await(t, collections.SharedRequests, 1)

	policies.Reset(nil)

	await(t, collections.SharedRequests, 0)
}

func TestStoreTracksSharedRequestCollectionLifecycle(t *testing.T) {
	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	requests := dynamicSharedJwksRequests(t, []SharedJwksRequest{
		testStoreSharedJwksRequest("https://issuer.example/a", 5*time.Minute),
	}, krtOpts)

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	persisted := NewPersistedEntriesFromCollection(
		staticJwksConfigMaps(t, krtOpts, nil),
		"agentgateway-system",
	)

	store := NewStore(requests, persisted)
	store.Driver.DefaultClient = &http.Client{Transport: offlineTransport{}}
	go func() {
		_ = store.Start(ctx)
	}()

	assert.Eventually(t, store.HasSynced, eventuallyTimeout, eventuallyPoll)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		assert.Equal(c, 1, store.Fetcher.RequestCountForTest())
	}, eventuallyTimeout, eventuallyPoll)

	updatedRequests := []SharedJwksRequest{
		testStoreSharedJwksRequest("https://issuer.example/b", 10*time.Minute),
	}
	requests.Reset(updatedRequests)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		assert.Equal(c, 1, store.Fetcher.RequestCountForTest())
	}, eventuallyTimeout, eventuallyPoll)

	requests.Reset(nil)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		assert.Equal(c, 0, store.Fetcher.RequestCountForTest())
	}, eventuallyTimeout, eventuallyPoll)
}

func TestStoreDropsOldFetchStateWhenPolicyRetargets(t *testing.T) {
	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	policies := dynamicRemotePolicies(t, []*agentgateway.AgentgatewayPolicy{
		testRemotePolicy("one", "https://issuer.example/v1", 5*time.Minute),
	}, krtOpts)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Backends:             staticBackends(t, krtOpts),
		Resolver: jwksResolverFunc(func(owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
			return resolvedJwksRequest(owner, owner.Remote.JwksPath), nil
		}),
		KrtOpts: krtOpts,
	})

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	persisted := NewPersistedEntriesFromCollection(
		staticJwksConfigMaps(t, krtOpts, nil),
		"agentgateway-system",
	)
	store := NewStore(collections.SharedRequests, persisted)
	store.Driver.DefaultClient = &http.Client{Transport: offlineTransport{}}
	go func() {
		_ = store.Start(ctx)
	}()

	assert.Eventually(t, store.HasSynced, eventuallyTimeout, eventuallyPoll)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		assert.Equal(c, 1, store.Fetcher.RequestCountForTest())
	}, eventuallyTimeout, eventuallyPoll)

	policies.Reset([]*agentgateway.AgentgatewayPolicy{
		testRemotePolicy("one", "https://issuer.example/v2", 5*time.Minute),
	})

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		assert.Equal(c, 1, store.Fetcher.RequestCountForTest())
	}, eventuallyTimeout, eventuallyPoll)
}

func TestStoreLoadsPersistedKeysetsBeforeServing(t *testing.T) {
	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	target := remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}
	keyset := Keyset{
		RequestKey: target.Key(),
		URL:        target.URL,
		JwksJSON:   `{"keys":[]}`,
	}

	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "jwks-store-legacy-name",
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultJwksStorePrefix),
		},
	}
	assert.NoError(t, SetJwksInConfigMap(cm, keyset))

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	requests := krt.NewStaticCollection[SharedJwksRequest](alwaysSynced{}, []SharedJwksRequest{
		testStoreSharedJwksRequest(target.URL, 5*time.Minute),
	}, krtOpts.ToOptions("jwks/PersistedStartupSharedRequests")...)
	persisted := NewPersistedEntriesFromCollection(
		staticJwksConfigMaps(t, krtOpts, []*corev1.ConfigMap{cm}),
		"agentgateway-system",
	)
	store := NewStore(requests, persisted)
	store.Driver.DefaultClient = &http.Client{Transport: offlineTransport{}}
	go func() {
		_ = store.Start(ctx)
	}()

	assert.Eventually(t, store.HasSynced, eventuallyTimeout, eventuallyPoll)
	actual, ok := store.FetchedResults().Get(keyset.RequestKey)
	assert.True(t, ok)
	assert.Equal(t, keyset, actual)
}

func TestStoreClearsResultWhenLastPolicyDeleted(t *testing.T) {
	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	uri := "https://issuer.example/jwks"
	requestKey := remotehttp.FetchTarget{URL: uri}.Key()

	policies := dynamicRemotePolicies(t, []*agentgateway.AgentgatewayPolicy{
		testRemotePolicy("one", uri, 5*time.Minute),
	}, krtOpts)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Backends:             staticBackends(t, krtOpts),
		Resolver: jwksResolverFunc(func(owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
			return resolvedJwksRequest(owner, owner.Remote.JwksPath), nil
		}),
		KrtOpts: krtOpts,
	})

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	persisted := NewPersistedEntriesFromCollection(
		staticJwksConfigMaps(t, krtOpts, nil),
		"agentgateway-system",
	)
	store := NewStore(collections.SharedRequests, persisted)
	store.Driver.DefaultClient = &http.Client{Transport: offlineTransport{}}
	go func() {
		_ = store.Start(ctx)
	}()

	assert.Eventually(t, store.HasSynced, eventuallyTimeout, eventuallyPoll)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		assert.Equal(c, 1, store.Fetcher.RequestCountForTest())
	}, eventuallyTimeout, eventuallyPoll)

	seedStoreJwksResultForTest(store.FetchedResults(), requestKey, uri)
	_, ok := store.FetchedResults().Get(requestKey)
	assert.True(t, ok, "result should be populated before policy deletion")

	// Delete the AgentPolicy.
	policies.Reset(nil)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		_, ok := store.FetchedResults().Get(requestKey)
		assert.False(c, ok, "result should be cleared when last policy is deleted")
	}, eventuallyTimeout, eventuallyPoll)
}

func TestStoreClearsResultWhenAllSharedPoliciesDeleted(t *testing.T) {
	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	uri := "https://issuer.example/jwks"
	requestKey := remotehttp.FetchTarget{URL: uri}.Key()

	policies := dynamicRemotePolicies(t, []*agentgateway.AgentgatewayPolicy{
		testRemotePolicy("one", uri, 5*time.Minute),
		testRemotePolicy("two", uri, 5*time.Minute),
	}, krtOpts)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Backends:             staticBackends(t, krtOpts),
		Resolver: jwksResolverFunc(func(owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
			return resolvedJwksRequest(owner, owner.Remote.JwksPath), nil
		}),
		KrtOpts: krtOpts,
	})

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	persisted := NewPersistedEntriesFromCollection(
		staticJwksConfigMaps(t, krtOpts, nil),
		"agentgateway-system",
	)
	store := NewStore(collections.SharedRequests, persisted)
	store.Driver.DefaultClient = &http.Client{Transport: offlineTransport{}}
	go func() {
		_ = store.Start(ctx)
	}()

	assert.Eventually(t, store.HasSynced, eventuallyTimeout, eventuallyPoll)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		assert.Equal(c, 1, store.Fetcher.RequestCountForTest())
	}, eventuallyTimeout, eventuallyPoll)

	seedStoreJwksResultForTest(store.FetchedResults(), requestKey, uri)

	policies.Reset(nil)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		_, ok := store.FetchedResults().Get(requestKey)
		assert.False(c, ok)
	}, eventuallyTimeout, eventuallyPoll)
}

func TestStoreClearsResultWhenPolicyDeletedAfterWarmStart(t *testing.T) {
	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	uri := "https://issuer.example/jwks"
	requestKey := remotehttp.FetchTarget{URL: uri}.Key()

	persistedKeyset := Keyset{
		RequestKey: requestKey,
		URL:        uri,
		JwksJSON:   `{"keys":[]}`,
	}
	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      remotecache.ConfigMapName(DefaultJwksStorePrefix, requestKey),
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultJwksStorePrefix),
		},
	}
	assert.NoError(t, SetJwksInConfigMap(cm, persistedKeyset))

	policies := dynamicRemotePolicies(t, []*agentgateway.AgentgatewayPolicy{
		testRemotePolicy("one", uri, 5*time.Minute),
	}, krtOpts)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Backends:             staticBackends(t, krtOpts),
		Resolver: jwksResolverFunc(func(owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
			return resolvedJwksRequest(owner, owner.Remote.JwksPath), nil
		}),
		KrtOpts: krtOpts,
	})

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	persisted := NewPersistedEntriesFromCollection(
		staticJwksConfigMaps(t, krtOpts, []*corev1.ConfigMap{cm}),
		"agentgateway-system",
	)
	store := NewStore(collections.SharedRequests, persisted)
	store.Driver.DefaultClient = &http.Client{Transport: offlineTransport{}}
	go func() {
		_ = store.Start(ctx)
	}()

	assert.Eventually(t, store.HasSynced, eventuallyTimeout, eventuallyPoll)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		assert.Equal(c, 1, store.Fetcher.RequestCountForTest())
	}, eventuallyTimeout, eventuallyPoll)

	_, ok := store.FetchedResults().Get(requestKey)
	assert.True(t, ok, "result should be seeded from persisted ConfigMap")

	policies.Reset(nil)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		_, ok := store.FetchedResults().Get(requestKey)
		assert.False(c, ok)
	}, eventuallyTimeout, eventuallyPoll)
}

func TestStoreClearsOrphanResultAtStartup(t *testing.T) {
	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	uri := "https://issuer.example/jwks"
	requestKey := remotehttp.FetchTarget{URL: uri}.Key()

	persistedKeyset := Keyset{
		RequestKey: requestKey,
		URL:        uri,
		JwksJSON:   `{"keys":[]}`,
	}
	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      remotecache.ConfigMapName(DefaultJwksStorePrefix, requestKey),
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultJwksStorePrefix),
		},
	}
	assert.NoError(t, SetJwksInConfigMap(cm, persistedKeyset))

	// No AgentPolicies exist.
	policies := dynamicRemotePolicies(t, nil, krtOpts)
	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Backends:             staticBackends(t, krtOpts),
		Resolver: jwksResolverFunc(func(owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
			return resolvedJwksRequest(owner, owner.Remote.JwksPath), nil
		}),
		KrtOpts: krtOpts,
	})

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	persisted := NewPersistedEntriesFromCollection(
		staticJwksConfigMaps(t, krtOpts, []*corev1.ConfigMap{cm}),
		"agentgateway-system",
	)
	store := NewStore(collections.SharedRequests, persisted)
	store.Driver.DefaultClient = &http.Client{Transport: offlineTransport{}}
	go func() {
		_ = store.Start(ctx)
	}()

	assert.Eventually(t, store.HasSynced, eventuallyTimeout, eventuallyPoll)

	// After sync, the orphan result should be cleared.
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		_, ok := store.FetchedResults().Get(requestKey)
		assert.False(c, ok, "orphan result should be cleared after sync")
	}, eventuallyTimeout, eventuallyPoll)
}

type jwksResolverFunc func(owner RemoteJwksOwner) (*ResolvedJwksRequest, error)

func (f jwksResolverFunc) ResolveOwner(_ krt.HandlerContext, owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
	return f(owner)
}

func testRemotePolicy(name, uri string, ttl time.Duration) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      name,
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: make([]agentgateway.LocalPolicyTargetReferenceWithSectionName, 1),
			Traffic: &agentgateway.Traffic{
				JWTAuthentication: &agentgateway.JWTAuthentication{
					Providers: []agentgateway.JWTProvider{{
						JWKS: agentgateway.JWKS{
							Remote: &agentgateway.RemoteJWKS{
								JwksPath:      uri,
								CacheDuration: &metav1.Duration{Duration: ttl},
							},
						},
					}},
				},
			},
		},
	}
}

func testBackend(name, uri string, ttl time.Duration) *agentgateway.AgentgatewayBackend {
	return &agentgateway.AgentgatewayBackend{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      name,
		},
		Spec: agentgateway.AgentgatewayBackendSpec{
			MCP: &agentgateway.MCPBackend{},
			Policies: &agentgateway.BackendFull{
				MCP: &agentgateway.BackendMCP{
					Authentication: &agentgateway.MCPAuthentication{
						JWKS: agentgateway.RemoteJWKS{
							JwksPath:      uri,
							CacheDuration: &metav1.Duration{Duration: ttl},
						},
					},
				},
			},
		},
	}
}

func dynamicRemotePolicies(
	t *testing.T,
	initial []*agentgateway.AgentgatewayPolicy,
	krtOpts krtutil.KrtOptions,
) krt.StaticCollection[*agentgateway.AgentgatewayPolicy] {
	t.Helper()

	return krt.NewStaticCollection(alwaysSynced{}, initial, krtOpts.ToOptions("jwks/Policies")...)
}

func staticBackends(t *testing.T, krtOpts krtutil.KrtOptions) krt.StaticCollection[*agentgateway.AgentgatewayBackend] {
	t.Helper()

	return krt.NewStaticCollection[*agentgateway.AgentgatewayBackend](alwaysSynced{}, nil, krtOpts.ToOptions("jwks/Backends")...)
}

func staticJwksConfigMaps(
	t *testing.T,
	krtOpts krtutil.KrtOptions,
	initial []*corev1.ConfigMap,
) krt.StaticCollection[*corev1.ConfigMap] {
	t.Helper()

	return krt.NewStaticCollection(alwaysSynced{}, initial, krtOpts.ToOptions("jwks/PersistedConfigMaps")...)
}

func dynamicSharedJwksRequests(
	t *testing.T,
	initial []SharedJwksRequest,
	krtOpts krtutil.KrtOptions,
) krt.StaticCollection[SharedJwksRequest] {
	t.Helper()

	return krt.NewStaticCollection(alwaysSynced{}, initial, krtOpts.ToOptions("jwks/SharedRequestsInput")...)
}

func resolvedJwksRequest(owner RemoteJwksOwner, requestURL string) *ResolvedJwksRequest {
	target := remotehttp.FetchTarget{URL: requestURL}
	return &ResolvedJwksRequest{
		OwnerID: owner.ID,
		Target: remotehttp.ResolvedTarget{
			Target: target,
		},
		TTL: owner.TTL,
	}
}

func testStoreSharedJwksRequest(requestURL string, ttl time.Duration) SharedJwksRequest {
	target := remotehttp.FetchTarget{URL: requestURL}
	return SharedJwksRequest{jwksRequestSpec{
		RequestKey: target.Key(),
		Target:     target,
		TTL:        ttl,
	}}
}

func seedStoreJwksResultForTest(results *JwksResults, requestKey remotehttp.FetchKey, url string) {
	results.Put(Keyset{
		RequestKey: requestKey,
		URL:        url,
		JwksJSON:   `{"keys":[]}`,
	})
}

func jwksRequestsByKey(requests []SharedJwksRequest) map[remotehttp.FetchKey]SharedJwksRequest {
	out := make(map[remotehttp.FetchKey]SharedJwksRequest, len(requests))
	for _, request := range requests {
		out[request.RequestKey] = request
	}
	return out
}
