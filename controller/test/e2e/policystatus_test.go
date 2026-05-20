//go:build e2e

package e2e_test

import (
	"testing"

	"github.com/onsi/gomega"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/helpers"
)

func TestAgwPolicyClearStaleStatus(t *testing.T) {
	agw := New(t)
	agw.Apply(manifest("policystatus", "policy-with-gw.yaml"))

	agwControllerName := wellknown.DefaultAgwControllerName
	otherControllerName := "other-controller.example.com/controller"

	addAncestorStatus(t, agw, "example-policy", base.Namespace, "other-gw", otherControllerName)

	assertAncestorStatuses(t, agw, "gateway", map[string]bool{
		agwControllerName: true,
	})
	assertAncestorStatuses(t, agw, "other-gw", map[string]bool{
		otherControllerName: true,
	})

	agw.Apply(manifest("policystatus", "policy-with-missing-gw.yaml"))

	assertAncestorStatuses(t, agw, "gateway", map[string]bool{
		agwControllerName: false,
	})
	assertAncestorStatuses(t, agw, "other-gw", map[string]bool{
		otherControllerName: true,
	})
}

func addAncestorStatus(t *testing.T, agw *base.BaseTestingSuite, policyName, policyNamespace, gwName, controllerName string) {
	t.Helper()
	currentTimeout, pollingInterval := helpers.GetTimeouts()
	agw.TestInstallation.AssertionsT(t).Gomega.Eventually(func(g gomega.Gomega) {
		policy := &agentgateway.AgentgatewayPolicy{}
		err := agw.TestInstallation.ClusterContext.Client.Get(
			agw.Ctx,
			types.NamespacedName{Name: policyName, Namespace: policyNamespace},
			policy,
		)
		g.Expect(err).NotTo(gomega.HaveOccurred())

		fakeStatus := gwv1.PolicyAncestorStatus{
			AncestorRef:    gwv1.ParentReference{Name: gwv1.ObjectName(gwName)},
			ControllerName: gwv1.GatewayController(controllerName),
			Conditions: []metav1.Condition{
				{
					Type:               string(shared.PolicyConditionAccepted),
					Status:             metav1.ConditionTrue,
					Reason:             string(shared.PolicyReasonValid),
					Message:            "Accepted by fake controller",
					LastTransitionTime: metav1.Now(),
				},
			},
		}

		policy.Status.Ancestors = append(policy.Status.Ancestors, fakeStatus)
		err = agw.TestInstallation.ClusterContext.Client.Status().Update(agw.Ctx, policy)
		g.Expect(err).NotTo(gomega.HaveOccurred())
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

func assertAncestorStatuses(t *testing.T, agw *base.BaseTestingSuite, ancestorName string, expectedControllers map[string]bool) {
	t.Helper()
	currentTimeout, pollingInterval := helpers.GetTimeouts()
	agw.TestInstallation.AssertionsT(t).Gomega.Eventually(func(g gomega.Gomega) {
		policy := &agentgateway.AgentgatewayPolicy{}
		err := agw.TestInstallation.ClusterContext.Client.Get(
			agw.Ctx,
			types.NamespacedName{Name: "example-policy", Namespace: base.Namespace},
			policy,
		)
		g.Expect(err).NotTo(gomega.HaveOccurred())

		foundControllers := make(map[string]bool)
		for _, ancestor := range policy.Status.Ancestors {
			if string(ancestor.AncestorRef.Name) == ancestorName {
				foundControllers[string(ancestor.ControllerName)] = true
			}
		}

		for controller, shouldExist := range expectedControllers {
			exists := foundControllers[controller]
			if shouldExist {
				g.Expect(exists).To(gomega.BeTrue(), "Expected controller %s to exist in status", controller)
			} else {
				g.Expect(exists).To(gomega.BeFalse(), "Expected controller %s to not exist in status", controller)
			}
		}
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}
