package wellknown

import (
	"fmt"

	"istio.io/istio/pkg/config"
	istiogvk "istio.io/istio/pkg/config/schema/gvk"
	"k8s.io/apimachinery/pkg/runtime/schema"

	agwv1alpha1 "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

var (
	AgentgatewayBackendGVK    = buildAgwGvk("AgentgatewayBackend")
	AgentgatewayParametersGVK = buildAgwGvk("AgentgatewayParameters")
	AgentgatewayPolicyGVK     = buildAgwGvk("AgentgatewayPolicy")
	AgentgatewayBackendGVR    = AgentgatewayBackendGVK.GroupVersion().WithResource("agentgatewaybackends")
	AgentgatewayParametersGVR = AgentgatewayParametersGVK.GroupVersion().WithResource("agentgatewayparameters")
	AgentgatewayPolicyGVR     = AgentgatewayPolicyGVK.GroupVersion().WithResource("agentgatewaypolicies")
)

func buildAgwGvk(kind string) schema.GroupVersionKind {
	return schema.GroupVersionKind{
		Group:   agwv1alpha1.GroupName,
		Version: agwv1alpha1.GroupVersion.Version,
		Kind:    kind,
	}
}

// GVKToGVR maps a known agentgateway GVK to its corresponding GVR
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

	// Try agentgateway types
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
