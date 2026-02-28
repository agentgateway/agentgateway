package plugins

import (
	networkingclient "istio.io/client-go/pkg/apis/networking/v1"
	"istio.io/istio/pkg/config/schema/gvr"
	istiokube "istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/kube/kubetypes"
	corev1 "k8s.io/api/core/v1"
	discovery "k8s.io/api/discovery/v1"
	"k8s.io/apimachinery/pkg/types"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"
	gwv1b1 "sigs.k8s.io/gateway-api/apis/v1beta1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	kgwversioned "github.com/agentgateway/agentgateway/controller/pkg/client/clientset/versioned"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	utilkrt "github.com/agentgateway/agentgateway/controller/pkg/utils/krtutil"
)

type AgwCollections struct {
	OurClient kgwversioned.Interface
	Client    apiclient.Client
	KrtOpts   krtutil.KrtOptions

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
	GatewayClasses                krt.Collection[*gwv1.GatewayClass]
	Gateways                      krt.Collection[*gwv1.Gateway]
	HTTPRoutes                    krt.Collection[*gwv1.HTTPRoute]
	GRPCRoutes                    krt.Collection[*gwv1.GRPCRoute]
	TCPRoutes                     krt.Collection[*gwv1a2.TCPRoute]
	TLSRoutes                     krt.Collection[*gwv1a2.TLSRoute]
	ReferenceGrants               krt.Collection[*gwv1b1.ReferenceGrant]
	BackendTLSPolicies            krt.Collection[*gwv1.BackendTLSPolicy]
	BackendTLSPoliciesByTargetRef krt.Index[utils.TypedNamespacedName, *gwv1.BackendTLSPolicy]
	XListenerSets                 krt.Collection[*gwxv1a1.XListenerSet]

	// Extended resources
	InferencePools krt.Collection[*inf.InferencePool]

	// agentgateway resources
	Backends             krt.Collection[*agentgateway.AgentgatewayBackend]
	AgentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy]
	PoliciesByTargetRef  krt.Index[targetRefIndexKey, *agentgateway.AgentgatewayPolicy]

	// ControllerName is the name of the Gateway controller.
	ControllerName string
	// SystemNamespace is control plane system namespace (default is kgateway-system)
	SystemNamespace string
	// IstioNamespace is the Istio control plane namespace (default is istio-system)
	IstioNamespace string
	// ClusterID is the cluster ID of the cluster the proxy is running in.
	ClusterID string
}

// NewAgwCollections initializes the core krt collections.
// Collections that rely on plugins aren't initialized here,
// and InitPlugins must be called.
func NewAgwCollections(
	commoncol *collections.CommonCollections,
	agwControllerName string,
	systemNamespace string,
	clusterID string,
) (*AgwCollections, error) {
	agwCollections := &AgwCollections{
		Client:          commoncol.Client,
		KrtOpts:         commoncol.KrtOpts,
		ControllerName:  agwControllerName,
		SystemNamespace: systemNamespace,
		IstioNamespace:  commoncol.Settings.IstioNamespace,
		ClusterID:       clusterID,

		// Core Kubernetes resources
		Namespaces: krt.NewInformer[*corev1.Namespace](commoncol.Client),
		Nodes: krt.NewFilteredInformer[*corev1.Node](commoncol.Client, kclient.Filter{
			ObjectFilter: commoncol.Client.ObjectFilter(),
		}, commoncol.KrtOpts.ToOptions("informer/Nodes")...),
		Pods: krt.NewFilteredInformer[*corev1.Pod](commoncol.Client, kclient.Filter{
			ObjectTransform: istiokube.StripPodUnusedFields,
			ObjectFilter:    commoncol.Client.ObjectFilter(),
		}, commoncol.KrtOpts.ToOptions("informer/Pods")...),

		Secrets: krt.WrapClient(
			kclient.NewFiltered[*corev1.Secret](commoncol.Client, kubetypes.Filter{
				ObjectFilter: commoncol.Client.ObjectFilter(),
			}),
		),
		ConfigMaps: krt.WrapClient(
			kclient.NewFiltered[*corev1.ConfigMap](commoncol.Client, kubetypes.Filter{
				ObjectFilter: commoncol.Client.ObjectFilter(),
			}),
			commoncol.KrtOpts.ToOptions("informer/ConfigMaps")...,
		),
		Services: krt.WrapClient(
			kclient.NewFiltered[*corev1.Service](commoncol.Client, kubetypes.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}),
			commoncol.KrtOpts.ToOptions("informer/Services")...),
		EndpointSlices: krt.WrapClient(
			kclient.NewFiltered[*discovery.EndpointSlice](commoncol.Client, kubetypes.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}),
			commoncol.KrtOpts.ToOptions("informer/EndpointSlices")...),

		// Istio resources
		WorkloadEntries: krt.WrapClient(
			kclient.NewDelayedInformer[*networkingclient.WorkloadEntry](commoncol.Client, gvr.WorkloadEntry, kubetypes.StandardInformer, kclient.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}),
			commoncol.KrtOpts.ToOptions("informer/WorkloadEntries")...),
		ServiceEntries: krt.WrapClient(
			kclient.NewDelayedInformer[*networkingclient.ServiceEntry](commoncol.Client, gvr.ServiceEntry, kubetypes.StandardInformer, kclient.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}),
			commoncol.KrtOpts.ToOptions("informer/ServiceEntries")...),

		// Gateway API resources
		GatewayClasses:     krt.WrapClient(kclient.NewFilteredDelayed[*gwv1.GatewayClass](commoncol.Client, wellknown.GatewayClassGVR, kubetypes.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}), commoncol.KrtOpts.ToOptions("informer/GatewayClasses")...),
		Gateways:           krt.WrapClient(kclient.NewFilteredDelayed[*gwv1.Gateway](commoncol.Client, wellknown.GatewayGVR, kubetypes.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}), commoncol.KrtOpts.ToOptions("informer/Gateways")...),
		HTTPRoutes:         krt.WrapClient(kclient.NewFilteredDelayed[*gwv1.HTTPRoute](commoncol.Client, wellknown.HTTPRouteGVR, kubetypes.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}), commoncol.KrtOpts.ToOptions("informer/HTTPRoutes")...),
		GRPCRoutes:         krt.WrapClient(kclient.NewFilteredDelayed[*gwv1.GRPCRoute](commoncol.Client, wellknown.GRPCRouteGVR, kubetypes.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}), commoncol.KrtOpts.ToOptions("informer/GRPCRoutes")...),
		BackendTLSPolicies: krt.WrapClient(kclient.NewDelayedInformer[*gwv1.BackendTLSPolicy](commoncol.Client, gvr.BackendTLSPolicy, kubetypes.StandardInformer, kubetypes.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}), commoncol.KrtOpts.ToOptions("informer/BackendTLSPolicies")...),

		// Gateway API alpha
		TCPRoutes:       krt.WrapClient(kclient.NewDelayedInformer[*gwv1a2.TCPRoute](commoncol.Client, gvr.TCPRoute, kubetypes.StandardInformer, kubetypes.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}), commoncol.KrtOpts.ToOptions("informer/TCPRoutes")...),
		TLSRoutes:       krt.WrapClient(kclient.NewDelayedInformer[*gwv1a2.TLSRoute](commoncol.Client, gvr.TLSRoute, kubetypes.StandardInformer, kubetypes.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}), commoncol.KrtOpts.ToOptions("informer/TLSRoutes")...),
		ReferenceGrants: krt.WrapClient(kclient.NewFilteredDelayed[*gwv1b1.ReferenceGrant](commoncol.Client, wellknown.ReferenceGrantGVR, kubetypes.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}), commoncol.KrtOpts.ToOptions("informer/ReferenceGrants")...),
		XListenerSets:   krt.WrapClient(kclient.NewDelayedInformer[*gwxv1a1.XListenerSet](commoncol.Client, gvr.XListenerSet, kubetypes.StandardInformer, kubetypes.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}), commoncol.KrtOpts.ToOptions("informer/XListenerSets")...),
		// BackendTrafficPolicy?

		// inference extensions need to be enabled so control plane has permissions to watch resource. Disable by default
		InferencePools: krt.NewStaticCollection[*inf.InferencePool](nil, nil, commoncol.KrtOpts.ToOptions("disable/inferencepools")...),

		// agentgateway-specific CRDs
		AgentgatewayPolicies: krt.NewInformer[*agentgateway.AgentgatewayPolicy](commoncol.Client),
		Backends:             krt.NewInformer[*agentgateway.AgentgatewayBackend](commoncol.Client),
	}

	if commoncol.Settings.EnableInferExt {
		// inference extensions cluster watch permissions are controlled by enabling EnableInferExt
		inferencePoolGVR := wellknown.InferencePoolGVK.GroupVersion().WithResource("inferencepools")
		agwCollections.InferencePools = krt.WrapClient(kclient.NewDelayedInformer[*inf.InferencePool](commoncol.Client, inferencePoolGVR, kubetypes.StandardInformer, kclient.Filter{ObjectFilter: commoncol.Client.ObjectFilter()}), commoncol.KrtOpts.ToOptions("informer/InferencePools")...)
	}
	agwCollections.SetupIndexes()

	return agwCollections, nil
}

func (c *AgwCollections) SetupIndexes() {
	c.SecretsByNamespace = krt.NewNamespaceIndex(c.Secrets)
	c.ServicesByNamespace = krt.NewNamespaceIndex(c.Services)
	c.PoliciesByTargetRef = utilkrt.UnnamedIndex(c.AgentgatewayPolicies, func(in *agentgateway.AgentgatewayPolicy) []targetRefIndexKey {
		keys := make([]targetRefIndexKey, 0, len(in.Spec.TargetRefs))
		for _, ref := range in.Spec.TargetRefs {
			keys = append(keys, targetRefIndexKey{
				Name:      string(ref.Name),
				Kind:      string(ref.Kind),
				Group:     string(ref.Group),
				Namespace: in.Namespace,
			})
		}
		return keys
	})
	c.BackendTLSPoliciesByTargetRef = utilkrt.UnnamedIndex(c.BackendTLSPolicies, func(in *gwv1.BackendTLSPolicy) []utils.TypedNamespacedName {
		keys := make([]utils.TypedNamespacedName, 0, len(in.Spec.TargetRefs))
		for _, ref := range in.Spec.TargetRefs {
			keys = append(keys, utils.TypedNamespacedName{
				NamespacedName: types.NamespacedName{
					Name:      string(ref.Name),
					Namespace: in.Namespace,
				},
				Kind: string(ref.Kind),
			})
		}
		return keys
	})
}
