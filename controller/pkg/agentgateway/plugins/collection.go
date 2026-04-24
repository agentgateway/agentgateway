package plugins

import (
	networkingclient "istio.io/client-go/pkg/apis/networking/v1"
	"istio.io/istio/pkg/config/schema/gvr"
	istiokube "istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/kube/kubetypes"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	discovery "k8s.io/api/discovery/v1"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	kgwversioned "github.com/agentgateway/agentgateway/controller/pkg/client/clientset/versioned"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	krtpkg "github.com/agentgateway/agentgateway/controller/pkg/utils/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

type AgwCollections struct {
	OurClient kgwversioned.Interface
	Client    apiclient.Client
	KrtOpts   krtutil.KrtOptions
	Settings  apisettings.Settings

	GatewaysForDeployer krt.Collection[collections.GatewayForDeployer]
	// GatewaysRequiringOIDC enumerates Gateways whose Pods need the managed
	// OIDC cookie Secret because at least one OIDC-bearing AgentgatewayPolicy
	// is attached, directly or via a child ListenerSet/HTTPRoute/GRPCRoute.
	GatewaysRequiringOIDC krt.Collection[OIDCRequiredGateway]

	// Core Kubernetes resources
	Namespaces          krt.Collection[*corev1.Namespace]
	Nodes               krt.Collection[*corev1.Node]
	Pods                krt.Collection[*corev1.Pod]
	Services            krt.Collection[*corev1.Service]
	ServicesByNamespace krt.Index[string, *corev1.Service]
	Secrets             krt.Collection[*corev1.Secret]
	SecretsByNamespace  krt.Index[string, *corev1.Secret]
	ConfigMaps          krt.Collection[*corev1.ConfigMap]
	EndpointSlices      krt.Collection[*discovery.EndpointSlice]

	// Istio resources for ambient mesh
	WorkloadEntries krt.Collection[*networkingclient.WorkloadEntry]
	ServiceEntries  krt.Collection[*networkingclient.ServiceEntry]

	// Gateway API resources
	GatewayClasses     krt.Collection[*gwv1.GatewayClass]
	Gateways           krt.Collection[*gwv1.Gateway]
	HTTPRoutes         krt.Collection[*gwv1.HTTPRoute]
	GRPCRoutes         krt.Collection[*gwv1.GRPCRoute]
	TCPRoutes          krt.Collection[*gwv1a2.TCPRoute]
	TLSRoutes          krt.Collection[*gwv1.TLSRoute]
	ReferenceGrants    krt.Collection[*gwv1b1.ReferenceGrant]
	BackendTLSPolicies krt.Collection[*gwv1.BackendTLSPolicy]
	ListenerSets       krt.Collection[*gwv1.ListenerSet]

	// Extended resources
	InferencePools krt.Collection[*inf.InferencePool]

	// agentgateway resources
	Backends             krt.Collection[*agentgateway.AgentgatewayBackend]
	AgentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy]

	// ControllerName is the name of the Gateway controller.
	ControllerName string
	// SystemNamespace is control plane system namespace (default is agentgateway-system)
	SystemNamespace string
	// IstioNamespace is the Istio control plane namespace (default is istio-system)
	IstioNamespace string
	// IstioRevision is the Istio revision of the Istio control plane (default is "default").
	IstioRevision string
	// ClusterID is the cluster ID of the cluster the proxy is running in.
	ClusterID string
}

// NewAgwCollections initializes the core krt collections.
// Collections that rely on plugins aren't initialized here,
// and InitPlugins must be called.
func NewAgwCollections(
	krtOptions krtutil.KrtOptions,
	client apiclient.Client,
	agwControllerName string,
	settings apisettings.Settings,
	systemNamespace string,
	clusterID string,
) (*AgwCollections, error) {
	filter := kclient.Filter{ObjectFilter: client.ObjectFilter()}
	gateways := krt.WrapClient(kclient.NewFilteredDelayed[*gwv1.Gateway](
		client, wellknown.GatewayGVR, filter), krtOptions.ToOptions("informer/Gateways")...)
	gatewayClasses := krt.WrapClient(kclient.NewFilteredDelayed[*gwv1.GatewayClass](
		client, wellknown.GatewayClassGVR, filter), krtOptions.ToOptions("informer/GatewayClasses")...)
	listenerSets := krt.WrapClient(kclient.NewDelayedInformer[*gwv1.ListenerSet](
		client, wellknown.ListenerSetGVR, kubetypes.StandardInformer, filter), krtOptions.ToOptions("informer/ListenerSets")...)

	byParentRefIndex := krtpkg.UnnamedIndex(listenerSets, func(in *gwv1.ListenerSet) []collections.TargetRefIndexKey {
		pRef := in.Spec.ParentRef
		ns := ptr.OrDefault(pRef.Namespace, gwv1.Namespace(in.GetNamespace()))
		// lookup by the root object
		return []collections.TargetRefIndexKey{{
			Group:     wellknown.GatewayGroup,
			Kind:      wellknown.GatewayKind,
			Name:      string(pRef.Name),
			Namespace: string(ns),
			// this index intentionally doesn't include sectionName
		}}
	})

	httpRoutes := krt.WrapClient(kclient.NewFilteredDelayed[*gwv1.HTTPRoute](client, wellknown.HTTPRouteGVR, kubetypes.Filter{ObjectFilter: client.ObjectFilter()}), krtOptions.ToOptions("informer/HTTPRoutes")...)
	grpcRoutes := krt.WrapClient(kclient.NewFilteredDelayed[*gwv1.GRPCRoute](client, wellknown.GRPCRouteGVR, kubetypes.Filter{ObjectFilter: client.ObjectFilter()}), krtOptions.ToOptions("informer/GRPCRoutes")...)
	agentgatewayPolicies := krt.NewInformer[*agentgateway.AgentgatewayPolicy](client)

	agwCollections := &AgwCollections{
		Client:   client,
		KrtOpts:  krtOptions,
		Settings: settings,
		GatewaysForDeployer: krt.NewCollection(
			gateways,
			collections.GatewaysForDeployerTransformationFunc(gatewayClasses, listenerSets, byParentRefIndex, agwControllerName),
		),
		GatewaysRequiringOIDC: buildGatewaysRequiringOIDC(
			krtOptions,
			gateways,
			listenerSets,
			httpRoutes,
			grpcRoutes,
			agentgatewayPolicies,
			byParentRefIndex,
		),
		ControllerName:  agwControllerName,
		SystemNamespace: systemNamespace,
		IstioNamespace:  settings.IstioNamespace,
		IstioRevision:   settings.IstioRevision,
		ClusterID:       clusterID,

		// Core Kubernetes resources
		Namespaces: krt.NewInformer[*corev1.Namespace](client, krtOptions.ToOptions("informer/Namespaces")...),
		Nodes: krt.NewFilteredInformer[*corev1.Node](client, kclient.Filter{
			ObjectFilter: client.ObjectFilter(),
		}, krtOptions.ToOptions("informer/Nodes")...),
		Pods: krt.NewFilteredInformer[*corev1.Pod](client, kclient.Filter{
			ObjectTransform: istiokube.StripPodUnusedFields,
			ObjectFilter:    client.ObjectFilter(),
		}, krtOptions.ToOptions("informer/Pods")...),

		Secrets: krt.WrapClient(
			kclient.NewFiltered[*corev1.Secret](client, kubetypes.Filter{
				FieldSelector: apiclient.SecretsFieldSelector,
				ObjectFilter:  client.ObjectFilter(),
			}),
		),
		ConfigMaps: krt.WrapClient(
			kclient.NewFiltered[*corev1.ConfigMap](client, kubetypes.Filter{
				ObjectFilter: client.ObjectFilter(),
			}),
			krtOptions.ToOptions("informer/ConfigMaps")...,
		),
		Services: krt.WrapClient(
			kclient.NewFiltered[*corev1.Service](client, kubetypes.Filter{ObjectFilter: client.ObjectFilter()}),
			krtOptions.ToOptions("informer/Services")...),
		EndpointSlices: krt.WrapClient(
			kclient.NewFiltered[*discovery.EndpointSlice](client, kubetypes.Filter{ObjectFilter: client.ObjectFilter()}),
			krtOptions.ToOptions("informer/EndpointSlices")...),

		// Istio resources
		WorkloadEntries: krt.WrapClient(
			kclient.NewDelayedInformer[*networkingclient.WorkloadEntry](client, gvr.WorkloadEntry, kubetypes.StandardInformer, kclient.Filter{ObjectFilter: client.ObjectFilter()}),
			krtOptions.ToOptions("informer/WorkloadEntries")...),
		ServiceEntries: krt.WrapClient(
			kclient.NewDelayedInformer[*networkingclient.ServiceEntry](client, gvr.ServiceEntry, kubetypes.StandardInformer, kclient.Filter{ObjectFilter: client.ObjectFilter()}),
			krtOptions.ToOptions("informer/ServiceEntries")...),

		// Gateway API resources
		GatewayClasses:     gatewayClasses,
		Gateways:           gateways,
		HTTPRoutes:         httpRoutes,
		GRPCRoutes:         grpcRoutes,
		TLSRoutes:          krt.WrapClient(kclient.NewDelayedInformer[*gwv1.TLSRoute](client, gvr.TLSRoute, kubetypes.StandardInformer, kubetypes.Filter{ObjectFilter: client.ObjectFilter()}), krtOptions.ToOptions("informer/TLSRoutes")...),
		BackendTLSPolicies: krt.WrapClient(kclient.NewDelayedInformer[*gwv1.BackendTLSPolicy](client, gvr.BackendTLSPolicy, kubetypes.StandardInformer, kubetypes.Filter{ObjectFilter: client.ObjectFilter()}), krtOptions.ToOptions("informer/BackendTLSPolicies")...),
		ListenerSets:       listenerSets,

		// Gateway API alpha
		TCPRoutes:       krt.WrapClient(kclient.NewDelayedInformer[*gwv1a2.TCPRoute](client, gvr.TCPRoute, kubetypes.StandardInformer, kubetypes.Filter{ObjectFilter: client.ObjectFilter()}), krtOptions.ToOptions("informer/TCPRoutes")...),
		ReferenceGrants: krt.WrapClient(kclient.NewFilteredDelayed[*gwv1b1.ReferenceGrant](client, wellknown.ReferenceGrantGVR, kubetypes.Filter{ObjectFilter: client.ObjectFilter()}), krtOptions.ToOptions("informer/ReferenceGrants")...),
		// BackendTrafficPolicy?

		// inference extensions need to be enabled so control plane has permissions to watch resource. Disable by default
		InferencePools: krt.NewStaticCollection[*inf.InferencePool](nil, nil, krtOptions.ToOptions("disable/inferencepools")...),

		// agentgateway-specific CRDs
		AgentgatewayPolicies: agentgatewayPolicies,
		Backends:             krt.NewInformer[*agentgateway.AgentgatewayBackend](client),
	}

	if settings.EnableInferExt {
		// inference extensions cluster watch permissions are controlled by enabling EnableInferExt
		inferencePoolGVR := wellknown.InferencePoolGVK.GroupVersion().WithResource("inferencepools")
		agwCollections.InferencePools = krt.WrapClient(kclient.NewDelayedInformer[*inf.InferencePool](client, inferencePoolGVR, kubetypes.StandardInformer, kclient.Filter{ObjectFilter: client.ObjectFilter()}), krtOptions.ToOptions("informer/InferencePools")...)
	}
	agwCollections.SetupIndexes()

	return agwCollections, nil
}

func (c *AgwCollections) SetupIndexes() {
	c.SecretsByNamespace = krt.NewNamespaceIndex(c.Secrets)
	c.ServicesByNamespace = krt.NewNamespaceIndex(c.Services)
}

func (c *AgwCollections) HasSynced() bool {
	return c.GatewaysForDeployer.HasSynced() &&
		c.GatewaysRequiringOIDC.HasSynced()
}
