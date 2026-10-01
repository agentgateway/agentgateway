package jwks

import (
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test"
	corev1 "k8s.io/api/core/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

type alwaysSynced struct{}

func (alwaysSynced) WaitUntilSynced(stop <-chan struct{}) bool {
	return true
}

func (alwaysSynced) HasSynced() bool {
	return true
}

func TestLookupFailsClosedWhenKeysetIsMissing(t *testing.T) {
	stop := test.NewStop(t)
	target := remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}
	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, nil, krt.WithName("jwks/LookupMissingPersistedConfigMaps"), krt.WithStop(stop)),
		DefaultJwksStorePrefix,
		"agentgateway-system",
		krt.WithStop(stop),
	)
	lookupIndex := NewLookup(
		persisted,
		krt.NewStaticCollection(alwaysSynced{}, []ResolvedOwner{{
			Source: JwksSource{RequestKey: target.Key(), Target: target},
		}}, krt.WithStop(stop)),
	)
	lookupImpl := lookupIndex.(*lookup)
	lookupImpl.cache.persisted.entries.WaitUntilSynced(stop)

	_, err := lookupIndex.InlineForOwner(krt.TestingDummyContext{}, RemoteJwksOwner{})

	assert.EqualError(t, err, `jwks keyset for "https://issuer.example/jwks" isn't available (not yet fetched or fetch failed)`)
}

func TestLookupReturnsPersistedKeyset(t *testing.T) {
	stop := test.NewStop(t)
	target := remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}
	keyset := Keyset{
		RequestKey: target.Key(),
		URL:        target.URL,
		JwksJSON:   `{"keys":[]}`,
	}
	cm := &corev1.ConfigMap{
		Name:      JwksConfigMapName(DefaultJwksStorePrefix, target.Key()),
		Namespace: "agentgateway-system",
		Labels:    JwksStoreConfigMapLabel(DefaultJwksStorePrefix),
	}
	assert.NoError(t, SetJwksInConfigMap(cm, keyset))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{cm}, krt.WithName("jwks/LookupPersistedConfigMaps"), krt.WithStop(stop)),
		DefaultJwksStorePrefix,
		"agentgateway-system",
		krt.WithStop(stop),
	)
	lookupIndex := NewLookup(
		persisted,
		krt.NewStaticCollection(alwaysSynced{}, []ResolvedOwner{{
			Source: JwksSource{RequestKey: target.Key(), Target: target},
		}}, krt.WithStop(stop)),
	)
	lookupImpl := lookupIndex.(*lookup)
	lookupImpl.cache.persisted.entries.WaitUntilSynced(stop)

	inline, err := lookupIndex.InlineForOwner(krt.TestingDummyContext{}, RemoteJwksOwner{})

	assert.NoError(t, err)
	assert.Equal(t, keyset.JwksJSON, inline)

	missingOwner := RemoteJwksOwner{ID: JwksOwnerID{Name: "missing"}}
	inline, err = lookupIndex.InlineForOwner(krt.TestingDummyContext{}, missingOwner)
	assert.Empty(t, inline)
	assert.ErrorContains(t, err, `jwks resolution for "//missing#" isn't available`)
}

func TestLookupTracksTLSAndPersistedKeysetChanges(t *testing.T) {
	opts := testKrtOptions(t)
	policy := testRemotePolicy("owner", "/jwks", 5*time.Minute)
	policy.Spec.Traffic.JWTAuthentication.Providers[0].JWKS.Remote.BackendRef = &gwv1.BackendObjectReference{
		Group: new(gwv1.Group(wellknown.AgentgatewayBackendGVK.Group)),
		Kind:  new(gwv1.Kind(wellknown.AgentgatewayBackendGVK.Kind)),
		Name:  "issuer",
	}
	backend := &agentgateway.AgentgatewayBackend{
		Name: "issuer", Namespace: "default",
		Spec: agentgateway.AgentgatewayBackendSpec{
			Static:   &agentgateway.StaticBackend{Host: "issuer.example", Port: 443},
			Policies: &agentgateway.BackendFull{TLS: &agentgateway.BackendTLS{Sni: new("old.example")}},
		},
	}
	backends := krt.NewMutableCollection(alwaysSynced{}, []*agentgateway.AgentgatewayBackend{backend}, opts.ToOptions("backends")...)
	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: dynamicRemotePolicies(t, []*agentgateway.AgentgatewayPolicy{policy}, opts).AsCollection(),
		Backends:             backends.AsCollection(),
		Resolver:             NewResolver(remotehttp.NewResolver(remotehttp.Inputs{Backends: backends.AsCollection()}), nil, apisettings.BackendRefGrantModeNone),
		KrtOpts:              opts,
	})
	configMaps := krt.NewMutableCollection(alwaysSynced{}, []*corev1.ConfigMap(nil), opts.ToOptions("configmaps")...)
	persisted := NewPersistedEntriesFromCollection(configMaps.AsCollection(), DefaultJwksStorePrefix, "agentgateway-system", opts.ToOptions("persisted")...)
	lookup := NewLookup(persisted, collections.ResolvedOwners)
	owner := OwnersFromPolicy(policy)[0]
	inline := krt.NewSingleton(func(ctx krt.HandlerContext) *string {
		value, err := lookup.InlineForOwner(ctx, owner)
		if err != nil {
			value = err.Error()
		}
		return &value
	}, opts.ToOptions("inline")...)

	for _, serverName := range []string{"old.example", "new.example"} {
		if serverName == "new.example" {
			backend = backend.DeepCopy()
			backend.Spec.Policies.TLS.Sni = &serverName
			backends.UpdateObject(backend)
		}
		require.Eventually(t, func() bool {
			value := inline.Get()
			return value != nil && *value == `jwks keyset for "https://issuer.example:443/jwks" isn't available (not yet fetched or fetch failed)`
		}, testEventuallyTimeout, testEventuallyPoll)
		target := remotehttp.FetchTarget{
			URL:       "https://issuer.example:443/jwks",
			Transport: remotehttp.TransportFingerprint{ServerName: serverName},
		}
		cm := &corev1.ConfigMap{
			Name: JwksConfigMapName(DefaultJwksStorePrefix, target.Key()), Namespace: "agentgateway-system",
			Labels: JwksStoreConfigMapLabel(DefaultJwksStorePrefix),
		}
		keys := `{"keys":[{"kid":"` + serverName + `"}]}`
		require.NoError(t, SetJwksInConfigMap(cm, Keyset{RequestKey: target.Key(), URL: target.URL, JwksJSON: keys}))
		configMaps.UpdateObject(cm)
		require.Eventually(t, func() bool { return inline.Get() != nil && *inline.Get() == keys }, testEventuallyTimeout, testEventuallyPoll)
	}

	backends.Reset(nil)
	require.Eventually(t, func() bool {
		value := inline.Get()
		return value != nil && *value == "backend default/issuer not found, referenced by AgentgatewayPolicy default/owner"
	}, testEventuallyTimeout, testEventuallyPoll)
	awaitSharedJwksRequests(t, collections.SharedRequests, 0)
	backends.UpdateObject(backend)
	require.Eventually(t, func() bool {
		value := inline.Get()
		return value != nil && *value == `{"keys":[{"kid":"new.example"}]}`
	}, testEventuallyTimeout, testEventuallyPoll)
	awaitSharedJwksRequests(t, collections.SharedRequests, 1)
}

func TestLookupRequiresCanonicalPersistedKeysetName(t *testing.T) {
	stop := test.NewStop(t)
	target := remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}
	keyset := Keyset{
		RequestKey: target.Key(),
		URL:        target.URL,
		JwksJSON:   `{"keys":[{"kid":"legacy"}]}`,
	}
	cm := &corev1.ConfigMap{
		Name:      "jwks-store-legacy-name",
		Namespace: "agentgateway-system",
		Labels:    JwksStoreConfigMapLabel(DefaultJwksStorePrefix),
	}
	assert.NoError(t, SetJwksInConfigMap(cm, keyset))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{cm}, krt.WithName("jwks/LookupLegacyNameConfigMaps"), krt.WithStop(stop)),
		DefaultJwksStorePrefix,
		"agentgateway-system",
		krt.WithStop(stop),
	)
	lookupIndex := NewLookup(
		persisted,
		krt.NewStaticCollection(alwaysSynced{}, []ResolvedOwner{{
			Source: JwksSource{RequestKey: target.Key(), Target: target},
		}}, krt.WithStop(stop)),
	)
	lookupImpl := lookupIndex.(*lookup)
	lookupImpl.cache.persisted.entries.WaitUntilSynced(stop)

	_, err := lookupIndex.InlineForOwner(krt.TestingDummyContext{}, RemoteJwksOwner{})

	assert.EqualError(t, err, `jwks keyset for "https://issuer.example/jwks" isn't available (not yet fetched or fetch failed)`)
}

func TestLookupPropagatesResolverError(t *testing.T) {
	sentinel := "resolver failed"
	stop := test.NewStop(t)
	lookupIndex := NewLookup(
		NewPersistedEntriesFromCollection(
			krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, nil, krt.WithName("jwks/LookupResolverErrorConfigMaps"), krt.WithStop(stop)),
			DefaultJwksStorePrefix,
			"agentgateway-system",
			krt.WithStop(stop),
		),
		krt.NewStaticCollection(alwaysSynced{}, []ResolvedOwner{{Error: sentinel}}, krt.WithStop(stop)),
	)

	_, err := lookupIndex.InlineForOwner(krt.TestingDummyContext{}, RemoteJwksOwner{})

	assert.EqualError(t, err, sentinel)
}

func TestLookupFailsWhenPersistedCacheIsNotConfigured(t *testing.T) {
	lookupIndex := &lookup{}

	_, err := lookupIndex.InlineForOwner(krt.TestingDummyContext{}, RemoteJwksOwner{})

	assert.EqualError(t, err, "jwks persisted cache is not configured")
}
