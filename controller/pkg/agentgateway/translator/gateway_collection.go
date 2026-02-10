package translator

import (
	"bytes"
	"context"
	"fmt"

	"github.com/agentgateway/agentgateway/go/api"
	"istio.io/istio/pilot/pkg/util/protoconv"
	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/config/constants"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gatewayx "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/ir"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/plugins"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/utils"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/logging"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/krtutil"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/reporter"
	"github.com/kgateway-dev/kgateway/v2/pkg/reports"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
)

var (
	logger = logging.New("agentgateway/translator")
)

// ToAgwResource converts an internal representation to a resource for agentgateway
func ToAgwResource(t any) *api.Resource {
	switch tt := t.(type) {
	case AgwBind:
		return &api.Resource{Kind: &api.Resource_Bind{Bind: tt.Bind}}
	case AgwListener:
		return &api.Resource{Kind: &api.Resource_Listener{Listener: tt.Listener}}
	case AgwRoute:
		return &api.Resource{Kind: &api.Resource_Route{Route: tt.Route}}
	case AgwTCPRoute:
		return &api.Resource{Kind: &api.Resource_TcpRoute{TcpRoute: tt.TCPRoute}}
	case AgwPolicy:
		return &api.Resource{Kind: &api.Resource_Policy{Policy: tt.Policy}}
	case *api.Resource:
		return tt
	}
	panic(fmt.Sprintf("unknown resource kind %T", t))
}

func ToResourceForGateway(gw types.NamespacedName, resource any) ir.AgwResource {
	return ir.AgwResource{
		Resource: ToAgwResource(resource),
		Gateway:  gw,
	}
}

func ToResourceGlobal(resource any) ir.AgwResource {
	return ir.AgwResource{
		Resource: ToAgwResource(resource),
	}
}

// AgwBind is a wrapper type that contains the bind on the gateway, as well as the status for the bind.
type AgwBind struct {
	*api.Bind
}

func (g AgwBind) ResourceName() string {
	return g.Key
}

func (g AgwBind) Equals(other AgwBind) bool {
	return protoconv.Equals(g, other)
}

// AgwListener is a wrapper type that contains the listener on the gateway, as well as the status for the listener.
type AgwListener struct {
	*api.Listener
}

func (g AgwListener) ResourceName() string {
	return g.Key
}

func (g AgwListener) Equals(other AgwListener) bool {
	return protoconv.Equals(g, other)
}

// AgwPolicy is a wrapper type that contains the policy on the gateway, as well as the status for the policy.
type AgwPolicy = plugins.AgwPolicy

// AgwBackend is a wrapper type that contains the backend on the gateway, as well as the status for the backend.
type AgwBackend struct {
	*api.Backend
}

func (g AgwBackend) ResourceName() string {
	return g.Key
}

func (g AgwBackend) Equals(other AgwBackend) bool {
	return protoconv.Equals(g, other)
}

// AgwRoute is a wrapper type that contains the route on the gateway, as well as the status for the route.
type AgwRoute struct {
	*api.Route
}

func (g AgwRoute) ResourceName() string {
	return g.Key
}

func (g AgwRoute) Equals(other AgwRoute) bool {
	return protoconv.Equals(g, other)
}

// AgwTCPRoute is a wrapper type that contains the tcp route on the gateway, as well as the status for the tcp route.
type AgwTCPRoute struct {
	*api.TCPRoute
}

func (g AgwTCPRoute) ResourceName() string {
	return g.Key
}

func (g AgwTCPRoute) Equals(other AgwTCPRoute) bool {
	return protoconv.Equals(g, other)
}

// TLSInfo contains the TLS certificate and key for a gateway listener.
type TLSInfo struct {
	Cert   []byte
	CaCert []byte
	Key    []byte `json:"-"`
}

// PortBindings is a wrapper type that contains the listener on the gateway, as well as the status for the listener.
type PortBindings struct {
	GatewayListener
	Port string
}

func (g PortBindings) ResourceName() string {
	return g.GatewayListener.Name
}

func (g PortBindings) Equals(other PortBindings) bool {
	return g.GatewayListener.Equals(other.GatewayListener) &&
		g.Port == other.Port
}

// GatewayListener is a wrapper type that contains the listener on the gateway, as well as the status for the listener.
// This allows binding to a specific listener.
type GatewayListener struct {
	Name string
	// The Gateway this listener is bound to
	ParentGateway types.NamespacedName
	// The actual real parent (could be a ListenerSet)
	ParentObject ParentKey
	ParentInfo   ParentInfo
	TLSInfo      *TLSInfo
	Valid        bool
}

func (g GatewayListener) ResourceName() string {
	return g.Name
}

func (g GatewayListener) Equals(other GatewayListener) bool {
	if (g.TLSInfo != nil) != (other.TLSInfo != nil) {
		return false
	}
	if g.TLSInfo != nil {
		if !bytes.Equal(g.TLSInfo.Cert, other.TLSInfo.Cert) ||
			!bytes.Equal(g.TLSInfo.Key, other.TLSInfo.Key) ||
			!bytes.Equal(g.TLSInfo.CaCert, other.TLSInfo.CaCert) {
			return false
		}
	}
	return g.Valid == other.Valid && g.Name == other.Name && g.ParentGateway == other.ParentGateway && g.ParentObject == other.ParentObject && g.ParentInfo.Equals(other.ParentInfo)
}

func (g ParentInfo) Equals(other ParentInfo) bool {
	return g.ParentGateway == other.ParentGateway &&
		g.InternalName == other.InternalName &&
		g.OriginalHostname == other.OriginalHostname &&
		g.SectionName == other.SectionName &&
		g.Port == other.Port &&
		g.Protocol == other.Protocol &&
		g.TLSPassthrough == other.TLSPassthrough &&
		slices.EqualFunc(g.AllowedKinds, other.AllowedKinds, func(a, b gwv1.RouteGroupKind) bool {
			return a.Kind == b.Kind && ptr.Equal(a.Group, b.Group)
		}) &&
		slices.Equal(g.Hostnames, other.Hostnames)
}

type GatewayCollectionConfig struct {
	ControllerName string
	Gateways       krt.Collection[*gwv1.Gateway]
	ListenerSets   krt.Collection[ListenerSet]
	GatewayClasses krt.Collection[GatewayClass]
	Namespaces     krt.Collection[*corev1.Namespace]
	Grants         ReferenceGrants
	Secrets        krt.Collection[*corev1.Secret]
	ConfigMaps     krt.Collection[*corev1.ConfigMap]
	KrtOpts        krtutil.KrtOptions

	listenerIndex      krt.Index[types.NamespacedName, ListenerSet]
	transformationFunc GatewayTransformationFunction
}

// GatewayCollection returns a collection of the internal representations GatewayListeners for the given gateway.
func GatewayCollection(
	cfg GatewayCollectionConfig,
	opts ...GatewayCollectionConfigOption,
) (
	krt.StatusCollection[*gwv1.Gateway, gwv1.GatewayStatus],
	krt.Collection[*GatewayListener],
) {
	processGatewayCollectionOptions(&cfg, opts...)
	statusCol, gw := krt.NewStatusManyCollection(cfg.Gateways, cfg.transformationFunc(cfg), cfg.KrtOpts.ToOptions("KubernetesGateway")...)
	return statusCol, gw
}

func GatewayTransformationFunc(cfg GatewayCollectionConfig) func(ctx krt.HandlerContext, obj *gwv1.Gateway) (*gwv1.GatewayStatus, []*GatewayListener) {
	return func(ctx krt.HandlerContext, obj *gwv1.Gateway) (*gwv1.GatewayStatus, []*GatewayListener) {
		class := krt.FetchOne(ctx, cfg.GatewayClasses, krt.FilterKey(string(obj.Spec.GatewayClassName)))
		if class == nil {
			logger.Debug("gateway class not found, skipping", "gw_name", obj.GetName(), "gatewayClassName", obj.Spec.GatewayClassName)
			return nil, nil
		}
		if string(class.Controller) != cfg.ControllerName {
			logger.Debug("skipping gateway not managed by our controller", "gw_name", obj.GetName(), "gatewayClassName", obj.Spec.GatewayClassName, "controllerName", class.Controller)
			return nil, nil // ignore gateways not managed by our controller
		}
		rm := reports.NewReportMap()
		statusReporter := reports.NewReporter(&rm)
		gwReporter := statusReporter.Gateway(obj)
		logger.Debug("translating Gateway", "gw_name", obj.GetName(), "resource_version", obj.GetResourceVersion())

		var result []*GatewayListener
		kgw := obj.Spec
		status := obj.Status.DeepCopy()

		// Extract the addresses. A gwv1 will bind to a specific Service
		gatewayServices, err := ExtractGatewayServices(obj)
		if len(gatewayServices) == 0 && err != nil {
			// Short circuit if it's a hard failure
			logger.Error("failed to translate gwv1", "name", obj.GetName(), "namespace", obj.GetNamespace(), "err", err.Message)
			gwReporter.SetCondition(reporter.GatewayCondition{
				Type:    gwv1.GatewayConditionAccepted,
				Status:  metav1.ConditionFalse,
				Reason:  gwv1.GatewayReasonInvalid,
				Message: err.Message,
			})
			return rm.BuildGWStatus(context.Background(), *obj, nil), nil
		}

		for i, l := range kgw.Listeners {
			// Attached Routes count starts at 0 and gets updated later in the status syncer
			// when the real count is available after route processing

			hostnames, tlsInfo, updatedStatus, programmed := BuildListener(ctx, cfg.Secrets, cfg.ConfigMaps, cfg.Grants, cfg.Namespaces, obj, status.Listeners, kgw, l, i, nil)
			status.Listeners = updatedStatus

			lstatus := status.Listeners[i]

			// Generate supported kinds for the listener
			allowed, _ := GenerateSupportedKinds(l)

			// Set all listener conditions from the actual status
			for _, lcond := range lstatus.Conditions {
				gwReporter.Listener(&l).SetCondition(reporter.ListenerCondition{
					Type:    gwv1.ListenerConditionType(lcond.Type),
					Status:  lcond.Status,
					Reason:  gwv1.ListenerConditionReason(lcond.Reason),
					Message: lcond.Message,
				})
			}

			// Set supported kinds for the listener
			gwReporter.Listener(&l).SetSupportedKinds(allowed)

			name := utils.InternalGatewayName(obj.Namespace, obj.Name, string(l.Name))
			pri := ParentInfo{
				ParentGateway:          config.NamespacedName(obj),
				ParentGatewayClassName: string(obj.Spec.GatewayClassName),
				InternalName:           utils.InternalGatewayName(obj.Namespace, name, ""),
				AllowedKinds:           allowed,
				Hostnames:              hostnames,
				OriginalHostname:       string(ptr.OrEmpty(l.Hostname)),
				SectionName:            l.Name,
				Port:                   l.Port,
				Protocol:               l.Protocol,
				TLSPassthrough:         l.TLS != nil && l.TLS.Mode != nil && *l.TLS.Mode == gwv1.TLSModePassthrough,
			}

			res := &GatewayListener{
				Name:          name,
				Valid:         programmed,
				TLSInfo:       tlsInfo,
				ParentGateway: config.NamespacedName(obj),
				ParentObject: ParentKey{
					Kind:      wellknown.GatewayGVK,
					Name:      obj.Name,
					Namespace: obj.Namespace,
				},
				ParentInfo: pri,
			}
			gwReporter.SetCondition(reporter.GatewayCondition{
				Type:   gwv1.GatewayConditionAccepted,
				Status: metav1.ConditionTrue,
				Reason: gwv1.GatewayReasonAccepted,
			})
			result = append(result, res)
		}
		listenersFromSets := krt.Fetch(ctx, cfg.ListenerSets, krt.FilterIndex(cfg.listenerIndex, config.NamespacedName(obj)))
		for _, ls := range listenersFromSets {
			result = append(result, &GatewayListener{
				Name:          ls.Name,
				ParentGateway: config.NamespacedName(obj),
				ParentObject: ParentKey{
					Kind:      wellknown.XListenerSetGVK,
					Name:      ls.Parent.Name,
					Namespace: ls.Parent.Namespace,
				},
				TLSInfo:    ls.TLSInfo,
				ParentInfo: ls.ParentInfo,
				Valid:      ls.Valid,
			})
		}
		gws := rm.BuildGWStatus(context.Background(), *obj, nil)
		return gws, result
	}
}

type ListenerSet struct {
	Name string `json:"name"`
	// +krtEqualsTodo include parent gateway identity in equality check
	Parent types.NamespacedName `json:"parent"`
	// +krtEqualsTodo ensure parent metadata differences trigger equality
	ParentInfo    ParentInfo           `json:"parentInfo"`
	TLSInfo       *TLSInfo             `json:"tlsInfo"`
	GatewayParent types.NamespacedName `json:"gatewayParent"`
	Valid         bool                 `json:"valid"`
}

func (g ListenerSet) ResourceName() string {
	return g.Name
}

func (g ListenerSet) Equals(other ListenerSet) bool {
	if (g.TLSInfo != nil) != (other.TLSInfo != nil) {
		return false
	}
	if g.TLSInfo != nil {
		if !bytes.Equal(g.TLSInfo.Cert, other.TLSInfo.Cert) && !bytes.Equal(g.TLSInfo.Key, other.TLSInfo.Key) {
			return false
		}
	}
	return g.Name == other.Name &&
		g.GatewayParent == other.GatewayParent &&
		g.Valid == other.Valid
}

func ListenerSetCollection(
	controllerName string,
	listenerSets krt.Collection[*gatewayx.XListenerSet],
	gateways krt.Collection[*gwv1.Gateway],
	gatewayClasses krt.Collection[GatewayClass],
	namespaces krt.Collection[*corev1.Namespace],
	grants ReferenceGrants,
	secrets krt.Collection[*corev1.Secret],
	configMaps krt.Collection[*corev1.ConfigMap],
	krtopts krtutil.KrtOptions,
) (
	krt.StatusCollection[*gatewayx.XListenerSet, gatewayx.ListenerSetStatus],
	krt.Collection[ListenerSet],
) {
	return krt.NewStatusManyCollection(listenerSets,
		func(ctx krt.HandlerContext, obj *gatewayx.XListenerSet) (*gatewayx.ListenerSetStatus, []ListenerSet) {
			result := []ListenerSet{}
			ls := obj.Spec
			status := obj.Status.DeepCopy()

			p := ls.ParentRef
			if NormalizeReference(p.Group, p.Kind, wellknown.GatewayGVK) != wellknown.GatewayGVK {
				// Cannot report status since we don't know if it is for us
				return nil, nil
			}

			pns := ptr.OrDefault(p.Namespace, gatewayx.Namespace(obj.Namespace))
			parentGwObj := ptr.Flatten(krt.FetchOne(ctx, gateways, krt.FilterKey(string(pns)+"/"+string(p.Name))))
			if parentGwObj == nil {
				// Cannot report status since we don't know if it is for us
				return nil, nil
			}
			class := krt.FetchOne(ctx, gatewayClasses, krt.FilterKey(string(parentGwObj.Spec.GatewayClassName)))
			if class == nil {
				logger.Debug("gateway class not found, skipping", "gw_name", obj.GetName(), "gatewayClassName", parentGwObj.Spec.GatewayClassName)
				return nil, nil
			}
			if string(class.Controller) != controllerName {
				logger.Debug("skipping gateway not managed by our controller", "gw_name", obj.GetName(), "gatewayClassName", parentGwObj.Spec.GatewayClassName, "controllerName", class.Controller)
				return nil, nil // ignore gateways not managed by our controller
			}

			controllerName := class.Controller

			if !NamespaceAcceptedByAllowListeners(obj.Namespace, parentGwObj, func(s string) *corev1.Namespace {
				return ptr.Flatten(krt.FetchOne(ctx, namespaces, krt.FilterKey(s)))
			}) {
				//reportNotAllowedListenerSet(status, obj)
				return status, nil
			}

			//gatewayServices, err := extractGatewayServices(domainSuffix, parentGwObj, classInfo)
			//if len(gatewayServices) == 0 && err != nil {
			//	// Short circuit if it's a hard failure
			//	reportListenerSetStatus(context, parentGwObj, obj, status, gatewayServices, nil, err)
			//	return status, nil
			//}

			for i, l := range ls.Listeners {
				port, portErr := kubeutils.DetectListenerPortNumber(l.Protocol, l.Port)
				l.Port = port
				standardListener := convertListenerSetToListener(l)
				originalStatus := slices.Map(status.Listeners, convertListenerSetStatusToStandardStatus)
				hostnames, tlsInfo, updatedStatus, programmed := BuildListener(ctx, secrets, configMaps, grants, namespaces,
					obj, originalStatus, parentGwObj.Spec, standardListener, i, portErr)
				status.Listeners = slices.Map(updatedStatus, convertStandardStatusToListenerSetStatus(l))

				if controllerName == constants.ManagedGatewayMeshController || controllerName == constants.ManagedGatewayEastWestController {
					// Waypoint doesn't actually convert the routes to VirtualServices
					continue
				}
				name := utils.InternalGatewayName(obj.Namespace, obj.Name, string(l.Name))

				allowed, _ := GenerateSupportedKinds(standardListener)
				pri := ParentInfo{
					ParentGateway:    config.NamespacedName(parentGwObj),
					InternalName:     obj.Namespace + "/" + name,
					AllowedKinds:     allowed,
					Hostnames:        hostnames,
					OriginalHostname: string(ptr.OrEmpty(l.Hostname)),
					SectionName:      l.Name,
					Port:             l.Port,
					Protocol:         l.Protocol,
					TLSPassthrough:   l.TLS != nil && l.TLS.Mode != nil && *l.TLS.Mode == gwv1.TLSModePassthrough,
				}

				res := ListenerSet{
					Name:          name,
					Valid:         programmed,
					TLSInfo:       tlsInfo,
					Parent:        config.NamespacedName(obj),
					GatewayParent: config.NamespacedName(parentGwObj),
					ParentInfo:    pri,
				}
				result = append(result, res)
			}

			reportListenerSetStatus(obj, status)
			return status, result
		}, krtopts.ToOptions("ListenerSets")...)
}

// RouteParents holds information about things Routes can reference as parents.
type RouteParents struct {
	Gateways     krt.Collection[*GatewayListener]
	GatewayIndex krt.Index[ParentKey, *GatewayListener]
}

// Fetch returns the parents for a given parent key.
func (p RouteParents) Fetch(ctx krt.HandlerContext, pk ParentKey) []*ParentInfo {
	return slices.Map(krt.Fetch(ctx, p.Gateways, krt.FilterIndex(p.GatewayIndex, pk)), func(gw *GatewayListener) *ParentInfo {
		return &gw.ParentInfo
	})
}

// BuildRouteParents builds a RouteParents from a collection of gateways.
func BuildRouteParents(
	gateways krt.Collection[*GatewayListener],
) RouteParents {
	idx := krt.NewIndex(gateways, "Parent", func(o *GatewayListener) []ParentKey {
		return []ParentKey{o.ParentObject}
	})
	return RouteParents{
		Gateways:     gateways,
		GatewayIndex: idx,
	}
}

// NamespaceAcceptedByAllowListeners determines a list of allowed namespaces for a given AllowedListener
func NamespaceAcceptedByAllowListeners(localNamespace string, parent *gwv1.Gateway, lookupNamespace func(string) *corev1.Namespace) bool {
	lr := parent.Spec.AllowedListeners
	// Default allows none
	if lr == nil || lr.Namespaces == nil {
		return false
	}
	n := *lr.Namespaces
	if n.From != nil {
		switch *n.From {
		case gwv1.NamespacesFromAll:
			return true
		case gwv1.NamespacesFromSame:
			return localNamespace == parent.Namespace
		case gwv1.NamespacesFromNone:
			return false
		case gwv1.NamespacesFromSelector:
			// handled below
		default:
			// Unknown?
			return false
		}
	}
	if lr.Namespaces.Selector == nil {
		// Should never happen, invalid config
		return false
	}
	ls, err := metav1.LabelSelectorAsSelector(lr.Namespaces.Selector)
	if err != nil {
		return false
	}
	localNamespaceObject := lookupNamespace(localNamespace)
	if localNamespaceObject == nil {
		// Couldn't find the namespace
		return false
	}
	return ls.Matches(toNamespaceSet(localNamespaceObject.Name, localNamespaceObject.Labels))
}

func convertListenerSetToListener(l gatewayx.ListenerEntry) gwv1.Listener {
	// For now, structs are identical enough Go can cast them. I doubt this will hold up forever, but we can adjust as needed.
	return gwv1.Listener(l)
}

func convertStandardStatusToListenerSetStatus(l gatewayx.ListenerEntry) func(e gwv1.ListenerStatus) gatewayx.ListenerEntryStatus {
	return func(e gwv1.ListenerStatus) gatewayx.ListenerEntryStatus {
		return gatewayx.ListenerEntryStatus{
			Name:           e.Name,
			Port:           l.Port,
			SupportedKinds: e.SupportedKinds,
			AttachedRoutes: e.AttachedRoutes,
			Conditions:     e.Conditions,
		}
	}
}

func convertListenerSetStatusToStandardStatus(e gatewayx.ListenerEntryStatus) gwv1.ListenerStatus {
	return gwv1.ListenerStatus{
		Name:           e.Name,
		SupportedKinds: e.SupportedKinds,
		AttachedRoutes: e.AttachedRoutes,
		Conditions:     e.Conditions,
	}
}

func reportListenerSetStatus(
	obj *gatewayx.XListenerSet,
	gs *gatewayx.ListenerSetStatus,
) {
	//internal, _, _, _, warnings, allUsable := r.ResolveGatewayInstances(parentGwObj.Namespace, gatewayServices, servers)

	// Setup initial conditions to the success state. If we encounter errors, we will update this.
	// We have two status
	// Accepted: is the configuration valid. We only have errors in listeners, and the status is not supposed to
	// be tied to listeners, so this is always accepted
	// Programmed: is the data plane "ready" (note: eventually consistent)
	gatewayConditions := map[string]*Condition{
		string(gwv1.GatewayConditionAccepted): {
			Reason:  string(gwv1.GatewayReasonAccepted),
			Message: "Resource accepted",
		},
		string(gwv1.GatewayConditionProgrammed): {
			Reason:  string(gwv1.GatewayReasonProgrammed),
			Message: "Resource programmed",
		},
	}

	//setProgrammedCondition(gatewayConditions, internal, gatewayServices, warnings, allUsable)

	gs.Conditions = SetConditions(obj.Generation, gs.Conditions, gatewayConditions)
}
