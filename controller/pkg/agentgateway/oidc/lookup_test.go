package oidc

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
	resolved *ResolvedOidcRequest
	err      error
}

func (r staticLookupResolver) ResolveOwner(krt.HandlerContext, RemoteOidcOwner) (*ResolvedOidcRequest, error) {
	return r.resolved, r.err
}

func TestLookupFailsWhenProviderIsNotYetFetched(t *testing.T) {
	stop := test.NewStop(t)
	target := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}
	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, nil),
		DefaultStorePrefix,
		"agentgateway-system",
	)
	lkp := NewLookup(
		persisted,
		staticLookupResolver{resolved: &ResolvedOidcRequest{
			Target: remotehttp.ResolvedTarget{
				Key:    target.Key(),
				Target: target,
			},
		}},
	)
	lkpImpl := lkp.(*lookupImpl)
	lkpImpl.cache.persisted.entries.WaitUntilSynced(stop)

	_, err := lkp.ResolveForOwner(krt.TestingDummyContext{}, RemoteOidcOwner{})

	assert.EqualError(t, err, `oidc provider for "https://issuer.example/.well-known/openid-configuration" isn't available (not yet fetched or fetch failed)`)
}

func TestLookupReturnsPersistedProvider(t *testing.T) {
	stop := test.NewStop(t)
	target := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}
	expectedIssuer := "https://issuer.example"
	requestKey := oidcRequestKey(target, expectedIssuer)
	provider := DiscoveredProvider{
		RequestKey:    requestKey,
		IssuerURL:     expectedIssuer,
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
	assert.NoError(t, SetProviderInConfigMap(cm, provider))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{cm}),
		DefaultStorePrefix,
		"agentgateway-system",
	)
	lkp := NewLookup(
		persisted,
		staticLookupResolver{resolved: &ResolvedOidcRequest{
			ExpectedIssuer: expectedIssuer,
			Target: remotehttp.ResolvedTarget{
				Key:    target.Key(),
				Target: target,
			},
		}},
	)
	lkpImpl := lkp.(*lookupImpl)
	lkpImpl.cache.persisted.entries.WaitUntilSynced(stop)

	got, err := lkp.ResolveForOwner(krt.TestingDummyContext{}, RemoteOidcOwner{})

	assert.NoError(t, err)
	assert.Equal(t, sampleJWKS, got.JwksJSON)
	assert.Equal(t, expectedIssuer, got.IssuerURL)
}

func TestLookupRequiresCanonicalConfigMapName(t *testing.T) {
	stop := test.NewStop(t)
	target := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}
	provider := DiscoveredProvider{
		RequestKey:    target.Key(),
		IssuerURL:     "https://issuer.example",
		TokenEndpoint: "https://issuer.example/token",
		JwksURI:       "https://issuer.example/jwks",
		JwksJSON:      sampleJWKS,
	}
	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			// Non-canonical name.
			Name:      "oidc-store-legacy-name",
			Namespace: "agentgateway-system",
			Labels:    OidcStoreConfigMapLabel(DefaultStorePrefix),
		},
	}
	assert.NoError(t, SetProviderInConfigMap(cm, provider))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{cm}),
		DefaultStorePrefix,
		"agentgateway-system",
	)
	lkp := NewLookup(
		persisted,
		staticLookupResolver{resolved: &ResolvedOidcRequest{
			Target: remotehttp.ResolvedTarget{
				Key:    target.Key(),
				Target: target,
			},
		}},
	)
	lkpImpl := lkp.(*lookupImpl)
	lkpImpl.cache.persisted.entries.WaitUntilSynced(stop)

	_, err := lkp.ResolveForOwner(krt.TestingDummyContext{}, RemoteOidcOwner{})

	assert.EqualError(t, err, `oidc provider for "https://issuer.example/.well-known/openid-configuration" isn't available (not yet fetched or fetch failed)`)
}

func TestLookupPropagatesResolverError(t *testing.T) {
	sentinel := errors.New("resolver failed")
	lkp := NewLookup(
		NewPersistedEntriesFromCollection(
			krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, nil),
			DefaultStorePrefix,
			"agentgateway-system",
		),
		staticLookupResolver{err: sentinel},
	)

	_, err := lkp.ResolveForOwner(krt.TestingDummyContext{}, RemoteOidcOwner{})

	assert.ErrorIs(t, err, sentinel)
}

func TestLookupFailsWhenCacheIsNotConfigured(t *testing.T) {
	target := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}
	lkp := &lookupImpl{
		resolver: staticLookupResolver{resolved: &ResolvedOidcRequest{
			Target: remotehttp.ResolvedTarget{
				Key:    target.Key(),
				Target: target,
			},
		}},
		cache: nil,
	}

	_, err := lkp.ResolveForOwner(krt.TestingDummyContext{}, RemoteOidcOwner{})

	assert.EqualError(t, err, "oidc persisted cache is not configured")
}
