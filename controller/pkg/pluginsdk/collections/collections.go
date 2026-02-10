package collections

import (
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/kube/kubetypes"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	apisettings "github.com/kgateway-dev/kgateway/v2/api/settings"
	"github.com/kgateway-dev/kgateway/v2/pkg/apiclient"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/krtutil"
	krtpkg "github.com/kgateway-dev/kgateway/v2/pkg/utils/krtutil"
)

type CommonCollections struct {
	Client  apiclient.Client
	KrtOpts krtutil.KrtOptions

	GatewaysForDeployer krt.Collection[GatewayForDeployer]
	// static set of global Settings, non-krt based for dev speed
	// TODO: this should be refactored to a more correct location,
	// or even better, be removed entirely and done per Gateway (maybe in GwParams)
	Settings                   apisettings.Settings
	AgentgatewayControllerName string
}

// NewCommonCollections initializes the core krt collections.
// Collections that rely on plugins aren't initialized here,
// and InitPlugins must be called.
func NewCommonCollections(
	krtOptions krtutil.KrtOptions,
	client apiclient.Client,
	agentGatewayControllerName string,
	settings apisettings.Settings,
) (*CommonCollections, error) {
	filter := kclient.Filter{ObjectFilter: client.ObjectFilter()}
	kubeRawGateways := krt.WrapClient(kclient.NewFilteredDelayed[*gwv1.Gateway](client, wellknown.GatewayGVR, filter), krtOptions.ToOptions("KubeGateways")...)
	gatewayClasses := krt.WrapClient(kclient.NewFilteredDelayed[*gwv1.GatewayClass](client, wellknown.GatewayClassGVR, filter), krtOptions.ToOptions("KubeGatewayClasses")...)
	var kubeRawListenerSets krt.Collection[*gwxv1a1.XListenerSet]
	// ON_EXPERIMENTAL_PROMOTION : Remove this block
	// Ref: https://github.com/kgateway-dev/kgateway/issues/12827
	if settings.EnableExperimentalGatewayAPIFeatures {
		kubeRawListenerSets = krt.WrapClient(kclient.NewDelayedInformer[*gwxv1a1.XListenerSet](client, wellknown.XListenerSetGVR, kubetypes.StandardInformer, filter), krtOptions.ToOptions("KubeListenerSets")...)
	} else {
		// If disabled, still build a collection but make it always empty
		kubeRawListenerSets = krt.NewStaticCollection[*gwxv1a1.XListenerSet](nil, nil, krtOptions.ToOptions("disable/KubeListenerSets")...)
	}
	byParentRefIndex := krtpkg.UnnamedIndex(kubeRawListenerSets, func(in *gwxv1a1.XListenerSet) []TargetRefIndexKey {
		pRef := in.Spec.ParentRef
		ns := strOr(pRef.Namespace, "")
		if ns == "" {
			ns = in.GetNamespace()
		}
		// lookup by the root object
		return []TargetRefIndexKey{{
			Group:     wellknown.GatewayGroup,
			Kind:      wellknown.GatewayKind,
			Name:      string(pRef.Name),
			Namespace: ns,
			// this index intentionally doesn't include sectionName
		}}
	})
	gtw := krt.NewCollection(kubeRawGateways, GatewaysForDeployerTransformationFunc(gatewayClasses, kubeRawListenerSets, byParentRefIndex, agentGatewayControllerName))
	return &CommonCollections{
		Client:                     client,
		KrtOpts:                    krtOptions,
		Settings:                   settings,
		AgentgatewayControllerName: agentGatewayControllerName,
		GatewaysForDeployer:        gtw,
	}, nil
}

func (c *CommonCollections) HasSynced() bool {
	return c.GatewaysForDeployer.HasSynced()
}

func strOr[T ~string](s *T, def string) string {
	if s == nil {
		return def
	}
	return string(*s)
}
