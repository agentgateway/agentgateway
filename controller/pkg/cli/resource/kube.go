package resource

import (
	"fmt"

	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/client-go/dynamic"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/clientcmd"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	"github.com/agentgateway/agentgateway/controller/pkg/schemes"
)

// NewClient builds a controller-runtime client with all agentgateway types registered.
func NewClient() (client.Client, error) {
	restConfig, err := buildRestConfig()
	if err != nil {
		return nil, err
	}
	scheme := schemes.DefaultScheme()
	return client.New(restConfig, client.Options{Scheme: scheme})
}

// NewDynamicClient builds a dynamic Kubernetes client for unstructured operations.
func NewDynamicClient() (dynamic.Interface, error) {
	restConfig, err := buildRestConfig()
	if err != nil {
		return nil, err
	}
	return dynamic.NewForConfig(restConfig)
}

// SchemeForClient returns the scheme used by agentgateway clients.
func SchemeForClient() *runtime.Scheme {
	return schemes.DefaultScheme()
}

func buildRestConfig() (*rest.Config, error) {
	loadingRules := clientcmd.NewDefaultClientConfigLoadingRules()
	if kc := flag.Kubeconfig(); kc != "" {
		loadingRules.ExplicitPath = kc
	}
	cfg := clientcmd.NewNonInteractiveDeferredLoadingClientConfig(loadingRules, &clientcmd.ConfigOverrides{})
	restConfig, err := cfg.ClientConfig()
	if err != nil {
		return nil, fmt.Errorf("failed to build Kubernetes client config: %w", err)
	}
	restConfig.QPS = 50
	restConfig.Burst = 100
	return restConfig, nil
}
