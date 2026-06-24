package kubeutil

import (
	"context"
	"fmt"
	"io"
	"slices"
	"strings"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/util/errors"
	"k8s.io/client-go/tools/clientcmd"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
)

const agentgatewayLabelSelector = "agentgateway=agentgateway"

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

func ResolveResourceName(ctx context.Context, kubeClient CLIClient, namespace string, args []string) (string, error) {
	if len(args) == 1 {
		return args[0], nil
	}
	return inferSingleGatewayResourceName(ctx, kubeClient, namespace)
}

// ResolvePodForResource returns the first (alphabetically) pod backing the
// named resource. Use this only for commands that inherently target a single
// pod (e.g. opening a streaming connection). For commands that should reach
// all pods, use ResolvePodsForResource with ForEachPod.
func ResolvePodForResource(ctx context.Context, kubeClient CLIClient, resourceName, namespace string) (string, string, error) {
	pods, err := ResolvePodsForResource(ctx, kubeClient, resourceName, namespace)
	if err != nil {
		return "", "", err
	}
	// ResolvePodsForResource errors on empty, but guard explicitly in case that changes.
	if len(pods) == 0 {
		return "", "", fmt.Errorf("no pods found for resource %q", resourceName)
	}
	return pods[0].Name, pods[0].Namespace, nil
}

// ResolvePodsForResource returns all pods backing the named resource, sorted
// by name for deterministic ordering.
func ResolvePodsForResource(ctx context.Context, kubeClient CLIClient, resourceName, namespace string) ([]Pod, error) {
	names, podNamespace, err := inferPodsFromResource(ctx, kubeClient, resourceName, namespace)
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
func ResolveControllerPods(ctx context.Context, kubeClient CLIClient, namespace string) ([]Pod, error) {
	list, err := kubeClient.Kube().CoreV1().Pods(namespace).List(ctx, metav1.ListOptions{
		LabelSelector: agentgatewayLabelSelector,
	})
	if err != nil {
		return nil, fmt.Errorf("failed to list controller pods in namespace %q: %w", namespace, err)
	}
	if len(list.Items) == 0 {
		return nil, fmt.Errorf("no controller pods found in namespace %q (label %s)", namespace, agentgatewayLabelSelector)
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

func inferPodsFromResource(ctx context.Context, kubeClient CLIClient, resourceName, namespace string) ([]string, string, error) {
	resourceName, namespace = inferNamespace(resourceName, namespace)
	resourceType, name, typed := strings.Cut(resourceName, "/")
	if !typed {
		return []string{resourceName}, namespace, nil
	}
	switch strings.ToLower(resourceType) {
	case "pod", "pods", "po":
		return []string{name}, namespace, nil
	}

	selector, podNamespace, err := selectorForResource(ctx, kubeClient, strings.ToLower(resourceType), name, namespace)
	if err != nil {
		return nil, "", err
	}

	podList, err := kubeClient.Kube().CoreV1().Pods(podNamespace).List(ctx, metav1.ListOptions{LabelSelector: selector})
	if err != nil {
		return nil, "", err
	}
	names := make([]string, 0, len(podList.Items))
	for _, pod := range podList.Items {
		names = append(names, pod.Name)
	}
	return names, podNamespace, nil
}

func inferNamespace(name, namespace string) (string, string) {
	if idx := strings.LastIndex(name, "/"); idx > 0 {
		separator := strings.LastIndex(name[idx:], ".")
		if separator < 0 {
			return name, namespace
		}

		return name[0 : idx+separator], name[idx+separator+1:]
	}
	separator := strings.LastIndex(name, ".")
	if separator < 0 {
		return name, namespace
	}

	return name[0:separator], name[separator+1:]
}

func selectorForResource(ctx context.Context, kubeClient CLIClient, resourceType, name, namespace string) (string, string, error) {
	switch resourceType {
	case "gateway", "gateways", "gtw":
		if _, err := kubeClient.GatewayAPI().GatewayV1().Gateways(namespace).Get(ctx, name, metav1.GetOptions{}); err != nil {
			return "", "", fmt.Errorf("failed retrieving gateway/%s in namespace %q: %w", name, namespace, err)
		}
		return "gateway.networking.k8s.io/gateway-name=" + name, namespace, nil
	case "service", "services", "svc":
		svc, err := kubeClient.Kube().CoreV1().Services(namespace).Get(ctx, name, metav1.GetOptions{})
		if err != nil {
			return "", "", fmt.Errorf("failed retrieving service/%s in namespace %q: %w", name, namespace, err)
		}
		selector, err := selectorFromMap(svc.Spec.Selector)
		return selector, svc.Namespace, err
	case "deployment", "deployments", "deploy":
		deploy, err := kubeClient.Kube().AppsV1().Deployments(namespace).Get(ctx, name, metav1.GetOptions{})
		if err != nil {
			return "", "", fmt.Errorf("failed retrieving deployment/%s in namespace %q: %w", name, namespace, err)
		}
		selector, err := selectorFromLabelSelector(deploy.Spec.Selector)
		return selector, deploy.Namespace, err
	case "replicaset", "replicasets", "rs":
		rs, err := kubeClient.Kube().AppsV1().ReplicaSets(namespace).Get(ctx, name, metav1.GetOptions{})
		if err != nil {
			return "", "", fmt.Errorf("failed retrieving replicaset/%s in namespace %q: %w", name, namespace, err)
		}
		selector, err := selectorFromLabelSelector(rs.Spec.Selector)
		return selector, rs.Namespace, err
	case "statefulset", "statefulsets", "sts":
		sts, err := kubeClient.Kube().AppsV1().StatefulSets(namespace).Get(ctx, name, metav1.GetOptions{})
		if err != nil {
			return "", "", fmt.Errorf("failed retrieving statefulset/%s in namespace %q: %w", name, namespace, err)
		}
		selector, err := selectorFromLabelSelector(sts.Spec.Selector)
		return selector, sts.Namespace, err
	case "daemonset", "daemonsets", "ds":
		ds, err := kubeClient.Kube().AppsV1().DaemonSets(namespace).Get(ctx, name, metav1.GetOptions{})
		if err != nil {
			return "", "", fmt.Errorf("failed retrieving daemonset/%s in namespace %q: %w", name, namespace, err)
		}
		selector, err := selectorFromLabelSelector(ds.Spec.Selector)
		return selector, ds.Namespace, err
	default:
		return "", "", fmt.Errorf("%q does not refer to a supported pod resource", resourceType+"/"+name)
	}
}

func selectorFromLabelSelector(labelSelector *metav1.LabelSelector) (string, error) {
	if labelSelector == nil {
		return "", fmt.Errorf("resource has no pod selector")
	}
	selector, err := metav1.LabelSelectorAsSelector(labelSelector)
	if err != nil {
		return "", err
	}
	if selector.Empty() {
		return "", fmt.Errorf("resource has empty pod selector")
	}
	return selector.String(), nil
}

func selectorFromMap(labels map[string]string) (string, error) {
	if len(labels) == 0 {
		return "", fmt.Errorf("resource has no pod selector")
	}
	selector := make([]string, 0, len(labels))
	for k, v := range labels {
		selector = append(selector, k+"="+v)
	}
	slices.Sort(selector)
	return strings.Join(selector, ","), nil
}

func inferSingleGatewayResourceName(ctx context.Context, kubeClient CLIClient, namespace string) (string, error) {
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
