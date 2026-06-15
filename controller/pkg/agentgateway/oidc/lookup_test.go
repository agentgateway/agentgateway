package oidc

import (
	"testing"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

type staticLookupResolver struct {
	resolved *ResolvedOidcRequest
}

func (r staticLookupResolver) ResolveOwner(krt.HandlerContext, RemoteOidcOwner) (*ResolvedOidcRequest, error) {
	return r.resolved, nil
}

func TestLookupUsesResolvedTargetKey(t *testing.T) {
	stop := t.Context().Done()
	issuer := "https://issuer.example"
	directTarget := remotehttp.FetchTarget{URL: issuer + "/.well-known/openid-configuration"}
	backendTarget := remotehttp.FetchTarget{
		URL: "https://issuer.default.svc.cluster.local:8443/.well-known/openid-configuration",
		Transport: remotehttp.TransportFingerprint{
			ServerName: "issuer.example",
		},
	}
	requestKey := oidcRequestKey(backendTarget, issuer, true)
	require.NotEqual(t, oidcRequestKey(directTarget, issuer, false), requestKey)

	provider := DiscoveredProvider{
		RequestKey:            requestKey,
		IssuerURL:             issuer,
		AuthorizationEndpoint: issuer + "/auth",
		TokenEndpoint:         issuer + "/token",
		JwksURI:               issuer + "/jwks",
		JwksInline:            `{"keys":[]}`,
	}
	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      remotecache.ConfigMapName(DefaultStorePrefix, requestKey),
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultStorePrefix),
		},
	}
	require.NoError(t, SetProviderInConfigMap(cm, provider))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](nil, []*corev1.ConfigMap{cm}, krt.WithName("oidc/LookupResolvedTargetConfigMaps")),
		"agentgateway-system",
	)
	lookup := NewLookup(persisted, staticLookupResolver{resolved: &ResolvedOidcRequest{
		Target: remotehttp.ResolvedTarget{
			Target: backendTarget,
		},
		ExpectedIssuer: issuer,
		ViaBackendRef:  true,
	}})
	persisted.Collection().WaitUntilSynced(stop)

	got, err := lookup.ResolveForOwner(krt.TestingDummyContext{}, RemoteOidcOwner{
		Config: agentgateway.OIDC{IssuerURL: issuer},
	})

	require.NoError(t, err)
	require.Equal(t, requestKey, got.RequestKey)
}
