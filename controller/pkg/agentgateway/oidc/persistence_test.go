package oidc

import (
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	corev1 "k8s.io/api/core/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
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
