package jwks

import (
	"context"
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

func TestSharedJwksRequestsCollapseMinTTLAcrossOwners(t *testing.T) {
	krtOpts := testKrtOptions(t)
	policies := krt.NewStaticCollection(alwaysSynced{}, []*agentgateway.AgentgatewayPolicy{
		testRemotePolicy("one", "https://issuer.example/jwks", 10*time.Minute),
	})
	backends := krt.NewStaticCollection(alwaysSynced{}, []*agentgateway.AgentgatewayBackend{
		testBackend("shared-backend", "https://issuer.example/jwks", 5*time.Minute),
	})

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Backends:             backends,
		Resolver: jwksResolverFunc(func(owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
			return resolvedJwksRequest(owner, "https://issuer.example/jwks"), nil
		}),
		KrtOpts: krtOpts,
	})

	requests := awaitSharedJwksRequests(t, collections.SharedRequests, 1)
	assert.Equal(t, remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key(), requests[0].RequestKey)
	assert.Equal(t, 5*time.Minute, requests[0].TTL)
}

func TestSharedJwksRequestsRetargetOwnerAcrossRequestKeys(t *testing.T) {
	krtOpts := testKrtOptions(t)
	policies := dynamicRemotePolicies(t, []*agentgateway.AgentgatewayPolicy{
		testRemotePolicy("moving", "https://issuer.example/a", 5*time.Minute),
		testRemotePolicy("staying", "https://issuer.example/a", 10*time.Minute),
	}, krtOpts)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Backends:             krt.NewStaticCollection[*agentgateway.AgentgatewayBackend](alwaysSynced{}, nil),
		Resolver: jwksResolverFunc(func(owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
			return resolvedJwksRequest(owner, owner.Remote.JwksPath), nil
		}),
		KrtOpts: krtOpts,
	})

	requests := awaitSharedJwksRequests(t, collections.SharedRequests, 1)
	assert.Equal(t, 5*time.Minute, requests[0].TTL)

	updatedPolicies := []*agentgateway.AgentgatewayPolicy{
		testRemotePolicy("moving", "https://issuer.example/b", 5*time.Minute),
		testRemotePolicy("staying", "https://issuer.example/a", 10*time.Minute),
	}
	policies.Reset(updatedPolicies)

	requestsByKey := jwksRequestsByKey(awaitSharedJwksRequests(t, collections.SharedRequests, 2))
	assert.Equal(t, 10*time.Minute, requestsByKey[remotehttp.FetchTarget{URL: "https://issuer.example/a"}.Key()].TTL)
	assert.Equal(t, 5*time.Minute, requestsByKey[remotehttp.FetchTarget{URL: "https://issuer.example/b"}.Key()].TTL)
}

func TestSharedJwksRequestsRemoveLastOwnerDeletesRequest(t *testing.T) {
	krtOpts := testKrtOptions(t)
	policies := dynamicRemotePolicies(t, []*agentgateway.AgentgatewayPolicy{
		testRemotePolicy("one", "https://issuer.example/jwks", 5*time.Minute),
	}, krtOpts)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Backends:             krt.NewStaticCollection[*agentgateway.AgentgatewayBackend](alwaysSynced{}, nil),
		Resolver: jwksResolverFunc(func(owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
			return resolvedJwksRequest(owner, owner.Remote.JwksPath), nil
		}),
		KrtOpts: krtOpts,
	})

	awaitSharedJwksRequests(t, collections.SharedRequests, 1)

	policies.Reset(nil)

	awaitSharedJwksRequests(t, collections.SharedRequests, 0)
}

func TestStoreTracksSharedRequestCollectionLifecycle(t *testing.T) {
	krtOpts := testKrtOptions(t)
	requests := dynamicSharedJwksRequests(t, []SharedJwksRequest{
		testSharedJwksRequest("https://issuer.example/a", 5*time.Minute),
	}, krtOpts)

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, nil),
		DefaultJwksStorePrefix,
		"agentgateway-system",
	)
	store := NewStore(requests, persisted, DefaultJwksStorePrefix)
	go func() {
		_ = store.Start(ctx)
	}()

	assert.Eventually(t, store.HasSynced, testEventuallyTimeout, testEventuallyPoll)
	state := awaitJwksFetchState(t, store.jwksFetcher, remotehttp.FetchTarget{URL: "https://issuer.example/a"}.Key())
	assert.Equal(t, 5*time.Minute, state.source.TTL)

	updatedRequests := []SharedJwksRequest{
		testSharedJwksRequest("https://issuer.example/b", 10*time.Minute),
	}
	requests.Reset(updatedRequests)

	awaitNoJwksFetchState(t, store.jwksFetcher, remotehttp.FetchTarget{URL: "https://issuer.example/a"}.Key())
	newState := awaitJwksFetchState(t, store.jwksFetcher, remotehttp.FetchTarget{URL: "https://issuer.example/b"}.Key())
	assert.Equal(t, 10*time.Minute, newState.source.TTL)

	requests.Reset(nil)

	awaitNoJwksFetchState(t, store.jwksFetcher, remotehttp.FetchTarget{URL: "https://issuer.example/b"}.Key())
}

func TestStoreLoadsPersistedKeysetsBeforeServing(t *testing.T) {
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
			Labels:    JwksStoreConfigMapLabel(DefaultJwksStorePrefix),
		},
	}
	assert.NoError(t, SetJwksInConfigMap(cm, keyset))

	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	requests := krt.NewStaticCollection[SharedJwksRequest](alwaysSynced{}, nil)
	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{cm}),
		DefaultJwksStorePrefix,
		"agentgateway-system",
	)
	store := NewStore(requests, persisted, DefaultJwksStorePrefix)
	go func() {
		_ = store.Start(ctx)
	}()

	assert.Eventually(t, store.HasSynced, testEventuallyTimeout, testEventuallyPoll)
	actual, ok := store.JwksByRequestKey(keyset.RequestKey)
	assert.True(t, ok)
	assert.Equal(t, keyset, actual)
}

func TestStoreHasSyncedReflectsReadyState(t *testing.T) {
	store := &Store{
		ready: make(chan struct{}),
	}

	assert.False(t, store.HasSynced())

	close(store.ready)

	assert.True(t, store.HasSynced())
}

type jwksResolverFunc func(owner RemoteJwksOwner) (*ResolvedJwksRequest, error)

func (f jwksResolverFunc) ResolveOwner(_ krt.HandlerContext, owner RemoteJwksOwner) (*ResolvedJwksRequest, error) {
	return f(owner)
}

func testKrtOptions(t *testing.T) krtutil.KrtOptions {
	t.Helper()
	return krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
}

func testRemotePolicy(name, uri string, ttl time.Duration) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      name,
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: make([]shared.LocalPolicyTargetReferenceWithSectionName, 1),
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

	return krt.NewStaticCollection(alwaysSynced{}, initial, krtOpts.ToOptions("JwksPolicies")...)
}

func dynamicSharedJwksRequests(
	t *testing.T,
	initial []SharedJwksRequest,
	krtOpts krtutil.KrtOptions,
) krt.StaticCollection[SharedJwksRequest] {
	t.Helper()

	return krt.NewStaticCollection(alwaysSynced{}, initial, krtOpts.ToOptions("SharedJwksRequestsInput")...)
}

func resolvedJwksRequest(owner RemoteJwksOwner, requestURL string) *ResolvedJwksRequest {
	target := remotehttp.FetchTarget{URL: requestURL}
	return &ResolvedJwksRequest{
		OwnerID: owner.ID,
		Target: remotehttp.ResolvedTarget{
			Key:    target.Key(),
			Target: target,
		},
		TTL: owner.TTL,
	}
}

func testSharedJwksRequest(requestURL string, ttl time.Duration) SharedJwksRequest {
	target := remotehttp.FetchTarget{URL: requestURL}
	return SharedJwksRequest{
		RequestKey: target.Key(),
		Target:     target,
		TTL:        ttl,
	}
}

func jwksRequestsByKey(requests []SharedJwksRequest) map[remotehttp.FetchKey]SharedJwksRequest {
	out := make(map[remotehttp.FetchKey]SharedJwksRequest, len(requests))
	for _, request := range requests {
		out[request.RequestKey] = request
	}
	return out
}

func awaitJwksFetchState(t *testing.T, f *Fetcher, requestKey remotehttp.FetchKey) fetchState {
	t.Helper()

	var state fetchState
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		var ok bool
		state, ok = f.lookup(requestKey)
		assert.True(c, ok)
	}, testEventuallyTimeout, testEventuallyPoll)

	return state
}

func awaitNoJwksFetchState(t *testing.T, f *Fetcher, requestKey remotehttp.FetchKey) {
	t.Helper()

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		_, ok := f.lookup(requestKey)
		assert.False(c, ok)
	}, testEventuallyTimeout, testEventuallyPoll)
}

func awaitSharedJwksRequests(t *testing.T, requests krt.Collection[SharedJwksRequest], expectedLen int) []SharedJwksRequest {
	t.Helper()

	var shared []SharedJwksRequest
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		shared = requests.List()
		assert.Len(c, shared, expectedLen)
	}, testEventuallyTimeout, testEventuallyPoll)

	return shared
}
