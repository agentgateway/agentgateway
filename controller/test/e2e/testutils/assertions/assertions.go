//go:build e2e

package assertions

import (
	"context"
	"time"

	"github.com/onsi/gomega/types"
	"istio.io/istio/pkg/test"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/cluster"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/install"
)

func EventuallyObjectsExist(t Test, objects ...client.Object) {
	providerFor(t).EventuallyObjectsExist(t.E2EContext(), objects...)
}

func EventuallyPodsRunning(t Test, podNamespace string, listOpt metav1.ListOptions, timeout ...time.Duration) {
	providerFor(t).EventuallyPodsRunning(t.E2EContext(), podNamespace, listOpt, timeout...)
}

func EventuallyPodsMatches(t Test, podNamespace string, listOpt metav1.ListOptions, matcher types.GomegaMatcher, timeout ...time.Duration) {
	providerFor(t).EventuallyPodsMatches(t.E2EContext(), podNamespace, listOpt, matcher, timeout...)
}

func EventuallyGatewayCondition(t Test, gatewayName string, gatewayNamespace string, cond gwv1.GatewayConditionType, expect metav1.ConditionStatus, timeout ...time.Duration) {
	providerFor(t).EventuallyGatewayCondition(t.E2EContext(), gatewayName, gatewayNamespace, cond, expect, timeout...)
}

func EventuallyGatewayListenerAttachedRoutes(t Test, gatewayName string, gatewayNamespace string, listener gwv1.SectionName, routes int32, timeout ...time.Duration) {
	providerFor(t).EventuallyGatewayListenerAttachedRoutes(t.E2EContext(), gatewayName, gatewayNamespace, listener, routes, timeout...)
}

func EventuallyHTTPRouteCondition(t Test, routeName string, routeNamespace string, cond gwv1.RouteConditionType, expect metav1.ConditionStatus, timeout ...time.Duration) {
	providerFor(t).EventuallyHTTPRouteCondition(t.E2EContext(), routeName, routeNamespace, cond, expect, timeout...)
}

func EventuallyGRPCRouteCondition(t Test, routeName string, routeNamespace string, cond gwv1.RouteConditionType, expect metav1.ConditionStatus, timeout ...time.Duration) {
	providerFor(t).EventuallyGRPCRouteCondition(t.E2EContext(), routeName, routeNamespace, cond, expect, timeout...)
}

func EventuallyInferencePoolCondition(t Test, poolName string, poolNamespace string, cond inf.InferencePoolConditionType, expect metav1.ConditionStatus, timeout ...time.Duration) {
	providerFor(t).EventuallyInferencePoolCondition(t.E2EContext(), poolName, poolNamespace, cond, expect, timeout...)
}

func EventuallyAgwBackendCondition(t Test, name string, namespace string, condition string, expect metav1.ConditionStatus, timeout ...time.Duration) {
	providerFor(t).EventuallyAgwBackendCondition(t.E2EContext(), name, namespace, condition, expect, timeout...)
}

func EventuallyAgwPolicyCondition(t Test, name string, namespace string, condType string, expect metav1.ConditionStatus, timeout ...time.Duration) {
	providerFor(t).EventuallyAgwPolicyCondition(t.E2EContext(), name, namespace, condType, expect, timeout...)
}

func EventuallyGatewayAddress(t test.Failer, ctx context.Context, clusterContext *cluster.Context, gatewayName string, gatewayNamespace string, timeout ...time.Duration) string {
	return newProvider(t, clusterContext, nil).EventuallyGatewayAddress(ctx, gatewayName, gatewayNamespace, timeout...)
}

func EventuallyGatewayInstallSucceeded(t test.Failer, ctx context.Context, clusterContext *cluster.Context, installContext *install.Context) {
	newProvider(t, clusterContext, installContext).EventuallyGatewayInstallSucceeded(ctx)
}

func EventuallyGatewayUninstallSucceeded(t test.Failer, ctx context.Context, clusterContext *cluster.Context, installContext *install.Context) {
	newProvider(t, clusterContext, installContext).EventuallyGatewayUninstallSucceeded(ctx)
}
