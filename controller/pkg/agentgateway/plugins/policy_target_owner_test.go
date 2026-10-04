package plugins

import (
	"testing"

	"istio.io/istio/pkg/kube/krt"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

const (
	ownControllerName   = "agentgateway.dev/controller"
	otherControllerName = "example.com/other-controller"
)

func TestForeignPolicyTarget(t *testing.T) {
	collections := policyTargetOwnerCollections()
	tests := []struct {
		name    string
		kind    string
		target  string
		foreign bool
	}{
		{name: "gateway of our class", kind: wellknown.GatewayKind, target: "our-gw"},
		{name: "gateway of another class", kind: wellknown.GatewayKind, target: "their-gw", foreign: true},
		{name: "gateway whose class is missing", kind: wellknown.GatewayKind, target: "orphan-gw"},
		{name: "gateway that does not exist", kind: wellknown.GatewayKind, target: "missing-gw"},
		{name: "route parented to our gateway", kind: wellknown.HTTPRouteKind, target: "our-route"},
		{name: "route parented to another controller's gateway", kind: wellknown.HTTPRouteKind, target: "their-route", foreign: true},
		{name: "route parented to both", kind: wellknown.HTTPRouteKind, target: "mixed-route"},
		{name: "route without parents", kind: wellknown.HTTPRouteKind, target: "unparented-route"},
		{name: "route with only a non-gateway parent", kind: wellknown.HTTPRouteKind, target: "mesh-route"},
		{name: "route that does not exist", kind: wellknown.HTTPRouteKind, target: "missing-route"},
		{name: "route parented through another controller's listenerset", kind: wellknown.HTTPRouteKind, target: "their-ls-route", foreign: true},
		{name: "listenerset under another controller's gateway", kind: wellknown.ListenerSetGVK.Kind, target: "their-ls", foreign: true},
		{name: "backend-like target", kind: wellknown.ServiceKind, target: "svc"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			target := utils.TypedNamespacedName{Kind: tt.kind}
			target.Namespace, target.Name = "default", tt.target
			if got := foreignPolicyTarget(krt.TestingDummyContext{}, collections, target); got != tt.foreign {
				t.Fatalf("foreignPolicyTarget(%s %s) = %v, want %v", tt.kind, tt.target, got, tt.foreign)
			}
		})
	}
}

func policyTargetOwnerCollections() *AgwCollections {
	gateway := func(name, class string) *gwv1.Gateway {
		return &gwv1.Gateway{Name: name, Namespace: "default", Spec: gwv1.GatewaySpec{GatewayClassName: gwv1.ObjectName(class)}}
	}
	gatewayClass := func(name, controller string) *gwv1.GatewayClass {
		return &gwv1.GatewayClass{Name: name, Spec: gwv1.GatewayClassSpec{ControllerName: gwv1.GatewayController(controller)}}
	}
	parent := func(name string) gwv1.ParentReference {
		return gwv1.ParentReference{Name: gwv1.ObjectName(name)}
	}
	route := func(name string, parents ...gwv1.ParentReference) *gwv1.HTTPRoute {
		return &gwv1.HTTPRoute{Name: name, Namespace: "default", Spec: gwv1.HTTPRouteSpec{
			CommonRouteSpec: gwv1.CommonRouteSpec{ParentRefs: parents},
		}}
	}
	return &AgwCollections{
		ControllerName: ownControllerName,
		GatewayClasses: krt.NewStaticCollection(nil, []*gwv1.GatewayClass{
			gatewayClass("ours", ownControllerName),
			gatewayClass("theirs", otherControllerName),
		}, krt.WithName("plugins/policyTargetOwnerGatewayClasses")),
		Gateways: krt.NewStaticCollection(nil, []*gwv1.Gateway{
			gateway("our-gw", "ours"),
			gateway("their-gw", "theirs"),
			gateway("orphan-gw", "missing"),
		}, krt.WithName("plugins/policyTargetOwnerGateways")),
		ListenerSets: krt.NewStaticCollection(nil, []*gwv1.ListenerSet{{
			Name: "their-ls", Namespace: "default",
			Spec: gwv1.ListenerSetSpec{ParentRef: gwv1.ParentGatewayReference{Name: "their-gw"}},
		}}, krt.WithName("plugins/policyTargetOwnerListenerSets")),
		HTTPRoutes: krt.NewStaticCollection(nil, []*gwv1.HTTPRoute{
			route("our-route", parent("our-gw")),
			route("their-route", parent("their-gw")),
			route("mixed-route", parent("our-gw"), parent("their-gw")),
			route("unparented-route"),
			route("mesh-route", gwv1.ParentReference{Group: new(gwv1.Group("")), Kind: new(gwv1.Kind(wellknown.ServiceKind)), Name: "svc"}),
			route("their-ls-route", gwv1.ParentReference{Kind: new(gwv1.Kind(wellknown.ListenerSetGVK.Kind)), Name: "their-ls"}),
		}, krt.WithName("plugins/policyTargetOwnerHTTPRoutes")),
		GRPCRoutes: krt.NewStaticCollection[*gwv1.GRPCRoute](nil, nil, krt.WithName("plugins/policyTargetOwnerGRPCRoutes")),
		TCPRoutes:  krt.NewStaticCollection[*gwv1.TCPRoute](nil, nil, krt.WithName("plugins/policyTargetOwnerTCPRoutes")),
		TLSRoutes:  krt.NewStaticCollection[*gwv1.TLSRoute](nil, nil, krt.WithName("plugins/policyTargetOwnerTLSRoutes")),
	}
}
