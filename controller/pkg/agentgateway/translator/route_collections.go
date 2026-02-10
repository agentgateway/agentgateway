package translator

import (
	"context"
	"fmt"
	"iter"
	"strings"

	"github.com/agentgateway/agentgateway/go/api"
	networkingclient "istio.io/client-go/pkg/apis/networking/v1"
	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/util/protomarshal"
	"istio.io/istio/pkg/util/sets"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	agwir "github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/ir"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer/status"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/krtutil"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/reporter"
	"github.com/kgateway-dev/kgateway/v2/pkg/reports"
)

// AgwRouteCollection creates the collection of translated Routes
func AgwRouteCollection(
	queue *status.StatusCollections,
	httpRouteCol krt.Collection[*gwv1.HTTPRoute],
	grpcRouteCol krt.Collection[*gwv1.GRPCRoute],
	tcpRouteCol krt.Collection[*gwv1a2.TCPRoute],
	tlsRouteCol krt.Collection[*gwv1a2.TLSRoute],
	inputs RouteContextInputs,
	krtopts krtutil.KrtOptions,
) (krt.Collection[agwir.AgwResource], krt.Collection[*RouteAttachment]) {
	httpRouteStatus, httpRoutes := createRouteCollection(httpRouteCol, inputs, krtopts, "HTTPRoutes",
		func(ctx RouteContext, obj *gwv1.HTTPRoute, rep reporter.Reporter) (RouteContext, iter.Seq2[AgwRoute, *reporter.RouteCondition]) {
			route := obj.Spec
			return ctx, func(yield func(AgwRoute, *reporter.RouteCondition) bool) {
				for n, r := range route.Rules {
					// split the rule to make sure each rule has up to one match
					matches := slices.Reference(r.Matches)
					if len(matches) == 0 {
						matches = append(matches, nil)
					}
					for idx, m := range matches {
						if m != nil {
							r.Matches = []gwv1.HTTPRouteMatch{*m}
						}
						res, err := ConvertHTTPRouteToAgw(ctx, r, obj, n, idx)
						if !yield(AgwRoute{Route: res}, err) {
							return
						}
					}
				}
			}
		}, func(status gwv1.RouteStatus) gwv1.HTTPRouteStatus {
			return gwv1.HTTPRouteStatus{RouteStatus: status}
		})
	status.RegisterStatus(queue, httpRouteStatus, GetStatus)

	grpcRouteStatus, grpcRoutes := createRouteCollection(grpcRouteCol, inputs, krtopts, "GRPCRoutes",
		func(ctx RouteContext, obj *gwv1.GRPCRoute, rep reporter.Reporter) (RouteContext, iter.Seq2[AgwRoute, *reporter.RouteCondition]) {
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

	tcpRouteStatus, tcpRoutes := createTCPRouteCollection(tcpRouteCol, inputs, krtopts, "TCPRoutes",
		func(ctx RouteContext, obj *gwv1a2.TCPRoute, rep reporter.Reporter) (RouteContext, iter.Seq2[AgwTCPRoute, *reporter.RouteCondition]) {
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

	tlsRouteStatus, tlsRoutes := createTCPRouteCollection(tlsRouteCol, inputs, krtopts, "TLSRoutes",
		func(ctx RouteContext, obj *gwv1a2.TLSRoute, rep reporter.Reporter) (RouteContext, iter.Seq2[AgwTCPRoute, *reporter.RouteCondition]) {
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
		}, func(status gwv1.RouteStatus) gwv1a2.TLSRouteStatus {
			return gwv1a2.TLSRouteStatus{RouteStatus: status}
		})
	status.RegisterStatus(queue, tlsRouteStatus, GetStatus)

	routes := krt.JoinCollection([]krt.Collection[agwir.AgwResource]{httpRoutes, grpcRoutes, tcpRoutes, tlsRoutes}, krtopts.ToOptions("ADPRoutes")...)

	routeAttachments := krt.JoinCollection([]krt.Collection[*RouteAttachment]{
		gatewayRouteAttachmentCountCollection(inputs, httpRouteCol, wellknown.HTTPRouteGVK, krtopts),
		gatewayRouteAttachmentCountCollection(inputs, grpcRouteCol, wellknown.GRPCRouteGVK, krtopts),
		gatewayRouteAttachmentCountCollection(inputs, tlsRouteCol, wellknown.TLSRouteGVK, krtopts),
		gatewayRouteAttachmentCountCollection(inputs, tcpRouteCol, wellknown.TCPRouteGVK, krtopts),
	})

	return routes, routeAttachments
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
	resourceMapper func(T, RouteParentReference) *api.Resource,
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
		rep        RouteParentReference
	}
	agg := make(map[types.NamespacedName]*gwAgg)
	crossNS := sets.New[types.NamespacedName]()
	denied := make(map[types.NamespacedName]*ParentError)

	for _, p := range parentRefs {
		gwNN := p.ParentGateway
		if _, ok := agg[gwNN]; !ok {
			agg[gwNN] = &gwAgg{anyAllowed: false, rep: p}
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
		parent := a.rep
		prStatusRef := parent.OriginalReference
		{
			stringPtr := func(s string) *string { return &s }
			prStatusRef.Group = (*gwv1.Group)(stringPtr(wellknown.GatewayGVK.Group))
			prStatusRef.Kind = (*gwv1.Kind)(stringPtr(wellknown.GatewayGVK.Kind))
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
			} else if a.rep.OriginalReference.SectionName != nil || a.rep.OriginalReference.Port != nil {
				// Use string literal to avoid compile issues if the constant name differs.
				reason = gwv1.RouteConditionReason("NoMatchingParent")
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
	return resources
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
	return gwv1.RouteConditionReason("Invalid")
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
		if parent.ParentKey.Kind != wellknown.GatewayGVK {
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
	translator func(ctx RouteContext, obj T, rep reporter.Reporter) (RouteContext, iter.Seq2[R, *reporter.RouteCondition]),
	resourceTransformer func(route R, parent RouteParentReference) *api.Resource,
	buildStatus func(status gwv1.RouteStatus) ST,
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
		ctx, translatorSeq := translator(ctx, obj, rep)

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
			resourceTransformer,
		)
		status := rm.BuildRouteStatusWithParentRefDefaulting(context.Background(), obj, inputs.ControllerName, true)
		return ptr.Of(buildStatus(*status)), resources
	}, krtopts.ToOptions(collectionName)...)
}

// Simplified HTTP route collection function
func createRouteCollection[T controllers.Object, ST any](
	routeCol krt.Collection[T],
	inputs RouteContextInputs,
	krtopts krtutil.KrtOptions,
	collectionName string,
	translator func(ctx RouteContext, obj T, rep reporter.Reporter) (RouteContext, iter.Seq2[AgwRoute, *reporter.RouteCondition]),
	buildStatus func(status gwv1.RouteStatus) ST,
) (
	krt.StatusCollection[T, ST],
	krt.Collection[agwir.AgwResource],
) {
	return createRouteCollectionGeneric(
		routeCol,
		inputs,
		krtopts,
		collectionName,
		translator,
		func(e AgwRoute, parent RouteParentReference) *api.Resource {
			// safety: a shallow clone is ok because we only modify a top level field (Key)
			inner := protomarshal.ShallowClone(e.Route)
			_, name, _ := strings.Cut(parent.InternalName, "/")
			inner.ListenerKey = name
			if sec := string(parent.ParentSection); sec != "" {
				inner.Key = inner.GetKey() + "." + sec
			} else {
				inner.Key = inner.GetKey()
			}
			return ToAgwResource(AgwRoute{Route: inner})
		},
		buildStatus,
	)
}

// Simplified TCP route collection function (plugins parameter removed)
func createTCPRouteCollection[T controllers.Object, ST any](
	routeCol krt.Collection[T],
	inputs RouteContextInputs,
	krtopts krtutil.KrtOptions,
	collectionName string,
	translator func(ctx RouteContext, obj T, rep reporter.Reporter) (RouteContext, iter.Seq2[AgwTCPRoute, *reporter.RouteCondition]),
	buildStatus func(status gwv1.RouteStatus) ST,
) (
	krt.StatusCollection[T, ST],
	krt.Collection[agwir.AgwResource],
) {
	return createRouteCollectionGeneric(
		routeCol,
		inputs,
		krtopts,
		collectionName,
		translator,
		func(e AgwTCPRoute, parent RouteParentReference) *api.Resource {
			inner := protomarshal.Clone(e.TCPRoute)
			_, name, _ := strings.Cut(parent.InternalName, "/")
			inner.ListenerKey = name
			if sec := string(parent.ParentSection); sec != "" {
				inner.Key = inner.GetKey() + "." + sec
			} else {
				inner.Key = inner.GetKey()
			}
			return ToAgwResource(AgwTCPRoute{TCPRoute: inner})
		},
		buildStatus,
	)
}

// ListenersPerGateway returns the set of listener sectionNames referenced for each parent Gateway,
// regardless of whether they are allowed.
func ListenersPerGateway(parentRefs []RouteParentReference) map[types.NamespacedName]map[string]struct{} {
	l := make(map[types.NamespacedName]map[string]struct{})
	for _, p := range parentRefs {
		if p.ParentKey.Kind != wellknown.GatewayGVK {
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
	RouteParents   RouteParents
	Services       krt.Collection[*corev1.Service]
	InferencePools krt.Collection[*inf.InferencePool]
	Namespaces     krt.Collection[*corev1.Namespace]
	ServiceEntries krt.Collection[*networkingclient.ServiceEntry]
	Backends       krt.Collection[*agentgateway.AgentgatewayBackend]
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

// gatewayRouteAttachmentCountCollection holds the generic logic to determine the parents a route is attached to, used for
// computing the aggregated `attachedRoutes` status in Gateway.
func gatewayRouteAttachmentCountCollection[T controllers.Object](
	inputs RouteContextInputs,
	col krt.Collection[T],
	kind schema.GroupVersionKind,
	opts krtutil.KrtOptions,
) krt.Collection[*RouteAttachment] {
	return krt.NewManyCollection(col, func(krtctx krt.HandlerContext, obj T) []*RouteAttachment {
		ctx := inputs.WithCtx(krtctx)
		from := TypedResource{
			Kind: kind,
			Name: config.NamespacedName(obj),
		}

		parentRefs := extractParentReferenceInfo(ctx, inputs.RouteParents, obj)
		return slices.MapFilter(FilteredReferences(parentRefs), func(e RouteParentReference) **RouteAttachment {
			if e.ParentKey.Kind != wellknown.GatewayGVK {
				return nil
			}
			return ptr.Of(&RouteAttachment{
				From: from,
				To: types.NamespacedName{
					Name:      e.ParentKey.Name,
					Namespace: e.ParentKey.Namespace,
				},
				ListenerName: string(e.ParentSection),
			})
		})
	}, opts.ToOptions(kind.Kind+"/count")...)
}

type RouteAttachment struct {
	From TypedResource
	// To is assumed to be a Gateway
	To           types.NamespacedName
	ListenerName string
}

func (r RouteAttachment) ResourceName() string {
	return r.From.Kind.String() + "/" + r.From.Name.String() + "/" + r.To.String() + "/" + r.ListenerName
}

func (r RouteAttachment) Equals(other RouteAttachment) bool {
	return r.From == other.From && r.To == other.To && r.ListenerName == other.ListenerName
}
