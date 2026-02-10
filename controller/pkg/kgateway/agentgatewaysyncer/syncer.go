package agentgatewaysyncer

import (
	"context"
	"fmt"
	"strconv"
	"sync/atomic"

	"github.com/agentgateway/agentgateway/go/api"
	securityclient "istio.io/client-go/pkg/apis/security/v1"
	"istio.io/istio/pilot/pkg/model"
	"istio.io/istio/pilot/pkg/serviceregistry/kube/controller/ambient"
	"istio.io/istio/pkg/cluster"
	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/config/mesh"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/network"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/util/sets"
	"istio.io/istio/pkg/workloadapi"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/tools/cache"
	"sigs.k8s.io/controller-runtime/pkg/manager"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gatewayx "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	agwir "github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/ir"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/plugins"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/translator"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/utils"
	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient"
	"github.com/kgateway-dev/kgateway/v2/pkg/deployer"
	agentgatewaybackend "github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer/backend"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer/krtxds"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer/nack"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer/status"
	"github.com/kgateway-dev/kgateway/v2/pkg/logging"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/krtutil"
	krtpkg "github.com/kgateway-dev/kgateway/v2/pkg/utils/krtutil"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
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
	workloadAddressProviderFunc func(model.WorkloadInfo) *workloadapi.Address
	serviceAddressProviderFunc  func(model.ServiceInfo) *workloadapi.Address
}

func NewAgwSyncer(
	ctx context.Context,
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
			translator.WithGatewayTransformationFunc(cfg.GatewayTransformationFunc)},
		customResourceCollections:   cfg.CustomResourceCollections,
		workloadAddressProviderFunc: cfg.WorkloadAddressProviderFunc,
		serviceAddressProviderFunc:  cfg.ServiceAddressProviderFunc,
	}
	logger.Debug("init agentgateway Syncer", "controllername", controllerName)

	syncer.buildResourceCollections(krtopts.WithPrefix("agentgateway"))
	return syncer
}

func (s *Syncer) StatusCollections() *status.StatusCollections {
	return s.statusCollections
}

type OutputCollections struct {
	Resources krt.Collection[agwir.AgwResource]
	Addresses krt.Collection[Address]
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
	listenerSetStatus, listenerSets := s.buildListenerSetCollection(gatewayClasses, refGrants, krtopts)
	status.RegisterStatus(s.statusCollections, listenerSetStatus, translator.GetStatus)
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
	agwResources, routeAttachments, policyStatuses, backendStatuses := s.buildAgwResources(gateways, refGrants, krtopts)
	status.RegisterStatus(s.statusCollections, backendStatuses, translator.GetStatus)
	for _, col := range policyStatuses {
		status.RegisterStatus(s.statusCollections, col, translator.GetStatus)
	}

	gatewayFinalStatus := s.buildFinalGatewayStatus(gatewayInitialStatus, routeAttachments, krtopts)
	status.RegisterStatus(s.statusCollections, gatewayFinalStatus, translator.GetStatus)

	// Build address collections
	addresses := s.buildAddressCollections(krtopts)

	// Build XDS collection
	s.buildXDSCollection(agwResources, addresses, krtopts)

	// Set up sync dependencies
	s.setupSyncDependencies(agwResources, addresses)

	s.Outputs.Resources = agwResources
	s.Outputs.Addresses = addresses
}

func (s *Syncer) buildFinalGatewayStatus(
	gatewayStatuses krt.StatusCollection[*gwv1.Gateway, gwv1.GatewayStatus],
	routeAttachments krt.Collection[*translator.RouteAttachment],
	krtopts krtutil.KrtOptions,
) krt.StatusCollection[*gwv1.Gateway, gwv1.GatewayStatus] {
	routeAttachmentsIndex := krt.NewIndex(routeAttachments, "to", func(o *translator.RouteAttachment) []types.NamespacedName {
		return []types.NamespacedName{o.To}
	})
	return krt.NewCollection(
		gatewayStatuses,
		func(ctx krt.HandlerContext, i krt.ObjectWithStatus[*gwv1.Gateway, gwv1.GatewayStatus]) *krt.ObjectWithStatus[*gwv1.Gateway, gwv1.GatewayStatus] {
			tcpRoutes := krt.Fetch(ctx, routeAttachments, krt.FilterIndex(routeAttachmentsIndex, config.NamespacedName(i.Obj)))
			counts := map[string]int32{}
			for _, r := range tcpRoutes {
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
	krt.StatusCollection[*gatewayx.XListenerSet, gatewayx.ListenerSetStatus],
	krt.Collection[translator.ListenerSet],
) {
	return translator.ListenerSetCollection(
		s.controllerName,
		s.agwCollections.XListenerSets,
		s.agwCollections.Gateways,
		gatewayClasses,
		s.agwCollections.Namespaces,
		refGrants,
		s.agwCollections.Secrets,
		s.agwCollections.ConfigMaps,
		krtopts,
	)
}

func (s *Syncer) buildAgwResources(
	gateways krt.Collection[*translator.GatewayListener],
	refGrants translator.ReferenceGrants,
	krtopts krtutil.KrtOptions,
) (
	krt.Collection[agwir.AgwResource],
	krt.Collection[*translator.RouteAttachment],
	PolicyStatusCollections,
	krt.StatusCollection[*agentgateway.AgentgatewayBackend, agentgateway.AgentgatewayBackendStatus],
) {
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
		var protocol = api.Bind_Protocol(0)
		for _, gw := range object.Objects {
			uniq.Insert(types.NamespacedName{
				Namespace: gw.ParentGateway.Namespace,
				Name:      gw.ParentGateway.Name,
			})
			// TODO: better handle conflicts of protocols. For now, we arbitrarily treat TLS > plain
			if gw.Valid {
				protocol = max(protocol, s.getBindProtocol(gw))
			}
		}
		return slices.Map(uniq.UnsortedList(), func(e types.NamespacedName) agwir.AgwResource {
			bind := translator.AgwBind{
				Bind: &api.Bind{
					Key:      object.Key + "/" + e.String(),
					Port:     uint32(port), //nolint:gosec // G115: port is always in valid port range
					Protocol: protocol,
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
	routeParents := translator.BuildRouteParents(filteredGateways)

	routeInputs := translator.RouteContextInputs{
		Grants:         refGrants,
		RouteParents:   routeParents,
		ControllerName: s.controllerName,
		Services:       s.agwCollections.Services,
		Namespaces:     s.agwCollections.Namespaces,
		ServiceEntries: s.agwCollections.ServiceEntries,
		InferencePools: s.agwCollections.InferencePools,
		Backends:       s.agwCollections.Backends,
	}

	agwRoutes, routeAttachments := translator.AgwRouteCollection(s.statusCollections, s.agwCollections.HTTPRoutes, s.agwCollections.GRPCRoutes, s.agwCollections.TCPRoutes, s.agwCollections.TLSRoutes, routeInputs, krtopts)
	if s.agwPlugins.AddResourceExtension != nil && s.agwPlugins.AddResourceExtension.Routes != nil {
		agwRoutes = krt.JoinCollection([]krt.Collection[agwir.AgwResource]{agwRoutes, s.agwPlugins.AddResourceExtension.Routes})
	}

	agwPolicies, policyStatuses := AgwPolicyCollection(s.agwPlugins, krtopts)

	// Create an agentgateway backend collection from the kgateway backend resources
	agwBackendStatus, agwBackends := s.newAgwBackendCollection(s.agwCollections.Backends, krtopts)

	// Join all Agw resources
	allAgwResources := krt.JoinCollection([]krt.Collection[agwir.AgwResource]{binds, listeners, agwRoutes, agwPolicies, agwBackends}, krtopts.ToOptions("Resources")...)

	return allAgwResources, routeAttachments, policyStatuses, agwBackendStatus
}

// buildListenerFromGateway creates a listener resource from a gateway
func (s *Syncer) buildListenerFromGateway(obj *translator.GatewayListener) *agwir.AgwResource {
	l := &api.Listener{
		Key:      obj.ResourceName(),
		Name:     utils.ListenerName(obj.ParentGateway.Namespace, obj.ParentGateway.Name, string(obj.ParentInfo.SectionName)),
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

	return ptr.Of(translator.ToResourceForGateway(types.NamespacedName{
		Namespace: obj.ParentGateway.Namespace,
		Name:      obj.ParentGateway.Name,
	}, translator.AgwListener{l}))
}

// newAgwBackendCollection creates the ADP backend collection for agent gateway resources
func (s *Syncer) newAgwBackendCollection(finalBackends krt.Collection[*agentgateway.AgentgatewayBackend], krtopts krtutil.KrtOptions) (
	krt.StatusCollection[*agentgateway.AgentgatewayBackend, agentgateway.AgentgatewayBackendStatus],
	krt.Collection[agwir.AgwResource],
) {
	return krt.NewStatusManyCollection(finalBackends, func(ctx krt.HandlerContext, backend *agentgateway.AgentgatewayBackend) (
		*agentgateway.AgentgatewayBackendStatus,
		[]agwir.AgwResource,
	) {
		pc := plugins.PolicyCtx{
			Krt:         ctx,
			Collections: s.agwCollections,
		}
		return agentgatewaybackend.TranslateAgwBackend(pc, backend)
	}, krtopts.ToOptions("Backends")...)
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
		if len(obj.TLSInfo.CaCert) > 0 {
			tlsConfig.Root = obj.TLSInfo.CaCert
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
	default:
		return api.Bind_HTTP
	}
}

func (s *Syncer) buildAddressCollections(krtopts krtutil.KrtOptions) krt.Collection[Address] {
	cols := s.agwCollections
	opts := krtopts.ToIstio()
	clusterId := cluster.ID(cols.ClusterID)
	Networks := ambient.BuildNetworkCollections(cols.Namespaces, cols.Gateways, ambient.Options{
		SystemNamespace: cols.IstioNamespace,
		ClusterID:       clusterId,
	}, opts)
	builder := ambient.Builder{
		DomainSuffix:      kubeutils.GetClusterDomainName(),
		ClusterID:         clusterId,
		NetworkGateways:   Networks.NetworkGateways,
		GatewaysByNetwork: Networks.GatewaysByNetwork,
		Flags: ambient.FeatureFlags{
			EnableK8SServiceSelectWorkloadEntries: true,
			EnableMtlsTransportProtocol:           true,
		},
		Network: func(ctx krt.HandlerContext) network.ID {
			return ""
		},
	}
	// Dummy empty mesh config
	meshConfig := krt.NewStatic[ambient.MeshConfig](&ambient.MeshConfig{MeshConfig: mesh.DefaultMeshConfig()}, true, krtopts.ToOptions("MeshConfig")...)

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
	inferencePoolsInfo := krt.NewCollection(cols.InferencePools, inferencePoolBuilder(),
		krtopts.ToOptions("InferencePools")...)
	services = krt.JoinCollection([]krt.Collection[model.ServiceInfo]{services, inferencePoolsInfo}, krt.WithJoinUnchecked())

	// TODO: add InferencePools
	nodeLocality := ambient.NodesCollection(cols.Nodes, opts.WithName("NodeLocality")...)
	workloads := builder.WorkloadsCollection(
		cols.Pods,
		nodeLocality, // NodeLocality,
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

	// Build address collections
	workloadAddresses := krt.MapCollection(workloads, func(t model.WorkloadInfo) Address {
		// If AsAddress is not populated and we have a provider function, use it to populate AsAddress
		// This is called after WorkloadInfo objects are created from Kubernetes resources by Istio's
		// ambient workload builder, but before they are wrapped in Address structs for XDS.
		if t.AsAddress.Address == nil && s.workloadAddressProviderFunc != nil {
			if addr := s.workloadAddressProviderFunc(t); addr != nil {
				setWorkloadAddress(&t, addr)
			}
		}
		return Address{Workload: &t}
	})
	svcAddresses := krt.MapCollection(services, func(t model.ServiceInfo) Address {
		// If AsAddress is not populated and we have a provider function, use it to populate AsAddress
		// This is called after ServiceInfo objects are created from Kubernetes resources by Istio's
		// ambient service builder, but before they are wrapped in Address structs for XDS.
		if t.AsAddress.Address == nil && s.serviceAddressProviderFunc != nil {
			if addr := s.serviceAddressProviderFunc(t); addr != nil {
				setServiceAddress(&t, addr)
			}
		}
		return Address{Service: &t}
	})

	adpAddresses := krt.JoinCollection([]krt.Collection[Address]{svcAddresses, workloadAddresses}, krtopts.ToOptions("Addresses")...)
	return adpAddresses
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
) {
	s.waitForSync = []cache.InformerSynced{
		s.agwCollections.HasSynced,
		s.agwPlugins.HasSynced,
		agwResources.HasSynced,
		addresses.HasSynced,
		s.NackPublisher.HasSynced,
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
