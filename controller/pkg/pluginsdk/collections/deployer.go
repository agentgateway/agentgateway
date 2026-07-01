package collections

import (
	"fmt"

	"istio.io/istio/pilot/pkg/serviceregistry/ambient"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/util/sets"
	"istio.io/istio/pkg/util/smallset"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/annotations"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
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
	listenerSets krt.Collection[*gwv1.ListenerSet],
	byParentRefIndex krt.Index[TargetRefIndexKey, *gwv1.ListenerSet],
	meshConfig krt.Singleton[ambient.MeshConfig],
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

		td := ptr.OrEmpty(slices.First(krt.PartialFetch(kctx, meshConfig.AsCollection(), func(mc ambient.MeshConfig) string {
			return mc.TrustDomain
		}, nil)))

		ir := &GatewayForDeployer{
			ObjectSource: ObjectSource{
				Group:     gwv1.GroupVersion.Group,
				Kind:      wellknown.GatewayKind,
				Namespace: gw.Namespace,
				Name:      gw.Name,
			},
			ControllerName:  string(gwClass.Spec.ControllerName),
			Ports:           smallset.New(ports.UnsortedList()...),
			InternalPorts:   ComputeInternalPorts(gw, lsets),
			MeshTrustDomain: td,
		}
		return ir
	}
}

// ComputeInternalPorts returns the ports whose bind is internal, mirroring the bind-mode
// decision in the syncer: a port is internal only if every contributing listener (across
// the Gateway and its ListenerSets) agrees. Disagreement leaves the port standard (and is
// surfaced as Accepted=False during translation). Invalid annotations are ignored here;
// translation reports them.
func ComputeInternalPorts(gw *gwv1.Gateway, lsets []*gwv1.ListenerSet) smallset.Set[int32] {
	sawInternal := sets.New[int32]()
	sawStandard := sets.New[int32]()

	gwInternal, _ := annotations.ParseInternalPorts(
		gw.GetAnnotations()[annotations.InternalPorts],
		func(p int32) bool {
			for _, l := range gw.Spec.Listeners {
				if int32(l.Port) == p {
					return true
				}
			}
			return false
		},
	)
	for _, l := range gw.Spec.Listeners {
		if gwInternal.Has(l.Port) {
			sawInternal.Insert(l.Port)
		} else {
			sawStandard.Insert(l.Port)
		}
	}

	for _, ls := range lsets {
		lsInternal, _ := annotations.ParseInternalPorts(
			ls.GetAnnotations()[annotations.InternalPorts],
			func(p int32) bool {
				for _, l := range ls.Spec.Listeners {
					if port, err := kubeutils.DetectListenerPortNumber(l.Protocol, l.Port); err == nil && int32(port) == p {
						return true
					}
				}
				return false
			},
		)
		for _, l := range ls.Spec.Listeners {
			port, err := kubeutils.DetectListenerPortNumber(l.Protocol, l.Port)
			if err != nil {
				continue
			}
			if lsInternal.Has(port) {
				sawInternal.Insert(port)
			} else {
				sawStandard.Insert(port)
			}
		}
	}

	internal := []int32{}
	for p := range sawInternal {
		if !sawStandard.Contains(p) {
			internal = append(internal, p)
		}
	}
	return smallset.New(internal...)
}

type GatewayForDeployer struct {
	ObjectSource
	// Controller name for the gateway
	ControllerName string
	// All ports from all listeners
	Ports smallset.Set[int32]
	// InternalPorts are ports whose bind is internal (routing-only). They are excluded
	// from the generated Service and container ports. Derived from the
	// agentgateway.dev/internal-ports annotation on the Gateway and its ListenerSets.
	InternalPorts smallset.Set[int32]
	// MeshTrustDomain changes should trigger reconciliation
	// this field isn't read outside of Equals for a trigger
	MeshTrustDomain string
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
		c.MeshTrustDomain == in.MeshTrustDomain &&
		slices.Equal(c.Ports.List(), in.Ports.List()) &&
		slices.Equal(c.InternalPorts.List(), in.InternalPorts.List())
}
