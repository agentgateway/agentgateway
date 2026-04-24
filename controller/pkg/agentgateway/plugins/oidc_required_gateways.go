package plugins

import (
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"k8s.io/apimachinery/pkg/labels"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	krtpkg "github.com/agentgateway/agentgateway/controller/pkg/utils/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

// OIDCRequiredGateway identifies a Gateway that has at least one OIDC-bearing
// AgentgatewayPolicy attached, directly or via a child ListenerSet/HTTPRoute/
// GRPCRoute. It is the element type of `AgwCollections.GatewaysRequiringOIDC`,
// the collection the deployer reads to decide whether to mint and mount the
// managed OIDC cookie Secret on a Gateway's Pod.
type OIDCRequiredGateway struct {
	Namespace string
	Name      string
}

func (g OIDCRequiredGateway) ResourceName() string {
	return g.Namespace + "/" + g.Name
}

func (g OIDCRequiredGateway) String() string {
	return g.ResourceName()
}

func (g OIDCRequiredGateway) Equals(other OIDCRequiredGateway) bool {
	return g.Namespace == other.Namespace && g.Name == other.Name
}

// gatewayObjectKey identifies a single Gateway-API resource by `(Kind,
// Namespace, Name)`. Used both as the OIDC-policy-attachment key (a policy
// `targetRef`) and as the route-parent key (a route `parentRef`); the two
// share an addressing shape.
type gatewayObjectKey struct {
	Kind      string
	Namespace string
	Name      string
}

func (k gatewayObjectKey) String() string {
	return k.Kind + "/" + k.Namespace + "/" + k.Name
}

// buildGatewaysRequiringOIDC returns a per-Gateway krt collection whose
// elements identify Gateways that need the managed OIDC cookie Secret
// because at least one OIDC-bearing AgentgatewayPolicy is attached, either
// directly to the Gateway or transitively via a child ListenerSet, HTTPRoute,
// or GRPCRoute (including routes parented to a ListenerSet whose parent is
// the Gateway). Attachment is sourced from `spec.targetRefs` and
// `spec.targetSelectors` (the user's declarative intent) rather than
// `policy.status.ancestors` (which the translator writes downstream and is
// therefore not authoritative for the deployer's decision). Selector matches
// are scoped to the policy's own namespace, mirroring the translator's
// `PolicyTargetsBySelector` semantics.
func buildGatewaysRequiringOIDC(
	krtOptions krtutil.KrtOptions,
	gateways krt.Collection[*gwv1.Gateway],
	listenerSets krt.Collection[*gwv1.ListenerSet],
	httpRoutes krt.Collection[*gwv1.HTTPRoute],
	grpcRoutes krt.Collection[*gwv1.GRPCRoute],
	agentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy],
	listenerSetsByParentGateway krt.Index[collections.TargetRefIndexKey, *gwv1.ListenerSet],
) krt.Collection[OIDCRequiredGateway] {
	policiesByOIDCAttachment := krtpkg.UnnamedIndex(agentgatewayPolicies, func(in *agentgateway.AgentgatewayPolicy) []gatewayObjectKey {
		if in == nil || in.Spec.Traffic == nil || in.Spec.Traffic.OIDC == nil {
			return nil
		}
		keys := make([]gatewayObjectKey, 0, len(in.Spec.TargetRefs))
		for _, ref := range in.Spec.TargetRefs {
			if string(ref.Group) != wellknown.GatewayGroup {
				continue
			}
			kind := string(ref.Kind)
			switch kind {
			case wellknown.GatewayKind, wellknown.ListenerSetKind, wellknown.HTTPRouteKind, wellknown.GRPCRouteKind:
			default:
				continue
			}
			keys = append(keys, gatewayObjectKey{
				Kind:      kind,
				Namespace: in.Namespace,
				Name:      string(ref.Name),
			})
		}
		return keys
	})

	// Per-namespace index of OIDC-bearing policies that use targetSelectors.
	// Selector resolution can't be precomputed (target labels change at
	// runtime), so we narrow the candidate set by namespace and walk the
	// remaining selectors during the per-Gateway transformation.
	oidcSelectorPoliciesByNamespace := krtpkg.UnnamedIndex(agentgatewayPolicies, func(in *agentgateway.AgentgatewayPolicy) []string {
		if in == nil || in.Spec.Traffic == nil || in.Spec.Traffic.OIDC == nil {
			return nil
		}
		if len(in.Spec.TargetSelectors) == 0 {
			return nil
		}
		return []string{in.Namespace}
	})

	httpRoutesByParent := krtpkg.UnnamedIndex(httpRoutes, func(rt *gwv1.HTTPRoute) []gatewayObjectKey {
		if rt == nil {
			return nil
		}
		return routeParentRefsToKeys(rt.Namespace, rt.Spec.ParentRefs)
	})
	grpcRoutesByParent := krtpkg.UnnamedIndex(grpcRoutes, func(rt *gwv1.GRPCRoute) []gatewayObjectKey {
		if rt == nil {
			return nil
		}
		return routeParentRefsToKeys(rt.Namespace, rt.Spec.ParentRefs)
	})

	return krt.NewCollection(gateways, func(kctx krt.HandlerContext, gw *gwv1.Gateway) *OIDCRequiredGateway {
		if gw == nil {
			return nil
		}
		gwKey := OIDCRequiredGateway{Namespace: gw.Namespace, Name: gw.Name}

		hasAttachedOIDC := func(kind, namespace, name string) bool {
			attached := krt.Fetch(kctx, agentgatewayPolicies, krt.FilterIndex(policiesByOIDCAttachment, gatewayObjectKey{
				Kind:      kind,
				Namespace: namespace,
				Name:      name,
			}))
			return len(attached) > 0
		}
		// Memoize the namespace-scoped selector-policy fetch so the same lookup
		// isn't repeated for every route / listenerSet under this Gateway.
		selectorPoliciesByNs := map[string][]*agentgateway.AgentgatewayPolicy{}
		fetchSelectorPolicies := func(namespace string) []*agentgateway.AgentgatewayPolicy {
			if cached, ok := selectorPoliciesByNs[namespace]; ok {
				return cached
			}
			ps := krt.Fetch(kctx, agentgatewayPolicies, krt.FilterIndex(oidcSelectorPoliciesByNamespace, namespace))
			selectorPoliciesByNs[namespace] = ps
			return ps
		}
		hasSelectorAttachedOIDC := func(kind, namespace string, objectLabels map[string]string) bool {
			target := labels.Set(objectLabels)
			for _, p := range fetchSelectorPolicies(namespace) {
				if p == nil {
					continue
				}
				for _, sel := range p.Spec.TargetSelectors {
					if string(sel.Group) != wellknown.GatewayGroup || string(sel.Kind) != kind {
						continue
					}
					if labels.SelectorFromSet(sel.MatchLabels).Matches(target) {
						return true
					}
				}
			}
			return false
		}
		attachesOIDC := func(kind, namespace, name string, objectLabels map[string]string) bool {
			return hasAttachedOIDC(kind, namespace, name) || hasSelectorAttachedOIDC(kind, namespace, objectLabels)
		}
		anyRouteAttachesOIDC := func(parent gatewayObjectKey) bool {
			for _, rt := range krt.Fetch(kctx, httpRoutes, krt.FilterIndex(httpRoutesByParent, parent)) {
				if attachesOIDC(wellknown.HTTPRouteKind, rt.Namespace, rt.Name, rt.Labels) {
					return true
				}
			}
			for _, rt := range krt.Fetch(kctx, grpcRoutes, krt.FilterIndex(grpcRoutesByParent, parent)) {
				if attachesOIDC(wellknown.GRPCRouteKind, rt.Namespace, rt.Name, rt.Labels) {
					return true
				}
			}
			return false
		}

		// Direct Gateway attachment.
		if attachesOIDC(wellknown.GatewayKind, gw.Namespace, gw.Name, gw.Labels) {
			return &gwKey
		}
		// Routes parented directly to the Gateway.
		if anyRouteAttachesOIDC(gatewayObjectKey{
			Kind:      wellknown.GatewayKind,
			Namespace: gw.Namespace,
			Name:      gw.Name,
		}) {
			return &gwKey
		}
		// Child ListenerSet attachment: either the ListenerSet itself carries
		// OIDC, or a route parented to that ListenerSet does.
		for _, ls := range krt.Fetch(kctx, listenerSets, krt.FilterIndex(listenerSetsByParentGateway, collections.TargetRefIndexKey{
			Group:     wellknown.GatewayGroup,
			Kind:      wellknown.GatewayKind,
			Name:      gw.Name,
			Namespace: gw.Namespace,
		})) {
			if attachesOIDC(wellknown.ListenerSetKind, ls.Namespace, ls.Name, ls.Labels) {
				return &gwKey
			}
			if anyRouteAttachesOIDC(gatewayObjectKey{
				Kind:      wellknown.ListenerSetKind,
				Namespace: ls.Namespace,
				Name:      ls.Name,
			}) {
				return &gwKey
			}
		}
		return nil
	}, krtOptions.ToOptions("GatewaysRequiringOIDC")...)
}

// routeParentRefsToKeys emits a `gatewayObjectKey` for each `parentRef` that
// points at a Gateway-API parent the OIDC collection cares about (Gateway or
// ListenerSet), defaulting Group/Kind/Namespace per the Gateway API spec for
// `ParentReference`.
func routeParentRefsToKeys(routeNamespace string, parentRefs []gwv1.ParentReference) []gatewayObjectKey {
	keys := make([]gatewayObjectKey, 0, len(parentRefs))
	for _, p := range parentRefs {
		group := ptr.OrDefault(p.Group, gwv1.Group(wellknown.GatewayGroup))
		if string(group) != wellknown.GatewayGroup {
			continue
		}
		kind := string(ptr.OrDefault(p.Kind, gwv1.Kind(wellknown.GatewayKind)))
		switch kind {
		case wellknown.GatewayKind, wellknown.ListenerSetKind:
		default:
			continue
		}
		ns := ptr.OrDefault(p.Namespace, gwv1.Namespace(routeNamespace))
		keys = append(keys, gatewayObjectKey{
			Kind:      kind,
			Namespace: string(ns),
			Name:      string(p.Name),
		})
	}
	return keys
}
