package kubeutil

import (
	"context"
	"fmt"
	"slices"

	istiocli "istio.io/istio/istioctl/pkg/cli"
	"istio.io/istio/istioctl/pkg/util/handlers"
	"istio.io/istio/pkg/kube"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/tools/clientcmd"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
)

func LoadNamespace(namespaceOverride string) (string, error) {
	loadingRules := clientcmd.NewDefaultClientConfigLoadingRules()
	if kubeconfig := flag.Kubeconfig(); kubeconfig != "" {
		loadingRules.ExplicitPath = kubeconfig
	}

	configLoader := clientcmd.NewNonInteractiveDeferredLoadingClientConfig(loadingRules, &clientcmd.ConfigOverrides{})
	namespace, _, err := configLoader.Namespace()
	if err != nil {
		return "", fmt.Errorf("failed to resolve namespace from kubeconfig: %w", err)
	}
	if namespaceOverride != "" {
		namespace = namespaceOverride
	}

	return namespace, nil
}

func NewCLIClient() (kube.CLIClient, error) {
	restConfig, err := kube.DefaultRestConfig(flag.Kubeconfig(), "")
	if err != nil {
		return nil, fmt.Errorf("failed to build Kubernetes client config: %w", err)
	}

	restConfig.QPS = 50
	restConfig.Burst = 100

	return kube.NewCLIClient(kube.NewClientConfigForRestConfig(restConfig))
}

func ResolveResourceName(ctx context.Context, kubeClient kube.CLIClient, namespace string, args []string) (string, error) {
	if len(args) == 1 {
		return args[0], nil
	}
	return inferSingleGatewayResourceName(ctx, kubeClient, namespace)
}

func ResolvePodForResource(kubeClient kube.CLIClient, resourceName, namespace string) (string, string, error) {
	factory := istiocli.MakeKubeFactory(kubeClient)
	pods, podNamespace, err := handlers.InferPodsFromTypedResource(resourceName, namespace, factory)
	if err != nil {
		return "", "", err
	}
	if len(pods) == 0 {
		return "", "", fmt.Errorf("no pods found for resource %q", resourceName)
	}
	slices.Sort(pods)
	return pods[0], podNamespace, nil
}

func inferSingleGatewayResourceName(ctx context.Context, kubeClient kube.CLIClient, namespace string) (string, error) {
	gateways, err := kubeClient.GatewayAPI().GatewayV1().Gateways(namespace).List(ctx, metav1.ListOptions{})
	if err != nil {
		return "", fmt.Errorf("failed to list Gateways in namespace %q: %w", namespace, err)
	}

	return singleGatewayResourceName(gateways.Items, namespace)
}

func singleGatewayResourceName(gateways []gwv1.Gateway, namespace string) (string, error) {
	switch len(gateways) {
	case 0:
		return "", fmt.Errorf("no Gateways found in namespace %q; pass a resource explicitly", namespace)
	case 1:
		return "gateway/" + gateways[0].Name, nil
	default:
		return "", fmt.Errorf("found %d Gateways in namespace %q; pass a resource explicitly", len(gateways), namespace)
	}
}
