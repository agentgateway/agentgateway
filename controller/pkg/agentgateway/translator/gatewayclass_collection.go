package translator

import (
	"istio.io/istio/pkg/kube/krt"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/krtutil"
)

// GatewayClass is an internal representation of a k8s GatewayClass object that contains the GatewayClass name and controller name.
type GatewayClass struct {
	Name       string
	Controller gwv1.GatewayController
}

func (g GatewayClass) ResourceName() string {
	return g.Name
}

// GatewayClassesCollection returns a collection of internal presentations of GatewayClass objects.
func GatewayClassesCollection(
	gatewayClasses krt.Collection[*gwv1.GatewayClass],
	krtopts krtutil.KrtOptions,
) krt.Collection[GatewayClass] {
	return krt.NewCollection(gatewayClasses, func(ctx krt.HandlerContext, obj *gwv1.GatewayClass) *GatewayClass {
		return &GatewayClass{
			Name:       obj.Name,
			Controller: obj.Spec.ControllerName,
		}
	}, krtopts.ToOptions("GatewayClasses")...)
}
