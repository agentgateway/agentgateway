package collections

import (
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/util/sets"
	"istio.io/istio/pkg/util/smallset"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
)

type TargetRefIndexKey struct {
	Group       string
	Kind        string
	Name        string
	Namespace   string
	SectionName string
}

func (k TargetRefIndexKey) String() string {
	return fmt.Sprintf("%s/%s/%s/%s/%s", k.Group, k.Kind, k.Name, k.Namespace, k.SectionName)
}

func GatewaysForDeployerTransformationFunc(
	gatewayClasses krt.Collection[*gwv1.GatewayClass],
	listenerSets krt.Collection[*gwxv1a1.XListenerSet],
	byParentRefIndex krt.Index[TargetRefIndexKey, *gwxv1a1.XListenerSet],
	controllerName string,
) func(kctx krt.HandlerContext, gw *gwv1.Gateway) *GatewayForDeployer {
	return func(kctx krt.HandlerContext, gw *gwv1.Gateway) *GatewayForDeployer {
		// only care about gateways use a class controlled by us (envoy or agentgateway)
		gwClass := ptr.Flatten(krt.FetchOne(kctx, gatewayClasses, krt.FilterKey(string(gw.Spec.GatewayClassName))))
		if gwClass == nil || controllerName != string(gwClass.Spec.ControllerName) {
			return nil
		}
		ports := sets.New[int32]()
		for _, l := range gw.Spec.Listeners {
			ports.Insert(l.Port)
		}

		lsets := krt.Fetch(kctx, listenerSets, krt.FilterIndex(byParentRefIndex, TargetRefIndexKey{
			Group:     wellknown.GatewayGroup,
			Kind:      wellknown.GatewayKind,
			Name:      gw.GetName(),
			Namespace: gw.GetNamespace(),
		}))

		for _, ls := range lsets {
			for _, l := range ls.Spec.Listeners {
				port, portErr := kubeutils.DetectListenerPortNumber(l.Protocol, l.Port)
				// Don't need to log an error for the deployer as it will be reflected in the listener status during reconciliation
				if portErr != nil {
					continue
				}
				ports.Insert(port)
			}
		}
		ir := &GatewayForDeployer{
			ObjectSource: ObjectSource{
				Group:     gwv1.GroupVersion.Group,
				Kind:      wellknown.GatewayKind,
				Namespace: gw.Namespace,
				Name:      gw.Name,
			},
			ControllerName: string(gwClass.Spec.ControllerName),
			Ports:          smallset.New(ports.UnsortedList()...),
		}
		return ir
	}
}

type GatewayForDeployer struct {
	ObjectSource
	// Controller name for the gateway
	ControllerName string
	// All ports from all listeners
	Ports smallset.Set[int32]
}

type ObjectSource struct {
	Group     string `json:"group,omitempty"`
	Kind      string `json:"kind,omitempty"`
	Namespace string `json:"namespace,omitempty"`
	Name      string `json:"name"`
}

// GetKind returns the kind of the route.
func (c ObjectSource) GetGroupKind() schema.GroupKind {
	return schema.GroupKind{
		Group: c.Group,
		Kind:  c.Kind,
	}
}

// GetName returns the name of the route.
func (c ObjectSource) GetName() string {
	return c.Name
}

// GetNamespace returns the namespace of the route.
func (c ObjectSource) GetNamespace() string {
	return c.Namespace
}

func (c ObjectSource) ResourceName() string {
	return fmt.Sprintf("%s/%s/%s/%s", c.Group, c.Kind, c.Namespace, c.Name)
}

func (c ObjectSource) String() string {
	return fmt.Sprintf("%s/%s/%s/%s", c.Group, c.Kind, c.Namespace, c.Name)
}

func (c ObjectSource) Equals(in ObjectSource) bool {
	return c.Namespace == in.Namespace && c.Name == in.Name && c.Group == in.Group && c.Kind == in.Kind
}

func (c ObjectSource) NamespacedName() types.NamespacedName {
	return types.NamespacedName{
		Namespace: c.Namespace,
		Name:      c.Name,
	}
}
func (c GatewayForDeployer) ResourceName() string {
	return c.ObjectSource.ResourceName()
}

func (c GatewayForDeployer) Equals(in GatewayForDeployer) bool {
	return c.ObjectSource.Equals(in.ObjectSource) &&
		c.ControllerName == in.ControllerName &&
		slices.Equal(c.Ports.List(), in.Ports.List())
}
