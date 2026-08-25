//go:build e2e

package e2e_test

import (
	"fmt"
	"testing"

	"istio.io/istio/pkg/test/util/retry"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
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
		policy, status, err := getAgwPolicyStatus(t, policyName, policyNamespace)
		if err != nil {
			return err
		}

		fakeStatus := gwv1.PolicyAncestorStatus{
			AncestorRef:    gwv1.ParentReference{Name: gwv1.ObjectName(gwName)},
			ControllerName: gwv1.GatewayController(controllerName),
			Conditions: []metav1.Condition{
				{
					Type:               agentgateway.PolicyConditionAccepted,
					Status:             metav1.ConditionTrue,
					Reason:             agentgateway.PolicyReasonValid,
					Message:            "Accepted by fake controller",
					LastTransitionTime: metav1.Now(),
				},
			},
		}

		status.Ancestors = append(status.Ancestors, fakeStatus)
		if err := setAgwPolicyAncestors(policy, status.Ancestors); err != nil {
			return err
		}
		return t.TestInstallation.ClusterContext.ControllerClient.Status().Update(t.Ctx, policy)
	})
}

func assertAncestorStatuses(t base.Test, ancestorName string, expectedControllers map[string]bool) {
	t.Helper()
	retry.UntilSuccessOrFail(t, func() error {
		_, status, err := getAgwPolicyStatus(t, "example-policy", base.Namespace)
		if err != nil {
			return err
		}

		foundControllers := make(map[string]bool)
		for _, ancestor := range status.Ancestors {
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

func getAgwPolicyStatus(t base.Test, name, namespace string) (*unstructured.Unstructured, gwv1.PolicyStatus, error) {
	t.Helper()
	gvk := t.AgentgatewayPolicyGVK()
	policy := &unstructured.Unstructured{}
	policy.SetGroupVersionKind(gvk)
	if err := t.TestInstallation.ClusterContext.ControllerClient.Get(
		t.Ctx,
		types.NamespacedName{Name: name, Namespace: namespace},
		policy,
	); err != nil {
		return nil, gwv1.PolicyStatus{}, fmt.Errorf("get %s %s/%s: %w", gvk.Kind, namespace, name, err)
	}

	statusData, found, err := unstructured.NestedMap(policy.Object, "status")
	if err != nil {
		return nil, gwv1.PolicyStatus{}, fmt.Errorf("read %s %s/%s status: %w", gvk.Kind, namespace, name, err)
	}
	if !found {
		return nil, gwv1.PolicyStatus{}, fmt.Errorf("%s %s/%s status is not set", gvk.Kind, namespace, name)
	}
	var status gwv1.PolicyStatus
	if err := runtime.DefaultUnstructuredConverter.FromUnstructured(statusData, &status); err != nil {
		return nil, gwv1.PolicyStatus{}, fmt.Errorf("decode %s %s/%s status: %w", gvk.Kind, namespace, name, err)
	}
	return policy, status, nil
}

func setAgwPolicyAncestors(policy *unstructured.Unstructured, ancestors []gwv1.PolicyAncestorStatus) error {
	kind := policy.GetKind()
	statusData, err := runtime.DefaultUnstructuredConverter.ToUnstructured(&gwv1.PolicyStatus{Ancestors: ancestors})
	if err != nil {
		return fmt.Errorf("encode %s ancestors: %w", kind, err)
	}
	ancestorData, found, err := unstructured.NestedSlice(statusData, "ancestors")
	if err != nil {
		return fmt.Errorf("read encoded %s ancestors: %w", kind, err)
	}
	if !found {
		return fmt.Errorf("encoded %s status has no ancestors", kind)
	}
	if err := unstructured.SetNestedSlice(policy.Object, ancestorData, "status", "ancestors"); err != nil {
		return fmt.Errorf("set %s ancestors: %w", kind, err)
	}
	return nil
}
