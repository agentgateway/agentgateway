package kubeutil

import (
	"context"
	"fmt"
	"io"
	"slices"

	istiocli "istio.io/istio/istioctl/pkg/cli"
	"istio.io/istio/istioctl/pkg/util/handlers"
	"istio.io/istio/pkg/kube"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/util/errors"
	"k8s.io/client-go/tools/clientcmd"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

// Pod identifies a single Kubernetes pod by name and namespace.
type Pod struct {
	Name      string
	Namespace string
}

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

// ResolvePodForResource returns the first (alphabetically) pod backing the
// named resource. Use this only for commands that inherently target a single
// pod (e.g. opening a streaming connection). For commands that should reach
// all pods, use ResolvePodsForResource with ForEachPod.
func ResolvePodForResource(kubeClient kube.CLIClient, resourceName, namespace string) (string, string, error) {
	pods, err := ResolvePodsForResource(kubeClient, resourceName, namespace)
	// ResolvePodsForResource errors on empty, but guard explicitly in case that changes.
	if len(pods) == 0 {
		return "", "", fmt.Errorf("no pods found for resource %q", resourceName)
	}
	if err != nil {
		return "", "", err
	}
	return pods[0].Name, pods[0].Namespace, nil
}

// ResolvePodsForResource returns all pods backing the named resource, sorted
// by name for deterministic ordering.
func ResolvePodsForResource(kubeClient kube.CLIClient, resourceName, namespace string) ([]Pod, error) {
	factory := istiocli.MakeKubeFactory(kubeClient)
	names, podNamespace, err := handlers.InferPodsFromTypedResource(resourceName, namespace, factory)
	if err != nil {
		return nil, err
	}
	if len(names) == 0 {
		return nil, fmt.Errorf("no pods found for resource %q", resourceName)
	}
	slices.Sort(names)
	pods := make([]Pod, len(names))
	for i, n := range names {
		pods[i] = Pod{Name: n, Namespace: podNamespace}
	}
	return pods, nil
}

// ResolveControllerPods returns all controller pods in namespace, identified
// by the app.kubernetes.io/name=agentgateway label, sorted by name.
func ResolveControllerPods(ctx context.Context, kubeClient kube.CLIClient, namespace string) ([]Pod, error) {
	selector := fmt.Sprintf("%s=%s", wellknown.AgentgatewayLabel, wellknown.AgentgatewayLabelValue)
	list, err := kubeClient.Kube().CoreV1().Pods(namespace).List(ctx, metav1.ListOptions{
		LabelSelector: selector,
	})
	if err != nil {
		return nil, fmt.Errorf("failed to list controller pods in namespace %q: %w", namespace, err)
	}
	if len(list.Items) == 0 {
		return nil, fmt.Errorf("no controller pods found in namespace %q (label %s)", namespace, selector)
	}
	pods := make([]Pod, len(list.Items))
	for i, p := range list.Items {
		pods[i] = Pod{Name: p.Name, Namespace: p.Namespace}
	}
	slices.SortFunc(pods, func(a, b Pod) int {
		if a.Name < b.Name {
			return -1
		}
		if a.Name > b.Name {
			return 1
		}
		return 0
	})
	return pods, nil
}

// ForEachPod calls fn for every pod in pods, writes prefixed output to w, and
// returns a combined error for any pods that failed. All pods are attempted
// regardless of individual failures.
//
// Output format per pod:
//
//	<name>.<namespace>:
//	<fn output>
func ForEachPod(ctx context.Context, pods []Pod, w io.Writer, fn func(ctx context.Context, pod Pod) (string, error)) error {
	prefix := len(pods) > 1
	var errs []error
	for _, pod := range pods {
		out, err := fn(ctx, pod)
		if err != nil {
			errs = append(errs, fmt.Errorf("%s.%s: %w", pod.Name, pod.Namespace, err))
			continue
		}
		if prefix {
			fmt.Fprintf(w, "%s.%s:\n", pod.Name, pod.Namespace)
		}
		fmt.Fprint(w, out)
	}
	return errors.NewAggregate(errs)
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
