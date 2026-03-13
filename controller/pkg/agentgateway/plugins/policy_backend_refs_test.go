package plugins

import (
	"testing"

	"github.com/stretchr/testify/require"
	networkingclient "istio.io/client-go/pkg/apis/networking/v1"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/kube/krt/krttest"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	discovery "k8s.io/api/discovery/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/identity/jwks_url"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
)

func TestCollectPolicyAncestorBackendsForOAuth2GatewayTarget(t *testing.T) {
	policy := testOAuth2PolicyWithTarget(
		"oauth2-gateway",
		shared.LocalPolicyTargetReferenceWithSectionName{
			LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
				Group: gwv1.Group(wellknown.GatewayGVK.Group),
				Kind:  gwv1.Kind(wellknown.GatewayGVK.Kind),
				Name:  "gw",
			},
		},
	)

	ctx := buildMockPolicyCtx(t, []any{
		&gwv1.Gateway{
			ObjectMeta: metav1.ObjectMeta{
				Namespace: "default",
				Name:      "gw",
			},
		},
		policy,
	})

	got := collectPolicyAncestorBackends(ctx.Krt, policy, ctx.Collections)
	require.Equal(t, []*utils.AncestorBackend{
		{
			Gateway: types.NamespacedName{
				Namespace: "default",
				Name:      "gw",
			},
			Backend: utils.TypedNamespacedName{
				NamespacedName: types.NamespacedName{
					Namespace: "default",
					Name:      "dummy-idp",
				},
				Kind: wellknown.ServiceKind,
			},
			Source: utils.TypedNamespacedName{
				NamespacedName: types.NamespacedName{
					Namespace: "default",
					Name:      "oauth2-gateway",
				},
				Kind: wellknown.AgentgatewayPolicyGVK.Kind,
			},
		},
	}, got)
}

func TestCollectPolicyAncestorBackendsForOAuth2RouteTarget(t *testing.T) {
	policy := testOAuth2PolicyWithTarget(
		"oauth2-route",
		shared.LocalPolicyTargetReferenceWithSectionName{
			LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
				Group: gwv1.Group(wellknown.HTTPRouteGVK.Group),
				Kind:  gwv1.Kind(wellknown.HTTPRouteGVK.Kind),
				Name:  "route",
			},
		},
	)

	ctx := buildMockPolicyCtx(t, []any{
		&gwv1.Gateway{
			ObjectMeta: metav1.ObjectMeta{
				Namespace: "default",
				Name:      "gw",
			},
		},
		&gwv1.HTTPRoute{
			ObjectMeta: metav1.ObjectMeta{
				Namespace: "default",
				Name:      "route",
			},
			Spec: gwv1.HTTPRouteSpec{
				CommonRouteSpec: gwv1.CommonRouteSpec{
					ParentRefs: []gwv1.ParentReference{
						{
							Group: ptr.Of(gwv1.Group(wellknown.GatewayGVK.Group)),
							Kind:  ptr.Of(gwv1.Kind(wellknown.GatewayGVK.Kind)),
							Name:  "gw",
						},
					},
				},
			},
		},
		policy,
	})

	got := collectPolicyAncestorBackends(ctx.Krt, policy, ctx.Collections)
	require.Equal(t, []*utils.AncestorBackend{
		{
			Gateway: types.NamespacedName{
				Namespace: "default",
				Name:      "gw",
			},
			Backend: utils.TypedNamespacedName{
				NamespacedName: types.NamespacedName{
					Namespace: "default",
					Name:      "dummy-idp",
				},
				Kind: wellknown.ServiceKind,
			},
			Source: utils.TypedNamespacedName{
				NamespacedName: types.NamespacedName{
					Namespace: "default",
					Name:      "oauth2-route",
				},
				Kind: wellknown.AgentgatewayPolicyGVK.Kind,
			},
		},
	}, got)
}

func testOAuth2PolicyWithTarget(
	name string,
	targetRef shared.LocalPolicyTargetReferenceWithSectionName,
) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      name,
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{targetRef},
			Traffic: &agentgateway.Traffic{
				OAuth2: &agentgateway.OAuth2{
					ClientID: "agw-client",
					ClientSecret: agentgateway.OAuth2ClientSecret{
						Inline: ptr.Of("super-secret"),
					},
					Issuer: ptr.Of(agentgateway.LongString("https://issuer.example.com")),
					BackendRef: &gwv1.BackendObjectReference{
						Group: ptr.Of(gwv1.Group("")),
						Kind:  ptr.Of(gwv1.Kind(wellknown.ServiceKind)),
						Name:  "dummy-idp",
						Port:  ptr.Of(gwv1.PortNumber(8443)),
					},
					RedirectURI: agentgateway.LongString("https://gateway.example.com/oauth2/callback"),
				},
			},
		},
	}
}

func buildMockPolicyCtx(t *testing.T, inputs []any) PolicyCtx {
	t.Helper()
	mock := krttest.NewMock(t, inputs)
	col := &AgwCollections{
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
		Backends:             krttest.GetMockCollection[*agentgateway.AgentgatewayBackend](mock),
		AgentgatewayPolicies: krttest.GetMockCollection[*agentgateway.AgentgatewayPolicy](mock),
		ControllerName:       wellknown.DefaultAgwControllerName,
		SystemNamespace:      "agentgateway-system",
		IstioNamespace:       "istio-system",
		ClusterID:            "Kubernetes",
	}
	col.SetupIndexes()
	return PolicyCtx{
		Krt:            krt.TestingDummyContext{},
		Collections:    col,
		JWKSURLBuilder: jwks_url.NewJwksUrlFactory(col.BackendTransportLookup),
	}
}
