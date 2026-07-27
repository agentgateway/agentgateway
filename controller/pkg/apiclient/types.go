package apiclient

import (
	"context"

	"istio.io/istio/pkg/config/schema/kubeclient"
	"istio.io/istio/pkg/kube/kubetypes"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/watch"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	agwv1alpha1 "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

// RegisterTypes registers all the types used by our API Client
func RegisterTypes() {
	kubeclient.Register(
		wellknown.AgentgatewayModelGVR,
		wellknown.AgentgatewayModelGVK,
		func(c kubeclient.ClientGetter, namespace string, o metav1.ListOptions) (runtime.Object, error) {
			return c.(Client).Kgateway().AgentgatewayAgentgateway().AgentgatewayModels(namespace).List(context.Background(), o)
		},
		func(c kubeclient.ClientGetter, namespace string, o metav1.ListOptions) (watch.Interface, error) {
			return c.(Client).Kgateway().AgentgatewayAgentgateway().AgentgatewayModels(namespace).Watch(context.Background(), o)
		},
		func(c kubeclient.ClientGetter, namespace string) kubetypes.WriteAPI[*agwv1alpha1.AgentgatewayModel] {
			return c.(Client).Kgateway().AgentgatewayAgentgateway().AgentgatewayModels(namespace)
		},
	)
	kubeclient.Register(
		wellknown.AgentgatewayPolicyGVR,
		wellknown.AgentgatewayPolicyGVK,
		func(c kubeclient.ClientGetter, namespace string, o metav1.ListOptions) (runtime.Object, error) {
			return c.(Client).Kgateway().AgentgatewayAgentgateway().AgentgatewayPolicies(namespace).List(context.Background(), o)
		},
		func(c kubeclient.ClientGetter, namespace string, o metav1.ListOptions) (watch.Interface, error) {
			return c.(Client).Kgateway().AgentgatewayAgentgateway().AgentgatewayPolicies(namespace).Watch(context.Background(), o)
		},
		func(c kubeclient.ClientGetter, namespace string) kubetypes.WriteAPI[*agwv1alpha1.AgentgatewayPolicy] {
			return c.(Client).Kgateway().AgentgatewayAgentgateway().AgentgatewayPolicies(namespace)
		},
	)
	kubeclient.Register(
		wellknown.AgentgatewayBackendGVR,
		wellknown.AgentgatewayBackendGVK,
		func(c kubeclient.ClientGetter, namespace string, o metav1.ListOptions) (runtime.Object, error) {
			return c.(Client).Kgateway().AgentgatewayAgentgateway().AgentgatewayBackends(namespace).List(context.Background(), o)
		},
		func(c kubeclient.ClientGetter, namespace string, o metav1.ListOptions) (watch.Interface, error) {
			return c.(Client).Kgateway().AgentgatewayAgentgateway().AgentgatewayBackends(namespace).Watch(context.Background(), o)
		},
		func(c kubeclient.ClientGetter, namespace string) kubetypes.WriteAPI[*agwv1alpha1.AgentgatewayBackend] {
			return c.(Client).Kgateway().AgentgatewayAgentgateway().AgentgatewayBackends(namespace)
		},
	)
	kubeclient.Register(
		wellknown.AgentgatewayParametersGVR,
		wellknown.AgentgatewayParametersGVK,
		func(c kubeclient.ClientGetter, namespace string, o metav1.ListOptions) (runtime.Object, error) {
			return c.(Client).Kgateway().AgentgatewayAgentgateway().AgentgatewayParameters(namespace).List(context.Background(), o)
		},
		func(c kubeclient.ClientGetter, namespace string, o metav1.ListOptions) (watch.Interface, error) {
			return c.(Client).Kgateway().AgentgatewayAgentgateway().AgentgatewayParameters(namespace).Watch(context.Background(), o)
		},
		func(c kubeclient.ClientGetter, namespace string) kubetypes.WriteAPI[*agwv1alpha1.AgentgatewayParameters] {
			return c.(Client).Kgateway().AgentgatewayAgentgateway().AgentgatewayParameters(namespace)
		},
	)
	kubeclient.Register(
		wellknown.TCPRouteGVR,
		wellknown.TCPRouteGVK,
		func(c kubeclient.ClientGetter, namespace string, o metav1.ListOptions) (runtime.Object, error) {
			return c.(Client).GatewayAPI().GatewayV1().TCPRoutes(namespace).List(context.Background(), o)
		},
		func(c kubeclient.ClientGetter, namespace string, o metav1.ListOptions) (watch.Interface, error) {
			return c.(Client).GatewayAPI().GatewayV1().TCPRoutes(namespace).Watch(context.Background(), o)
		},
		func(c kubeclient.ClientGetter, namespace string) kubetypes.WriteAPI[*gwv1.TCPRoute] {
			return c.(Client).GatewayAPI().GatewayV1().TCPRoutes(namespace)
		},
	)
}
