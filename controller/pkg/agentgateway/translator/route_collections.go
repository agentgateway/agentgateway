package translator

import (
	"context"
	"fmt"
	"iter"
	"strings"

	networkingclient "istio.io/client-go/pkg/apis/networking/v1"
	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/log"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/util/protomarshal"
	"istio.io/istio/pkg/util/sets"
	"istio.io/istio/pkg/workloadapi"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	agwir "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/reporter"
	"github.com/agentgateway/agentgateway/controller/pkg/reports"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer/status"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

type resolvedBinding struct {
	Gateway    types.NamespacedName
	Parent     types.NamespacedName
	Target     types.NamespacedName
	ServiceKey *types.NamespacedName
}

func (b resolvedBinding) RouteGroupKey() string {
	return utils.InternalRouteGroupKey(b.Target.Namespace, b.Target.Name)
}

type routeGroupBindingKey struct {
	Gateway    *types.NamespacedName // +noKrtEquals
	Source     types.NamespacedName
	Namespace  string
	Name       string
	ServiceKey *types.NamespacedName // +noKrtEquals
}

func (k routeGroupBindingKey) String() string {
	return strings.Join([]string{k.gatewayKey(), k.serviceKey(), k.Source.String(), k.Namespace, k.Name}, "/")
}

func (b routeGroupBindingKey) ResourceName() string {
	return b.String()
}

func (b routeGroupBindingKey) Equals(other routeGroupBindingKey) bool {
	return b.Source == other.Source &&
		b.Namespace == other.Namespace &&
		b.Name == other.Name &&
		b.GatewayNN() == other.GatewayNN() &&
		b.ServiceKeyNN() == other.ServiceKeyNN()
}

func (b routeGroupBindingKey) GatewayNN() types.NamespacedName {
	if b.Gateway == nil {
		return types.NamespacedName{}
	}
	return *b.Gateway
}

func (b routeGroupBindingKey) ServiceKeyNN() types.NamespacedName {
	if b.ServiceKey == nil {
		return types.NamespacedName{}
	}
	return *b.ServiceKey
}

func (b routeGroupBindingKey) gatewayKey() string {
	if b.Gateway == nil {
		return ""
	}
	return b.Gateway.String()
}

func (b routeGroupBindingKey) serviceKey() string {
	if b.ServiceKey == nil {
		return ""
	}
	return b.ServiceKey.String()
}

// isDelegatedChildHTTPRoute returns true for routes that may be delegated children:
// 1. routes with no parentRefs (orphan routes adoptable by any parent)
// 2. routes with explicit HTTPRoute parentRefs.
// Routes with only Gateway parentRefs are directly-attached and cannot be delegated children.
func isDelegatedChildHTTPRoute(obj *gwv1.HTTPRoute) bool {
	if len(obj.Spec.ParentRefs) == 0 {
		// No explicit parents, all good
		return true
	}
	// Else, we need an explicit reference to an HTTPRoute
	for _, ref := range obj.Spec.ParentRefs {
		if ref.Group != nil && *ref.Group == wellknown.GatewayGroup &&
			ref.Kind != nil && *ref.Kind == wellknown.HTTPRouteKind {
			return true
		}
	}
	return false
}
func childAllowsParent(obj *gwv1.HTTPRoute, parentRef resolvedBinding) bool {
	allowedParents := slices.MapFilter(obj.Spec.ParentRefs, func(ref gwv1.ParentReference) *types.NamespacedName {
		if NormalizeReference(ref.Group, ref.Kind, wellknown.GatewayGVK) != wellknown.HTTPRouteGVK {
			return nil
		}
		return new(types.NamespacedName{
			Namespace: defaultString(ref.Namespace, obj.Namespace),
			Name:      string(ref.Name),
		})
	})
	if len(allowedParents) == 0 {
		return true
	}
	return slices.Contains(allowedParents, parentRef.Parent)
}

func extractHTTPRouteGroupRefs(rule gwv1.HTTPRouteRule, routeNamespace string) []routeGroupBindingKey {
	var res []routeGroupBindingKey
	for _, backend := range rule.BackendRefs {
		ref := NormalizeReference(backend.Group, backend.Kind, wellknown.ServiceGVK)
		if ref != wellknown.HTTPRouteGVK {
			continue
		}
		namespace := routeNamespace
		if backend.Namespace != nil {
			namespace = string(*backend.Namespace)
		}
		res = append(res, routeGroupBindingKey{
			Namespace: namespace,
			Name:      string(backend.Name),
		})
	}
	return res
}

func routeMatchesRouteGroup(obj *gwv1.HTTPRoute, binding routeGroupBindingKey) bool {
	if obj.Namespace != binding.Namespace {
		return false
	}
	if binding.Name == "*" {
		return true
	}
	if obj.Name == binding.Name {
		return true
	}
	if k, v, ok := strings.Cut(binding.Name, "="); ok && obj.Labels[k] == v {
		return true
	}
	return false
}

func delegatedRouteKey(routeKey, routeGroupKey string) string {
	replacer := strings.NewReplacer("/", ".", "*", "wildcard")
	return routeKey + ".routegroup." + replacer.Replace(routeGroupKey)
}

func buildHTTPRouteGroupBindings(
	httpRouteCol krt.Collection[*gwv1.HTTPRoute],
	inputs RouteContextInputs,
	krtopts krtutil.KrtOptions,
) krt.Collection[routeGroupBindingKey] {
	// TODO: this can only get 1 layer. If we have multiple levels we will not have a binding since there is no parent.
	raw := krt.NewManyCollection(httpRouteCol, func(krtctx krt.HandlerContext, obj *gwv1.HTTPRoute) []routeGroupBindingKey {
		ctx := inputs.WithCtx(krtctx)
		parentRefs := extractParentReferenceInfo(ctx, inputs.RouteParents, obj)
		allowedParents := FilteredReferences(parentRefs)
		bindings := make([]routeGroupBindingKey, 0)
		seen := sets.New[string]()
		source := config.NamespacedName(obj)
		if len(parentRefs) == 0 { // TODO: this should consider HTTPRoute parentRefs not just empty
			// If there are no parents, this may be a delegated route pointing to another delegated route.
			// Note this must have no parents; if it has a Gateway parent it disqualifies it from being a delegated route.
			// TODO: test when this middle route has an explicit parent HTTPRoute byt name
			allowedParents = []RouteParentReference{{
				ParentGateway: types.NamespacedName{},
			}}
		}
		for _, parent := range allowedParents {
			for _, rule := range obj.Spec.Rules {
				for _, binding := range extractHTTPRouteGroupRefs(rule, obj.Namespace) {
					binding.Source = source
					if parent.ParentGateway != (types.NamespacedName{}) {
						binding.Gateway = new(parent.ParentGateway)
					}
					if parent.ServiceKey != nil {
						binding.ServiceKey = parent.ServiceKey
					}
					if seen.InsertContains(binding.ResourceName()) {
						continue
					}
					bindings = append(bindings, binding)
				}
			}
		}

		return bindings
	}, krtopts.ToOptions("HTTPRouteGroupBindingsRaw")...)
	return raw
}

// NOTE: We intentionally do not send RouteGroup resources over xDS. The data plane has no use for
// them today (routes already carry route_group_key). If RouteGroup gains meaningful fields in the
// future, re-add this and handle XdsKind::RouteGroup in the data plane's insert_xds().

func buildDelegatedHTTPRoutes(
	httpRouteCol krt.Collection[*gwv1.HTTPRoute],
	bindings krt.Collection[routeGroupBindingKey],
	inputs RouteContextInputs,
	krtopts krtutil.KrtOptions,
) (krt.Collection[agwir.AgwResource], krt.Collection[*utils.AncestorBackend]) {
	bindingsByNamespace := krt.NewIndex(bindings, "HTTPRouteGroupBindingsByNamespace", func(binding routeGroupBindingKey) []string {
		return []string{binding.Namespace}
	})
	findMatchingBindings := func(krtctx krt.HandlerContext, obj *gwv1.HTTPRoute) []routeGroupBindingKey {
		candidates := krt.Fetch(krtctx, bindings, krt.FilterIndex(bindingsByNamespace, obj.Namespace))
		return slices.Filter(candidates, func(binding routeGroupBindingKey) bool {
			return routeMatchesRouteGroup(obj, binding)
		})
	}
	var resolveGateways func(krtctx krt.HandlerContext, binding routeGroupBindingKey, seen sets.Set[string]) []types.NamespacedName
	resolveGateways = func(krtctx krt.HandlerContext, binding routeGroupBindingKey, seen sets.Set[string]) []types.NamespacedName {
		if binding.Gateway != nil {
			return []types.NamespacedName{*binding.Gateway}
		}
		if seen.InsertContains(binding.Source.String()) {
			return nil
		}
		sourceRoute := ptr.Flatten(krt.FetchOne(krtctx, httpRouteCol, krt.FilterObjectName(binding.Source)))
		if sourceRoute == nil {
			return nil
		}
		gateways := sets.New[types.NamespacedName]()
		for _, parentBinding := range findMatchingBindings(krtctx, sourceRoute) {
			gateways.InsertAll(resolveGateways(krtctx, parentBinding, seen.Copy())...)
		}
		return slices.SortBy(gateways.UnsortedList(), types.NamespacedName.String)
	}
	matchingBindings := func(krtctx krt.HandlerContext, obj *gwv1.HTTPRoute) []resolvedBinding {
		candidates := findMatchingBindings(krtctx, obj)
		resolved := make([]resolvedBinding, 0, len(candidates))
		seen := sets.New[string]()
		for _, c := range candidates {
			target := types.NamespacedName{
				Namespace: c.Namespace,
				Name:      c.Name,
			}
			for _, gateway := range resolveGateways(krtctx, c, sets.New[string]()) {
				resolvedBinding := resolvedBinding{
					Gateway:    gateway,
					Parent:     c.Source,
					Target:     target,
					ServiceKey: c.ServiceKey,
				}
				dedupeKey := resolvedBinding.Gateway.String() + "/" + resolvedBinding.RouteGroupKey()
				if c.ServiceKey != nil {
					dedupeKey += "/" + c.ServiceKey.String()
				}
				if seen.InsertContains(dedupeKey) {
					continue
				}
				resolved = append(resolved, resolvedBinding)
			}
		}
		return resolved
	}

	routes := krt.NewManyCollection(httpRouteCol, func(krtctx krt.HandlerContext, obj *gwv1.HTTPRoute) []agwir.AgwResource {
		if !isDelegatedChildHTTPRoute(obj) {
			return nil
		}
		ctx := inputs.WithCtx(krtctx)
		var resources []agwir.AgwResource
		for _, binding := range matchingBindings(krtctx, obj) {
			if !childAllowsParent(obj, binding) {
				continue
			}
			for n, rule := range obj.Spec.Rules {
				route, err := ConvertHTTPRouteToAgw(ctx, rule, obj, n)
				if err != nil {
					log.Warnf("skipping delegated route %s/%s rule %d: %v", obj.Namespace, obj.Name, n, err)
					continue
				}
				route.ListenerKey = ""
				route.RouteGroupKey = new(binding.RouteGroupKey())
				route.Key = delegatedRouteKey(route.GetKey(), binding.RouteGroupKey())
				if binding.ServiceKey != nil {
					route.ServiceKey = &workloadapi.NamespacedHostname{
						Namespace: binding.ServiceKey.Namespace,
						Hostname:  binding.ServiceKey.Name,
					}
					route.Hostnames = nil
				}
				resources = append(resources, ToResourceForGateway(binding.Gateway, AgwRoute{Route: route}))
			}
		}
		return resources
	}, krtopts.ToOptions("DelegatedHTTPRoutes")...)

	ancestors := krt.NewManyCollection(httpRouteCol, func(krtctx krt.HandlerContext, obj *gwv1.HTTPRoute) []*utils.AncestorBackend {
		if !isDelegatedChildHTTPRoute(obj) {
			return nil
		}
		source := utils.TypedNamespacedName{
			NamespacedName: types.NamespacedName{
				Namespace: obj.Namespace,
				Name:      obj.Name,
			},
			Kind: wellknown.HTTPRouteKind,
		}
		gateways := sets.New[types.NamespacedName]()
		for _, binding := range matchingBindings(krtctx, obj) {
			if !childAllowsParent(obj, binding) {
				continue
			}
			gateways.Insert(binding.Gateway)
		}
		if len(gateways) == 0 {
			return nil
		}
		backends := sets.New[utils.TypedNamespacedName]()
		for _, rule := range obj.Spec.Rules {
			for _, backend := range rule.BackendRefs {
				ref, refNs, refName := GetBackendRef(backend)
				if ref == wellknown.HTTPRouteGVK {
					continue
				}
				backends.Insert(utils.TypedNamespacedName{
					NamespacedName: types.NamespacedName{
						Namespace: defaultString(refNs, obj.Namespace),
						Name:      string(refName),
					},
					Kind: ref.Kind,
				})
			}
		}
		res := make([]*utils.AncestorBackend, 0, len(gateways)*len(backends))
		for _, gateway := range slices.SortBy(gateways.UnsortedList(), types.NamespacedName.String) {
			for _, backend := range slices.SortBy(backends.UnsortedList(), utils.TypedNamespacedName.String) {
				res = append(res, &utils.AncestorBackend{
					Gateway: gateway,
					Backend: backend,
					Source:  source,
				})
			}
		}
		return res
	}, krtopts.ToOptions("DelegatedHTTPRouteAncestors")...)

	return routes, ancestors
}

// setDelegatedRouteParentStatus sets parent status for HTTPRoute parentRefs on delegated child routes.
// For each HTTPRoute parentRef, it checks whether the parent actually delegates to this child via bindings,
// and sets Accepted and ResolvedRefs conditions accordingly.
func setDelegatedRouteParentStatus(
	krtctx krt.HandlerContext,
	obj *gwv1.HTTPRoute,
	routeReporter reporter.RouteReporter,
	bindings krt.Collection[routeGroupBindingKey],
	bindingsBySource krt.Index[string, routeGroupBindingKey],
) {
	for _, ref := range obj.Spec.ParentRefs {
		// Only handle HTTPRoute parentRefs
		if ref.Group == nil || string(*ref.Group) != wellknown.GatewayGroup ||
			ref.Kind == nil || string(*ref.Kind) != wellknown.HTTPRouteKind {
			continue
		}

		parentNN := types.NamespacedName{
			Namespace: defaultString(ref.Namespace, obj.Namespace),
			Name:      string(ref.Name),
		}

		// Check if the parent delegates to this child by looking at bindings
		parentBindings := krt.Fetch(krtctx, bindings, krt.FilterIndex(bindingsBySource, parentNN.String()))
		accepted := false
		for _, b := range parentBindings {
			if routeMatchesRouteGroup(obj, b) {
				accepted = true
				break
			}
		}

		// Build a normalized parentRef with namespace filled in so it matches
		// what BuildRouteStatusWithParentRefDefaulting will look up.
		statusRef := ref
		if statusRef.Namespace == nil {
			ns := gwv1.Namespace(obj.Namespace)
			statusRef.Namespace = &ns
		}

		pr := routeReporter.ParentRef(&statusRef)
		if accepted {
			pr.SetCondition(reporter.RouteCondition{
				Type:   gwv1.RouteConditionAccepted,
				Status: metav1.ConditionTrue,
				Reason: gwv1.RouteReasonAccepted,
			})
		} else {
			pr.SetCondition(reporter.RouteCondition{
				Type:    gwv1.RouteConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  "NoMatchingParent",
				Message: "Parent HTTPRoute does not delegate to this route",
			})
		}
		pr.SetCondition(reporter.RouteCondition{
			Type:   gwv1.RouteConditionResolvedRefs,
			Status: metav1.ConditionTrue,
			Reason: gwv1.RouteReasonResolvedRefs,
		})
	}
}

// AgwRouteCollection creates the collection of translated Routes
func AgwRouteCollection(
	queue *status.StatusCollections,
	httpRouteCol krt.Collection[*gwv1.HTTPRoute],
	grpcRouteCol krt.Collection[*gwv1.GRPCRoute],
	tcpRouteCol krt.Collection[*gwv1a2.TCPRoute],
	tlsRouteCol krt.Collection[*gwv1.TLSRoute],
	inputs RouteContextInputs,
	krtopts krtutil.KrtOptions,
) (krt.Collection[agwir.AgwResource], krt.Collection[*plugins.RouteAttachment], krt.Collection[*utils.AncestorBackend]) {
	// Build delegation bindings before creating the HTTPRoute status collection,
	// so we can set parent status for delegated routes.
	httpRouteGroupBindings := buildHTTPRouteGroupBindings(httpRouteCol, inputs, krtopts)
	bindingsBySource := krt.NewIndex(httpRouteGroupBindings, "HTTPRouteGroupBindingsBySource", func(binding routeGroupBindingKey) []string {
		return []string{binding.Source.String()}
	})

	httpRouteStatus, httpRoutes := createRouteCollectionGeneric(httpRouteCol, inputs, krtopts, "HTTPRoutes",
		func(ctx RouteContext, obj *gwv1.HTTPRoute) (RouteContext, iter.Seq2[AgwRoute, *reporter.RouteCondition]) {
			route := obj.Spec
			return ctx, func(yield func(AgwRoute, *reporter.RouteCondition) bool) {
				for n, r := range route.Rules {
					res, err := ConvertHTTPRouteToAgw(ctx, r, obj, n)
					if !yield(AgwRoute{Route: res}, err) {
						return
					}
				}
			}
		}, func(status gwv1.RouteStatus) gwv1.HTTPRouteStatus {
			return gwv1.HTTPRouteStatus{RouteStatus: status}
		},
		func(krtctx krt.HandlerContext, obj *gwv1.HTTPRoute, routeReporter reporter.RouteReporter) {
			setDelegatedRouteParentStatus(krtctx, obj, routeReporter, httpRouteGroupBindings, bindingsBySource)
		},
	)
	status.RegisterStatus(queue, httpRouteStatus, GetStatus)
	delegatedHTTPRoutes, delegatedHTTPAncestors := buildDelegatedHTTPRoutes(httpRouteCol, httpRouteGroupBindings, inputs, krtopts)

	grpcRouteStatus, grpcRoutes := createRouteCollectionGeneric(grpcRouteCol, inputs, krtopts, "GRPCRoutes",
		func(ctx RouteContext, obj *gwv1.GRPCRoute) (RouteContext, iter.Seq2[AgwRoute, *reporter.RouteCondition]) {
			route := obj.Spec
			return ctx, func(yield func(AgwRoute, *reporter.RouteCondition) bool) {
				for n, r := range route.Rules {
					// Convert the entire rule with all matches at once
					res, err := ConvertGRPCRouteToAgw(ctx, r, obj, n)
					if !yield(AgwRoute{Route: res}, err) {
						return
					}
				}
			}
		}, func(status gwv1.RouteStatus) gwv1.GRPCRouteStatus {
			return gwv1.GRPCRouteStatus{RouteStatus: status}
		})
	status.RegisterStatus(queue, grpcRouteStatus, GetStatus)

	tcpRouteStatus, tcpRoutes := createRouteCollectionGeneric(tcpRouteCol, inputs, krtopts, "TCPRoutes",
		func(ctx RouteContext, obj *gwv1a2.TCPRoute) (RouteContext, iter.Seq2[AgwTCPRoute, *reporter.RouteCondition]) {
			route := obj.Spec
			return ctx, func(yield func(AgwTCPRoute, *reporter.RouteCondition) bool) {
				for n, r := range route.Rules {
					// Convert the entire rule with all matches at once
					res, err := ConvertTCPRouteToAgw(ctx, r, obj, n)
					if !yield(AgwTCPRoute{TCPRoute: res}, err) {
						return
					}
				}
			}
		}, func(status gwv1.RouteStatus) gwv1a2.TCPRouteStatus {
			return gwv1a2.TCPRouteStatus{RouteStatus: status}
		})
	status.RegisterStatus(queue, tcpRouteStatus, GetStatus)

	tlsRouteStatus, tlsRoutes := createRouteCollectionGeneric(tlsRouteCol, inputs, krtopts, "TLSRoutes",
		func(ctx RouteContext, obj *gwv1.TLSRoute) (RouteContext, iter.Seq2[AgwTCPRoute, *reporter.RouteCondition]) {
			route := obj.Spec
			return ctx, func(yield func(AgwTCPRoute, *reporter.RouteCondition) bool) {
				for n, r := range route.Rules {
					// Convert the entire rule with all matches at once
					res, err := ConvertTLSRouteToAgw(ctx, r, obj, n)
					if !yield(AgwTCPRoute{TCPRoute: res}, err) {
						return
					}
				}
			}
		}, func(status gwv1.RouteStatus) gwv1.TLSRouteStatus {
			return gwv1.TLSRouteStatus{RouteStatus: status}
		})
	status.RegisterStatus(queue, tlsRouteStatus, GetStatus)

	routes := krt.JoinCollection(
		[]krt.Collection[agwir.AgwResource]{
			httpRoutes,
			delegatedHTTPRoutes,
			grpcRoutes,
			tcpRoutes,
			tlsRoutes,
		},
		krtopts.ToOptions("ADPRoutes")...,
	)

	routeAttachments := krt.JoinCollection([]krt.Collection[*plugins.RouteAttachment]{
		gatewayRouteAttachmentCollection(inputs, httpRouteCol, wellknown.HTTPRouteGVK, krtopts),
		delegatedGatewayRouteAttachmentCountCollection(delegatedHTTPRoutes, krtopts),
		gatewayRouteAttachmentCollection(inputs, grpcRouteCol, wellknown.GRPCRouteGVK, krtopts),
		gatewayRouteAttachmentCollection(inputs, tlsRouteCol, wellknown.TLSRouteGVK, krtopts),
		gatewayRouteAttachmentCollection(inputs, tcpRouteCol, wellknown.TCPRouteGVK, krtopts),
	})

	ancestorBackends := krt.JoinCollection([]krt.Collection[*utils.AncestorBackend]{
		krt.NewManyCollection(httpRouteCol, func(krtctx krt.HandlerContext, obj *gwv1.HTTPRoute) []*utils.AncestorBackend {
			ctx := inputs.WithCtx(krtctx)
			return extractAncestorBackends(ctx, obj, "HTTPRoute", obj.Spec.Rules, func(r gwv1.HTTPRouteRule) []gwv1.HTTPBackendRef {
				return r.BackendRefs
			})
		}, krtopts.ToOptions("HTTPAncestors")...),
		delegatedHTTPAncestors,
		krt.NewManyCollection(grpcRouteCol, func(krtctx krt.HandlerContext, obj *gwv1.GRPCRoute) []*utils.AncestorBackend {
			ctx := inputs.WithCtx(krtctx)
			return extractAncestorBackends(ctx, obj, "GRPCRoute", obj.Spec.Rules, func(r gwv1.GRPCRouteRule) []gwv1.GRPCBackendRef {
				return r.BackendRefs
			})
		}, krtopts.ToOptions("GRPCAncestors")...),
		krt.NewManyCollection(tlsRouteCol, func(krtctx krt.HandlerContext, obj *gwv1.TLSRoute) []*utils.AncestorBackend {
			ctx := inputs.WithCtx(krtctx)
			return extractAncestorBackends(ctx, obj, "TLSRoute", obj.Spec.Rules, func(r gwv1.TLSRouteRule) []gwv1a2.BackendRef {
				return r.BackendRefs
			})
		}, krtopts.ToOptions("TLSAncestors")...),
		krt.NewManyCollection(tcpRouteCol, func(krtctx krt.HandlerContext, obj *gwv1a2.TCPRoute) []*utils.AncestorBackend {
			ctx := inputs.WithCtx(krtctx)
			return extractAncestorBackends(ctx, obj, "TCPRoute", obj.Spec.Rules, func(r gwv1a2.TCPRouteRule) []gwv1a2.BackendRef {
				return r.BackendRefs
			})
		}, krtopts.ToOptions("TCPAncestors")...),
	})

	return routes, routeAttachments, ancestorBackends
}

// ProcessParentReferences processes filtered parent references and builds resources per gateway.
// It emits exactly one ParentStatus per Gateway (aggregate across listeners).
// If no listeners are allowed, the Accepted reason is:
//   - NotAllowedByListeners  => when the parent Gateway is cross-namespace w.r.t. the route
//   - NoMatchingListenerHostname => otherwise
func ProcessParentReferences[T any](
	parentRefs []RouteParentReference,
	gwResult ConversionResult[T],
	routeNN types.NamespacedName, // <-- route namespace/name so we can detect cross-NS parents
	routeReporter reporter.RouteReporter,
) []agwir.AgwResource {
	resources := make([]agwir.AgwResource, 0, len(parentRefs))

	// Build the "allowed" set from FilteredReferences (listener-scoped).
	allowed := make(map[string]struct{})
	for _, p := range FilteredReferences(parentRefs) {
		k := fmt.Sprintf("%s/%s/%s/%s", p.ParentKey.Namespace, p.ParentKey.Name, p.ParentKey.Kind, string(p.ParentSection))
		allowed[k] = struct{}{}
	}

	// Aggregate per Gateway for status; also track whether any raw parent was cross-namespace.
	type gwAgg struct {
		anyAllowed bool
		parentRefs []RouteParentReference
	}
	agg := make(map[types.NamespacedName]*gwAgg)
	crossNS := sets.New[types.NamespacedName]()
	denied := make(map[types.NamespacedName]*ParentError)

	for _, p := range parentRefs {
		gwNN := p.ParentGateway
		if _, ok := agg[gwNN]; !ok {
			agg[gwNN] = &gwAgg{anyAllowed: false, parentRefs: []RouteParentReference{p}}
		} else {
			agg[gwNN].parentRefs = append(agg[gwNN].parentRefs, p)
		}
		if p.ParentKey.Namespace != routeNN.Namespace {
			crossNS.Insert(gwNN)
		}
		if p.DeniedReason != nil {
			denied[gwNN] = p.DeniedReason
		}
	}

	// If conversion (backend/filter resolution) failed, ResolvedRefs=False for all parents.
	resolvedOK := gwResult.Error == nil

	// Consider each raw parentRef (listener-scoped) for mapping.
	for _, parent := range parentRefs {
		gwNN := parent.ParentGateway
		listener := string(parent.ParentSection)
		keyStr := fmt.Sprintf("%s/%s/%s/%s", parent.ParentKey.Namespace, parent.ParentKey.Name, parent.ParentKey.Kind, listener)
		_, isAllowed := allowed[keyStr]

		if isAllowed {
			if a := agg[gwNN]; a != nil {
				a.anyAllowed = true
			}
		}
		// Only attach resources when listener is allowed. Even if ResolvedRefs is false,
		// we still attach so any DirectResponse policy can return 5xx as required.
		if !isAllowed {
			continue
		}
		routes := gwResult.Routes
		for i := range routes {
			if r := resourceMapper(routes[i], parent); r != nil {
				resources = append(resources, ToResourceForGateway(gwNN, r))
			}
		}
	}

	// Emit exactly ONE ParentStatus per Gateway (aggregate across listeners; no SectionName).
	for gwNN, a := range agg {
		for _, parent := range a.parentRefs {
			prStatusRef := parent.OriginalReference
			{
				stringPtr := func(s string) *string { return &s }
				prStatusRef.Kind = (*gwv1.Kind)(stringPtr(parent.ParentKey.Kind))
				prStatusRef.Namespace = (*gwv1.Namespace)(stringPtr(parent.ParentKey.Namespace))
				prStatusRef.Name = gwv1.ObjectName(parent.ParentKey.Name)
				prStatusRef.SectionName = nil
			}
			pr := routeReporter.ParentRef(&prStatusRef)
			resolvedReason := reasonResolvedRefs(gwResult.Error, resolvedOK)

			if a.anyAllowed {
				pr.SetCondition(reporter.RouteCondition{
					Type:   gwv1.RouteConditionAccepted,
					Status: metav1.ConditionTrue,
					Reason: gwv1.RouteReasonAccepted,
				})
			} else {
				// Nothing attached: choose reason based on *why* it wasn't allowed.
				// Priority:
				// 1) Denied
				// 2) Cross-namespace and listeners don’t allow it -> NotAllowedByListeners
				// 3) sectionName specified but no such listener on the parent -> NoMatchingParent
				// 4) Otherwise, no hostname intersection -> NoMatchingListenerHostname
				reason := gwv1.RouteConditionReason("NoMatchingListenerHostname")
				msg := "No route hostnames intersect any listener hostname"
				if dr := denied[gwNN]; dr != nil {
					reason = gwv1.RouteConditionReason(dr.Reason)
					msg = dr.Message
				}
				if crossNS.Contains(gwNN) {
					reason = gwv1.RouteReasonNotAllowedByListeners
					msg = "Parent listener not usable or not permitted"
				} else if parent.OriginalReference.SectionName != nil || parent.OriginalReference.Port != nil {
					// Use string literal to avoid compile issues if the constant name differs.
					reason = "NoMatchingParent"
					msg = "No listener with the specified sectionName on the parent Gateway"
				}
				pr.SetCondition(reporter.RouteCondition{
					Type:    gwv1.RouteConditionAccepted,
					Status:  metav1.ConditionFalse,
					Reason:  reason,
					Message: msg,
				})
			}

			pr.SetCondition(reporter.RouteCondition{
				Type: gwv1.RouteConditionResolvedRefs,
				Status: func() metav1.ConditionStatus {
					if resolvedOK {
						return metav1.ConditionTrue
					}
					return metav1.ConditionFalse
				}(),
				Reason: resolvedReason,
				Message: func() string {
					if gwResult.Error != nil {
						return gwResult.Error.Message
					}
					return ""
				}(),
			})
		}
	}
	return resources
}

func resourceMapper(t any, parent RouteParentReference) *api.Resource {
	var serviceKey *workloadapi.NamespacedHostname
	if parent.ServiceKey != nil {
		serviceKey = &workloadapi.NamespacedHostname{
			Namespace: parent.ServiceKey.Namespace,
			Hostname:  parent.ServiceKey.Name,
		}
	}

	switch tt := t.(type) {
	case AgwTCPRoute:
		// safety: a shallow clone is ok because we only modify a top level field (Key)
		inner := protomarshal.ShallowClone(tt.TCPRoute)
		inner.ListenerKey = parent.ListenerKey
		inner.ServiceKey = serviceKey
		inner.Key += routeKeySuffix(parent)
		if inner.ServiceKey != nil {
			// if linked by Service, no need for hostname matching
			inner.Hostnames = nil
		}

		return ToAgwResource(AgwTCPRoute{TCPRoute: inner})
	case AgwRoute:
		// safety: a shallow clone is ok because we only modify a top level field (Key)
		inner := protomarshal.ShallowClone(tt.Route)
		inner.ListenerKey = parent.ListenerKey
		inner.ServiceKey = serviceKey
		inner.Key += routeKeySuffix(parent)
		if inner.ServiceKey != nil {
			// if linked by Service, no need for hostname matching
			inner.Hostnames = nil
		}

		return ToAgwResource(AgwRoute{Route: inner})
	default:
		log.Fatalf("unknown route kind %T", t)
		return nil
	}
}

func routeKeySuffix(parent RouteParentReference) string {
	if parent.ServiceKey != nil {
		return ".svc." + parent.ServiceKey.Namespace + "." + parent.ServiceKey.Name
	}
	if sec := string(parent.ParentSection); sec != "" {
		return "." + sec
	}
	return ""
}

// reasonResolvedRefs picks a ResolvedRefs reason from a conversion failure condition.
// Falls back to "ResolvedRefs" (when ok) or "Invalid" (when not ok and no specific reason).
func reasonResolvedRefs(cond *reporter.RouteCondition, ok bool) gwv1.RouteConditionReason {
	if ok {
		return gwv1.RouteReasonResolvedRefs
	}
	if cond != nil && cond.Reason != "" {
		return cond.Reason
	}
	return "Invalid"
}

// buildAttachedRoutesMapAllowed is the same as buildAttachedRoutesMap,
// but only for already-evaluated, allowed parentRefs.
func buildAttachedRoutesMapAllowed(
	allowedParents []RouteParentReference,
	routeNN types.NamespacedName,
) map[types.NamespacedName]map[string]uint {
	attached := make(map[types.NamespacedName]map[string]uint)
	type attachKey struct {
		gw       types.NamespacedName
		listener string
		route    types.NamespacedName
	}
	seen := make(map[attachKey]struct{})

	for _, parent := range allowedParents {
		if parent.ParentKey.Kind != wellknown.GatewayGVK.Kind {
			continue
		}
		gw := types.NamespacedName{Namespace: parent.ParentKey.Namespace, Name: parent.ParentKey.Name}
		lis := string(parent.ParentSection)

		k := attachKey{gw: gw, listener: lis, route: routeNN}
		if _, ok := seen[k]; ok {
			continue
		}
		seen[k] = struct{}{}

		if attached[gw] == nil {
			attached[gw] = make(map[string]uint)
		}
		attached[gw][lis]++
	}
	return attached
}

// Generic function that handles the common logic
func createRouteCollectionGeneric[T controllers.Object, R comparable, ST any](
	routeCol krt.Collection[T],
	inputs RouteContextInputs,
	krtopts krtutil.KrtOptions,
	collectionName string,
	translator func(ctx RouteContext, obj T) (RouteContext, iter.Seq2[R, *reporter.RouteCondition]),
	buildStatus func(status gwv1.RouteStatus) ST,
	postProcess ...func(krtctx krt.HandlerContext, obj T, routeReporter reporter.RouteReporter),
) (
	krt.StatusCollection[T, ST],
	krt.Collection[agwir.AgwResource],
) {
	return krt.NewStatusManyCollection(routeCol, func(krtctx krt.HandlerContext, obj T) (*ST, []agwir.AgwResource) {
		logger.Debug("translating route", "route_name", obj.GetName(), "resource_version", obj.GetResourceVersion())

		ctx := inputs.WithCtx(krtctx)
		rm := reports.NewReportMap()
		rep := reports.NewReporter(&rm)
		routeReporter := rep.Route(obj)

		// Apply route-specific preprocessing and get the translator
		ctx, translatorSeq := translator(ctx, obj)

		parentRefs, gwResult := computeRoute(ctx, obj, func(obj T) iter.Seq2[R, *reporter.RouteCondition] {
			return translatorSeq
		})

		// gateway -> section name -> route count
		routeNN := types.NamespacedName{Namespace: obj.GetNamespace(), Name: obj.GetName()}
		ln := ListenersPerGateway(parentRefs)
		allowedParents := FilteredReferences(parentRefs)
		attachedRoutes := buildAttachedRoutesMapAllowed(allowedParents, routeNN)
		EnsureZeroes(attachedRoutes, ln)

		resources := ProcessParentReferences[R](
			parentRefs,
			gwResult,
			routeNN,
			routeReporter,
		)

		// Apply post-processing to enrich status (e.g., delegation parent status).
		for _, pp := range postProcess {
			pp(krtctx, obj, routeReporter)
		}

		status := rm.BuildRouteStatusWithParentRefDefaulting(context.Background(), obj, inputs.ControllerName, true)
		return new(buildStatus(*status)), resources
	}, krtopts.ToOptions(collectionName)...)
}

// ListenersPerGateway returns the set of listener sectionNames referenced for each parent Gateway,
// regardless of whether they are allowed.
func ListenersPerGateway(parentRefs []RouteParentReference) map[types.NamespacedName]map[string]struct{} {
	l := make(map[types.NamespacedName]map[string]struct{})
	for _, p := range parentRefs {
		if p.ParentKey.Kind != wellknown.GatewayGVK.Kind {
			continue
		}
		gw := types.NamespacedName{Namespace: p.ParentKey.Namespace, Name: p.ParentKey.Name}
		if l[gw] == nil {
			l[gw] = make(map[string]struct{})
		}
		l[gw][string(p.ParentSection)] = struct{}{}
	}
	return l
}

// EnsureZeroes pre-populates AttachedRoutes with explicit 0 entries for every referenced listener,
// so writers that "replace" rather than "merge" will correctly set zero.
func EnsureZeroes(
	attached map[types.NamespacedName]map[string]uint,
	ln map[types.NamespacedName]map[string]struct{},
) {
	for gw, set := range ln {
		if attached[gw] == nil {
			attached[gw] = make(map[string]uint)
		}
		for lis := range set {
			if _, ok := attached[gw][lis]; !ok {
				attached[gw][lis] = 0
			}
		}
	}
}

type ConversionResult[O any] struct {
	Error  *reporter.RouteCondition
	Routes []O
}

// IsNil works around comparing generic types
func IsNil[O comparable](o O) bool {
	var t O
	return o == t
}

// computeRoute holds the common route building logic shared amongst all types
func computeRoute[T controllers.Object, O comparable](ctx RouteContext, obj T, translator func(
	obj T,
) iter.Seq2[O, *reporter.RouteCondition],
) ([]RouteParentReference, ConversionResult[O]) {
	parentRefs := extractParentReferenceInfo(ctx, ctx.RouteParents, obj)

	convertRules := func() ConversionResult[O] {
		res := ConversionResult[O]{}
		for vs, err := range translator(obj) {
			// This was a hard Error
			if err != nil && IsNil(vs) {
				res.Error = err
				return ConversionResult[O]{Error: err}
			}
			// Got an error but also Routes
			if err != nil {
				res.Error = err
			}
			res.Routes = append(res.Routes, vs)
		}
		return res
	}
	gwResult := buildGatewayRoutes(convertRules)

	return parentRefs, gwResult
}

// RouteContext defines a common set of inputs to a route collection for agentgateway.
// This should be built once per route translation and not shared outside of that.
// The embedded RouteContextInputs is typically based into a collection, then translated to a RouteContext with RouteContextInputs.WithCtx().
type RouteContext struct {
	Krt krt.HandlerContext
	RouteContextInputs
}

// RouteContextInputs defines the collections needed to translate a route.
type RouteContextInputs struct {
	Grants         ReferenceGrants
	RouteParents   ParentResolver
	Services       krt.Collection[*corev1.Service]
	InferencePools krt.Collection[*inf.InferencePool]
	Namespaces     krt.Collection[*corev1.Namespace]
	ServiceEntries krt.Collection[*networkingclient.ServiceEntry]
	Backends       krt.Collection[*agentgateway.AgentgatewayBackend]
	References     plugins.ReferenceTypes
	ControllerName string
}

func (i RouteContextInputs) WithCtx(krtctx krt.HandlerContext) RouteContext {
	return RouteContext{
		Krt:                krtctx,
		RouteContextInputs: i,
	}
}

// RouteWithKey is a wrapper for a Route
type RouteWithKey struct {
	*Config
}

func (r RouteWithKey) ResourceName() string {
	return config.NamespacedName(r.Config).String()
}

func (r RouteWithKey) Equals(o RouteWithKey) bool {
	return r.Config.Equals(o.Config)
}

// buildGatewayRoutes contains common logic to build a set of Routes with v1/alpha2 semantics
func buildGatewayRoutes[T any](convertRules func() T) T {
	return convertRules()
}

// gatewayRouteAttachmentCollection holds the generic logic to determine the parents a route is attached to.
// Used for computing `attachedRoutes` status and for resolving route-to-gateway associations in the ReferenceIndex.
func gatewayRouteAttachmentCollection[T controllers.Object](
	inputs RouteContextInputs,
	col krt.Collection[T],
	kind schema.GroupVersionKind,
	opts krtutil.KrtOptions,
) krt.Collection[*plugins.RouteAttachment] {
	return krt.NewManyCollection(col, func(krtctx krt.HandlerContext, obj T) []*plugins.RouteAttachment {
		ctx := inputs.WithCtx(krtctx)
		from := utils.TypedNamespacedName{
			Kind:           kind.Kind,
			NamespacedName: config.NamespacedName(obj),
		}

		parentRefs := extractParentReferenceInfo(ctx, inputs.RouteParents, obj)
		return slices.MapFilter(FilteredReferences(parentRefs), func(e RouteParentReference) **plugins.RouteAttachment {
			if e.ParentKey.Kind == wellknown.ListenerSetGVK.Kind {
				return new(&plugins.RouteAttachment{
					From:         from,
					To:           e.ParentKey,
					Gateway:      e.ParentGateway,
					ListenerName: string(e.ParentSection),
				})
			}
			if e.ParentGateway.Name == "" {
				return nil
			}
			return new(&plugins.RouteAttachment{
				From: from,
				To: utils.TypedNamespacedName{
					Kind:           wellknown.GatewayGVK.Kind,
					NamespacedName: e.ParentGateway,
				},
				Gateway:      e.ParentGateway,
				ListenerName: string(e.ParentSection),
			})
		})
	}, opts.ToOptions(kind.Kind+"/count")...)
}

// gatewayRouteAttachmentCountCollection holds the generic logic to determine the parents a route is attached to, used for
// computing the aggregated `attachedRoutes` status in Gateway.
func delegatedGatewayRouteAttachmentCountCollection(
	col krt.Collection[agwir.AgwResource],
	opts krtutil.KrtOptions,
) krt.Collection[*plugins.RouteAttachment] {
	return krt.NewManyCollection(col, func(krtctx krt.HandlerContext, obj agwir.AgwResource) []*plugins.RouteAttachment {
		//ctx := inputs.WithCtx(krtctx)
		n := obj.Resource.GetRoute().GetName()
		from := utils.TypedNamespacedName{
			Kind:           wellknown.HTTPRouteKind,
			NamespacedName: types.NamespacedName{Namespace: n.Namespace, Name: n.Name},
		}
		return []*plugins.RouteAttachment{{
			From: from,
			// ?? If we set this to the Gateway, we can get attachedRoutes added. If we don't, we will not. Our choice.
			// However, if we *do* we need to also figure out ListenerName which makes it probably more complex than its worth.
			To: utils.TypedNamespacedName{},
			// Never set
			ListenerName: "",
			Gateway:      obj.Gateway,
		}}
	}, opts.ToOptions("DelegatedHTTPRoute/count")...)
}

func extractAncestorBackends[T controllers.Object, RT, BT any](ctx RouteContext, obj T, kind string, rules []RT, extract func(RT) []BT) []*utils.AncestorBackend {
	source := utils.TypedNamespacedName{
		NamespacedName: types.NamespacedName{
			Namespace: obj.GetNamespace(),
			Name:      obj.GetName(),
		},
		Kind: kind,
	}
	gateways := sets.Set[types.NamespacedName]{}
	for _, parent := range FilteredReferences(extractParentReferenceInfo(ctx, ctx.RouteParents, obj)) {
		gateways.Insert(parent.ParentGateway)
	}
	backends := sets.Set[utils.TypedNamespacedName]{}
	for _, r := range rules {
		for _, b := range extract(r) {
			ref, refNs, refName := GetBackendRef(b)
			if ref == wellknown.HTTPRouteGVK {
				continue
			}
			be := utils.TypedNamespacedName{
				NamespacedName: types.NamespacedName{
					Namespace: defaultString(refNs, obj.GetNamespace()),
					Name:      string(refName),
				},
				Kind: ref.Kind,
			}
			backends.Insert(be)
		}
	}
	gtw := slices.SortBy(gateways.UnsortedList(), types.NamespacedName.String)
	bes := slices.SortBy(backends.UnsortedList(), utils.TypedNamespacedName.String)
	res := make([]*utils.AncestorBackend, 0, len(gtw)*len(bes))
	for _, gw := range gtw {
		for _, be := range bes {
			res = append(res, &utils.AncestorBackend{
				Gateway: gw,
				Backend: be,
				Source:  source,
			})
		}
	}
	return res
}

func GetBackendRef[I any](spec I) (schema.GroupVersionKind, *gwv1.Namespace, gwv1.ObjectName) {
	switch t := any(spec).(type) {
	case gwv1.HTTPBackendRef:
		return NormalizeReference(t.Group, t.Kind, wellknown.ServiceGVK), t.Namespace, t.Name
	case gwv1.GRPCBackendRef:
		return NormalizeReference(t.Group, t.Kind, wellknown.ServiceGVK), t.Namespace, t.Name
	case gwv1.BackendRef:
		return NormalizeReference(t.Group, t.Kind, wellknown.ServiceGVK), t.Namespace, t.Name
	default:
		log.Fatalf("unknown GetBackendRef type %T", t)
		return schema.GroupVersionKind{}, nil, ""
	}
}
