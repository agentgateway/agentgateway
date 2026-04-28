package plugins

import (
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	krtpkg "github.com/agentgateway/agentgateway/controller/pkg/utils/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

const testNamespace = "ns"

type alwaysSynced struct{}

func (alwaysSynced) WaitUntilSynced(stop <-chan struct{}) bool { return true }
func (alwaysSynced) HasSynced() bool                           { return true }

func TestGatewaysRequiringOIDC(t *testing.T) {
	// Each Gateway is named for the attachment path it exercises.
	gateways := krt.NewStaticCollection(alwaysSynced{}, []*gwv1.Gateway{
		newGateway("gw-direct"),
		newGateway("gw-via-listenerset"),
		newGateway("gw-via-httproute"),
		newGateway("gw-via-grpcroute"),
		newGateway("gw-via-listenerset-route"),
		newGateway("gw-attached-but-no-oidc"),
		newGateway("gw-with-no-attachments"),
	})

	listenerSets := krt.NewStaticCollection(alwaysSynced{}, []*gwv1.ListenerSet{
		newListenerSet("ls-child", "gw-via-listenerset"),
		newListenerSet("ls-route-parent", "gw-via-listenerset-route"),
	})

	httpRoutes := krt.NewStaticCollection(alwaysSynced{}, []*gwv1.HTTPRoute{
		newHTTPRoute("rt-http", wellknown.GatewayKind, "gw-via-httproute"),
		// Route parented to a ListenerSet — its OIDC attachment must propagate
		// to that ListenerSet's parent Gateway.
		newHTTPRoute("rt-http-via-ls", wellknown.ListenerSetKind, "ls-route-parent"),
	})

	grpcRoutes := krt.NewStaticCollection(alwaysSynced{}, []*gwv1.GRPCRoute{
		newGRPCRoute("rt-grpc", wellknown.GatewayKind, "gw-via-grpcroute"),
	})

	policies := krt.NewStaticCollection(alwaysSynced{}, []*agentgateway.AgentgatewayPolicy{
		newOIDCPolicyTargeting("p-direct", wellknown.GatewayKind, "gw-direct"),
		newOIDCPolicyTargeting("p-via-listenerset", wellknown.ListenerSetKind, "ls-child"),
		newOIDCPolicyTargeting("p-via-httproute", wellknown.HTTPRouteKind, "rt-http"),
		newOIDCPolicyTargeting("p-via-grpcroute", wellknown.GRPCRouteKind, "rt-grpc"),
		newOIDCPolicyTargeting("p-via-ls-route", wellknown.HTTPRouteKind, "rt-http-via-ls"),
		// Attached but no OIDC field set: should not contribute.
		newPolicyTargeting("p-no-oidc", wellknown.GatewayKind, "gw-attached-but-no-oidc"),
	})

	listenerSetsByParentGateway := krtpkg.UnnamedIndex(listenerSets, func(in *gwv1.ListenerSet) []collections.TargetRefIndexKey {
		ns := ptr.OrDefault(in.Spec.ParentRef.Namespace, gwv1.Namespace(in.Namespace))
		return []collections.TargetRefIndexKey{{
			Group:     wellknown.GatewayGroup,
			Kind:      wellknown.GatewayKind,
			Name:      string(in.Spec.ParentRef.Name),
			Namespace: string(ns),
		}}
	})

	required := buildGatewaysRequiringOIDC(
		testKrtOptions(t),
		gateways,
		listenerSets,
		httpRoutes,
		grpcRoutes,
		policies,
		listenerSetsByParentGateway,
	)

	want := []OIDCRequiredGateway{
		{Namespace: testNamespace, Name: "gw-direct"},
		{Namespace: testNamespace, Name: "gw-via-grpcroute"},
		{Namespace: testNamespace, Name: "gw-via-httproute"},
		{Namespace: testNamespace, Name: "gw-via-listenerset"},
		{Namespace: testNamespace, Name: "gw-via-listenerset-route"},
	}

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		got := required.List()
		assert.ElementsMatch(c, want, got)
	}, 2*time.Second, 10*time.Millisecond)
}

func testKrtOptions(t *testing.T) krtutil.KrtOptions {
	t.Helper()
	return krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
}

func newGateway(name string) *gwv1.Gateway {
	return &gwv1.Gateway{ObjectMeta: metav1.ObjectMeta{Namespace: testNamespace, Name: name}}
}

func newListenerSet(name, parentGateway string) *gwv1.ListenerSet {
	return &gwv1.ListenerSet{
		ObjectMeta: metav1.ObjectMeta{Namespace: testNamespace, Name: name},
		Spec: gwv1.ListenerSetSpec{
			ParentRef: gwv1.ParentGatewayReference{
				Group: ptr.Of(gwv1.Group(wellknown.GatewayGroup)),
				Kind:  ptr.Of(gwv1.Kind(wellknown.GatewayKind)),
				Name:  gwv1.ObjectName(parentGateway),
			},
		},
	}
}

func newHTTPRoute(name, parentKind, parentName string) *gwv1.HTTPRoute {
	return &gwv1.HTTPRoute{
		ObjectMeta: metav1.ObjectMeta{Namespace: testNamespace, Name: name},
		Spec: gwv1.HTTPRouteSpec{
			CommonRouteSpec: gwv1.CommonRouteSpec{
				ParentRefs: []gwv1.ParentReference{{
					Kind: ptr.Of(gwv1.Kind(parentKind)),
					Name: gwv1.ObjectName(parentName),
				}},
			},
		},
	}
}

func newGRPCRoute(name, parentKind, parentName string) *gwv1.GRPCRoute {
	return &gwv1.GRPCRoute{
		ObjectMeta: metav1.ObjectMeta{Namespace: testNamespace, Name: name},
		Spec: gwv1.GRPCRouteSpec{
			CommonRouteSpec: gwv1.CommonRouteSpec{
				ParentRefs: []gwv1.ParentReference{{
					Kind: ptr.Of(gwv1.Kind(parentKind)),
					Name: gwv1.ObjectName(parentName),
				}},
			},
		},
	}
}

func newPolicyTargeting(name, targetKind, targetName string) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{Namespace: testNamespace, Name: name},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: []shared.LocalPolicyTargetReferenceWithSectionName{{
				LocalPolicyTargetReference: shared.LocalPolicyTargetReference{
					Group: gwv1.Group(wellknown.GatewayGroup),
					Kind:  gwv1.Kind(targetKind),
					Name:  gwv1.ObjectName(targetName),
				},
			}},
		},
	}
}

func newOIDCPolicyTargeting(name, targetKind, targetName string) *agentgateway.AgentgatewayPolicy {
	p := newPolicyTargeting(name, targetKind, targetName)
	p.Spec.Traffic = &agentgateway.Traffic{
		OIDC: &agentgateway.OIDC{
			IssuerURL:   "https://issuer.example",
			ClientID:    "test-client",
			RedirectURI: "https://app.example/callback",
		},
	}
	return p
}
