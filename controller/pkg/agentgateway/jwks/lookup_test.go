package jwks

import (
	"errors"
	"testing"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

type staticLookupResolver struct {
	resolved *ResolvedJwksRequest
	err      error
}

type alwaysSynced struct{}

func (r staticLookupResolver) ResolveOwner(krt.HandlerContext, RemoteJwksOwner) (*ResolvedJwksRequest, error) {
	return r.resolved, r.err
}

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
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, nil),
		DefaultJwksStorePrefix,
		"agentgateway-system",
	)
	lookupIndex := NewLookup(
		persisted,
		staticLookupResolver{resolved: &ResolvedJwksRequest{
			Target: remotehttp.ResolvedTarget{
				Key:    target.Key(),
				Target: target,
			},
		}},
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
		ObjectMeta: metav1.ObjectMeta{
			Name:      JwksConfigMapName(DefaultJwksStorePrefix, target.Key()),
			Namespace: "agentgateway-system",
			Labels:    JwksStoreConfigMapLabel(DefaultJwksStorePrefix),
		},
	}
	assert.NoError(t, SetJwksInConfigMap(cm, keyset))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{cm}),
		DefaultJwksStorePrefix,
		"agentgateway-system",
	)
	lookupIndex := NewLookup(
		persisted,
		staticLookupResolver{resolved: &ResolvedJwksRequest{
			Target: remotehttp.ResolvedTarget{
				Key:    target.Key(),
				Target: target,
			},
		}},
	)
	lookupImpl := lookupIndex.(*lookup)
	lookupImpl.cache.persisted.entries.WaitUntilSynced(stop)

	inline, err := lookupIndex.InlineForOwner(krt.TestingDummyContext{}, RemoteJwksOwner{})

	assert.NoError(t, err)
	assert.Equal(t, keyset.JwksJSON, inline)
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
		ObjectMeta: metav1.ObjectMeta{
			Name:      "jwks-store-legacy-name",
			Namespace: "agentgateway-system",
			Labels:    JwksStoreConfigMapLabel(DefaultJwksStorePrefix),
		},
	}
	assert.NoError(t, SetJwksInConfigMap(cm, keyset))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{cm}),
		DefaultJwksStorePrefix,
		"agentgateway-system",
	)
	lookupIndex := NewLookup(
		persisted,
		staticLookupResolver{resolved: &ResolvedJwksRequest{
			Target: remotehttp.ResolvedTarget{
				Key:    target.Key(),
				Target: target,
			},
		}},
	)
	lookupImpl := lookupIndex.(*lookup)
	lookupImpl.cache.persisted.entries.WaitUntilSynced(stop)

	_, err := lookupIndex.InlineForOwner(krt.TestingDummyContext{}, RemoteJwksOwner{})

	assert.EqualError(t, err, `jwks keyset for "https://issuer.example/jwks" isn't available (not yet fetched or fetch failed)`)
}

func TestLookupPropagatesResolverError(t *testing.T) {
	sentinel := errors.New("resolver failed")
	lookupIndex := NewLookup(
		NewPersistedEntriesFromCollection(
			krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, nil),
			DefaultJwksStorePrefix,
			"agentgateway-system",
		),
		staticLookupResolver{err: sentinel},
	)

	_, err := lookupIndex.InlineForOwner(krt.TestingDummyContext{}, RemoteJwksOwner{})

	assert.ErrorIs(t, err, sentinel)
}

func TestLookupFailsWhenPersistedCacheIsNotConfigured(t *testing.T) {
	target := remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}
	lookupIndex := &lookup{
		resolver: staticLookupResolver{resolved: &ResolvedJwksRequest{
			Target: remotehttp.ResolvedTarget{
				Key:    target.Key(),
				Target: target,
			},
		}},
		cache: nil,
	}

	_, err := lookupIndex.InlineForOwner(krt.TestingDummyContext{}, RemoteJwksOwner{})

	assert.EqualError(t, err, "jwks persisted cache is not configured")
}
