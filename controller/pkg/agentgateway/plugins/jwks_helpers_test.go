package plugins

import (
	"crypto/x509"
	"net/url"
	"os"
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/require"
	networkingclient "istio.io/client-go/pkg/apis/networking/v1"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/kube/krt/krttest"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	discovery "k8s.io/api/discovery/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"

	agwv1alpha1 "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/backendtransport"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/oidc"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
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

func TestBuildProviderRequestURLAndTLSUsesBackendTLSPolicyForService(t *testing.T) {
	caCert := mustReadDummyIDPCATestCert(t)

	ctx := buildMockPolicyCtx(t, []any{
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
	})

	targetURL, err := url.Parse("https://issuer.example.com/.well-known/openid-configuration")
	require.NoError(t, err)

	requestURL, tlsConfig, err := buildProviderRequestURLAndTLS(
		ctx,
		"oauth2-policy",
		"default",
		gwv1.BackendObjectReference{
			Name: gwv1.ObjectName("oauth2-discovery"),
			Kind: ptr.Of(gwv1.Kind("Service")),
			Port: ptr.Of(gwv1.PortNumber(8443)),
		},
		targetURL,
	)
	require.NoError(t, err)
	require.Equal(t, "https://oauth2-discovery.default.svc.cluster.local:8443/.well-known/openid-configuration", requestURL)
	require.NotNil(t, tlsConfig)
	require.Equal(t, "oauth2-discovery.default.svc.cluster.local", tlsConfig.ServerName)
	require.NotNil(t, tlsConfig.RootCAs)
	require.False(t, tlsConfig.RootCAs.Equal(x509.NewCertPool()))
}

func mustReadDummyIDPCATestCert(t *testing.T) string {
	t.Helper()

	path := filepath.Join("..", "..", "..", "hack", "testbox", "dummy-idp-ca.crt")
	contents, err := os.ReadFile(path)
	require.NoError(t, err)
	return string(contents)
}

func buildMockPolicyCtx(t *testing.T, inputs []any) PolicyCtx {
	mock := krttest.NewMock(t, inputs)
	collections := &AgwCollections{
		Namespaces:           krttest.GetMockCollection[*corev1.Namespace](mock),
		Nodes:                krttest.GetMockCollection[*corev1.Node](mock),
		Pods:                 krttest.GetMockCollection[*corev1.Pod](mock),
		Services:             krttest.GetMockCollection[*corev1.Service](mock),
		Secrets:              krttest.GetMockCollection[*corev1.Secret](mock),
		ConfigMaps:           krttest.GetMockCollection[*corev1.ConfigMap](mock),
		EndpointSlices:       krttest.GetMockCollection[*discovery.EndpointSlice](mock),
		WorkloadEntries:      krttest.GetMockCollection[*networkingclient.WorkloadEntry](mock),
		ServiceEntries:       krttest.GetMockCollection[*networkingclient.ServiceEntry](mock),
		GatewayClasses:       krttest.GetMockCollection[*gwv1.GatewayClass](mock),
		Gateways:             krttest.GetMockCollection[*gwv1.Gateway](mock),
		HTTPRoutes:           krttest.GetMockCollection[*gwv1.HTTPRoute](mock),
		GRPCRoutes:           krttest.GetMockCollection[*gwv1.GRPCRoute](mock),
		TCPRoutes:            krttest.GetMockCollection[*gwv1a2.TCPRoute](mock),
		TLSRoutes:            krttest.GetMockCollection[*gwv1.TLSRoute](mock),
		ReferenceGrants:      krttest.GetMockCollection[*gwv1b1.ReferenceGrant](mock),
		BackendTLSPolicies:   krttest.GetMockCollection[*gwv1.BackendTLSPolicy](mock),
		ListenerSets:         krttest.GetMockCollection[*gwv1.ListenerSet](mock),
		InferencePools:       krttest.GetMockCollection[*inf.InferencePool](mock),
		Backends:             krttest.GetMockCollection[*agwv1alpha1.AgentgatewayBackend](mock),
		AgentgatewayPolicies: krttest.GetMockCollection[*agwv1alpha1.AgentgatewayPolicy](mock),
		ControllerName:       wellknown.DefaultAgwControllerName,
		SystemNamespace:      "kgateway-system",
		IstioNamespace:       "istio-system",
		ClusterID:            "Kubernetes",
	}
	collections.SetupIndexes()
	return PolicyCtx{
		Krt:                    krt.TestingDummyContext{},
		Collections:            collections,
		BackendTransportLookup: backendtransport.NewBackendTransportLookup(collections.ConfigMaps, collections.Services, collections.Backends, collections.AgentgatewayPolicies, collections.BackendTLSPolicies),
	}
}
