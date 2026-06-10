//go:build e2e

package e2e_test

import (
	"fmt"
	"testing"

	"istio.io/istio/pkg/test/util/retry"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
)

func TestAgwPolicyClearStaleStatus(tt *testing.T) {
	t := New(tt)
	t.Apply(manifest("policystatus", "policy-with-gw.yaml"))

	agwControllerName := base.AgentgatewayControllerName
	otherControllerName := "other-controller.example.com/controller"

	addAncestorStatus(t, "example-policy", base.Namespace, "other-gw", otherControllerName)

	assertAncestorStatuses(t, "gateway", map[string]bool{
		agwControllerName: true,
	})
	assertAncestorStatuses(t, "other-gw", map[string]bool{
		otherControllerName: true,
	})

	t.Apply(manifest("policystatus", "policy-with-missing-gw.yaml"))

	assertAncestorStatuses(t, "gateway", map[string]bool{
		agwControllerName: false,
	})
	assertAncestorStatuses(t, "other-gw", map[string]bool{
		otherControllerName: true,
	})
}

func addAncestorStatus(t base.Test, policyName, policyNamespace, gwName, controllerName string) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		policy := &agentgateway.AgentgatewayPolicy{}
		if err := t.TestInstallation.ClusterContext.ControllerClient.Get(
			t.Ctx,
			types.NamespacedName{Name: policyName, Namespace: policyNamespace},
			policy,
		); err != nil {
			return err
		}

		fakeStatus := gwv1.PolicyAncestorStatus{
			AncestorRef:    gwv1.ParentReference{Name: gwv1.ObjectName(gwName)},
			ControllerName: gwv1.GatewayController(controllerName),
			Conditions: []metav1.Condition{
				{
					Type:               string(agentgateway.PolicyConditionAccepted),
					Status:             metav1.ConditionTrue,
					Reason:             string(agentgateway.PolicyReasonValid),
					Message:            "Accepted by fake controller",
					LastTransitionTime: metav1.Now(),
				},
			},
		}

		policy.Status.Ancestors = append(policy.Status.Ancestors, fakeStatus)
		return t.TestInstallation.ClusterContext.ControllerClient.Status().Update(t.Ctx, policy)
	})
}

func assertAncestorStatuses(t base.Test, ancestorName string, expectedControllers map[string]bool) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		policy := &agentgateway.AgentgatewayPolicy{}
		if err := t.TestInstallation.ClusterContext.ControllerClient.Get(
			t.Ctx,
			types.NamespacedName{Name: "example-policy", Namespace: base.Namespace},
			policy,
		); err != nil {
			return err
		}

		foundControllers := make(map[string]bool)
		for _, ancestor := range policy.Status.Ancestors {
			if string(ancestor.AncestorRef.Name) == ancestorName {
				foundControllers[string(ancestor.ControllerName)] = true
			}
		}

		for controller, shouldExist := range expectedControllers {
			exists := foundControllers[controller]
			if exists != shouldExist {
				return fmt.Errorf("controller %s exists=%v, want %v for ancestor %s", controller, exists, shouldExist, ancestorName)
			}
		}
		return nil
	})
}
