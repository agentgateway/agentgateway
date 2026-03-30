package jwks_test

import (
	"crypto/tls"
	"crypto/x509"
	"testing"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestResolveEndpoint(t *testing.T) {
	serviceRemote := remoteProvider(
		"/org-one/keys",
		gwv1.BackendObjectReference{
			Group:     ptr.Of(gwv1.Group("")),
			Kind:      ptr.Of(gwv1.Kind("Service")),
			Name:      gwv1.ObjectName("dummy-idp"),
			Namespace: ptr.Of(gwv1.Namespace("default")),
			Port:      ptr.Of(gwv1.PortNumber(8443)),
		},
	)
	backendRemote := remoteProvider(
		"org-one/keys",
		gwv1.BackendObjectReference{
			Group: ptr.Of(gwv1.Group(wellknown.AgentgatewayBackendGVK.Group)),
			Kind:  ptr.Of(gwv1.Kind(wellknown.AgentgatewayBackendGVK.Kind)),
			Name:  gwv1.ObjectName("dummy-idp"),
			Port:  ptr.Of(gwv1.PortNumber(8443)),
		},
	)

	tests := []struct {
		name                string
		inputs              []any
		remoteProvider      agentgateway.RemoteJWKS
		disableAutoResolver bool
		expectedError       string
		expectedURL         string
		expectedTLS         *tls.Config
	}{
		{
			name:                "errors when resolver is not initialized",
			remoteProvider:      serviceRemote,
			disableAutoResolver: true,
			expectedError:       "remote http resolver hasn't been initialized",
		},
		{
			name: "service-backed remote jwks uses attached backend tls policy",
			inputs: []any{
				gatewayJWTPolicy(serviceRemote),
				testCAConfigMap(),
				attachedBackendPolicy(gwv1.Group(""), gwv1.Kind("Service"), "dummy-idp", &agentgateway.BackendTLS{
					CACertificateRefs: []corev1.LocalObjectReference{{Name: "ca"}},
					Sni:               ptr.Of(agentgateway.SNI("test.testns")),
					AlpnProtocols:     ptr.Of([]agentgateway.TinyString{"test1", "test2"}),
				}),
			},
			remoteProvider: serviceRemote,
			expectedURL:    "https://dummy-idp.default.svc.cluster.local:8443/org-one/keys",
			expectedTLS: &tls.Config{ //nolint:gosec
				ServerName: "test.testns",
				NextProtos: []string{"test1", "test2"},
				RootCAs:    testRootCAs(t),
			},
		},
		{
			name: "backend-backed remote jwks uses attached backend policy",
			inputs: []any{
				gatewayJWTPolicy(backendRemote),
				staticBackend("dummy-idp", "dummy-idp.default", 8443, nil),
				testCAConfigMap(),
				attachedBackendPolicy(
					gwv1.Group(wellknown.AgentgatewayBackendGVK.Group),
					gwv1.Kind(wellknown.AgentgatewayBackendGVK.Kind),
					"dummy-idp",
					&agentgateway.BackendTLS{
						CACertificateRefs: []corev1.LocalObjectReference{{Name: "ca"}},
						Sni:               ptr.Of(agentgateway.SNI("test.testns")),
						AlpnProtocols:     ptr.Of([]agentgateway.TinyString{"test1", "test2"}),
					},
				),
			},
			remoteProvider: backendRemote,
			expectedURL:    "https://dummy-idp.default:8443/org-one/keys",
			expectedTLS: &tls.Config{ //nolint:gosec
				ServerName: "test.testns",
				NextProtos: []string{"test1", "test2"},
				RootCAs:    testRootCAs(t),
			},
		},
		{
			name: "returns resolver error for missing backend",
			inputs: []any{
				gatewayJWTPolicy(backendRemote),
			},
			remoteProvider: backendRemote,
			expectedError:  "backend default/dummy-idp not found, policy default/gw-policy",
		},
		{
			name: "returns resolver error for non-static backend",
			inputs: []any{
				gatewayJWTPolicy(backendRemote),
				&agentgateway.AgentgatewayBackend{
					ObjectMeta: metav1.ObjectMeta{Name: "dummy-idp", Namespace: "default"},
				},
			},
			remoteProvider: backendRemote,
			expectedError:  "only static backends are supported; backend: default/dummy-idp, policy: default/gw-policy",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			ctx := testutils.BuildMockPolicyContext(t, tt.inputs)
			var resolver remotehttp.Resolver
			if !tt.disableAutoResolver {
				resolver = testutils.BuildRemoteHTTPResolver(ctx.Collections)
			}

			endpoint, err := jwks.ResolveEndpoint(ctx.Krt, resolver, "gw-policy", "default", tt.remoteProvider)
			if tt.expectedError != "" {
				require.EqualError(t, err, tt.expectedError)
				require.Nil(t, endpoint)
				return
			}

			require.NoError(t, err)
			require.NotNil(t, endpoint)
			require.Equal(t, tt.expectedURL, endpoint.Target.URL)
			require.Equal(t, endpoint.Key, endpoint.Target.Key())
			if tt.expectedTLS == nil {
				require.Nil(t, endpoint.TLSConfig)
				return
			}

			require.NotNil(t, endpoint.TLSConfig)
			require.Equal(t, tt.expectedTLS.ServerName, endpoint.TLSConfig.ServerName)
			require.Equal(t, tt.expectedTLS.NextProtos, endpoint.TLSConfig.NextProtos)
			require.Equal(t, tt.expectedTLS.InsecureSkipVerify, endpoint.TLSConfig.InsecureSkipVerify)
			require.True(t, tt.expectedTLS.RootCAs.Equal(endpoint.TLSConfig.RootCAs))
		})
	}
}

func gatewayJWTPolicy(remote agentgateway.RemoteJWKS) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{Name: "gw-policy", Namespace: "default"},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{{
				LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
					Group: gwv1.Group(gwv1.GroupVersion.Group),
					Kind:  gwv1.Kind("Gateway"),
					Name:  gwv1.ObjectName("super-gateway"),
				},
			}},
			Traffic: &agentgateway.Traffic{
				JWTAuthentication: &agentgateway.JWTAuthentication{
					Mode: agentgateway.JWTAuthenticationModeStrict,
					Providers: []agentgateway.JWTProvider{{
						Issuer: "https://kgateway.dev",
						JWKS:   agentgateway.JWKS{Remote: &remote},
					}},
				},
			},
		},
	}
}

func attachedBackendPolicy(group gwv1.Group, kind gwv1.Kind, name string, tlsPolicy *agentgateway.BackendTLS) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{Name: "idp-policy", Namespace: "default"},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{{
				LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
					Group: group,
					Kind:  kind,
					Name:  gwv1.ObjectName(name),
				},
			}},
			Backend: &agentgateway.BackendFull{
				BackendSimple: agentgateway.BackendSimple{
					TLS: tlsPolicy,
				},
			},
		},
	}
}

func staticBackend(name, host string, port int32, tlsPolicy *agentgateway.BackendTLS) *agentgateway.AgentgatewayBackend {
	return &agentgateway.AgentgatewayBackend{
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: "default"},
		Spec: agentgateway.AgentgatewayBackendSpec{
			Static: &agentgateway.StaticBackend{
				Host: host,
				Port: port,
			},
			Policies: &agentgateway.BackendFull{
				BackendSimple: agentgateway.BackendSimple{
					TLS: tlsPolicy,
				},
			},
		},
	}
}

func remoteProvider(path string, backendRef gwv1.BackendObjectReference) agentgateway.RemoteJWKS {
	return agentgateway.RemoteJWKS{
		JwksPath:   path,
		BackendRef: backendRef,
	}
}

func testCAConfigMap() *corev1.ConfigMap {
	return &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"},
		Data: map[string]string{
			"ca.crt": testCertPEM,
		},
	}
}

func testRootCAs(t *testing.T) *x509.CertPool {
	t.Helper()
	pool := x509.NewCertPool()
	require.True(t, pool.AppendCertsFromPEM([]byte(testCertPEM)))
	return pool
}

const testCertPEM = `-----BEGIN CERTIFICATE-----
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
-----END CERTIFICATE-----
`
