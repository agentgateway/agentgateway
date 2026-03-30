package remotehttp_test

import (
	"testing"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestResolve(t *testing.T) {
	systemCAs := gwv1.WellKnownCACertificatesSystem

	tests := []struct {
		name                string
		inputs              []any
		backendRef          gwv1.BackendObjectReference
		defaultPort         string
		wantURL             string
		wantTLSConfig       bool
		wantVerification    agentgateway.InsecureTLSMode
		wantServerName      string
		wantCABundleHash    bool
		wantVerifyConnCheck bool
	}{
		{
			name: "service uses default port and backend tls policy",
			inputs: []any{
				&gwv1.BackendTLSPolicy{
					ObjectMeta: metav1.ObjectMeta{Name: "oauth2-discovery-tls", Namespace: "default"},
					Spec: gwv1.BackendTLSPolicySpec{
						TargetRefs: []gwv1.LocalPolicyTargetReferenceWithSectionName{{
							LocalPolicyTargetReference: gwv1.LocalPolicyTargetReference{
								Group: gwv1.Group(""),
								Kind:  gwv1.Kind("Service"),
								Name:  gwv1.ObjectName("oauth2-discovery"),
							},
						}},
						Validation: gwv1.BackendTLSPolicyValidation{
							Hostname:                gwv1.PreciseHostname("oauth2-discovery.default.svc.cluster.local"),
							WellKnownCACertificates: ptr.Of(systemCAs),
						},
					},
				},
			},
			backendRef: gwv1.BackendObjectReference{
				Name: gwv1.ObjectName("oauth2-discovery"),
				Kind: ptr.Of(gwv1.Kind("Service")),
			},
			defaultPort:      "8443",
			wantURL:          "https://oauth2-discovery.default.svc.cluster.local:8443/",
			wantTLSConfig:    true,
			wantVerification: "",
			wantServerName:   "oauth2-discovery.default.svc.cluster.local",
		},
		{
			name: "backend prefers backend tls over policy and backend tls policy",
			inputs: []any{
				&agentgateway.AgentgatewayBackend{
					ObjectMeta: metav1.ObjectMeta{Name: "discovery-backend", Namespace: "default"},
					Spec: agentgateway.AgentgatewayBackendSpec{
						Static: &agentgateway.StaticBackend{Host: "dummy-idp.default", Port: 8443},
						Policies: &agentgateway.BackendFull{
							BackendSimple: agentgateway.BackendSimple{
								TLS: &agentgateway.BackendTLS{Sni: ptr.Of(agentgateway.SNI("backend.example.com"))},
							},
						},
					},
				},
				&agentgateway.AgentgatewayPolicy{
					ObjectMeta: metav1.ObjectMeta{Name: "backend-policy", Namespace: "default"},
					Spec: agentgateway.AgentgatewayPolicySpec{
						TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{{
							LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
								Group: gwv1.Group(wellknown.AgentgatewayBackendGVK.Group),
								Kind:  gwv1.Kind(wellknown.AgentgatewayBackendGVK.Kind),
								Name:  gwv1.ObjectName("discovery-backend"),
							},
						}},
						Backend: &agentgateway.BackendFull{
							BackendSimple: agentgateway.BackendSimple{
								TLS: &agentgateway.BackendTLS{Sni: ptr.Of(agentgateway.SNI("policy.example.com"))},
							},
						},
					},
				},
				&gwv1.BackendTLSPolicy{
					ObjectMeta: metav1.ObjectMeta{Name: "backend-tls-policy", Namespace: "default"},
					Spec: gwv1.BackendTLSPolicySpec{
						TargetRefs: []gwv1.LocalPolicyTargetReferenceWithSectionName{{
							LocalPolicyTargetReference: gwv1.LocalPolicyTargetReference{
								Group: gwv1.Group(wellknown.AgentgatewayBackendGVK.Group),
								Kind:  gwv1.Kind(wellknown.AgentgatewayBackendGVK.Kind),
								Name:  gwv1.ObjectName("discovery-backend"),
							},
						}},
						Validation: gwv1.BackendTLSPolicyValidation{
							Hostname:                gwv1.PreciseHostname("backendtls.example.com"),
							WellKnownCACertificates: ptr.Of(systemCAs),
						},
					},
				},
			},
			backendRef: gwv1.BackendObjectReference{
				Name:  gwv1.ObjectName("discovery-backend"),
				Group: ptr.Of(gwv1.Group(wellknown.AgentgatewayBackendGVK.Group)),
				Kind:  ptr.Of(gwv1.Kind(wellknown.AgentgatewayBackendGVK.Kind)),
			},
			wantURL:          "https://dummy-idp.default:8443/",
			wantTLSConfig:    true,
			wantVerification: "",
			wantServerName:   "backend.example.com",
		},
		{
			name: "service prefers exact section name policy match",
			inputs: []any{
				testService("oauth2-discovery", "default", []corev1.ServicePort{
					{Name: "http", Port: 8080},
					{Name: "https", Port: 8443},
				}),
				&agentgateway.AgentgatewayPolicy{
					ObjectMeta: metav1.ObjectMeta{
						Name:              "whole-service",
						Namespace:         "default",
						CreationTimestamp: metav1.Unix(20, 0),
					},
					Spec: agentgateway.AgentgatewayPolicySpec{
						TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{{
							LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
								Group: gwv1.Group(""),
								Kind:  gwv1.Kind("Service"),
								Name:  gwv1.ObjectName("oauth2-discovery"),
							},
						}},
						Backend: &agentgateway.BackendFull{
							BackendSimple: agentgateway.BackendSimple{
								TLS: &agentgateway.BackendTLS{Sni: ptr.Of(agentgateway.SNI("whole.example.com"))},
							},
						},
					},
				},
				&agentgateway.AgentgatewayPolicy{
					ObjectMeta: metav1.ObjectMeta{
						Name:              "port-specific",
						Namespace:         "default",
						CreationTimestamp: metav1.Unix(10, 0),
					},
					Spec: agentgateway.AgentgatewayPolicySpec{
						TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{{
							LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
								Group: gwv1.Group(""),
								Kind:  gwv1.Kind("Service"),
								Name:  gwv1.ObjectName("oauth2-discovery"),
							},
							SectionName: ptr.Of(gwv1.SectionName("8443")),
						}},
						Backend: &agentgateway.BackendFull{
							BackendSimple: agentgateway.BackendSimple{
								TLS: &agentgateway.BackendTLS{Sni: ptr.Of(agentgateway.SNI("port.example.com"))},
							},
						},
					},
				},
			},
			backendRef: gwv1.BackendObjectReference{
				Name: gwv1.ObjectName("oauth2-discovery"),
				Kind: ptr.Of(gwv1.Kind("Service")),
				Port: ptr.Of(gwv1.PortNumber(8443)),
			},
			wantURL:          "https://oauth2-discovery.default.svc.cluster.local:8443/",
			wantTLSConfig:    true,
			wantVerification: "",
			wantServerName:   "port.example.com",
		},
		{
			name: "service backend tls policy supports named port section match",
			inputs: []any{
				testService("oauth2-discovery", "default", []corev1.ServicePort{
					{Name: "http", Port: 8080},
					{Name: "https", Port: 8443},
				}),
				&gwv1.BackendTLSPolicy{
					ObjectMeta: metav1.ObjectMeta{
						Name:              "whole-service",
						Namespace:         "default",
						CreationTimestamp: metav1.Unix(20, 0),
					},
					Spec: gwv1.BackendTLSPolicySpec{
						TargetRefs: []gwv1.LocalPolicyTargetReferenceWithSectionName{{
							LocalPolicyTargetReference: gwv1.LocalPolicyTargetReference{
								Group: gwv1.Group(""),
								Kind:  gwv1.Kind("Service"),
								Name:  gwv1.ObjectName("oauth2-discovery"),
							},
						}},
						Validation: gwv1.BackendTLSPolicyValidation{
							Hostname:                gwv1.PreciseHostname("whole.example.com"),
							WellKnownCACertificates: ptr.Of(systemCAs),
						},
					},
				},
				&gwv1.BackendTLSPolicy{
					ObjectMeta: metav1.ObjectMeta{
						Name:              "port-specific",
						Namespace:         "default",
						CreationTimestamp: metav1.Unix(10, 0),
					},
					Spec: gwv1.BackendTLSPolicySpec{
						TargetRefs: []gwv1.LocalPolicyTargetReferenceWithSectionName{{
							LocalPolicyTargetReference: gwv1.LocalPolicyTargetReference{
								Group: gwv1.Group(""),
								Kind:  gwv1.Kind("Service"),
								Name:  gwv1.ObjectName("oauth2-discovery"),
							},
							SectionName: ptr.Of(gwv1.SectionName("https")),
						}},
						Validation: gwv1.BackendTLSPolicyValidation{
							Hostname:                gwv1.PreciseHostname("port.example.com"),
							WellKnownCACertificates: ptr.Of(systemCAs),
						},
					},
				},
			},
			backendRef: gwv1.BackendObjectReference{
				Name: gwv1.ObjectName("oauth2-discovery"),
				Kind: ptr.Of(gwv1.Kind("Service")),
				Port: ptr.Of(gwv1.PortNumber(8443)),
			},
			wantURL:          "https://oauth2-discovery.default.svc.cluster.local:8443/",
			wantTLSConfig:    true,
			wantVerification: "",
			wantServerName:   "port.example.com",
		},
		{
			name: "backend tls hostname mode preserves custom verification",
			inputs: []any{
				&agentgateway.AgentgatewayPolicy{
					ObjectMeta: metav1.ObjectMeta{Name: "backend-policy", Namespace: "default"},
					Spec: agentgateway.AgentgatewayPolicySpec{
						TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{{
							LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
								Group: gwv1.Group(""),
								Kind:  gwv1.Kind("Service"),
								Name:  gwv1.ObjectName("oauth2-discovery"),
							},
						}},
						Backend: &agentgateway.BackendFull{
							BackendSimple: agentgateway.BackendSimple{
								TLS: &agentgateway.BackendTLS{
									InsecureSkipVerify: ptr.Of(agentgateway.InsecureTLSModeHostname),
									Sni:                ptr.Of(agentgateway.SNI("oauth2-discovery.default.svc.cluster.local")),
								},
							},
						},
					},
				},
			},
			backendRef: gwv1.BackendObjectReference{
				Name: gwv1.ObjectName("oauth2-discovery"),
				Kind: ptr.Of(gwv1.Kind("Service")),
				Port: ptr.Of(gwv1.PortNumber(8443)),
			},
			wantURL:             "https://oauth2-discovery.default.svc.cluster.local:8443/",
			wantTLSConfig:       true,
			wantVerification:    agentgateway.InsecureTLSModeHostname,
			wantServerName:      "oauth2-discovery.default.svc.cluster.local",
			wantVerifyConnCheck: true,
		},
		{
			name: "service hashes ca bundle into resolved transport",
			inputs: []any{
				testService("oauth2-discovery", "default", []corev1.ServicePort{{Name: "https", Port: 8443}}),
				&corev1.ConfigMap{
					ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"},
					Data: map[string]string{
						"ca.crt": "-----BEGIN CERTIFICATE-----\nMIIFfDCCA2SgAwIBAgIUOBEwNkgGCBk5gTlks4MgZjBwcB0wDQYJKoZIhvcNAQEL\nBQAwKzEpMCcGA1UEAwwgZHVtbXktaWRwLmRlZmF1bHQsTz1rZ2F0ZXdheS5kZXYw\nHhcNMjUxMjEyMjIyNTAyWhcNMzUxMjEwMjIyNTAyWjArMSkwJwYDVQQDDCBkdW1t\neS1pZHAuZGVmYXVsdCxPPWtnYXRld2F5LmRldjCCAiIwDQYJKoZIhvcNAQEBBQAD\nggIPADCCAgoCggIBAKPDXO2JEDlruWLQACZqQyFoJTw9dUpay+QcVrgnDv8ULM9F\nwSVpIgiT7/reqfWQsyWH8bhyZ60SD2v6BqRdvU8d5G7Lzjjiv7D1kRmdoM05rHeW\nrFWrMsd3tTVYIdkDwsOqb/4/3YXhzZstI8N9I9mqQFfR0Oahjwub1fQqGkU4AldO\nWGTgsllI0ZDV8IDuARlOQ8ZysxL2axxXJ4Io4eDMZ6uwbeW5JXv/ajLz3Gx9vpWf\nLlfPHCB4/Z+EErw/g55PEM8ftvK5ijT2+QPULSdrkO/YjByV9IPNjYou9JEcI1KP\nQ2q4VcjQV83dcRFDw11o6MhOicVNwdTFBia6aStpxU/fsYaoaPiK0OWOZ3SjtoNV\nPT17geh5kX+4eTmzdC/9hFh+qncyzfHdomBFQlamQ5Pzg3ngLoNm5Iyk/OuUgLg8\nsgYf7coYDygzzagxxpTRS7VyfwqLlMaRbqBUrX9IHVpn17CqtsrI1ihadv9q4wc3\nMxt2rdT1GfpE7yCB/NrAzCe2ZVWkNkX8Zb0taD79r/daOBgakHf9L/EqYTsgGO3s\nXiF7G3lbRpLwOKHiHP9YbQCdoh8Y3qzGi9DLlmDIaQShtJPUmCb7u7kL9bW2SPRL\n+zH2ZY5258CZWndAGe06wQVgLv0aI7kre+Sf1YfZxRbzE595TBWQO/RRT3I7AgMB\nAAGjgZcwgZQwHQYDVR0OBBYEFAIkfyn6riDFT/LhatXG1uS5u8HKMB8GA1UdIwQY\nMBaAFAIkfyn6riDFT/LhatXG1uS5u8HKMA8GA1UdEwEB/wQFMAMBAf8wQQYDVR0R\nBDowOIIRZHVtbXktaWRwLmRlZmF1bHSCI2R1bW15LWlkcC5kZWZhdWx0LnN2Yy5j\nbHVzdGVyLmxvY2FsMA0GCSqGSIb3DQEBCwUAA4ICAQAxzxHhT9uvTBHKeu+7zOdU\nA+rju5gPjeItds3r2YdHqqjidkK53qWnvrqteoguT8lxGXaSL0QzL3l9eFp80BIP\n8MmlI+zs8Q/cO9gCeEf+3ul+nx2YzF33W/PNahHfLDbLIFDoQMkhTyemEh1GEqmm\n6frHgO2OgdIO6jyIF0GN0SFvCW6J32k3teRsN2OLRQCuCftJ/Q2dwuXZfmx0sf0R\nHz7JNBdH9U8iCYhSefd3VWCro2sPB3XT7evH9+orFikvbb5fggo4WGjvc7CPKlMj\n59PGlloJCUP9FIhR5/oot6yH9NsdOzDWY51makMhE4nq/ET8omaawSCclTE8mDWk\n+s/8MBQkk6T72zaVX6Eqnb0RatIHkr9C6zfy/ZE4E5A6Lw+EwdGPaXg5pCBO0miM\njImoFyNvXEGWY3w6AX8ho1L27ZiTApMTc2fYUYCy4QP+MDjEp1+yFrjFSFpUhF0Z\n+Tl37cUWZcm4nUxEcu/pfedKyliR2yKBfi3jg7cDzVB86tSHzIvPgxpl2ivEEb0E\nohncCC1Z//SKb7QFs1Obry3hIIBpEyVVvGB580AdxgLY9nhrvv/6gw01JtEPXczV\n1BTCWIUc6WafBlAiWrm3tR36kaRn2RrIlCAFrMznQMafCfMLCTWsYudkrabl7W9n\nyamda6yFfH9bkPO+XBK3lQ==\n-----END CERTIFICATE-----\n",
					},
				},
				&agentgateway.AgentgatewayPolicy{
					ObjectMeta: metav1.ObjectMeta{Name: "backend-policy", Namespace: "default"},
					Spec: agentgateway.AgentgatewayPolicySpec{
						TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{{
							LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
								Group: gwv1.Group(""),
								Kind:  gwv1.Kind("Service"),
								Name:  gwv1.ObjectName("oauth2-discovery"),
							},
						}},
						Backend: &agentgateway.BackendFull{
							BackendSimple: agentgateway.BackendSimple{
								TLS: &agentgateway.BackendTLS{
									CACertificateRefs: []corev1.LocalObjectReference{{Name: "ca"}},
								},
							},
						},
					},
				},
			},
			backendRef: gwv1.BackendObjectReference{
				Name: gwv1.ObjectName("oauth2-discovery"),
				Kind: ptr.Of(gwv1.Kind("Service")),
				Port: ptr.Of(gwv1.PortNumber(8443)),
			},
			wantURL:          "https://oauth2-discovery.default.svc.cluster.local:8443/",
			wantTLSConfig:    true,
			wantVerification: "",
			wantCABundleHash: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			ctx := testutils.BuildMockPolicyContext(t, tt.inputs)
			resolver := testutils.BuildRemoteHTTPResolver(ctx.Collections)

			resolved, err := resolver.Resolve(ctx.Krt, remotehttp.ResolveInput{
				ParentName:       "gw-policy",
				DefaultNamespace: "default",
				BackendRef:       tt.backendRef,
				Path:             "/",
				DefaultPort:      tt.defaultPort,
			})
			require.NoError(t, err)
			require.NotNil(t, resolved)
			require.Equal(t, tt.wantURL, resolved.Target.URL)
			if !tt.wantTLSConfig {
				require.Nil(t, resolved.TLSConfig)
				return
			}

			require.Equal(t, tt.wantVerification, resolved.Target.Transport.Verification)
			require.Equal(t, tt.wantServerName, resolved.Target.Transport.ServerName)
			require.Equal(t, tt.wantCABundleHash, resolved.Target.Transport.CABundleHash != "")
			require.NotNil(t, resolved.TLSConfig)
			require.Equal(t, tt.wantServerName, resolved.TLSConfig.ServerName)
			require.Equal(t, tt.wantVerifyConnCheck, resolved.TLSConfig.VerifyConnection != nil)
		})
	}
}

func testService(name, namespace string, ports []corev1.ServicePort) *corev1.Service {
	return &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: namespace,
		},
		Spec: corev1.ServiceSpec{
			Ports: ports,
		},
	}
}
