package backendtransport_test

import (
	"testing"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/backendtransport"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
)

func TestBackendTransportLookupResolve(t *testing.T) {
	systemCAs := gwv1.WellKnownCACertificatesSystem

	tests := []struct {
		name            string
		inputs          []any
		backendRef      gwv1.BackendObjectReference
		defaultPort     string
		wantConnectHost string
		wantServerName  string
		wantTLS         bool
	}{
		{
			name: "service uses default port and backend tls policy",
			inputs: []any{
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
			defaultPort:     "8443",
			wantConnectHost: "oauth2-discovery.default.svc.cluster.local:8443",
			wantServerName:  "oauth2-discovery.default.svc.cluster.local",
			wantTLS:         true,
		},
		{
			name: "backend prefers backend tls over policy and backend tls policy",
			inputs: []any{
				&agentgateway.AgentgatewayBackend{
					ObjectMeta: metav1.ObjectMeta{Name: "discovery-backend", Namespace: "default"},
					Spec: agentgateway.AgentgatewayBackendSpec{
						Static: &agentgateway.StaticBackend{
							Host: "dummy-idp.default",
							Port: 8443,
						},
						Policies: &agentgateway.BackendFull{
							BackendSimple: agentgateway.BackendSimple{
								TLS: &agentgateway.BackendTLS{
									Sni: ptr.Of(agentgateway.SNI("backend.example.com")),
								},
							},
						},
					},
				},
				&agentgateway.AgentgatewayPolicy{
					ObjectMeta: metav1.ObjectMeta{Name: "backend-policy", Namespace: "default"},
					Spec: agentgateway.AgentgatewayPolicySpec{
						TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{
							{
								LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
									Group: gwv1.Group(wellknown.AgentgatewayBackendGVK.Group),
									Kind:  gwv1.Kind(wellknown.AgentgatewayBackendGVK.Kind),
									Name:  gwv1.ObjectName("discovery-backend"),
								},
							},
						},
						Backend: &agentgateway.BackendFull{
							BackendSimple: agentgateway.BackendSimple{
								TLS: &agentgateway.BackendTLS{
									Sni: ptr.Of(agentgateway.SNI("policy.example.com")),
								},
							},
						},
					},
				},
				&gwv1.BackendTLSPolicy{
					ObjectMeta: metav1.ObjectMeta{Name: "backend-tls-policy", Namespace: "default"},
					Spec: gwv1.BackendTLSPolicySpec{
						TargetRefs: []gwv1.LocalPolicyTargetReferenceWithSectionName{
							{
								LocalPolicyTargetReference: gwv1.LocalPolicyTargetReference{
									Group: gwv1.Group(wellknown.AgentgatewayBackendGVK.Group),
									Kind:  gwv1.Kind(wellknown.AgentgatewayBackendGVK.Kind),
									Name:  gwv1.ObjectName("discovery-backend"),
								},
							},
						},
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
			wantConnectHost: "dummy-idp.default:8443",
			wantServerName:  "backend.example.com",
			wantTLS:         true,
		},
		{
			name: "service prefers exact numeric section policy match",
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
						TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{
							{
								LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
									Group: gwv1.Group(""),
									Kind:  gwv1.Kind("Service"),
									Name:  gwv1.ObjectName("oauth2-discovery"),
								},
							},
						},
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
						TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{
							{
								LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
									Group: gwv1.Group(""),
									Kind:  gwv1.Kind("Service"),
									Name:  gwv1.ObjectName("oauth2-discovery"),
								},
								SectionName: ptr.Of(gwv1.SectionName("8443")),
							},
						},
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
			wantConnectHost: "oauth2-discovery.default.svc.cluster.local:8443",
			wantServerName:  "port.example.com",
			wantTLS:         true,
		},
		{
			name: "service prefers matching backend tls policy section name",
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
						TargetRefs: []gwv1.LocalPolicyTargetReferenceWithSectionName{
							{
								LocalPolicyTargetReference: gwv1.LocalPolicyTargetReference{
									Group: gwv1.Group(""),
									Kind:  gwv1.Kind("Service"),
									Name:  gwv1.ObjectName("oauth2-discovery"),
								},
								SectionName: ptr.Of(gwv1.SectionName("https")),
							},
						},
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
			wantConnectHost: "oauth2-discovery.default.svc.cluster.local:8443",
			wantServerName:  "port.example.com",
			wantTLS:         true,
		},
		{
			name: "backend ignores section specific policy targets",
			inputs: []any{
				&agentgateway.AgentgatewayBackend{
					ObjectMeta: metav1.ObjectMeta{Name: "discovery-backend", Namespace: "default"},
					Spec: agentgateway.AgentgatewayBackendSpec{
						Static: &agentgateway.StaticBackend{
							Host: "dummy-idp.default",
							Port: 8443,
						},
					},
				},
				&agentgateway.AgentgatewayPolicy{
					ObjectMeta: metav1.ObjectMeta{Name: "section-policy", Namespace: "default"},
					Spec: agentgateway.AgentgatewayPolicySpec{
						TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{
							{
								LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
									Group: gwv1.Group(wellknown.AgentgatewayBackendGVK.Group),
									Kind:  gwv1.Kind(wellknown.AgentgatewayBackendGVK.Kind),
									Name:  gwv1.ObjectName("discovery-backend"),
								},
								SectionName: ptr.Of(gwv1.SectionName("provider-a")),
							},
						},
						Backend: &agentgateway.BackendFull{
							BackendSimple: agentgateway.BackendSimple{
								TLS: &agentgateway.BackendTLS{Sni: ptr.Of(agentgateway.SNI("section.example.com"))},
							},
						},
					},
				},
			},
			backendRef: gwv1.BackendObjectReference{
				Name:  gwv1.ObjectName("discovery-backend"),
				Group: ptr.Of(gwv1.Group(wellknown.AgentgatewayBackendGVK.Group)),
				Kind:  ptr.Of(gwv1.Kind(wellknown.AgentgatewayBackendGVK.Kind)),
			},
			wantConnectHost: "dummy-idp.default:8443",
		},
		{
			name: "service ignores backend tls policy from different group",
			inputs: []any{
				testService("oauth2-discovery", "default", []corev1.ServicePort{
					{Name: "https", Port: 8443},
				}),
				&gwv1.BackendTLSPolicy{
					ObjectMeta: metav1.ObjectMeta{Name: "wrong-group", Namespace: "default"},
					Spec: gwv1.BackendTLSPolicySpec{
						TargetRefs: []gwv1.LocalPolicyTargetReferenceWithSectionName{
							{
								LocalPolicyTargetReference: gwv1.LocalPolicyTargetReference{
									Group: gwv1.Group("example.com"),
									Kind:  gwv1.Kind("Service"),
									Name:  gwv1.ObjectName("oauth2-discovery"),
								},
							},
						},
						Validation: gwv1.BackendTLSPolicyValidation{
							Hostname:                gwv1.PreciseHostname("wrong-group.example.com"),
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
			wantConnectHost: "oauth2-discovery.default.svc.cluster.local:8443",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			transport := resolveTransport(t, tt.inputs, tt.backendRef, tt.defaultPort)
			require.Equal(t, tt.wantConnectHost, transport.ConnectHost)
			if !tt.wantTLS {
				require.Nil(t, transport.TLSConfig)
				return
			}
			require.NotNil(t, transport.TLSConfig)
			require.Equal(t, tt.wantServerName, transport.TLSConfig.ServerName)
		})
	}
}

func resolveTransport(
	t *testing.T,
	inputs []any,
	backendRef gwv1.BackendObjectReference,
	defaultPort string,
) *backendtransport.ResolvedBackendTransport {
	t.Helper()
	ctx := testutils.BuildMockPolicyContext(t, inputs)
	lookup := backendtransport.NewBackendTransportLookup(
		ctx.Collections.ConfigMaps,
		ctx.Collections.Services,
		ctx.Collections.Backends,
		ctx.Collections.AgentgatewayPolicies,
		ctx.Collections.BackendTLSPolicies,
	)

	transport, err := lookup.Resolve(
		ctx.Krt,
		"oauth2-policy",
		"default",
		backendRef,
		defaultPort,
	)
	require.NoError(t, err)
	return transport
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
