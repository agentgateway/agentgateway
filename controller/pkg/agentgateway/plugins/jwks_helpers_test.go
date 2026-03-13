package plugins_test

import (
	"crypto/x509"
	"os"
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/identity/oidc"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
)

func init() {
	oidc.BuildProviderConfigMapNamespacedNameFunc(oidc.DefaultStorePrefix, "agentgateway-system")
}

func TestBuildOIDCDiscoveryURL(t *testing.T) {
	tests := []struct {
		name    string
		issuer  string
		wantURL string
		wantErr string
	}{
		{
			name:    "appends well known path",
			issuer:  "https://issuer.example.com/realm/",
			wantURL: "https://issuer.example.com/realm/.well-known/openid-configuration",
		},
		{
			name:    "allows loopback http",
			issuer:  "http://127.0.0.1:8080",
			wantURL: "http://127.0.0.1:8080/.well-known/openid-configuration",
		},
		{
			name:    "rejects non loopback http",
			issuer:  "http://issuer.example.com",
			wantErr: "issuer must use https (or http on loopback hosts)",
		},
		{
			name:    "rejects query",
			issuer:  "https://issuer.example.com?tenant=a",
			wantErr: "issuer must not contain query or fragment",
		},
		{
			name:    "rejects fragment",
			issuer:  "https://issuer.example.com#frag",
			wantErr: "issuer must not contain query or fragment",
		},
		{
			name:    "rejects userinfo",
			issuer:  "https://user:pass@issuer.example.com",
			wantErr: "issuer must not include userinfo",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := oidc.BuildDiscoveryURL(tt.issuer)
			if tt.wantErr == "" {
				require.NoError(t, err)
				require.Equal(t, tt.wantURL, got)
				return
			}

			require.Error(t, err)
			require.ErrorContains(t, err, tt.wantErr)
		})
	}
}

func TestValidateOIDCDiscoveryMetadataEndpoints(t *testing.T) {
	tests := []struct {
		name     string
		metadata oidc.DiscoveryDocument
		wantErr  string
	}{
		{
			name: "rejects non-https jwks uri on non-loopback host",
			metadata: oidc.DiscoveryDocument{
				AuthorizationEndpoint: "https://issuer.example.com/authorize",
				TokenEndpoint:         "https://issuer.example.com/token",
				JwksURI:               "http://evil.example.com/jwks",
			},
			wantErr: "jwks_uri must use https (or http on loopback hosts)",
		},
		{
			name: "rejects userinfo in token endpoint",
			metadata: oidc.DiscoveryDocument{
				AuthorizationEndpoint: "https://issuer.example.com/authorize",
				TokenEndpoint:         "https://user:pass@issuer.example.com/token",
				JwksURI:               "https://issuer.example.com/jwks",
			},
			wantErr: "token_endpoint must not contain fragment or userinfo",
		},
		{
			name: "allows query and loopback http endpoint urls",
			metadata: oidc.DiscoveryDocument{
				AuthorizationEndpoint: "https://issuer.example.com/authorize?foo=bar",
				TokenEndpoint:         "http://127.0.0.1:8080/token",
				JwksURI:               "https://issuer.example.com/jwks?cache=1",
				EndSessionEndpoint:    "https://issuer.example.com/logout?next=%2F",
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := oidc.ValidateDiscoveryMetadataEndpoints(&tt.metadata)
			if tt.wantErr == "" {
				require.NoError(t, err)
				return
			}
			require.Error(t, err)
			require.ErrorContains(t, err, tt.wantErr)
		})
	}
}

func TestBuildOIDCProviderSourceUsesBackendTLSPolicyForService(t *testing.T) {
	caCert := mustReadDummyIDPCATestCert(t)

	ctx := testutils.BuildMockPolicyContext(t, []any{
		&corev1.ConfigMap{
			ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"},
			Data: map[string]string{
				"ca.crt": caCert,
			},
		},
		&gwv1.BackendTLSPolicy{
			ObjectMeta: metav1.ObjectMeta{Name: "oauth2-discovery-tls", Namespace: "default"},
			Spec: gwv1.BackendTLSPolicySpec{
				TargetRefs: []gwv1.LocalPolicyTargetReferenceWithSectionName{
					{
						LocalPolicyTargetReference: gwv1.LocalPolicyTargetReference{
							Group: gwv1.Group(""),
							Kind:  gwv1.Kind("Service"),
							Name:  gwv1.ObjectName("oauth2-discovery"),
						},
					},
				},
				Validation: gwv1.BackendTLSPolicyValidation{
					Hostname: gwv1.PreciseHostname("oauth2-discovery.default.svc.cluster.local"),
					CACertificateRefs: []gwv1.LocalObjectReference{
						{Name: "ca"},
					},
				},
			},
		},
		testService("oauth2-discovery", "default", 8443),
	})

	source, err := plugins.BuildOIDCProviderSource(
		ctx,
		"oauth2-policy",
		"default",
		"https://issuer.example.com",
		&gwv1.BackendObjectReference{
			Name: gwv1.ObjectName("oauth2-discovery"),
			Kind: ptr.Of(gwv1.Kind("Service")),
			Port: ptr.Of(gwv1.PortNumber(8443)),
		},
	)
	require.NoError(t, err)
	require.Equal(
		t,
		"https://oauth2-discovery.default.svc.cluster.local:8443/.well-known/openid-configuration",
		source.RequestURL,
	)
	require.Equal(t, "issuer.example.com", source.HostOverride)
	require.NotNil(t, source.TlsConfig)
	require.Equal(t, "oauth2-discovery.default.svc.cluster.local", source.TlsConfig.ServerName)
	require.NotNil(t, source.TlsConfig.RootCAs)
	require.False(t, source.TlsConfig.RootCAs.Equal(x509.NewCertPool()))
}

func mustReadDummyIDPCATestCert(t *testing.T) string {
	t.Helper()

	path := filepath.Join("..", "..", "..", "hack", "testbox", "dummy-idp-ca.crt")
	contents, err := os.ReadFile(path)
	require.NoError(t, err)
	return string(contents)
}
