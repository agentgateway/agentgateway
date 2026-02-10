package reports

import (
	"fmt"
	"log/slog"

	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/reporter"
)

type ReportMap struct {
	Gateways     map[types.NamespacedName]*GatewayReport
	ListenerSets map[schema.GroupVersionKind]map[types.NamespacedName]*ListenerSetReport
	HTTPRoutes   map[types.NamespacedName]*RouteReport
	GRPCRoutes   map[types.NamespacedName]*RouteReport
	TCPRoutes    map[types.NamespacedName]*RouteReport
	TLSRoutes    map[types.NamespacedName]*RouteReport
}

type GatewayReport struct {
	conditions         []metav1.Condition
	listeners          map[string]*ListenerReport
	observedGeneration int64
}

type ListenerSetReport struct {
	conditions         []metav1.Condition
	listeners          map[string]*ListenerReport
	observedGeneration int64
}

type ListenerReport struct {
	Status gwv1.ListenerStatus
}

type RouteReport struct {
	Parents            map[ParentRefKey]*ParentRefReport
	observedGeneration int64
}

// TODO: rename to e.g. RouteParentRefReport
type ParentRefReport struct {
	Conditions []metav1.Condition
}

type ParentRefKey struct {
	Group string
	Kind  string
	types.NamespacedName
}

func NewReportMap() ReportMap {
	return ReportMap{
		Gateways:     make(map[types.NamespacedName]*GatewayReport),
		ListenerSets: make(map[schema.GroupVersionKind]map[types.NamespacedName]*ListenerSetReport),
		HTTPRoutes:   make(map[types.NamespacedName]*RouteReport),
		GRPCRoutes:   make(map[types.NamespacedName]*RouteReport),
		TCPRoutes:    make(map[types.NamespacedName]*RouteReport),
		TLSRoutes:    make(map[types.NamespacedName]*RouteReport),
	}
}

func key(obj metav1.Object) types.NamespacedName {
	return types.NamespacedName{Namespace: obj.GetNamespace(), Name: obj.GetName()}
}

// Returns a GatewayReport for the provided Gateway, nil if there is not a report present.
// This is different than the Reporter.Gateway() method, as we need to understand when
// reports are not generated for a Gateway that has been translated.
//
// NOTE: Exported for unit testing, validation_test.go should be refactored to reduce this visibility
func (r *ReportMap) Gateway(gateway *gwv1.Gateway) *GatewayReport {
	key := key(gateway)
	return r.Gateways[key]
}

func (r *ReportMap) GatewayNamespaceName(key types.NamespacedName) *GatewayReport {
	return r.Gateways[key]
}

// GatewayObservedGenerationFor returns the observed generation stored in the
// GatewayReport for the provided namespaced name. The boolean indicates whether
// a report was present for the provided key.
func (r *ReportMap) GatewayObservedGenerationFor(key types.NamespacedName) (int64, bool) {
	gr := r.GatewayNamespaceName(key)
	if gr == nil {
		return 0, false
	}
	return gr.observedGeneration, true
}

func (r *ReportMap) newGatewayReport(gateway *gwv1.Gateway) *GatewayReport {
	gr := &GatewayReport{}
	gr.observedGeneration = gateway.Generation
	key := key(gateway)
	r.Gateways[key] = gr
	return gr
}

// Returns a ListenerSetReport for the provided ListenerSet, nil if there is not a report present.
// This is different than the Reporter.ListenerSet() method, as we need to understand when
// reports are not generated for a ListenerSet that has been translated.
//
// NOTE: Exported for unit testing, validation_test.go should be refactored to reduce this visibility
func (r *ReportMap) ListenerSet(listenerSet client.Object) *ListenerSetReport {
	gvk := listenerSet.GetObjectKind().GroupVersionKind()
	if gvk.Empty() {
		gvk = wellknown.XListenerSetGVK
	}
	if r.ListenerSets[gvk] == nil {
		r.ListenerSets[gvk] = make(map[types.NamespacedName]*ListenerSetReport)
	}
	key := key(listenerSet)
	return r.ListenerSets[gvk][key]
}

func (r *ReportMap) newListenerSetReport(listenerSet client.Object) *ListenerSetReport {
	lsr := &ListenerSetReport{
		observedGeneration: listenerSet.GetGeneration(),
	}

	gvk := listenerSet.GetObjectKind().GroupVersionKind()
	if gvk.Empty() {
		gvk = wellknown.XListenerSetGVK
	}
	if r.ListenerSets[gvk] == nil {
		r.ListenerSets[gvk] = make(map[types.NamespacedName]*ListenerSetReport)
	}
	key := key(listenerSet)
	r.ListenerSets[gvk][key] = lsr
	return lsr
}

// route returns a RouteReport for the provided route object, nil if a report is not present.
// This is different than the Reporter.Route() method, as we need to understand when
// reports are not generated for a route that has been translated. Supported object types are:
//
// * HTTPRoute
// * TCPRoute
// * TLSRoute
// * GRPCRoute
func (r *ReportMap) route(obj metav1.Object) *RouteReport {
	key := key(obj)

	switch obj.(type) {
	case *gwv1.HTTPRoute:
		return r.HTTPRoutes[key]
	case *gwv1a2.TCPRoute:
		return r.TCPRoutes[key]
	case *gwv1a2.TLSRoute:
		return r.TLSRoutes[key]
	case *gwv1.GRPCRoute:
		return r.GRPCRoutes[key]
	default:
		slog.Warn("unsupported route type", "route_type", fmt.Sprintf("%T", obj))
		return nil
	}
}

func (r *ReportMap) newRouteReport(obj metav1.Object) *RouteReport {
	rr := &RouteReport{
		observedGeneration: obj.GetGeneration(),
	}

	key := key(obj)

	switch obj.(type) {
	case *gwv1.HTTPRoute:
		r.HTTPRoutes[key] = rr
	case *gwv1a2.TCPRoute:
		r.TCPRoutes[key] = rr
	case *gwv1a2.TLSRoute:
		r.TLSRoutes[key] = rr
	case *gwv1.GRPCRoute:
		r.GRPCRoutes[key] = rr
	default:
		slog.Warn("unsupported route type", "route_type", fmt.Sprintf("%T", obj))
		return nil
	}

	return rr
}

func (g *GatewayReport) Listener(listener *gwv1.Listener) reporter.ListenerReporter {
	return g.listener(string(listener.Name))
}

func (g *GatewayReport) ListenerName(listenerName string) reporter.ListenerReporter {
	return g.listener(listenerName)
}

func (g *GatewayReport) listener(listenerName string) *ListenerReport {
	if g.listeners == nil {
		g.listeners = make(map[string]*ListenerReport)
	}

	// Return the ListenerReport if it already exists
	if lr, exists := g.listeners[string(listenerName)]; exists {
		return lr
	}

	// Create and add the new ListenerReport if it doesn't exist
	lr := NewListenerReport(string(listenerName))
	g.listeners[string(listenerName)] = lr
	return lr
}

func (g *GatewayReport) GetConditions() []metav1.Condition {
	if g == nil {
		return []metav1.Condition{}
	}
	return g.conditions
}

func (g *GatewayReport) SetCondition(gc reporter.GatewayCondition) {
	condition := metav1.Condition{
		Type:    string(gc.Type),
		Status:  gc.Status,
		Reason:  string(gc.Reason),
		Message: gc.Message,
	}
	meta.SetStatusCondition(&g.conditions, condition)
}

func (g *ListenerSetReport) Listener(listener *gwv1.Listener) reporter.ListenerReporter {
	return g.listener(string(listener.Name))
}

func (g *ListenerSetReport) ListenerName(listenerName string) reporter.ListenerReporter {
	return g.listener(listenerName)
}

func (g *ListenerSetReport) listener(listenerName string) *ListenerReport {
	if g.listeners == nil {
		g.listeners = make(map[string]*ListenerReport)
	}

	// Return the ListenerReport if it already exists
	if lr, exists := g.listeners[listenerName]; exists {
		return lr
	}

	// Create and add the new ListenerReport if it doesn't exist
	lr := NewListenerReport(listenerName)
	g.listeners[listenerName] = lr
	return lr
}

func (g *ListenerSetReport) GetConditions() []metav1.Condition {
	if g == nil {
		return []metav1.Condition{}
	}
	return g.conditions
}

func (g *ListenerSetReport) SetCondition(gc reporter.GatewayCondition) {
	condition := metav1.Condition{
		Type:    string(gc.Type),
		Status:  gc.Status,
		Reason:  string(gc.Reason),
		Message: gc.Message,
	}
	meta.SetStatusCondition(&g.conditions, condition)
}

func (g *ListenerSetReport) GetObservedGeneration() int64 {
	return g.observedGeneration
}

func NewListenerReport(name string) *ListenerReport {
	lr := ListenerReport{}
	// Set SupportedKinds to empty slice because it must be non-nil
	// without it, it will fail to set status
	lr.Status.SupportedKinds = []gwv1.RouteGroupKind{}
	lr.Status.Name = gwv1.SectionName(name)
	lr.Status.SupportedKinds = []gwv1.RouteGroupKind{} // Initialize with empty slice
	return &lr
}

func (l *ListenerReport) SetCondition(lc reporter.ListenerCondition) {
	condition := metav1.Condition{
		Type:    string(lc.Type),
		Status:  lc.Status,
		Reason:  string(lc.Reason),
		Message: lc.Message,
	}
	meta.SetStatusCondition(&l.Status.Conditions, condition)
}

func (l *ListenerReport) SetSupportedKinds(rgks []gwv1.RouteGroupKind) {
	l.Status.SupportedKinds = rgks
}

func (l *ListenerReport) SetAttachedRoutes(n uint) {
	l.Status.AttachedRoutes = int32(n) //nolint:gosec // G115: route count is always non-negative
}

type statusReporter struct {
	report *ReportMap
}

func (r *statusReporter) Gateway(gateway *gwv1.Gateway) reporter.GatewayReporter {
	gr := r.report.Gateway(gateway)
	if gr == nil {
		gr = r.report.newGatewayReport(gateway)
	}
	return gr
}

func (r *statusReporter) ListenerSet(listenerSet client.Object) reporter.ListenerSetReporter {
	lsr := r.report.ListenerSet(listenerSet)
	if lsr == nil {
		lsr = r.report.newListenerSetReport(listenerSet)
	}
	return lsr
}

func (r *statusReporter) Route(obj metav1.Object) reporter.RouteReporter {
	rr := r.report.route(obj)
	if rr == nil {
		rr = r.report.newRouteReport(obj)
	}
	return rr
}

// TODO: flesh out
func getParentRefKey(parentRef *gwv1.ParentReference) ParentRefKey {
	var group string
	if parentRef.Group != nil {
		group = string(*parentRef.Group)
	}
	var kind string
	if parentRef.Kind != nil {
		kind = string(*parentRef.Kind)
	}
	var ns string
	if parentRef.Namespace != nil {
		ns = string(*parentRef.Namespace)
	}
	return ParentRefKey{
		Group: group,
		Kind:  kind,
		NamespacedName: types.NamespacedName{
			Namespace: ns,
			Name:      string(parentRef.Name),
		},
	}
}

// getParentRefOrNil returns a ParentRefReport for the given parentRef if and only if
// that parentRef exists in the report (i.e. the parentRef was encountered during translation)
// If no report is found, nil is returned, signaling this parentRef is unknown to the report
func (r *RouteReport) getParentRefOrNil(parentRef *gwv1.ParentReference) *ParentRefReport {
	key := getParentRefKey(parentRef)
	if r.Parents == nil {
		r.Parents = make(map[ParentRefKey]*ParentRefReport)
	}
	return r.Parents[key]
}

func (r *RouteReport) parentRef(parentRef *gwv1.ParentReference) *ParentRefReport {
	key := getParentRefKey(parentRef)
	if r.Parents == nil {
		r.Parents = make(map[ParentRefKey]*ParentRefReport)
	}
	var prr *ParentRefReport
	prr, ok := r.Parents[key]
	if !ok {
		prr = &ParentRefReport{}
		r.Parents[key] = prr
	}
	return prr
}

// parentRefs returns a list of ParentReferences associated with the RouteReport.
// It is used to update the Status of delegatee routes who may not specify
// the parentRefs field.
func (r *RouteReport) parentRefs() []gwv1.ParentReference {
	var refs []gwv1.ParentReference
	for key := range r.Parents {
		var ns *gwv1.Namespace
		if key.Namespace != "" {
			ns = ptr.To(gwv1.Namespace(key.Namespace))
		}
		parentRef := gwv1.ParentReference{
			Group:     ptr.To(gwv1.Group(key.Group)),
			Kind:      ptr.To(gwv1.Kind(key.Kind)),
			Name:      gwv1.ObjectName(key.Name),
			Namespace: ns,
		}
		refs = append(refs, parentRef)
	}
	return refs
}

func (r *RouteReport) ParentRef(parentRef *gwv1.ParentReference) reporter.ParentRefReporter {
	return r.parentRef(parentRef)
}

func (prr *ParentRefReport) SetCondition(rc reporter.RouteCondition) {
	condition := metav1.Condition{
		Type:    string(rc.Type),
		Status:  rc.Status,
		Reason:  string(rc.Reason),
		Message: rc.Message,
	}
	meta.SetStatusCondition(&prr.Conditions, condition)
}

func NewReporter(reportMap *ReportMap) reporter.Reporter {
	return &statusReporter{
		report: reportMap,
	}
}
