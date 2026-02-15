package pluginsdk

import (
	"context"

	"istio.io/istio/pkg/kube/controllers"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/tools/cache"
)

func CloneObjectMetaForStatus(m metav1.ObjectMeta) metav1.ObjectMeta {
	return metav1.ObjectMeta{
		Name:            m.GetName(),
		Namespace:       m.GetNamespace(),
		ResourceVersion: m.GetResourceVersion(),
	}
}

// GatewayControllerExtension is an interface for extending the Gateway controller with custom behavior
type GatewayControllerExtension interface {
	// Register is called to allow the extension to interact with the Queue used to reconcile Gateways,
	// and access to a ResourceEventHandler that the extension can use to integrate additional Gateway parameter events
	// that should contribute to triggering Gateway reconciliation
	Register(gatewayQueue controllers.Queue, gatewayParamEventHandler cache.ResourceEventHandler)

	// Start is called to start the extension. It must be non-blocking.
	Start(context.Context) error

	// Stop is called to stop the extension.
	Stop() error
}
