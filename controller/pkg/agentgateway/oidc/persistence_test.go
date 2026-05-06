package oidc

import (
	"context"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil/krttest"
)

func TestSetAndReadConfigMapRoundTrip(t *testing.T) {
	original := DiscoveredProvider{
		RequestKey:            remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key(),
		IssuerURL:             "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
		JwksInline:            `{"keys":[]}`,
		FetchedAt:             time.Unix(1000, 0).UTC(),
	}
	cm := &corev1.ConfigMap{}

	require.NoError(t, SetProviderInConfigMap(cm, original))

	got, err := ProviderFromConfigMap(cm)

	require.NoError(t, err)
	require.Equal(t, original, got)
}

func TestPersistedEntriesLoadPrefersNewestProviderAcrossDuplicates(t *testing.T) {
	requestKey := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}.Key()
	canonical := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      remotecache.ConfigMapName(DefaultStorePrefix, requestKey),
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultStorePrefix),
		},
	}
	require.NoError(t, SetProviderInConfigMap(canonical, DiscoveredProvider{
		RequestKey: requestKey,
		IssuerURL:  "https://issuer.example",
		JwksInline: `{"keys":[]}`,
		FetchedAt:  time.Unix(100, 0).UTC(),
	}))

	legacy := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "oidc-store-legacy-name",
			Namespace: "agentgateway-system",
			Labels:    remotecache.ConfigMapLabels(DefaultStorePrefix),
		},
	}
	require.NoError(t, SetProviderInConfigMap(legacy, DiscoveredProvider{
		RequestKey: requestKey,
		IssuerURL:  "https://issuer.example",
		JwksInline: `{"keys":[]}`,
		FetchedAt:  time.Unix(200, 0).UTC(),
	}))

	persisted := NewPersistedEntriesFromCollection(
		krt.NewStaticCollection[*corev1.ConfigMap](krttest.AlwaysSynced{}, []*corev1.ConfigMap{legacy, canonical}),
		DefaultStorePrefix,
		"agentgateway-system",
	)
	providers, err := persisted.LoadAll(context.Background())

	require.NoError(t, err)
	require.Len(t, providers, 1)
	require.Equal(t, time.Unix(200, 0).UTC(), providers[0].FetchedAt)
}
