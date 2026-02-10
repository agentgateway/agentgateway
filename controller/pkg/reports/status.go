package reports

import (
	"context"
	"fmt"
	"log/slog"
	"reflect"
	"strings"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/reporter"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"
)

// Status message constants
const (
	GatewayAcceptedMessage       = "Successfully accepted Gateway"
	GatewayProgrammedMessage     = "Successfully programmed Gateway"
	ListenerSetAcceptedMessage   = "Successfully accepted ListenerSet"
	ListenerSetProgrammedMessage = "Successfully programmed ListenerSet"
	ListenerAcceptedMessage      = "Successfully accepted Listener"
	ListenerNoConflictsMessage   = "Successfully verified that Listener has no conflicts"
	ValidRefsMessage             = "Successfully resolved all references"
	ListenerProgrammedMessage    = "Successfully programmed Listener"
	RouteAcceptedMessage         = "Successfully accepted Route"
	GatewayClassAcceptedMessage  = "GatewayClass accepted by kgateway controller"
)

// TODO: refactor this struct + methods to better reflect the usage now in proxy_syncer

func (r *ReportMap) BuildGWStatus(ctx context.Context, gw gwv1.Gateway, attachedRoutes map[string]uint) *gwv1.GatewayStatus {
	gwReport := r.Gateway(&gw)
	if gwReport == nil {
		return nil
	}

	finalListeners := make([]gwv1.ListenerStatus, 0, len(gw.Spec.Listeners))
	var invalidListeners []string
	var invalidMessages []string

	for _, lis := range gw.Spec.Listeners {
		lisReport := gwReport.listener(string(lis.Name))
		AddMissingListenerConditions(lisReport)
		// Get attached routes for this listener
		if attachedRoutes != nil {
			if count, exists := attachedRoutes[string(lis.Name)]; exists {
				lisReport.Status.AttachedRoutes = int32(count) //nolint:gosec // G115: route count is always non-negative
			}
		}

		finalConditions := make([]metav1.Condition, 0, len(lisReport.Status.Conditions))
		oldLisStatusIndex := slices.IndexFunc(gw.Status.Listeners, func(l gwv1.ListenerStatus) bool {
			return l.Name == lis.Name
		})
		for _, lisCondition := range lisReport.Status.Conditions {
			lisCondition.ObservedGeneration = gwReport.observedGeneration

			// copy old condition from gw so LastTransitionTime is set correctly below by SetStatusCondition()
			if oldLisStatusIndex != -1 {
				if cond := meta.FindStatusCondition(gw.Status.Listeners[oldLisStatusIndex].Conditions, lisCondition.Type); cond != nil {
					finalConditions = append(finalConditions, *cond)
				}
			}
			meta.SetStatusCondition(&finalConditions, lisCondition)

			// Check if this is the Programmed condition and it's False
			if lisCondition.Type == string(gwv1.ListenerConditionProgrammed) && lisCondition.Status == metav1.ConditionFalse {
				invalidListeners = append(invalidListeners, string(lis.Name))
				if lisCondition.Message != "" {
					invalidMessages = append(invalidMessages, fmt.Sprintf("%s: %s", lis.Name, lisCondition.Message))
				}
			}
		}
		lisReport.Status.Conditions = finalConditions

		finalListeners = append(finalListeners, lisReport.Status)
	}

	// If any listeners have Programmed=False, set Gateway Accepted=True with ListenersNotValid reason
	if len(invalidListeners) > 0 {
		message := fmt.Sprintf("Some listeners are not programmed: %s", strings.Join(invalidMessages, "; "))
		if len(invalidMessages) == 0 {
			message = fmt.Sprintf("Some listeners are not programmed: %s", strings.Join(invalidListeners, ", "))
		}

		gwReport.SetCondition(reporter.GatewayCondition{
			Type:    gwv1.GatewayConditionAccepted,
			Status:  metav1.ConditionTrue,
			Reason:  gwv1.GatewayReasonListenersNotValid,
			Message: message,
		})
	}

	handleInvalidAddresses(gwReport, &gw)

	addMissingGatewayConditions(r.Gateway(&gw), &gw)

	finalConditions := make([]metav1.Condition, 0)
	for _, gwCondition := range gwReport.GetConditions() {
		gwCondition.ObservedGeneration = gwReport.observedGeneration

		// copy old condition from gw so LastTransitionTime is set correctly below by SetStatusCondition()
		if cond := meta.FindStatusCondition(gw.Status.Conditions, gwCondition.Type); cond != nil {
			finalConditions = append(finalConditions, *cond)
		}
		meta.SetStatusCondition(&finalConditions, gwCondition)
	}
	// If there are conditions on the Gateway that are not owned by our reporter, include
	// them in the final list of conditions to preseve conditions we do not own
	for _, condition := range gw.Status.Conditions {
		if meta.FindStatusCondition(finalConditions, condition.Type) == nil {
			finalConditions = append(finalConditions, condition)
		}
	}

	finalGwStatus := gwv1.GatewayStatus{}
	finalGwStatus.Addresses = gw.Status.Addresses
	finalGwStatus.Conditions = finalConditions
	finalGwStatus.Listeners = finalListeners
	return &finalGwStatus
}

func handleInvalidAddresses(report *GatewayReport, g *gwv1.Gateway) {
	for _, addr := range g.Spec.Addresses {
		if addr.Type == nil {
			continue
		}
		switch *addr.Type {
		case gwv1.IPAddressType:
		case gwv1.HostnameAddressType:
			report.SetCondition(reporter.GatewayCondition{
				Type:    gwv1.GatewayConditionProgrammed,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.GatewayReasonAddressNotUsable,
				Message: "Hostname addresses may not be used",
			})
		default:
			report.SetCondition(reporter.GatewayCondition{
				Type:    gwv1.GatewayConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.GatewayReasonUnsupportedAddress,
				Message: "Unknown address kind",
			})
		}
	}
}

// BuildRouteStatus returns a newly constructed and fully defined RouteStatus for the supplied route object
// according to the state of the ReportMap and the current status of the route.
// The gwv1.RouteStatus returned will contain all non-gateway parents from the provided current route status
// along with the newly built kgw status per ReportMap, sorted in deterministic fashion.
// If the ReportMap does not have a RouteReport for the given route, e.g. because it did not encounter
// the route during translation, or the object is an unsupported route kind, nil is returned.
// Supported route types are: HTTPRoute, TCPRoute, TLSRoute, GRPCRoute
func (r *ReportMap) BuildRouteStatus(
	ctx context.Context,
	obj client.Object,
	controller string,
) *gwv1.RouteStatus {
	return r.BuildRouteStatusWithParentRefDefaulting(ctx, obj, controller, false)
}

func (r *ReportMap) BuildRouteStatusWithParentRefDefaulting(
	ctx context.Context,
	obj client.Object,
	controller string,
	defaultParentRef bool,
) *gwv1.RouteStatus {
	routeReport := r.route(obj)
	if routeReport == nil {
		slog.Info("missing route report", "type", obj.GetObjectKind().GroupVersionKind().Kind, "name", obj.GetName(), "namespace", obj.GetNamespace())
		return nil
	}

	slog.Debug("building status", "type", obj.GetObjectKind().GroupVersionKind().Kind, "name", obj.GetName(), "namespace", obj.GetNamespace())

	var existingStatus gwv1.RouteStatus
	// Default to using spec.ParentRefs when building the parent statuses for a route.
	// However, for delegatee (child) routes, the parentRefs field is optional and such routes
	// may not specify it. In this case, we infer the parentRefs form the RouteReport
	// corresponding to the delegatee (child) route as the route's report is associated to a parentRef.
	var parentRefs []gwv1.ParentReference
	switch route := obj.(type) {
	case *gwv1.HTTPRoute:
		existingStatus = route.Status.RouteStatus
		parentRefs = append(parentRefs, route.Spec.ParentRefs...)
		if len(parentRefs) == 0 {
			parentRefs = append(parentRefs, routeReport.parentRefs()...)
		}
	case *gwv1a2.TCPRoute:
		existingStatus = route.Status.RouteStatus
		parentRefs = append(parentRefs, route.Spec.ParentRefs...)
		if len(parentRefs) == 0 {
			parentRefs = append(parentRefs, routeReport.parentRefs()...)
		}
	case *gwv1a2.TLSRoute:
		existingStatus = route.Status.RouteStatus
		parentRefs = append(parentRefs, route.Spec.ParentRefs...)
		if len(parentRefs) == 0 {
			parentRefs = append(parentRefs, routeReport.parentRefs()...)
		}
	case *gwv1.GRPCRoute:
		existingStatus = route.Status.RouteStatus
		parentRefs = append(parentRefs, route.Spec.ParentRefs...)
		if len(parentRefs) == 0 {
			parentRefs = append(parentRefs, routeReport.parentRefs()...)
		}
	default:
		slog.Error("unsupported route type for status reporting", "route_type", fmt.Sprintf("%T", obj))
		return nil
	}
	if defaultParentRef {
		parentRefs = ensureParentRefNamespaces(parentRefs, obj.GetNamespace())
	}

	newStatus := gwv1.RouteStatus{}
	// Process the parent references to build the RouteParentStatus
	for _, parentRef := range parentRefs {
		parentStatusReport := routeReport.getParentRefOrNil(&parentRef)
		if parentStatusReport == nil {
			// report doesn't have an entry for this parentRef, meaning we didn't translate it
			// probably because it's a parent that we don't control (e.g. Gateway from diff. controller)
			continue
		}
		addMissingParentRefConditions(parentStatusReport)

		// Get the status of the current parentRef conditions if they exist
		var currentParentRefConditions []metav1.Condition
		currentParentRefIdx := slices.IndexFunc(existingStatus.Parents, func(s gwv1.RouteParentStatus) bool {
			return reflect.DeepEqual(s.ParentRef, parentRef)
		})
		if currentParentRefIdx != -1 {
			currentParentRefConditions = existingStatus.Parents[currentParentRefIdx].Conditions
		}

		finalConditions := make([]metav1.Condition, 0, len(parentStatusReport.Conditions))
		for _, pCondition := range parentStatusReport.Conditions {
			pCondition.ObservedGeneration = routeReport.observedGeneration

			// Copy old condition to preserve LastTransitionTime, if it exists
			if cond := meta.FindStatusCondition(currentParentRefConditions, pCondition.Type); cond != nil {
				finalConditions = append(finalConditions, *cond)
			}
			meta.SetStatusCondition(&finalConditions, pCondition)
		}
		// If there are conditions on the route that are not owned by our reporter, include
		// them in the final list of conditions to preseve conditions we do not own
		for _, condition := range currentParentRefConditions {
			if meta.FindStatusCondition(finalConditions, condition.Type) == nil {
				finalConditions = append(finalConditions, condition)
			}
		}

		routeParentStatus := gwv1.RouteParentStatus{
			ParentRef:      parentRef,
			ControllerName: gwv1.GatewayController(controller),
			Conditions:     finalConditions,
		}
		newStatus.Parents = append(newStatus.Parents, routeParentStatus)
	}

	// now we have a status object reflecting the state of translation according to our reportMap
	// let's add status from other controllers on the current object status
	var kgwStatus = &newStatus
	for _, rps := range existingStatus.Parents {
		if rps.ControllerName != gwv1.GatewayController(controller) {
			kgwStatus.Parents = append(kgwStatus.Parents, rps)
		}
	}

	// sort all parents for consistency with Equals and for Update
	// match sorting semantics of istio/istio, see:
	// https://github.com/istio/istio/blob/6dcaa0206bcaf20e3e3b4e45e9376f0f96365571/pilot/pkg/config/kube/gateway/conditions.go#L188-L193
	slices.SortStableFunc(kgwStatus.Parents, func(a, b gwv1.RouteParentStatus) int {
		return strings.Compare(ParentString(a.ParentRef), ParentString(b.ParentRef))
	})
	if newStatus.Parents == nil {
		// Kubernetes will not let us send "nil", so we need an empty
		newStatus.Parents = []gwv1.RouteParentStatus{}
	}

	return &newStatus
}

func ensureParentRefNamespaces(parentRefs []gwv1.ParentReference, routeNamespace string) []gwv1.ParentReference {
	return slices.Map(parentRefs, func(e gwv1.ParentReference) gwv1.ParentReference {
		if e.Namespace == nil {
			routeNs := gwv1.Namespace(routeNamespace)
			e.Namespace = &routeNs
		}
		if e.Group == nil {
			e.Group = ptr.Of(gwv1.Group(wellknown.GatewayGVK.Group))
		}
		if e.Kind == nil {
			e.Kind = ptr.Of(gwv1.Kind(wellknown.GatewayGVK.Kind))
		}
		return e
	})
}

// match istio/istio logic, see:
// https://github.com/istio/istio/blob/6dcaa0206bcaf20e3e3b4e45e9376f0f96365571/pilot/pkg/config/kube/gateway/conversion.go#L2714-L2722
func ParentString(ref gwv1.ParentReference) string {
	return fmt.Sprintf("%s/%s/%s/%s/%d.%s",
		ptr.OrEmpty(ref.Group),
		ptr.OrEmpty(ref.Kind),
		ref.Name,
		ptr.OrEmpty(ref.SectionName),
		ptr.OrEmpty(ref.Port),
		ptr.OrEmpty(ref.Namespace))
}

// Reports will initially only contain negative conditions found during translation,
// so all missing conditions are assumed to be positive. Here we will add all missing conditions
// to a given report, i.e. set healthy conditions
func addMissingGatewayConditions(gwReport *GatewayReport, gw *gwv1.Gateway) {
	// If the existing Gateway status contains an Accepted=False with Reason=InvalidParameters,
	// we don't want to override it with a true Accepted status. The controller will set Accepted=True
	// when the GatewayParameters are valid again. Otherwise there is a race condition between the controller and reporter.
	// HACK: This is because both the controller and reporter set Accepted status.
	existingAccepted := meta.FindStatusCondition(gw.Status.Conditions, string(gwv1.GatewayConditionAccepted))
	hasInvalidParams := existingAccepted != nil && existingAccepted.Status == metav1.ConditionFalse && existingAccepted.Reason == string(gwv1.GatewayReasonInvalidParameters)
	if !hasInvalidParams && meta.FindStatusCondition(gwReport.GetConditions(), string(gwv1.GatewayConditionAccepted)) == nil {
		gwReport.SetCondition(reporter.GatewayCondition{
			Type:    gwv1.GatewayConditionAccepted,
			Status:  metav1.ConditionTrue,
			Reason:  gwv1.GatewayReasonAccepted,
			Message: GatewayAcceptedMessage,
		})
	}
	if cond := meta.FindStatusCondition(gwReport.GetConditions(), string(gwv1.GatewayConditionProgrammed)); cond == nil {
		gwReport.SetCondition(reporter.GatewayCondition{
			Type:    gwv1.GatewayConditionProgrammed,
			Status:  metav1.ConditionTrue,
			Reason:  gwv1.GatewayReasonProgrammed,
			Message: GatewayProgrammedMessage,
		})
	}
}

// Reports will initially only contain negative conditions found during translation,
// so all missing conditions are assumed to be positive. Here we will add all missing conditions
// to a given report, i.e. set healthy conditions
func AddMissingListenerConditions(lisReport *ListenerReport) {
	// set healthy conditions for Condition Types not set yet (i.e. no negative status yet, we can assume positive)
	if cond := meta.FindStatusCondition(lisReport.Status.Conditions, string(gwv1.ListenerConditionAccepted)); cond == nil {
		lisReport.SetCondition(reporter.ListenerCondition{
			Type:    gwv1.ListenerConditionAccepted,
			Status:  metav1.ConditionTrue,
			Reason:  gwv1.ListenerReasonAccepted,
			Message: ListenerAcceptedMessage,
		})
	}
	if cond := meta.FindStatusCondition(lisReport.Status.Conditions, string(gwv1.ListenerConditionConflicted)); cond == nil {
		lisReport.SetCondition(reporter.ListenerCondition{
			Type:    gwv1.ListenerConditionConflicted,
			Status:  metav1.ConditionFalse,
			Reason:  gwv1.ListenerReasonNoConflicts,
			Message: ListenerNoConflictsMessage,
		})
	}
	if cond := meta.FindStatusCondition(lisReport.Status.Conditions, string(gwv1.ListenerConditionResolvedRefs)); cond == nil {
		lisReport.SetCondition(reporter.ListenerCondition{
			Type:    gwv1.ListenerConditionResolvedRefs,
			Status:  metav1.ConditionTrue,
			Reason:  gwv1.ListenerReasonResolvedRefs,
			Message: ValidRefsMessage,
		})
	}
	if cond := meta.FindStatusCondition(lisReport.Status.Conditions, string(gwv1.ListenerConditionProgrammed)); cond == nil {
		lisReport.SetCondition(reporter.ListenerCondition{
			Type:    gwv1.ListenerConditionProgrammed,
			Status:  metav1.ConditionTrue,
			Reason:  gwv1.ListenerReasonProgrammed,
			Message: ListenerProgrammedMessage,
		})
	}
}

// Reports will initially only contain negative conditions found during translation,
// so all missing conditions are assumed to be positive. Here we will add all missing conditions
// to a given report, i.e. set healthy conditions
func addMissingParentRefConditions(report *ParentRefReport) {
	if cond := meta.FindStatusCondition(report.Conditions, string(gwv1.RouteConditionAccepted)); cond == nil {
		report.SetCondition(reporter.RouteCondition{
			Type:    gwv1.RouteConditionAccepted,
			Status:  metav1.ConditionTrue,
			Reason:  gwv1.RouteReasonAccepted,
			Message: RouteAcceptedMessage,
		})
	}
	if cond := meta.FindStatusCondition(report.Conditions, string(gwv1.RouteConditionResolvedRefs)); cond == nil {
		report.SetCondition(reporter.RouteCondition{
			Type:    gwv1.RouteConditionResolvedRefs,
			Status:  metav1.ConditionTrue,
			Reason:  gwv1.RouteReasonResolvedRefs,
			Message: ValidRefsMessage,
		})
	}
}
