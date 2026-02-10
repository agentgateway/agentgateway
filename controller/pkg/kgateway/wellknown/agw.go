package wellknown

import (
	"k8s.io/apimachinery/pkg/runtime/schema"

	agwv1alpha1 "github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
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
