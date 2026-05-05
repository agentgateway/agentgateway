package syncer

import (
	"context"
	"fmt"
	"strconv"
	"sync/atomic"

	securityclient "istio.io/client-go/pkg/apis/security/v1"
	"istio.io/istio/pilot/pkg/model"
	"istio.io/istio/pilot/pkg/serviceregistry/ambient"
	"istio.io/istio/pkg/cluster"
	"istio.io/istio/pkg/config/mesh"
	"istio.io/istio/pkg/config/protocol"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/util/sets"
	"istio.io/istio/pkg/workloadapi"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/tools/cache"
	"sigs.k8s.io/controller-runtime/pkg/manager"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	agwir "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer/krtxds"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer/nack"
	"github.com/agentgateway/agentgateway/controller/pkg/syncer/status"
	krtpkg "github.com/agentgateway/agentgateway/controller/pkg/utils/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

var (
	logger                                = logging.New("agentgateway/syncer")
	_      manager.LeaderElectionRunnable = &Syncer{}
)

// Syncer synchronizes Kubernetes Gateway API resources with xDS for agentgateway proxies.
// It watches Gateway resources with the agentgateway class and translates them to agentgateway configuration.
type Syncer struct {
	// Core collections and dependencies
	agwCollections *plugins.AgwCollections
	client         apiclient.Client
	agwPlugins     plugins.AgwPlugin

	// Configuration
	controllerName           string
	additionalGatewayClasses map[string]*deployer.GatewayClassInfo

	// Status reporting
	statusCollections *status.StatusCollections

	// Synchronization
	waitForSync []cache.InformerSynced
	ready       atomic.Bool

	// NACK handling
	NackPublisher *nack.Publisher

	// features
	Registrations []krtxds.Registration

	Outputs OutputCollections

	gatewayCollectionOptions []translator.GatewayCollectionConfigOption

	customResourceCollections   func(cfg CustomResourceCollectionsConfig)
	buildAddressCollectionsFunc AgentgatewayAddressBuilderFunc
	buildReferenceTypesFunc     func(agw *plugins.AgwCollections, base plugins.ReferenceTypes) plugins.ReferenceTypes
}

func NewAgwSyncer(
	controllerName string,
	client apiclient.Client,
	agwCollections *plugins.AgwCollections,
	agwPlugins plugins.AgwPlugin,
	additionalGatewayClasses map[string]*deployer.GatewayClassInfo,
	krtopts krtutil.KrtOptions,
	extraGVKs []schema.GroupVersionKind,
	opts ...AgentgatewaySyncerOption,
) *Syncer {
	cfg := processAgentgatewaySyncerOptions(opts...)
	syncer := &Syncer{
		agwCollections:           agwCollections,
		controllerName:           controllerName,
		agwPlugins:               agwPlugins,
		additionalGatewayClasses: additionalGatewayClasses,
		client:                   client,
		statusCollections:        status.NewStatusCollections(extraGVKs),
		NackPublisher:            nack.NewPublisher(client),
		gatewayCollectionOptions: []translator.GatewayCollectionConfigOption{
			translator.WithGatewayTransformationFunc(cfg.GatewayTransformationFunc),
		},
		customResourceCollections:   cfg.CustomResourceCollections,
		buildAddressCollectionsFunc: cfg.BuildAddressCollectionsFunc,
		buildReferenceTypesFunc:     cfg.BuildReferenceTypesFunc,
	}
	logger.Debug("init agentgateway Syncer", "controllername", controllerName)

	syncer.buildResourceCollections(krtopts.WithPrefix("agentgateway"))
	return syncer
}

func (s *Syncer) StatusCollections() *status.StatusCollections {
	return s.statusCollections
}

type OutputCollections struct {
	Resources  krt.Collection[agwir.AgwResource]
	Addresses  krt.Collection[Address]
	References plugins.ReferenceIndex
}

type CustomResourceCollectionsConfig struct {
	ControllerName    string
	Gateways          krt.Collection[*gwv1.Gateway]
	ListenerSets      krt.Collection[translator.ListenerSet]
	GatewayClasses    krt.Collection[translator.GatewayClass]
	Namespaces        krt.Collection[*corev1.Namespace]
	Grants            translator.ReferenceGrants
	Secrets           krt.Collection[*corev1.Secret]
	ConfigMaps        krt.Collection[*corev1.ConfigMap]
	KrtOpts           krtutil.KrtOptions
	StatusCollections *status.StatusCollections
}

func (s *Syncer) buildResourceCollections(krtopts krtutil.KrtOptions) {
	// Build core collections for irs
	gatewayClasses := translator.GatewayClassesCollection(s.agwCollections.GatewayClasses, krtopts)
	refGrants := translator.BuildReferenceGrants(translator.ReferenceGrantsCollection(s.agwCollections.ReferenceGrants, krtopts))
	listenerSetInitialStatus, listenerSets := s.buildListenerSetCollection(gatewayClasses, refGrants, krtopts)
	if s.customResourceCollections != nil {
		s.customResourceCollections(CustomResourceCollectionsConfig{
			ControllerName:    s.controllerName,
			Gateways:          s.agwCollections.Gateways,
			ListenerSets:      listenerSets,
			GatewayClasses:    gatewayClasses,
			Namespaces:        s.agwCollections.Namespaces,
			Grants:            refGrants,
			Secrets:           s.agwCollections.Secrets,
			ConfigMaps:        s.agwCollections.ConfigMaps,
			KrtOpts:           krtopts,
			StatusCollections: s.statusCollections,
		})
	}

	gatewayInitialStatus, gateways := s.buildGatewayCollection(gatewayClasses, listenerSets, refGrants, krtopts)

	// Build Agw resources for gateway
	agwResources, routeAttachments, ancestorCollection := s.buildAgwResources(gateways, listenerSets, refGrants, krtopts)

	gatewayFinalStatus := s.buildFinalGatewayStatus(gatewayInitialStatus, routeAttachments, krtopts)
	status.RegisterStatus(s.statusCollections, gatewayFinalStatus, translator.GetStatus)

	// Register plugin-provided gateway statuses. These statuses are scoped to a
	// specific gatewayclass as we already filter out those Gateways in
	// buildAgwResources and won't conflict with status written by the non-plugin
	// one above.
	if s.agwPlugins.AddResourceExtension != nil && s.agwPlugins.AddResourceExtension.GatewayStatuses != nil {
		pluginGwFinalStatus := s.buildFinalGatewayStatus(s.agwPlugins.AddResourceExtension.GatewayStatuses, routeAttachments, krtopts)
		status.RegisterStatus(s.statusCollections, pluginGwFinalStatus, translator.GetStatus)
	}

	listenerSetFinalStatus := s.buildFinalListenerSetStatus(gateways, listenerSetInitialStatus, routeAttachments, krtopts)
	status.RegisterStatus(s.statusCollections, listenerSetFinalStatus, translator.GetStatus)

	// Build address collections
	addressBuilder := s.buildAddressCollectionsFunc
	if addressBuilder == nil {
		addressBuilder = defaultBuildAddressCollections
	}
	addresses, hasSynced := addressBuilder(s.agwCollections, krtopts)

	// Build XDS collection
	s.buildXDSCollection(agwResources, addresses, krtopts)

	// Set up sync dependencies
	s.setupSyncDependencies(agwResources, addresses, hasSynced)

	s.Outputs.Resources = agwResources
	s.Outputs.Addresses = addresses
	s.Outputs.References = ancestorCollection
}

func (s *Syncer) buildFinalGatewayStatus(
	gatewayStatuses krt.StatusCollection[*gwv1.Gateway, gwv1.GatewayStatus],
	routeAttachments krt.Collection[*plugins.RouteAttachment],
	krtopts krtutil.KrtOptions,
) krt.StatusCollection[*gwv1.Gateway, gwv1.GatewayStatus] {
	routeAttachmentsIndex := krt.NewIndex(routeAttachments, "to", func(o *plugins.RouteAttachment) []utils.TypedNamespacedName {
		return []utils.TypedNamespacedName{o.To}
	})
	return krt.NewCollection(
		gatewayStatuses,
		func(ctx krt.HandlerContext, i krt.ObjectWithStatus[*gwv1.Gateway, gwv1.GatewayStatus]) *krt.ObjectWithStatus[*gwv1.Gateway, gwv1.GatewayStatus] {
			routes := krt.Fetch(ctx, routeAttachments, krt.FilterIndex(routeAttachmentsIndex, utils.TypedNamespacedName{
				Kind: wellknown.GatewayGVK.Kind,
				NamespacedName: types.NamespacedName{
					Namespace: i.Obj.Namespace,
					Name:      i.Obj.Name,
				},
			}))
			counts := map[string]int32{}
			for _, r := range routes {
				counts[r.ListenerName]++
			}
			status := i.Status.DeepCopy()
			for i, s := range status.Listeners {
				s.AttachedRoutes = counts[string(s.Name)]
				status.Listeners[i] = s
			}
			return &krt.ObjectWithStatus[*gwv1.Gateway, gwv1.GatewayStatus]{
				Obj:    i.Obj,
				Status: *status,
			}
		}, krtopts.ToOptions("GatewayFinalStatus")...)
}

func (s *Syncer) buildFinalListenerSetStatus(
	gateways krt.Collection[*translator.GatewayListener],
	listenerSetStatus krt.StatusCollection[*gwv1.ListenerSet, gwv1.ListenerSetStatus],
	routeAttachments krt.Collection[*plugins.RouteAttachment],
	krtopts krtutil.KrtOptions,
) krt.StatusCollection[*gwv1.ListenerSet, gwv1.ListenerSetStatus] {
	routeAttachmentsIndex := krt.NewIndex(routeAttachments, "to", func(o *plugins.RouteAttachment) []utils.TypedNamespacedName {
		return []utils.TypedNamespacedName{o.To}
	})

	gatewayIndex := krt.NewIndex(gateways, "gateway-parent-section-name", func(gwl *translator.GatewayListener) []utils.SectionedNamespacedName {
		return []utils.SectionedNamespacedName{{
			NamespacedName: types.NamespacedName{
				Namespace: gwl.ParentObject.Namespace,
				Name:      gwl.ParentObject.Name,
			},
			SectionName: gwl.ParentInfo.SectionName,
		}}
	}).AsCollection(append(krtopts.ToOptions("gatewayIndex"), utils.SectionedNamespacedNameIndexCollectionFunc)...)
	return krt.NewCollection(listenerSetStatus,
		func(ctx krt.HandlerContext, i krt.ObjectWithStatus[*gwv1.ListenerSet, gwv1.ListenerSetStatus]) *krt.ObjectWithStatus[*gwv1.ListenerSet, gwv1.ListenerSetStatus] {
			// Skip if listenerset not allowed
			if len(i.Status.Conditions) == 0 || i.Status.Conditions[0].Reason == string(gwv1.ListenerSetReasonNotAllowed) {
				return &i
			}

			invalidListenerCount := 0
			lsStatus := i.Status.DeepCopy()
			routes := krt.Fetch(ctx, routeAttachments, krt.FilterIndex(routeAttachmentsIndex, utils.TypedNamespacedName{
				Kind: wellknown.ListenerSetGVK.Kind,
				NamespacedName: types.NamespacedName{
					Namespace: i.Obj.Namespace,
					Name:      i.Obj.Name,
				},
			}))
			counts := map[string]int32{}
			for _, r := range routes {
				counts[r.ListenerName]++
			}
			for idx, l := range i.Obj.Spec.Listeners {
				gatewayListeners := krtutil.FetchIndexObjects(ctx, gatewayIndex, utils.SectionedNamespacedName{
					NamespacedName: types.NamespacedName{
						Namespace: i.Obj.Namespace,
						Name:      i.Obj.Name,
					},
					SectionName: l.Name,
				})
				if len(gatewayListeners) == 0 {
					continue
				}

				obj := gatewayListeners[0]
				if !obj.Valid {
					invalidListenerCount++
				} else {
					if obj.Conflict == translator.ListenerConflictHostname {
						invalidListenerCount++
						ListenerMessageHostnameConflict := "Found conflicting hostnames on listeners, all listeners on a single port must have unique hostnames"
						ReportListenerSetListenerConflicts(&lsStatus.Listeners[idx], i.Obj, string(gwv1.ListenerReasonHostnameConflict), ListenerMessageHostnameConflict)
					} else if obj.Conflict == translator.ListenerConflictProtocol {
						invalidListenerCount++
						ListenerMessageProtocolConflict := "Found conflicting protocols on listeners, a single port can only contain listeners with compatible protocols"
						ReportListenerSetListenerConflicts(&lsStatus.Listeners[idx], i.Obj, string(gwv1.ListenerReasonProtocolConflict), ListenerMessageProtocolConflict)
					}
				}
				lsStatus.Listeners[idx].AttachedRoutes = counts[string(l.Name)]
			}

			if invalidListenerCount > 0 {
				listenerSetAccepted := invalidListenerCount < len(i.Obj.Spec.Listeners)
				ReportListenerSetWithConflicts(lsStatus, i.Obj, listenerSetAccepted)
			}
			return &krt.ObjectWithStatus[*gwv1.ListenerSet, gwv1.ListenerSetStatus]{
				Obj:    i.Obj,
				Status: *lsStatus,
			}
		}, krtopts.ToOptions("ListenerSetFinalStatus")...)
}

func ReportListenerSetWithConflicts(status *gwv1.ListenerSetStatus, obj *gwv1.ListenerSet, accepted bool) {
	condition := metav1.ConditionFalse
	if accepted {
		condition = metav1.ConditionTrue
	}
	programmedReason := gwv1.ListenerSetReasonListenersNotValid
	if accepted {
		programmedReason = gwv1.ListenerSetReasonProgrammed
	}
	// In case any listeners are invalid, this status should be set even if the gateway / listenerset is accepted
	// https://github.com/kubernetes-sigs/gateway-api/blob/8fe8316f5792a7830a49c800f89fe689e0df042e/apisx/v1alpha1/xlistenerset_types.go#L396
	gatewayConditions := map[string]*translator.Condition{
		string(gwv1.GatewayConditionAccepted): {
			Status: condition,
			Reason: string(gwv1.ListenerSetReasonListenersNotValid),
		},
		string(gwv1.GatewayConditionProgrammed): {
			Status: condition,
			Reason: string(programmedReason),
		},
	}

	status.Conditions = translator.SetConditions(obj.Generation, status.Conditions, gatewayConditions)
}

func ReportListenerSetListenerConflicts(status *gwv1.ListenerEntryStatus, obj *gwv1.ListenerSet, reason string, message string) {
	gatewayConditions := map[string]*translator.Condition{
		string(gwv1.ListenerConditionConflicted): {
			Status:  metav1.ConditionTrue,
			Reason:  reason,
			Message: message,
		},
		string(gwv1.GatewayConditionAccepted): {
			Status:  metav1.ConditionFalse,
			Reason:  reason,
			Message: message,
		},
		string(gwv1.GatewayConditionProgrammed): {
			Status:  metav1.ConditionFalse,
			Reason:  reason,
			Message: message,
		},
	}

	status.Conditions = translator.SetConditions(obj.Generation, status.Conditions, gatewayConditions)
}

func (s *Syncer) buildGatewayCollection(
	gatewayClasses krt.Collection[translator.GatewayClass],
	listenerSets krt.Collection[translator.ListenerSet],
	refGrants translator.ReferenceGrants,
	krtopts krtutil.KrtOptions,
) (
	krt.StatusCollection[*gwv1.Gateway, gwv1.GatewayStatus],
	krt.Collection[*translator.GatewayListener],
) {
	return translator.GatewayCollection(translator.GatewayCollectionConfig{
		ControllerName: s.controllerName,
		Gateways:       s.agwCollections.Gateways,
		ListenerSets:   listenerSets,
		GatewayClasses: gatewayClasses,
		Namespaces:     s.agwCollections.Namespaces,
		Grants:         refGrants,
		Secrets:        s.agwCollections.Secrets,
		ConfigMaps:     s.agwCollections.ConfigMaps,
		KrtOpts:        krtopts,
	}, s.gatewayCollectionOptions...)
}

func (s *Syncer) buildListenerSetCollection(
	gatewayClasses krt.Collection[translator.GatewayClass],
	refGrants translator.ReferenceGrants,
	krtopts krtutil.KrtOptions,
) (
	krt.StatusCollection[*gwv1.ListenerSet, gwv1.ListenerSetStatus],
	krt.Collection[translator.ListenerSet],
) {
	return krt.NewStatusManyCollection(s.agwCollections.ListenerSets,
		func(ctx krt.HandlerContext, obj *gwv1.ListenerSet) (*gwv1.ListenerSetStatus, []translator.ListenerSet) {
			return translator.ListenerSetBuilder(
				ctx, obj,
				s.controllerName,
				s.agwCollections.Gateways,
				gatewayClasses,
				s.agwCollections.Namespaces,
				refGrants,
				s.agwCollections.Secrets,
				s.agwCollections.ConfigMaps,
			)
		}, krtopts.ToOptions("ListenerSets")...)
}

func (s *Syncer) buildAgwResources(
	gateways krt.Collection[*translator.GatewayListener],
	listenerSets krt.Collection[translator.ListenerSet],
	refGrants translator.ReferenceGrants,
	krtopts krtutil.KrtOptions,
) (krt.Collection[agwir.AgwResource], krt.Collection[*plugins.RouteAttachment], plugins.ReferenceIndex) {
	// filter gateway collections to only include gateways which use a built-in gateway class
	// (resources for additional gateway classes should be created by the downstream providing them)
	filteredGateways := krt.NewCollection(gateways, func(ctx krt.HandlerContext, gw *translator.GatewayListener) **translator.GatewayListener {
		if _, isAdditionalClass := s.additionalGatewayClasses[gw.ParentInfo.ParentGatewayClassName]; isAdditionalClass {
			return nil
		}
		return &gw
	}, krtopts.ToOptions("FilteredGateways")...)

	// Build ports and binds
	ports := krtpkg.UnnamedIndex(filteredGateways, func(l *translator.GatewayListener) []string {
		return []string{fmt.Sprint(l.ParentInfo.Port)}
	}).AsCollection(krtopts.ToOptions("PortBindings")...)

	binds := krt.NewManyCollection(ports, func(ctx krt.HandlerContext, object krt.IndexObject[string, *translator.GatewayListener]) []agwir.AgwResource {
		port, _ := strconv.Atoi(object.Key)
		uniq := sets.New[types.NamespacedName]()
		protocol := api.Bind_Protocol(0)
		var tunnelProtocol = api.Bind_DIRECT
		for _, gw := range object.Objects {
			uniq.Insert(types.NamespacedName{
				Namespace: gw.ParentGateway.Namespace,
				Name:      gw.ParentGateway.Name,
			})
			// TODO: better handle conflicts of protocols. For now, we arbitrarily treat TLS > plain
			if gw.Conflict == "" {
				protocol = max(protocol, s.getBindProtocol(gw))
				if tp := s.getTunnelProtocol(gw); tp != api.Bind_DIRECT {
					tunnelProtocol = tp
				}
			}
		}
		return slices.Map(uniq.UnsortedList(), func(e types.NamespacedName) agwir.AgwResource {
			bind := translator.AgwBind{
				Bind: &api.Bind{
					Key:            object.Key + "/" + e.String(),
					Port:           uint32(port), //nolint:gosec // G115: port is always in valid port range
					Protocol:       protocol,
					TunnelProtocol: tunnelProtocol,
				},
			}
			return translator.ToResourceForGateway(e, bind)
		})
	}, krtopts.ToOptions("Binds")...)
	if s.agwPlugins.AddResourceExtension != nil && s.agwPlugins.AddResourceExtension.Binds != nil {
		binds = krt.JoinCollection([]krt.Collection[agwir.AgwResource]{binds, s.agwPlugins.AddResourceExtension.Binds})
	}

	// Build listeners
	listeners := krt.NewCollection(filteredGateways, func(ctx krt.HandlerContext, obj *translator.GatewayListener) *agwir.AgwResource {
		return s.buildListenerFromGateway(obj)
	}, krtopts.ToOptions("Listeners")...)
	if s.agwPlugins.AddResourceExtension != nil && s.agwPlugins.AddResourceExtension.Listeners != nil {
		listeners = krt.JoinCollection([]krt.Collection[agwir.AgwResource]{listeners, s.agwPlugins.AddResourceExtension.Listeners})
	}

	// Build routes
	var routeParents translator.ParentResolver = translator.BuildRouteParents(filteredGateways)

	// Compose with plugin-provided parent resolvers.
	if ext := s.agwPlugins.AddResourceExtension; ext != nil && len(ext.ParentResolvers) > 0 {
		resolvers := []translator.ParentResolver{routeParents}
		for _, r := range ext.ParentResolvers {
			if r != nil {
				resolvers = append(resolvers, r)
			}
		}
		routeParents = &translator.CompositeParentResolver{Resolvers: resolvers}
	}

	referenceTypes := plugins.DefaultReferenceTypes(s.agwCollections)
	if s.buildReferenceTypesFunc != nil {
		referenceTypes = s.buildReferenceTypesFunc(s.agwCollections, referenceTypes)
	}

	routeInputs := translator.RouteContextInputs{
		Grants:         refGrants,
		RouteParents:   routeParents,
		ControllerName: s.controllerName,
		Services:       s.agwCollections.Services,
		Namespaces:     s.agwCollections.Namespaces,
		ServiceEntries: s.agwCollections.ServiceEntries,
		InferencePools: s.agwCollections.InferencePools,
		Backends:       s.agwCollections.Backends,
		References:     referenceTypes,
	}

	agwRoutes, routeAttachments, ancestorBackends := translator.AgwRouteCollection(s.statusCollections, s.agwCollections.HTTPRoutes, s.agwCollections.GRPCRoutes, s.agwCollections.TCPRoutes, s.agwCollections.TLSRoutes, routeInputs, krtopts)
	if s.agwPlugins.AddResourceExtension != nil {
		if s.agwPlugins.AddResourceExtension.Routes != nil {
			agwRoutes = krt.JoinCollection([]krt.Collection[agwir.AgwResource]{agwRoutes, s.agwPlugins.AddResourceExtension.Routes})
		}
		if s.agwPlugins.AddResourceExtension.AncestorBackends != nil {
			ancestorBackends = krt.JoinCollection([]krt.Collection[*utils.AncestorBackend]{ancestorBackends, s.agwPlugins.AddResourceExtension.AncestorBackends})
		}
	}
	routeAttachmentsIndex := krt.NewIndex(routeAttachments, "from", func(o *plugins.RouteAttachment) []utils.TypedNamespacedName {
		return []utils.TypedNamespacedName{o.From}
	}).AsCollection(append(krtopts.ToOptions("RouteAttachments"), utils.TypedNamespacedNameIndexCollectionFunc)...)

	ancestorsIndex := krt.NewIndex(ancestorBackends, "ancestors", func(o *utils.AncestorBackend) []utils.TypedNamespacedName {
		return []utils.TypedNamespacedName{o.Backend}
	})
	ancestorCollection := ancestorsIndex.AsCollection(append(krtopts.ToOptions("AncestorBackend"), utils.TypedNamespacedNameIndexCollectionFunc)...)

	// Build a per-ListenerSet → parent-Gateway attachment index.
	// Each entry maps the ListenerSet identity to its parent Gateway so that
	// LookupGatewaysForTarget can route ListenerSet-targeted policies to the
	// correct xDS snapshot.
	listenerSetAttachments := krt.NewManyCollection(listenerSets,
		func(ctx krt.HandlerContext, ls translator.ListenerSet) []*plugins.RouteAttachment {
			if !ls.Valid {
				return nil
			}
			lsKey := utils.TypedNamespacedName{
				Kind:           wellknown.ListenerSetGVK.Kind,
				NamespacedName: ls.Parent,
			}
			return []*plugins.RouteAttachment{{
				From:    lsKey,
				To:      lsKey,
				Gateway: ls.GatewayParent,
				// ListenerName omitted: this index only needs ListenerSet→Gateway;
				// omitting it lets krt deduplicate the N per-listener entries to one.
			}}
		}, krtopts.ToOptions("ListenerSetGatewayAttachments")...)
	listenerSetAttachmentsIdx := krt.NewIndex(listenerSetAttachments, "ls-to-gateway",
		func(o *plugins.RouteAttachment) []utils.TypedNamespacedName {
			return []utils.TypedNamespacedName{o.To}
		}).AsCollection(append(krtopts.ToOptions("ListenerSetGatewayIdx"), utils.TypedNamespacedNameIndexCollectionFunc)...)

	referenceIndex := plugins.BuildReferenceIndex(ancestorCollection, routeAttachmentsIndex, referenceTypes)

	// Phase 1: Collect policy references (e.g. ext_proc backendRefs) BEFORE building
	// policies. This ensures BackendTLSPolicy can look up gateways for backends that
	// are only reachable via PolicyAttachments (like ext_proc processor Services).
	policyReferences := CollectPolicyReferences(s.agwPlugins, referenceIndex, krtopts)
	backendPolicyReferences := AgwBackendReferencesCollection(s.agwPlugins, krtopts)
	joinedPolicyReferences := krt.JoinCollection([]krt.Collection[*plugins.PolicyAttachment]{policyReferences, backendPolicyReferences}, krtopts.ToOptions("JoinPolicyAttachment")...)
	policyReferencesIndex := krt.NewIndex(joinedPolicyReferences, "policyReferences", func(o *plugins.PolicyAttachment) []utils.TypedNamespacedName {
		return []utils.TypedNamespacedName{o.Backend}
	})
	policyReferencesIndexCollection := policyReferencesIndex.AsCollection(append(krtopts.ToOptions("PolicyReferencesIndex"), utils.TypedNamespacedNameIndexCollectionFunc)...)
	referenceIndex = referenceIndex.WithPolicyAttachments(policyReferencesIndexCollection)
	referenceIndex = referenceIndex.WithListenerSetAttachments(listenerSetAttachmentsIdx)

	// Phase 2: Build policies with the fully-populated reference index.
	agwPolicies, policyStatuses := BuildPolicies(s.agwPlugins, referenceIndex, krtopts)
	for _, col := range policyStatuses {
		status.RegisterStatus(s.statusCollections, col, translator.GetStatus)
	}

	// Build the backend collection with backend+route references
	agwBackends, agwBackendStatus := AgwBackendCollection(s.agwPlugins, referenceIndex, krtopts)
	for _, col := range agwBackendStatus {
		status.RegisterStatus(s.statusCollections, col, translator.GetStatus)
	}
	// Join all Agw resources
	allAgwResources := krt.JoinCollection([]krt.Collection[agwir.AgwResource]{binds, listeners, agwRoutes, agwPolicies, agwBackends}, krtopts.ToOptions("Resources")...)

	return allAgwResources, routeAttachments, referenceIndex
}

// buildListenerFromGateway creates a listener resource from a gateway
func (s *Syncer) buildListenerFromGateway(obj *translator.GatewayListener) *agwir.AgwResource {
	var ls *api.ResourceName
	if obj.ParentObject.Kind == wellknown.ListenerSetGVK.Kind {
		ls = &api.ResourceName{Name: obj.ParentObject.Name, Namespace: obj.ParentObject.Namespace}
	}
	listenerName := utils.ListenerName(obj.ParentGateway.Namespace, obj.ParentGateway.Name, string(obj.ParentInfo.SectionName), ls)
	l := &api.Listener{
		Key:      obj.ResourceName(),
		Name:     listenerName,
		BindKey:  fmt.Sprint(obj.ParentInfo.Port) + "/" + obj.ParentGateway.Namespace + "/" + obj.ParentGateway.Name,
		Hostname: obj.ParentInfo.OriginalHostname,
	}

	// Set protocol and TLS configuration
	protocol, tlsConfig, ok := s.getProtocolAndTLSConfig(obj)
	if !ok {
		return nil // Unsupported protocol or missing TLS config
	}

	l.Protocol = protocol
	l.Tls = tlsConfig

	return new(translator.ToResourceForGateway(types.NamespacedName{
		Namespace: obj.ParentGateway.Namespace,
		Name:      obj.ParentGateway.Name,
	}, translator.AgwListener{l}))
}

// getProtocolAndTLSConfig extracts protocol and TLS configuration from a gateway
func (s *Syncer) getProtocolAndTLSConfig(obj *translator.GatewayListener) (api.Protocol, *api.TLSConfig, bool) {
	var tlsConfig *api.TLSConfig

	// Build TLS config if needed
	if obj.TLSInfo != nil {
		tlsConfig = &api.TLSConfig{
			Cert:       obj.TLSInfo.Cert,
			PrivateKey: obj.TLSInfo.Key,
		}
		if obj.TLSInfo.IstioWorkloadCert {
			tlsConfig.CertificateSource = api.TLSConfig_ISTIO_WORKLOAD
		}
		if len(obj.TLSInfo.CaCert) > 0 {
			tlsConfig.Root = obj.TLSInfo.CaCert
		}
		if obj.TLSInfo.IstioMutual {
			tlsConfig.Root = nil
			tlsConfig.MtlsMode = api.TLSConfig_STRICT
		} else if obj.TLSInfo.IstioWorkloadCert {
			tlsConfig.MtlsMode = api.TLSConfig_DISABLE
		} else if obj.TLSInfo.MtlsFallbackEnabled {
			tlsConfig.MtlsMode = api.TLSConfig_ALLOW_INSECURE_FALLBACK
		}
	}

	switch obj.ParentInfo.Protocol {
	case gwv1.HTTPProtocolType:
		return api.Protocol_HTTP, nil, true
	case gwv1.HTTPSProtocolType:
		if tlsConfig == nil {
			return api.Protocol_HTTPS, nil, false // TLS required but not configured
		}
		return api.Protocol_HTTPS, tlsConfig, true
	case gwv1.TLSProtocolType:
		if tlsConfig == nil {
			if obj.ParentInfo.TLSPassthrough {
				// For passthrough, we don't want TLS config
				return api.Protocol_TLS, nil, true
			} else {
				// TLS required but not configured
				return api.Protocol_TLS, nil, false
			}
		}
		return api.Protocol_TLS, tlsConfig, true
	case gwv1.TCPProtocolType:
		return api.Protocol_TCP, nil, true
	case gwv1.ProtocolType(protocol.HBONE):
		return api.Protocol_HBONE, nil, true
	default:
		return api.Protocol_HTTP, nil, false // Unsupported protocol
	}
}

// getProtocolAndTLSConfig extracts protocol and TLS configuration from a gateway
func (s *Syncer) getBindProtocol(obj *translator.GatewayListener) api.Bind_Protocol {
	switch obj.ParentInfo.Protocol {
	case gwv1.HTTPProtocolType:
		return api.Bind_HTTP
	case gwv1.HTTPSProtocolType:
		return api.Bind_TLS
	case gwv1.TLSProtocolType:
		return api.Bind_TLS
	case gwv1.TCPProtocolType:
		return api.Bind_TCP
	case gwv1.ProtocolType(protocol.HBONE):
		// The bind protocol is not used for HBONE_GATEWAY in the data plane;
		// the actual inner protocol is determined at runtime from the other
		// listeners on the same port. Return HTTP as a placeholder.
		return api.Bind_HTTP
	default:
		return api.Bind_HTTP
	}
}

// getTunnelProtocol maps a Gateway listener protocol to its tunnel protocol.
// HBONE listeners use HBONE_GATEWAY mode: the proxy terminates inbound HBONE
// and routes CONNECT requests to local binds.
func (s *Syncer) getTunnelProtocol(obj *translator.GatewayListener) api.Bind_TunnelProtocol {
	switch obj.ParentInfo.Protocol {
	case gwv1.ProtocolType(protocol.HBONE):
		return api.Bind_HBONE_GATEWAY
	default:
		return api.Bind_DIRECT
	}
}

// defaultBuildAddressCollections is the default implementation for building address collections
// using the istio ambient builder. It can be passed via WithBuildAddressCollections to the syncer.
func defaultBuildAddressCollections(cols *plugins.AgwCollections, krtopts krtutil.KrtOptions) (krt.Collection[Address], func() bool) {
	opts := krtopts.ToIstio()
	clusterId := cluster.ID(cols.ClusterID)
	Networks := ambient.BuildNetworkCollections(cols.Namespaces, cols.Gateways, ambient.Options{
		SystemNamespace: cols.IstioNamespace,
		ClusterID:       clusterId,
	}, opts)
	builder := ambient.Builder{
		DomainSuffix: kubeutils.GetClusterDomainName(),
		ClusterID:    clusterId,
		Networks:     Networks,
		Flags: ambient.FeatureFlags{
			EnableK8SServiceSelectWorkloadEntries: true,
			EnableMtlsTransportProtocol:           true,
		},
	}

	meshConfigMapName := GetMeshConfigMapName(cols.IstioRevision)
	meshConfig := krt.NewSingleton(func(ctx krt.HandlerContext) *ambient.MeshConfig {
		cm := krt.FetchOne(ctx, cols.ConfigMaps, krt.FilterObjectName(types.NamespacedName{Namespace: cols.IstioNamespace, Name: meshConfigMapName}))
		if flattened := ptr.Flatten(cm); flattened != nil {
			if mc := ParseMeshConfigFromConfigMap(flattened); mc != nil {
				return &ambient.MeshConfig{MeshConfig: mc}
			}
		}
		return &ambient.MeshConfig{MeshConfig: mesh.DefaultMeshConfig()}
	}, krtopts.ToOptions("IstioMeshConfig")...)

	waypoints := builder.WaypointsCollection(clusterId, cols.Gateways, cols.GatewayClasses, cols.Pods, opts)
	services := builder.ServicesCollection(
		clusterId,
		cols.Services,
		cols.ServiceEntries,
		waypoints,
		cols.Namespaces,
		meshConfig,
		opts,
		true,
	)
	// Istio doesn't include InferencePools, but we need them; add our own after the Istio build
	inferencePoolsInfo := krt.NewCollection(cols.InferencePools, InferencePoolBuilder(),
		krtopts.ToOptions("InferencePools")...)
	services = krt.JoinCollection([]krt.Collection[model.ServiceInfo]{services, inferencePoolsInfo}, krt.WithJoinUnchecked())

	nodeLocality := ambient.NodesCollection(cols.Nodes, opts.WithName("NodeLocality")...)
	workloads := builder.WorkloadsCollection(
		cols.Pods,
		nodeLocality,
		meshConfig,
		// Authz/Authn are not use for agentgateway, ignore
		krt.NewStaticCollection[model.WorkloadAuthorization](nil, nil),
		krt.NewStaticCollection[*securityclient.PeerAuthentication](nil, nil),
		waypoints,
		services,
		cols.WorkloadEntries,
		cols.ServiceEntries,
		cols.EndpointSlices,
		cols.Namespaces,
		opts,
	)

	workloadAddresses := krt.MapCollection(workloads, func(t model.WorkloadInfo) Address {
		return Address{Workload: &t}
	})
	svcAddresses := krt.MapCollection(services, func(t model.ServiceInfo) Address {
		return Address{Service: &t}
	})

	adpAddresses := krt.JoinCollection([]krt.Collection[Address]{svcAddresses, workloadAddresses}, krtopts.ToOptions("Addresses")...)
	return adpAddresses, func() bool { return true }
}

func (s *Syncer) buildXDSCollection(
	agwResources krt.Collection[agwir.AgwResource],
	xdsAddresses krt.Collection[Address],
	krtopts krtutil.KrtOptions,
) {
	// Create an index on adpResources by Gateway to avoid fetching all resources
	agwResourcesByGateway := func(resource agwir.AgwResource) types.NamespacedName {
		return resource.Gateway
	}
	s.Registrations = append(s.Registrations, krtxds.Collection[Address, *workloadapi.Address](xdsAddresses, krtopts))
	s.Registrations = append(s.Registrations, krtxds.PerGatewayCollection[agwir.AgwResource, *api.Resource](agwResources, agwResourcesByGateway, krtopts))
}

func (s *Syncer) setupSyncDependencies(
	agwResources krt.Collection[agwir.AgwResource],
	addresses krt.Collection[Address],
	additionalSync func() bool,
) {
	if additionalSync == nil {
		additionalSync = func() bool { return true }
	}
	s.waitForSync = []cache.InformerSynced{
		agwResources.HasSynced,
		addresses.HasSynced,
		s.NackPublisher.HasSynced,
		additionalSync,
	}
}

func (s *Syncer) Start(ctx context.Context) error {
	logger.Info("starting agentgateway Syncer", "controllername", s.controllerName)
	logger.Info("waiting for agentgateway cache to sync")

	// wait for krt collections to sync
	logger.Info("waiting for cache to sync")
	s.client.WaitForCacheSync(
		"agent gateway status syncer",
		ctx.Done(),
		s.waitForSync...,
	)
	logger.Info("caches warm!")

	s.ready.Store(true)
	<-ctx.Done()
	return nil
}

func (s *Syncer) HasSynced() bool {
	return s.ready.Load()
}

// NeedLeaderElection returns false to ensure that the Syncer runs on all pods (leader and followers)
func (r *Syncer) NeedLeaderElection() bool {
	return false
}

// WaitForSync returns a list of functions that can be used to determine if all its informers have synced.
// This is useful for determining if caches have synced.
// It must be called only after `Init()`.
func (s *Syncer) CacheSyncs() []cache.InformerSynced {
	return s.waitForSync
}
