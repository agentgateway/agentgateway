package plugins

import (
	"crypto/x509"
	"net/url"
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
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	agwv1alpha1 "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
)

func TestBuildOIDCDiscoveryURLRequiresHTTPS(t *testing.T) {
	_, err := buildOIDCDiscoveryURL("http://issuer.example.com")
	require.Error(t, err)
	require.ErrorContains(t, err, "issuer must use https (or http on loopback hosts)")
}

func TestBuildOIDCDiscoveryURLAppendsWellKnownPath(t *testing.T) {
	got, err := buildOIDCDiscoveryURL("https://issuer.example.com/realm/")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if want := "https://issuer.example.com/realm/.well-known/openid-configuration"; got != want {
		t.Fatalf("unexpected discovery url: got %q want %q", got, want)
	}
}

func TestBuildOIDCDiscoveryURLAllowsLoopbackHTTP(t *testing.T) {
	got, err := buildOIDCDiscoveryURL("http://127.0.0.1:8080")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if want := "http://127.0.0.1:8080/.well-known/openid-configuration"; got != want {
		t.Fatalf("unexpected discovery url: got %q want %q", got, want)
	}
}

func TestBuildOIDCDiscoveryURLRejectsQuery(t *testing.T) {
	_, err := buildOIDCDiscoveryURL("https://issuer.example.com?tenant=a")
	require.Error(t, err)
	require.ErrorContains(t, err, "issuer must not contain query or fragment")
}

func TestBuildOIDCDiscoveryURLRejectsFragment(t *testing.T) {
	_, err := buildOIDCDiscoveryURL("https://issuer.example.com#frag")
	require.Error(t, err)
	require.ErrorContains(t, err, "issuer must not contain query or fragment")
}

func TestBuildOIDCDiscoveryURLRejectsUserinfo(t *testing.T) {
	_, err := buildOIDCDiscoveryURL("https://user:pass@issuer.example.com")
	require.Error(t, err)
	require.ErrorContains(t, err, "issuer must not include userinfo")
}

func TestValidateOIDCDiscoveryMetadataEndpoints(t *testing.T) {
	tests := []struct {
		name     string
		metadata oidcDiscoveryDocument
		wantErr  string
	}{
		{
			name: "rejects non-https jwks uri on non-loopback host",
			metadata: oidcDiscoveryDocument{
				AuthorizationEndpoint: "https://issuer.example.com/authorize",
				TokenEndpoint:         "https://issuer.example.com/token",
				JwksURI:               "http://evil.example.com/jwks",
			},
			wantErr: "jwks_uri must use https (or http on loopback hosts)",
		},
		{
			name: "rejects userinfo in token endpoint",
			metadata: oidcDiscoveryDocument{
				AuthorizationEndpoint: "https://issuer.example.com/authorize",
				TokenEndpoint:         "https://user:pass@issuer.example.com/token",
				JwksURI:               "https://issuer.example.com/jwks",
			},
			wantErr: "token_endpoint must not contain fragment or userinfo",
		},
		{
			name: "allows query and loopback http endpoint urls",
			metadata: oidcDiscoveryDocument{
				AuthorizationEndpoint: "https://issuer.example.com/authorize?foo=bar",
				TokenEndpoint:         "http://127.0.0.1:8080/token",
				JwksURI:               "https://issuer.example.com/jwks?cache=1",
				EndSessionEndpoint:    "https://issuer.example.com/logout?next=%2F",
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := validateOIDCDiscoveryMetadataEndpoints(&tt.metadata)
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
	ctx := buildMockPolicyCtx(t, []any{
		&corev1.ConfigMap{
			ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"},
			Data: map[string]string{
				"ca.crt": `-----BEGIN CERTIFICATE-----
MIIFfDCCA2SgAwIBAgIUOBEwNkgGCBk5gTlks4MgZjBwcB0wDQYJKoZIhvcNAQEL
BQAwKzEpMCcGA1UEAwwgZHVtbXktaWRwLmRlZmF1bHQsTz1rZ2F0ZXdheS5kZXYw
HhcNMjUxMjEyMjIyNTAyWhcNMzUxMjEwMjIyNTAyWjArMSkwJwYDVQQDDCBkdW1t
eS1pZHAuZGVmYXVsdCxPPWtnYXRld2F5LmRldjCCAiIwDQYJKoZIhvcNAQEBBQAD
ggIPADCCAgoCggIBAKPDXO2JEDlruWLQACZqQyFoJTw9dUpay+QcVrgnDv8ULM9F
wSVpIgiT7/reqfWQsyWH8bhyZ60SD2v6BqRdvU8d5G7Lzjjiv7D1kRmdoM05rHeW
rFWrMsd3tTVYIdkDwsOqb/4/3YXhzZstI8N9I9mqQFfR0Oahjwub1fQqGkU4AldO
WGTgsllI0ZDV8IDuARlOQ8ZysxL2axxXJ4Io4eDMZ6uwbeW5JXv/ajLz3Gx9vpWf
LlfPHCB4/Z+EErw/g55PEM8ftvK5ijT2+QPULSdrkO/YjByV9IPNjYou9JEcI1KP
Q2q4VcjQV83dcRFDw11o6MhOicVNwdTFBia6aStpxU/fsYaoaPiK0OWOZ3SjtoNV
PT17geh5kX+4eTmzdC/9hFh+qncyzfHdomBFQlamQ5Pzg3ngLoNm5Iyk/OuUgLg8
sgYf7coYDygzzagxxpTRS7VyfwqLlMaRbqBUrX9IHVpn17CqtsrI1ihadv9q4wc3
Mxt2rdT1GfpE7yCB/NrAzCe2ZVWkNkX8Zb0taD79r/daOBgakHf9L/EqYTsgGO3s
XiF7G3lbRpLwOKHiHP9YbQCdoh8Y3qzGi9DLlmDIaQShtJPUmCb7u7kL9bW2SPRL
+zH2ZY5258CZWndAGe06wQVgLv0aI7kre+Sf1YfZxRbzE595TBWQO/RRT3I7AgMB
AAGjgZcwgZQwHQYDVR0OBBYEFAIkfyn6riDFT/LhatXG1uS5u8HKMB8GA1UdIwQY
MBaAFAIkfyn6riDFT/LhatXG1uS5u8HKMA8GA1UdEwEB/wQFMAMBAf8wQQYDVR0R
BDowOIIRZHVtbXktaWRwLmRlZmF1bHSCI2R1bW15LWlkcC5kZWZhdWx0LnN2Yy5j
bHVzdGVyLmxvY2FsMA0GCSqGSIb3DQEBCwUAA4ICAQAxzxHhT9uvTBHKeu+7zOdU
A+rju5gPjeItds3r2YdHqqjidkK53qWnvrqteoguT8lxGXaSL0QzL3l9eFp80BIP
8MmlI+zs8Q/cO9gCeEf+3ul+nx2YzF33W/PNahHfLDbLIFDoQMkhTyemEh1GEqmm
6frHgO2OgdIO6jyIF0GN0SFvCW6J32k3teRsN2OLRQCuCftJ/Q2dwuXZfmx0sf0R
Hz7JNBdH9U8iCYhSefd3VWCro2sPB3XT7evH9+orFikvbb5fggo4WGjvc7CPKlMj
59PGlloJCUP9FIhR5/oot6yH9NsdOzDWY51makMhE4nq/ET8omaawSCclTE8mDWk
+s/8MBQkk6T72zaVX6Eqnb0RatIHkr9C6zfy/ZE4E5A6Lw+EwdGPaXg5pCBO0miM
jImoFyNvXEGWY3w6AX8ho1L27ZiTApMTc2fYUYCy4QP+MDjEp1+yFrjFSFpUhF0Z
+Tl37cUWZcm4nUxEcu/pfedKyliR2yKBfi3jg7cDzVB86tSHzIvPgxpl2ivEEb0E
ohncCC1Z//SKb7QFs1Obry3hIIBpEyVVvGB580AdxgLY9nhrvv/6gw01JtEPXczV
1BTCWIUc6WafBlAiWrm3tR36kaRn2RrIlCAFrMznQMafCfMLCTWsYudkrabl7W9n
yamda6yFfH9bkPO+XBK3lQ==
-----END CERTIFICATE-----`,
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
		TLSRoutes:            krttest.GetMockCollection[*gwv1a2.TLSRoute](mock),
		ReferenceGrants:      krttest.GetMockCollection[*gwv1b1.ReferenceGrant](mock),
		BackendTLSPolicies:   krttest.GetMockCollection[*gwv1.BackendTLSPolicy](mock),
		XListenerSets:        krttest.GetMockCollection[*gwxv1a1.XListenerSet](mock),
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
		Krt:         krt.TestingDummyContext{},
		Collections: collections,
	}
}
