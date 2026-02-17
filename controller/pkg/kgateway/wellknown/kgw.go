package wellknown

import (
	"fmt"

	"istio.io/istio/pkg/config"
	istiogvk "istio.io/istio/pkg/config/schema/gvk"
	"k8s.io/apimachinery/pkg/runtime/schema"
)

// GVKToGVR maps a known kgateway GVK to its corresponding GVR
func GVKToGVR(gvk schema.GroupVersionKind) (schema.GroupVersionResource, error) {
	// Try Istio lib to resolve common GVKs
	istioGVK := config.GroupVersionKind{
		Group:   gvk.Group,
		Version: gvk.Version,
		Kind:    gvk.Kind,
	}
	gvr, found := istiogvk.ToGVR(istioGVK)
	if found {
		return gvr, nil
	}

	// Try kgateway types
	switch gvk {
	case AgentgatewayParametersGVK:
		return AgentgatewayParametersGVR, nil
	case AgentgatewayPolicyGVK:
		return AgentgatewayPolicyGVR, nil
	case AgentgatewayBackendGVK:
		return AgentgatewayBackendGVR, nil
	default:
		return schema.GroupVersionResource{}, fmt.Errorf("unknown GVK: %v", gvk)
	}
}
