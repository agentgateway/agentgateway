//go:build e2e

package assertions

import (
	"context"
	"fmt"
	"time"

	"github.com/onsi/ginkgo/v2"
	"github.com/onsi/gomega"
	"github.com/onsi/gomega/gstruct"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/client"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwv1a2 "sigs.k8s.io/gateway-api/apis/v1alpha2"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
	"github.com/agentgateway/agentgateway/controller/test/helpers"
)

// ConditionHandler lets callers teach EventuallyCondition about types defined
// outside this package. Return handled=true when obj's type matched even if
// the assertion failed — failures must go through g so polling can retry.
type ConditionHandler func(g gomega.Gomega, obj client.Object, condType string) (handled bool)

// WithConditionHandlers registers handlers consulted by EventuallyCondition
// before its built-in type switch.
func (p *Provider) WithConditionHandlers(h ...ConditionHandler) *Provider {
	p.conditionHandlers = append(p.conditionHandlers, h...)
	return p
}

// EventuallyGatewayAddress asserts that eventually at least one of the HTTPRoute's route parent statuses contains
// the given message substring.
func (p *Provider) EventuallyGatewayAddress(
	ctx context.Context,
	gatewayName string,
	gatewayNamespace string,
	timeout ...time.Duration,
) string {
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	var addr string
	p.Gomega.Eventually(func(g gomega.Gomega) {
		gw := &gwv1.Gateway{}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: gatewayName, Namespace: gatewayNamespace}, gw)
		g.Expect(err).NotTo(gomega.HaveOccurred(), "can get gateway")
		if len(gw.Status.Addresses) == 0 {
			g.Expect(true).To(gomega.BeFalse(), "gateway is not ready")
		}
		addr = gw.Status.Addresses[0].Value
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
	return addr
}

// EventuallyHTTPRouteStatusContainsMessage asserts that eventually at least one of the HTTPRoute's route parent statuses contains
// the given message substring.
func (p *Provider) EventuallyHTTPRouteStatusContainsMessage(
	ctx context.Context,
	routeName string,
	routeNamespace string,
	message string,
	timeout ...time.Duration,
) {
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		matcher := matchers.HaveKubeGatewayRouteStatus(&matchers.KubeGatewayRouteStatus{
			Custom: gstruct.MatchFields(gstruct.IgnoreExtras, gstruct.Fields{
				"Parents": gomega.ContainElement(gstruct.MatchFields(gstruct.IgnoreExtras, gstruct.Fields{
					"Conditions": gomega.ContainElement(gstruct.MatchFields(gstruct.IgnoreExtras, gstruct.Fields{
						"Message": matchers.ContainSubstrings([]string{message}),
					})),
				})),
			}),
		})

		route := &gwv1.HTTPRoute{}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: routeName, Namespace: routeNamespace}, route)
		g.Expect(err).NotTo(gomega.HaveOccurred(), "can get httproute")
		g.Expect(route.Status.RouteStatus).To(gomega.HaveValue(matcher), fmt.Sprintf("Full status: %+v", route.Status))
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

// EventuallyHTTPRouteStatusContainsReason asserts that eventually at least one of the HTTPRoute's route parent statuses contains
// the given reason substring.
func (p *Provider) EventuallyHTTPRouteStatusContainsReason(
	ctx context.Context,
	routeName string,
	routeNamespace string,
	reason string,
	timeout ...time.Duration,
) {
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		matcher := matchers.HaveKubeGatewayRouteStatus(&matchers.KubeGatewayRouteStatus{
			Custom: gstruct.MatchFields(gstruct.IgnoreExtras, gstruct.Fields{
				"Parents": gomega.ContainElement(gstruct.MatchFields(gstruct.IgnoreExtras, gstruct.Fields{
					"Conditions": gomega.ContainElement(gstruct.MatchFields(gstruct.IgnoreExtras, gstruct.Fields{
						"Reason": matchers.ContainSubstrings([]string{reason}),
					})),
				})),
			}),
		})

		route := &gwv1.HTTPRoute{
			ObjectMeta: metav1.ObjectMeta{
				Name:      routeName,
				Namespace: routeNamespace,
			},
		}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: routeName, Namespace: routeNamespace}, route)
		g.Expect(err).NotTo(gomega.HaveOccurred(), "can get httproute")
		g.Expect(route.Status.RouteStatus).To(gomega.HaveValue(matcher), fmt.Sprintf("Full status: %+v", route.Status))
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

// EventuallyGatewayCondition checks the provided Gateway condition is set to expect.
func (p *Provider) EventuallyGatewayCondition(
	ctx context.Context,
	gatewayName string,
	gatewayNamespace string,
	cond gwv1.GatewayConditionType,
	expect metav1.ConditionStatus,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		gw := &gwv1.Gateway{}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: gatewayName, Namespace: gatewayNamespace}, gw)
		g.Expect(err).NotTo(gomega.HaveOccurred(), fmt.Sprintf("failed to get Gateway %s/%s", gatewayNamespace, gatewayName))
		g.Expect(gw.Status.Conditions).To(matchers.HaveCondition(string(cond), expect))
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

// EventuallyGatewayListenerAttachedRoutes checks the provided Gateway contains the expected attached routes for the listener.
func (p *Provider) EventuallyGatewayListenerAttachedRoutes(
	ctx context.Context,
	gatewayName string,
	gatewayNamespace string,
	listener gwv1.SectionName,
	routes int32,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		gw := &gwv1.Gateway{}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: gatewayName, Namespace: gatewayNamespace}, gw)
		g.Expect(err).NotTo(gomega.HaveOccurred(), fmt.Sprintf("failed to get Gateway %s/%s", gatewayNamespace, gatewayName))

		var found bool
		for _, l := range gw.Status.Listeners {
			if l.Name == listener {
				g.Expect(l.AttachedRoutes).To(gomega.Equal(routes),
					fmt.Sprintf("%v listener does not contain %d attached routes for Gateway %s/%s. Full status: %+v",
						listener, routes, gatewayNamespace, gatewayName, gw.Status))
				found = true
				break
			}
		}
		g.Expect(found).To(gomega.BeTrue(), fmt.Sprintf("listener %s not found in Gateway %s/%s", listener, gatewayNamespace, gatewayName))
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

func (p *Provider) EventuallyGatewayStatus(
	ctx context.Context,
	name string,
	namespace string,
	status gwv1.GatewayStatus,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		gw := &gwv1.Gateway{}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: name, Namespace: namespace}, gw)
		g.Expect(err).NotTo(gomega.HaveOccurred(), fmt.Sprintf("failed to get gateway %s/%s", namespace, name))

		for _, expected := range status.Conditions {
			condition := GetConditionByType(gw.Status.Conditions, expected.Type)
			g.Expect(condition).NotTo(gomega.BeNil(), fmt.Sprintf("%v condition not found for gateway %s/%s. Full status: %+v", expected.Type, namespace, name, gw.Status))
			g.Expect(condition.Status).To(gomega.Equal(expected.Status), fmt.Sprintf("%v status is not %v for gateway %s/%s. Full status: %+v", expected, expected.Status, namespace, name, gw.Status))
			if expected.Reason != "" {
				g.Expect(condition.Reason).To(gomega.Equal(expected.Reason), fmt.Sprintf("%v reason is not %v for gateway %s/%s. Full status: %+v", expected, expected.Reason, namespace, name, gw.Status))
			}
		}

		for _, expectedListener := range status.Listeners {
			listenerStatus := getListenerStatus(gw.Status.Listeners, string(expectedListener.Name))
			g.Expect(listenerStatus).NotTo(gomega.BeNil(), fmt.Sprintf("%v listener status not found for listener %s. Full status: %+v", expectedListener.Name, expectedListener.Name, gw.Status))
			if expectedListener.AttachedRoutes != 0 {
				g.Expect(listenerStatus.AttachedRoutes).To(gomega.Equal(expectedListener.AttachedRoutes), fmt.Sprintf("%v condition is not %v for listener %s. Full status: %+v", expectedListener, expectedListener.AttachedRoutes, expectedListener.Name, gw.Status))
			}
			if expectedListener.SupportedKinds != nil {
				g.Expect(listenerStatus.SupportedKinds).To(gomega.ContainElements(expectedListener.SupportedKinds), fmt.Sprintf("%v condition is not %v for listener %s. Full status: %+v", expectedListener, expectedListener.SupportedKinds, expectedListener.Name, gw.Status))
			}

			for _, expected := range expectedListener.Conditions {
				condition := GetConditionByType(listenerStatus.Conditions, expected.Type)
				g.Expect(condition).NotTo(gomega.BeNil(), fmt.Sprintf("%v condition not found for listener %s. Full status: %+v", expected, expectedListener.Name, gw.Status))
				g.Expect(condition.Status).To(gomega.Equal(expected.Status), fmt.Sprintf("%v condition is not %v for listener %s. Full status: %+v", expected, expected.Status, expectedListener.Name, gw.Status))
				if expected.Reason != "" {
					g.Expect(condition.Reason).To(gomega.Equal(expected.Reason), fmt.Sprintf("%v condition is not %v for listener %s. Full status: %+v", expected, expected.Reason, expectedListener.Name, gw.Status))
				}
			}
		}
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

// extractParentConditions extracts conditions from route parent statuses.
func extractParentConditions(parents []gwv1.RouteParentStatus) [][]metav1.Condition {
	result := make([][]metav1.Condition, len(parents))
	for i, p := range parents {
		result[i] = p.Conditions
	}
	return result
}

// EventuallyHTTPRouteCondition checks that provided HTTPRoute condition is set to expect.
func (p *Provider) EventuallyHTTPRouteCondition(
	ctx context.Context,
	routeName string,
	routeNamespace string,
	cond gwv1.RouteConditionType,
	expect metav1.ConditionStatus,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		route := &gwv1.HTTPRoute{}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: routeName, Namespace: routeNamespace}, route)
		g.Expect(err).NotTo(gomega.HaveOccurred(), fmt.Sprintf("failed to get HTTPRoute %s/%s", routeNamespace, routeName))
		g.Expect(extractParentConditions(route.Status.Parents)).To(matchers.HaveAnyParentCondition(string(cond), expect))
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

// EventuallyTCPRouteCondition checks that provided TCPRoute condition is set to expect.
func (p *Provider) EventuallyTCPRouteCondition(
	ctx context.Context,
	routeName string,
	routeNamespace string,
	cond gwv1.RouteConditionType,
	expect metav1.ConditionStatus,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		route := &gwv1a2.TCPRoute{}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: routeName, Namespace: routeNamespace}, route)
		g.Expect(err).NotTo(gomega.HaveOccurred(), fmt.Sprintf("failed to get TCPRoute %s/%s", routeNamespace, routeName))
		g.Expect(extractParentConditions(route.Status.Parents)).To(matchers.HaveAnyParentCondition(string(cond), expect))
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

// EventuallyTLSRouteCondition checks that provided TLSRoute condition is set to expect.
func (p *Provider) EventuallyTLSRouteCondition(
	ctx context.Context,
	routeName string,
	routeNamespace string,
	cond gwv1.RouteConditionType,
	expect metav1.ConditionStatus,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		route := &gwv1.TLSRoute{}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: routeName, Namespace: routeNamespace}, route)
		g.Expect(err).NotTo(gomega.HaveOccurred(), fmt.Sprintf("failed to get TLSRoute %s/%s", routeNamespace, routeName))
		g.Expect(extractParentConditions(route.Status.Parents)).To(matchers.HaveAnyParentCondition(string(cond), expect))
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

// EventuallyGRPCRouteCondition checks that provided GRPCRoute condition is set to expect.
func (p *Provider) EventuallyGRPCRouteCondition(
	ctx context.Context,
	routeName string,
	routeNamespace string,
	cond gwv1.RouteConditionType,
	expect metav1.ConditionStatus,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		route := &gwv1.GRPCRoute{}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: routeName, Namespace: routeNamespace}, route)
		g.Expect(err).NotTo(gomega.HaveOccurred(), fmt.Sprintf("failed to get GRPCRoute %s/%s", routeNamespace, routeName))
		g.Expect(extractParentConditions(route.Status.Parents)).To(matchers.HaveAnyParentCondition(string(cond), expect))
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

// extractInferencePoolParentConditions extracts conditions from InferencePool parent statuses.
func extractInferencePoolParentConditions(parents []inf.ParentStatus) [][]metav1.Condition {
	result := make([][]metav1.Condition, len(parents))
	for i, p := range parents {
		result[i] = p.Conditions
	}
	return result
}

// EventuallyInferencePoolCondition checks that the specified InferencePool condition
// eventually has the desired status on any parent managed by agentgateway.
func (p *Provider) EventuallyInferencePoolCondition(
	ctx context.Context,
	poolName string,
	poolNamespace string,
	cond inf.InferencePoolConditionType,
	expect metav1.ConditionStatus,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		pool := &inf.InferencePool{}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: poolName, Namespace: poolNamespace}, pool)
		g.Expect(err).NotTo(gomega.HaveOccurred(), fmt.Sprintf("failed to get InferencePool %s/%s", poolNamespace, poolName))
		g.Expect(extractInferencePoolParentConditions(pool.Status.Parents)).To(matchers.HaveAnyParentCondition(string(cond), expect))
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

// Helper function to retrieve a condition by type from a list of conditions.
func GetConditionByType(conditions []metav1.Condition, conditionType string) *metav1.Condition {
	for i := range conditions {
		if conditions[i].Type == conditionType {
			return &conditions[i]
		}
	}
	return nil
}

func (p *Provider) EventuallyListenerSetStatus(
	ctx context.Context,
	name string,
	namespace string,
	status gwv1.ListenerSetStatus,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		ls := &gwv1.ListenerSet{}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: name, Namespace: namespace}, ls)
		g.Expect(err).NotTo(gomega.HaveOccurred(), fmt.Sprintf("failed to get listenerset %s/%s", namespace, name))

		for _, expected := range status.Conditions {
			condition := GetConditionByType(ls.Status.Conditions, expected.Type)
			g.Expect(condition).NotTo(gomega.BeNil(), fmt.Sprintf("%v condition not found for listenerset %s/%s. Full status: %+v", expected.Type, namespace, name, ls.Status))
			g.Expect(condition.Status).To(gomega.Equal(expected.Status), fmt.Sprintf("%v status is not %v for listenerset %s/%s. Full status: %+v", expected, expected.Status, namespace, name, ls.Status))
			if expected.Reason != "" {
				g.Expect(condition.Reason).To(gomega.Equal(expected.Reason), fmt.Sprintf("%v reason is not %v for listenerset %s/%s. Full status: %+v", expected, expected.Reason, namespace, name, ls.Status))
			}
		}

		for _, expectedListener := range status.Listeners {
			listenerStatus := getListenerEntryStatus(ls.Status.Listeners, string(expectedListener.Name))
			g.Expect(listenerStatus).NotTo(gomega.BeNil(), fmt.Sprintf("%v listener status not found for listener %s. Full status: %+v", expectedListener.Name, expectedListener.Name, ls.Status))
			if expectedListener.AttachedRoutes != 0 {
				g.Expect(listenerStatus.AttachedRoutes).To(gomega.Equal(expectedListener.AttachedRoutes), fmt.Sprintf("%v condition is not %v for listener %s. Full status: %+v", expectedListener, expectedListener.AttachedRoutes, expectedListener.Name, ls.Status))
			}
			if expectedListener.SupportedKinds != nil {
				g.Expect(listenerStatus.SupportedKinds).To(gomega.ContainElements(expectedListener.SupportedKinds), fmt.Sprintf("%v condition is not %v for listener %s. Full status: %+v", expectedListener, expectedListener.SupportedKinds, expectedListener.Name, ls.Status))
			}

			for _, expected := range expectedListener.Conditions {
				condition := GetConditionByType(listenerStatus.Conditions, expected.Type)
				g.Expect(condition).NotTo(gomega.BeNil(), fmt.Sprintf("%v condition not found for listener %s. Full status: %+v", expected, expectedListener.Name, ls.Status))
				g.Expect(condition.Status).To(gomega.Equal(expected.Status), fmt.Sprintf("%v condition is not %v for listener %s. Full status: %+v", expected, expected.Status, expectedListener.Name, ls.Status))
				if expected.Reason != "" {
					g.Expect(condition.Reason).To(gomega.Equal(expected.Reason), fmt.Sprintf("%v condition is not %v for listener %s. Full status: %+v", expected, expected.Reason, expectedListener.Name, ls.Status))
				}
			}
		}
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

func getListenerEntryStatus(listeners []gwv1.ListenerEntryStatus, name string) *gwv1.ListenerEntryStatus {
	for i := range listeners {
		if string(listeners[i].Name) == name {
			return &listeners[i]
		}
	}
	return nil
}

func getListenerStatus(listeners []gwv1.ListenerStatus, name string) *gwv1.ListenerStatus {
	for i := range listeners {
		if string(listeners[i].Name) == name {
			return &listeners[i]
		}
	}
	return nil
}

// EventuallyAgwBackendCondition checks that provided AgentgatewayBackend condition is set to expect.
func (p *Provider) EventuallyAgwBackendCondition(
	ctx context.Context,
	name string,
	namespace string,
	condition string,
	expect metav1.ConditionStatus,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		backend := &agentgateway.AgentgatewayBackend{}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: name, Namespace: namespace}, backend)
		g.Expect(err).NotTo(gomega.HaveOccurred(), fmt.Sprintf("failed to get AgentgatewayBackend %s/%s", namespace, name))
		g.Expect(backend.Status.Conditions).To(matchers.HaveCondition(condition, expect))
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

// extractAgwPolicyAncestorConditions extracts conditions from AgentgatewayPolicy ancestor statuses.
func extractAgwPolicyAncestorConditions(ancestors []gwv1.PolicyAncestorStatus) [][]metav1.Condition {
	result := make([][]metav1.Condition, len(ancestors))
	for i, a := range ancestors {
		result[i] = a.Conditions
	}
	return result
}

// EventuallyAgwPolicyCondition checks that provided AgentgatewayPolicy condition is set to expect.
func (p *Provider) EventuallyAgwPolicyCondition(
	ctx context.Context,
	name string,
	namespace string,
	condType string,
	expect metav1.ConditionStatus,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		policy := &agentgateway.AgentgatewayPolicy{}
		err := p.clusterContext.Client.Get(ctx, types.NamespacedName{Name: name, Namespace: namespace}, policy)
		g.Expect(err).NotTo(gomega.HaveOccurred(), fmt.Sprintf("failed to get AgentgatewayPolicy %s/%s", namespace, name))
		g.Expect(extractAgwPolicyAncestorConditions(policy.Status.Ancestors)).To(matchers.HaveAnyAncestorCondition(condType, expect))
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

// EventuallyAccepted polls until obj reports Accepted=True.
func (p *Provider) EventuallyAccepted(
	ctx context.Context,
	obj client.Object,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	p.EventuallyCondition(ctx, obj, "Accepted", timeout...)
}

// EventuallyAllAccepted runs EventuallyAccepted for each object in order.
func (p *Provider) EventuallyAllAccepted(
	ctx context.Context,
	objs []client.Object,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	for _, obj := range objs {
		p.EventuallyAccepted(ctx, obj, timeout...)
	}
}

// EventuallyCondition polls until obj reports condType=True. Dispatches on the
// Go type of obj to know where conditions live (top-level vs parents vs ancestors).
func (p *Provider) EventuallyCondition(
	ctx context.Context,
	obj client.Object,
	condType string,
	timeout ...time.Duration,
) {
	ginkgo.GinkgoHelper()
	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)
	key := client.ObjectKeyFromObject(obj)
	kind := fmt.Sprintf("%T", obj)

	p.Gomega.Eventually(func(g gomega.Gomega) {
		err := p.clusterContext.Client.Get(ctx, key, obj)
		g.Expect(err).NotTo(gomega.HaveOccurred(), fmt.Sprintf("failed to get %s %s/%s", kind, key.Namespace, key.Name))

		for _, h := range p.conditionHandlers {
			if h(g, obj, condType) {
				return
			}
		}

		switch o := obj.(type) {
		case *gwv1.Gateway:
			g.Expect(o.Status.Conditions).To(matchers.HaveCondition(condType, metav1.ConditionTrue),
				fmt.Sprintf("Gateway %s/%s status: %+v", key.Namespace, key.Name, o.Status))
		case *gwv1.HTTPRoute:
			g.Expect(extractParentConditions(o.Status.Parents)).To(matchers.HaveAnyParentCondition(condType, metav1.ConditionTrue),
				fmt.Sprintf("HTTPRoute %s/%s status: %+v", key.Namespace, key.Name, o.Status))
		case *gwv1a2.TCPRoute:
			g.Expect(extractParentConditions(o.Status.Parents)).To(matchers.HaveAnyParentCondition(condType, metav1.ConditionTrue),
				fmt.Sprintf("TCPRoute %s/%s status: %+v", key.Namespace, key.Name, o.Status))
		case *gwv1.TLSRoute:
			g.Expect(extractParentConditions(o.Status.Parents)).To(matchers.HaveAnyParentCondition(condType, metav1.ConditionTrue),
				fmt.Sprintf("TLSRoute %s/%s status: %+v", key.Namespace, key.Name, o.Status))
		case *gwv1.GRPCRoute:
			g.Expect(extractParentConditions(o.Status.Parents)).To(matchers.HaveAnyParentCondition(condType, metav1.ConditionTrue),
				fmt.Sprintf("GRPCRoute %s/%s status: %+v", key.Namespace, key.Name, o.Status))
		case *gwv1.ListenerSet:
			g.Expect(o.Status.Conditions).To(matchers.HaveCondition(condType, metav1.ConditionTrue),
				fmt.Sprintf("ListenerSet %s/%s status: %+v", key.Namespace, key.Name, o.Status))
		case *agentgateway.AgentgatewayBackend:
			g.Expect(o.Status.Conditions).To(matchers.HaveCondition(condType, metav1.ConditionTrue),
				fmt.Sprintf("AgentgatewayBackend %s/%s status: %+v", key.Namespace, key.Name, o.Status))
		case *agentgateway.AgentgatewayPolicy:
			g.Expect(extractAgwPolicyAncestorConditions(o.Status.Ancestors)).To(matchers.HaveAnyAncestorCondition(condType, metav1.ConditionTrue),
				fmt.Sprintf("AgentgatewayPolicy %s/%s status: %+v", key.Namespace, key.Name, o.Status))
		case *inf.InferencePool:
			g.Expect(extractInferencePoolParentConditions(o.Status.Parents)).To(matchers.HaveAnyParentCondition(condType, metav1.ConditionTrue),
				fmt.Sprintf("InferencePool %s/%s status: %+v", key.Namespace, key.Name, o.Status))
		default:
			gomega.StopTrying(fmt.Sprintf(
				"EventuallyCondition: unsupported type %s — add a case here or register a ConditionHandler",
				kind,
			)).Now()
		}
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}
