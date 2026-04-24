package oidc_test

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
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/oidc"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestResolveOwner(t *testing.T) {
	serviceBackend := gwv1.BackendObjectReference{
		Group:     ptr.Of(gwv1.Group("")),
		Kind:      ptr.Of(gwv1.Kind("Service")),
		Name:      gwv1.ObjectName("dummy-idp"),
		Namespace: ptr.Of(gwv1.Namespace("default")),
		Port:      ptr.Of(gwv1.PortNumber(8443)),
	}
	agentgatewayBackend := gwv1.BackendObjectReference{
		Group: ptr.Of(gwv1.Group(wellknown.AgentgatewayBackendGVK.Group)),
		Kind:  ptr.Of(gwv1.Kind(wellknown.AgentgatewayBackendGVK.Kind)),
		Name:  gwv1.ObjectName("dummy-idp"),
		Port:  ptr.Of(gwv1.PortNumber(8443)),
	}

	tests := []struct {
		name                string
		inputs              []any
		backendRef          *gwv1.BackendObjectReference
		disableAutoResolver bool
		expectedError       string
		expectedURL         string
		expectedTLS         *tls.Config
	}{
		{
			name:                "errors when resolver is not initialized",
			backendRef:          &serviceBackend,
			disableAutoResolver: true,
			expectedError:       "remote http resolver hasn't been initialized",
		},
		{
			name: "service-backed oidc uses attached backend tls policy",
			inputs: []any{
				testCAConfigMap(),
				attachedBackendPolicy(gwv1.Group(""), gwv1.Kind("Service"), "dummy-idp", &agentgateway.BackendTLS{
					CACertificateRefs: []corev1.LocalObjectReference{{Name: "ca"}},
					Sni:               ptr.Of(agentgateway.SNI("test.testns")),
					AlpnProtocols:     ptr.Of([]agentgateway.TinyString{"test1", "test2"}),
				}),
			},
			backendRef:  &serviceBackend,
			expectedURL: "https://dummy-idp.default.svc.cluster.local:8443/.well-known/openid-configuration",
			expectedTLS: &tls.Config{ //nolint:gosec
				ServerName: "test.testns",
				NextProtos: []string{"test1", "test2"},
				RootCAs:    testRootCAs(t),
			},
		},
		{
			name: "agentgateway-backend oidc uses attached backend policy",
			inputs: []any{
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
			backendRef:  &agentgatewayBackend,
			expectedURL: "https://dummy-idp.default:8443/.well-known/openid-configuration",
			expectedTLS: &tls.Config{ //nolint:gosec
				ServerName: "test.testns",
				NextProtos: []string{"test1", "test2"},
				RootCAs:    testRootCAs(t),
			},
		},
		{
			name:          "returns resolver error for missing backend",
			backendRef:    &agentgatewayBackend,
			expectedError: "backend default/dummy-idp not found, policy default/gw-policy",
		},
		{
			name: "returns resolver error for non-static backend",
			inputs: []any{
				&agentgateway.AgentgatewayBackend{
					ObjectMeta: metav1.ObjectMeta{Name: "dummy-idp", Namespace: "default"},
				},
			},
			backendRef:    &agentgatewayBackend,
			expectedError: "only static backends are supported; backend: default/dummy-idp, policy: default/gw-policy",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			policy := gatewayOIDCPolicy(tt.backendRef)
			ctx := testutils.BuildMockPolicyContext(t, append([]any{policy}, tt.inputs...))
			owner, ok := oidc.PolicyOIDCLookupOwner(policy.Namespace, policy.Name, policy.Spec.Traffic.OIDC)
			require.True(t, ok)

			resolver := oidc.NewResolver(ctx.Resolver)
			if tt.disableAutoResolver {
				resolver = oidc.NewResolver(nil)
			}

			resolved, err := resolver.ResolveOwner(ctx.Krt, owner)
			if tt.expectedError != "" {
				require.EqualError(t, err, tt.expectedError)
				require.Nil(t, resolved)
				return
			}

			require.NoError(t, err)
			require.NotNil(t, resolved)
			require.Equal(t, tt.expectedURL, resolved.Target.Target.URL)
			require.Equal(t, resolved.Target.Target.Key(), resolved.Target.Key)
			if tt.expectedTLS == nil {
				require.Nil(t, resolved.Target.TLSConfig)
				return
			}

			require.NotNil(t, resolved.Target.TLSConfig)
			require.Equal(t, tt.expectedTLS.ServerName, resolved.Target.TLSConfig.ServerName)
			require.Equal(t, tt.expectedTLS.NextProtos, resolved.Target.TLSConfig.NextProtos)
			require.True(t, tt.expectedTLS.RootCAs.Equal(resolved.Target.TLSConfig.RootCAs))
		})
	}
}

func gatewayOIDCPolicy(backendRef *gwv1.BackendObjectReference) *agentgateway.AgentgatewayPolicy {
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
				OIDC: &agentgateway.OIDC{
					IssuerURL:   "https://issuer.example",
					ClientID:    "test-client",
					RedirectURI: "https://app.example/callback",
					Backend:     backendRef,
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
-----END CERTIFICATE-----`
