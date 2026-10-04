package plugins

import (
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

// foreignPolicyTarget reports whether a policy target is parented only to Gateways managed by other
// controllers. Following the Gateway API convention for route parents, such a target is not this
// controller's to report on: no ancestor status, no attachment error, no log line.
//
// A target with no resolvable parent Gateway, or with at least one parent this controller manages or
// cannot classify (missing Gateway or GatewayClass), is not foreign, so existing attachment errors still fire.
func foreignPolicyTarget(ctx krt.HandlerContext, agw *AgwCollections, target utils.TypedNamespacedName) bool {
	parents := policyTargetParentGateways(ctx, agw, target)
	if len(parents) == 0 {
		return false
	}
	for _, gateway := range parents {
		controller := gatewayController(ctx, agw, gateway)
		if controller == "" || controller == agw.ControllerName {
			return false
		}
	}
	return true
}

// policyTargetParentGateways resolves a target's parent Gateways from the target's own spec rather than
// from this controller's attachment indexes, so Gateways of other classes are included.
// Backend-like targets declare no parents and resolve to nothing.
func policyTargetParentGateways(ctx krt.HandlerContext, agw *AgwCollections, target utils.TypedNamespacedName) []types.NamespacedName {
	key := krt.FilterKey(target.NamespacedName.String())
	switch target.Kind {
	case wellknown.GatewayGVK.Kind:
		return []types.NamespacedName{target.NamespacedName}
	case wellknown.ListenerSetGVK.Kind:
		return listenerSetParentGateway(ctx, agw, target.NamespacedName)
	case wellknown.HTTPRouteGVK.Kind:
		return routeParentGateways(ctx, agw, target.Namespace, krtutil.FetchOneSpec(ctx, agw.HTTPRoutes,
			func(r *gwv1.HTTPRoute) []gwv1.ParentReference { return r.Spec.ParentRefs }, key))
	case wellknown.GRPCRouteGVK.Kind:
		return routeParentGateways(ctx, agw, target.Namespace, krtutil.FetchOneSpec(ctx, agw.GRPCRoutes,
			func(r *gwv1.GRPCRoute) []gwv1.ParentReference { return r.Spec.ParentRefs }, key))
	case wellknown.TCPRouteGVK.Kind:
		return routeParentGateways(ctx, agw, target.Namespace, krtutil.FetchOneSpec(ctx, agw.TCPRoutes,
			func(r *gwv1.TCPRoute) []gwv1.ParentReference { return r.Spec.ParentRefs }, key))
	case wellknown.TLSRouteGVK.Kind:
		return routeParentGateways(ctx, agw, target.Namespace, krtutil.FetchOneSpec(ctx, agw.TLSRoutes,
			func(r *gwv1.TLSRoute) []gwv1.ParentReference { return r.Spec.ParentRefs }, key))
	default:
		return nil
	}
}

// routeParentGateways maps a route's parentRefs to Gateways, following ListenerSet parents to their
// Gateway and ignoring parents of any other kind (for example Service parents used by a mesh).
func routeParentGateways(ctx krt.HandlerContext, agw *AgwCollections, routeNamespace string, route *krtutil.SpecOnly[[]gwv1.ParentReference]) []types.NamespacedName {
	if route == nil {
		return nil
	}
	var gateways []types.NamespacedName
	for _, ref := range route.Spec {
		if string(ptr.OrDefault(ref.Group, wellknown.GatewayGroup)) != wellknown.GatewayGroup {
			continue
		}
		parent := types.NamespacedName{
			Namespace: string(ptr.OrDefault(ref.Namespace, gwv1.Namespace(routeNamespace))),
			Name:      string(ref.Name),
		}
		switch string(ptr.OrDefault(ref.Kind, wellknown.GatewayKind)) {
		case wellknown.GatewayKind:
			gateways = append(gateways, parent)
		case wellknown.ListenerSetGVK.Kind:
			gateways = append(gateways, listenerSetParentGateway(ctx, agw, parent)...)
		}
	}
	return gateways
}

// listenerSetParentGateway follows a ListenerSet's spec.parentRef to its Gateway, defaulting the namespace
// to the ListenerSet's own. It returns nothing when the ListenerSet cannot be found, so the caller cannot
// classify the parent and falls back to reporting an attachment error.
func listenerSetParentGateway(ctx krt.HandlerContext, agw *AgwCollections, listenerSet types.NamespacedName) []types.NamespacedName {
	ls := krtutil.FetchOneSpec(ctx, agw.ListenerSets, func(ls *gwv1.ListenerSet) gwv1.ParentGatewayReference {
		return ls.Spec.ParentRef
	}, krt.FilterKey(listenerSet.String()))
	if ls == nil {
		return nil
	}
	return []types.NamespacedName{{
		Namespace: string(ptr.OrDefault(ls.Spec.Namespace, gwv1.Namespace(listenerSet.Namespace))),
		Name:      string(ls.Spec.Name),
	}}
}

// gatewayController returns the controllerName of the GatewayClass a Gateway uses, or an empty string
// when either the Gateway or its GatewayClass cannot be found.
func gatewayController(ctx krt.HandlerContext, agw *AgwCollections, gateway types.NamespacedName) string {
	gw := krtutil.FetchOneSpec(ctx, agw.Gateways, func(gw *gwv1.Gateway) gwv1.ObjectName {
		return gw.Spec.GatewayClassName
	}, krt.FilterKey(gateway.String()))
	if gw == nil {
		return ""
	}
	gc := ptr.Flatten(krt.FetchOne(ctx, agw.GatewayClasses, krt.FilterKey(string(gw.Spec))))
	if gc == nil {
		return ""
	}
	return string(gc.Spec.ControllerName)
}
