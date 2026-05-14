package apiclient

import (
	"context"
	"sync"

	"istio.io/istio/pkg/config/schema/kubeclient"
	"istio.io/istio/pkg/kube/kubetypes"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/watch"
	"k8s.io/client-go/rest"

	// POC: ainetworking types
	ainetworking "github.com/agentgateway/agentgateway/controller/api/v0alpha0/ainetworking"
	agwv1alpha1 "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

// POC: lazily-initialized PayloadProcessor client
var (
	ppClient     *ainetworking.PayloadProcessorClient
	ppClientOnce sync.Once
	ppClientErr  error
)

func getOrCreatePPClient(restCfg *rest.Config) (*ainetworking.PayloadProcessorClient, error) {
	ppClientOnce.Do(func() {
		ppClient, ppClientErr = ainetworking.NewPayloadProcessorClient(restCfg)
	})
	return ppClient, ppClientErr
}

// RegisterTypes registers all the types used by our API Client
func RegisterTypes(restCfg *rest.Config) {
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

	// POC: Register PayloadProcessor type with Istio's kubeclient registry
	payloadProcessorGVR := ainetworking.SchemeGroupVersion.WithResource("payloadprocessors")
	payloadProcessorGVK := ainetworking.SchemeGroupVersion.WithKind("PayloadProcessor")
	kubeclient.Register(
		payloadProcessorGVR,
		payloadProcessorGVK,
		func(c kubeclient.ClientGetter, namespace string, o metav1.ListOptions) (runtime.Object, error) {
			cli, err := getOrCreatePPClient(restCfg)
			if err != nil {
				return nil, err
			}
			return cli.PayloadProcessors(namespace).List(context.Background(), o)
		},
		func(c kubeclient.ClientGetter, namespace string, o metav1.ListOptions) (watch.Interface, error) {
			cli, err := getOrCreatePPClient(restCfg)
			if err != nil {
				return nil, err
			}
			return cli.PayloadProcessors(namespace).Watch(context.Background(), o)
		},
		func(c kubeclient.ClientGetter, namespace string) kubetypes.WriteAPI[*ainetworking.PayloadProcessor] {
			return nil // POC: read-only, no write API needed
		},
	)
}
