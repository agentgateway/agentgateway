//go:build e2e

package assertions

import (
	"context"
	"fmt"

	"github.com/onsi/gomega/types"
	"istio.io/istio/pkg/test"
	"istio.io/istio/pkg/test/util/retry"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	ktypes "k8s.io/apimachinery/pkg/types"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/cluster"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

const agentgatewayLabelSelector = "app.kubernetes.io/name=agentgateway"

func EventuallyPodsRunning(t Test, podNamespace string, listOpt metav1.ListOptions) {
	t.Helper()
	EventuallyPodsMatches(t, podNamespace, listOpt, matchers.PodMatches(matchers.ExpectedPod{Status: corev1.PodRunning, Ready: true}))
}

func EventuallyPodsMatches(t Test, podNamespace string, listOpt metav1.ListOptions, matcher types.GomegaMatcher) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		pods, err := t.E2EClusterContext().Client.Kube().CoreV1().Pods(podNamespace).List(t.E2EContext(), listOpt)
		if err != nil {
			return fmt.Errorf("failed to list pods: %w", err)
		}
		if len(pods.Items) == 0 {
			return fmt.Errorf("no pods found in namespace %s matching %v", podNamespace, listOpt)
		}
		for _, pod := range pods.Items {
			ok, err := matcher.Match(pod)
			if err != nil {
				return err
			}
			if !ok {
				return fmt.Errorf("pod %s/%s did not match expected state: phase=%s ready=%v", pod.Namespace, pod.Name, pod.Status.Phase, podReady(&pod))
			}
		}
		return nil
	})
}

func EventuallyGatewayCondition(t Test, gatewayName string, gatewayNamespace string, cond gwv1.GatewayConditionType, expect metav1.ConditionStatus) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		gw := &gwv1.Gateway{}
		if err := t.E2EClusterContext().ControllerClient.Get(t.E2EContext(), ktypes.NamespacedName{Name: gatewayName, Namespace: gatewayNamespace}, gw); err != nil {
			return fmt.Errorf("failed to get Gateway %s/%s: %w", gatewayNamespace, gatewayName, err)
		}
		return expectMatch(gw.Status.Conditions, matchers.HaveCondition(string(cond), expect), "Gateway %s/%s condition %s=%s", gatewayNamespace, gatewayName, cond, expect)
	})
}

func EventuallyGatewayListenerAttachedRoutes(t Test, gatewayName string, gatewayNamespace string, listener gwv1.SectionName, routes int32) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		gw := &gwv1.Gateway{}
		if err := t.E2EClusterContext().ControllerClient.Get(t.E2EContext(), ktypes.NamespacedName{Name: gatewayName, Namespace: gatewayNamespace}, gw); err != nil {
			return fmt.Errorf("failed to get Gateway %s/%s: %w", gatewayNamespace, gatewayName, err)
		}
		for _, l := range gw.Status.Listeners {
			if l.Name == listener {
				if l.AttachedRoutes != routes {
					return fmt.Errorf("%s listener attached routes = %d, want %d; full status: %+v", listener, l.AttachedRoutes, routes, gw.Status)
				}
				return nil
			}
		}
		return fmt.Errorf("listener %s not found in Gateway %s/%s; full status: %+v", listener, gatewayNamespace, gatewayName, gw.Status)
	})
}

func EventuallyHTTPRouteCondition(t Test, routeName string, routeNamespace string, cond gwv1.RouteConditionType, expect metav1.ConditionStatus) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		route := &gwv1.HTTPRoute{}
		if err := t.E2EClusterContext().ControllerClient.Get(t.E2EContext(), ktypes.NamespacedName{Name: routeName, Namespace: routeNamespace}, route); err != nil {
			return fmt.Errorf("failed to get HTTPRoute %s/%s: %w", routeNamespace, routeName, err)
		}
		return expectMatch(extractParentConditions(route.Status.Parents), matchers.HaveAnyParentCondition(string(cond), expect), "HTTPRoute %s/%s parent condition %s=%s", routeNamespace, routeName, cond, expect)
	})
}

func EventuallyGRPCRouteCondition(t Test, routeName string, routeNamespace string, cond gwv1.RouteConditionType, expect metav1.ConditionStatus) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		route := &gwv1.GRPCRoute{}
		if err := t.E2EClusterContext().ControllerClient.Get(t.E2EContext(), ktypes.NamespacedName{Name: routeName, Namespace: routeNamespace}, route); err != nil {
			return fmt.Errorf("failed to get GRPCRoute %s/%s: %w", routeNamespace, routeName, err)
		}
		return expectMatch(extractParentConditions(route.Status.Parents), matchers.HaveAnyParentCondition(string(cond), expect), "GRPCRoute %s/%s parent condition %s=%s", routeNamespace, routeName, cond, expect)
	})
}

func EventuallyInferencePoolCondition(t Test, poolName string, poolNamespace string, cond inf.InferencePoolConditionType, expect metav1.ConditionStatus) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		pool := &inf.InferencePool{}
		if err := t.E2EClusterContext().ControllerClient.Get(t.E2EContext(), ktypes.NamespacedName{Name: poolName, Namespace: poolNamespace}, pool); err != nil {
			return fmt.Errorf("failed to get InferencePool %s/%s: %w", poolNamespace, poolName, err)
		}
		return expectMatch(extractInferencePoolParentConditions(pool.Status.Parents), matchers.HaveAnyParentCondition(string(cond), expect), "InferencePool %s/%s parent condition %s=%s", poolNamespace, poolName, cond, expect)
	})
}

func EventuallyAgwBackendCondition(t Test, name string, namespace string, condition string, expect metav1.ConditionStatus) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		backend := &agentgateway.AgentgatewayBackend{}
		if err := t.E2EClusterContext().ControllerClient.Get(t.E2EContext(), ktypes.NamespacedName{Name: name, Namespace: namespace}, backend); err != nil {
			return fmt.Errorf("failed to get AgentgatewayBackend %s/%s: %w", namespace, name, err)
		}
		return expectMatch(backend.Status.Conditions, matchers.HaveCondition(condition, expect), "AgentgatewayBackend %s/%s condition %s=%s", namespace, name, condition, expect)
	})
}

func EventuallyAgwPolicyCondition(t Test, name string, namespace string, condType string, expect metav1.ConditionStatus) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		policy := &agentgateway.AgentgatewayPolicy{}
		if err := t.E2EClusterContext().ControllerClient.Get(t.E2EContext(), ktypes.NamespacedName{Name: name, Namespace: namespace}, policy); err != nil {
			return fmt.Errorf("failed to get AgentgatewayPolicy %s/%s: %w", namespace, name, err)
		}
		return expectMatch(extractAgwPolicyAncestorConditions(policy.Status.Ancestors), matchers.HaveAnyAncestorCondition(condType, expect), "AgentgatewayPolicy %s/%s ancestor condition %s=%s", namespace, name, condType, expect)
	})
}

func EventuallyGatewayAddress(t test.Failer, ctx context.Context, clusterContext *cluster.Context, gatewayName string, gatewayNamespace string) string {
	t.Helper()
	var addr string
	retry.UntilSuccessOrFail(t, func() error {
		gw := &gwv1.Gateway{}
		if err := clusterContext.ControllerClient.Get(ctx, ktypes.NamespacedName{Name: gatewayName, Namespace: gatewayNamespace}, gw); err != nil {
			return fmt.Errorf("failed to get Gateway %s/%s: %w", gatewayNamespace, gatewayName, err)
		}
		if len(gw.Status.Addresses) == 0 {
			return fmt.Errorf("gateway %s/%s has no addresses", gatewayNamespace, gatewayName)
		}
		addr = gw.Status.Addresses[0].Value
		return nil
	})
	return addr
}

func EventuallyGatewayInstallSucceeded(t test.Failer, ctx context.Context, clusterContext *cluster.Context, installNamespace string) {
	t.Helper()
	eventuallyPodsRunning(t, ctx, clusterContext, installNamespace, metav1.ListOptions{LabelSelector: agentgatewayLabelSelector})
}

func EventuallyGatewayUninstallSucceeded(t test.Failer, ctx context.Context, clusterContext *cluster.Context, installNamespace string) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		pods, err := clusterContext.Client.Kube().CoreV1().Pods(installNamespace).List(ctx, metav1.ListOptions{LabelSelector: agentgatewayLabelSelector})
		if err != nil {
			return fmt.Errorf("failed to list pods: %w", err)
		}
		if len(pods.Items) != 0 {
			return fmt.Errorf("expected no agentgateway pods, got %d", len(pods.Items))
		}
		return nil
	})
}

func eventuallyPodsRunning(t test.Failer, ctx context.Context, clusterContext *cluster.Context, podNamespace string, listOpt metav1.ListOptions) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		pods, err := clusterContext.Client.Kube().CoreV1().Pods(podNamespace).List(ctx, listOpt)
		if err != nil {
			return fmt.Errorf("failed to list pods: %w", err)
		}
		if len(pods.Items) == 0 {
			return fmt.Errorf("no pods found in namespace %s matching %v", podNamespace, listOpt)
		}
		for _, pod := range pods.Items {
			if pod.Status.Phase != corev1.PodRunning || !podReady(&pod) {
				return fmt.Errorf("pod %s/%s phase=%s ready=%v", pod.Namespace, pod.Name, pod.Status.Phase, podReady(&pod))
			}
		}
		return nil
	})
}

func expectMatch(actual any, matcher types.GomegaMatcher, format string, args ...any) error {
	ok, err := matcher.Match(actual)
	if err != nil {
		return err
	}
	if ok {
		return nil
	}
	msg := fmt.Sprintf(format, args...)
	if failure := matcher.FailureMessage(actual); failure != "" {
		msg += ": " + failure
	}
	return fmt.Errorf("%s", msg)
}

func podReady(pod *corev1.Pod) bool {
	for _, cond := range pod.Status.Conditions {
		if cond.Type == corev1.PodReady && cond.Status == corev1.ConditionTrue {
			return true
		}
	}
	return false
}

func extractParentConditions(parents []gwv1.RouteParentStatus) [][]metav1.Condition {
	result := make([][]metav1.Condition, len(parents))
	for i, p := range parents {
		result[i] = p.Conditions
	}
	return result
}

func extractInferencePoolParentConditions(parents []inf.ParentStatus) [][]metav1.Condition {
	result := make([][]metav1.Condition, len(parents))
	for i, p := range parents {
		result[i] = p.Conditions
	}
	return result
}

func extractAgwPolicyAncestorConditions(ancestors []gwv1.PolicyAncestorStatus) [][]metav1.Condition {
	result := make([][]metav1.Condition, len(ancestors))
	for i, a := range ancestors {
		result[i] = a.Conditions
	}
	return result
}
