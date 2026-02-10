//go:build e2e

package policystatus

import (
	"context"

	"github.com/onsi/gomega"
	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/shared"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
	"github.com/kgateway-dev/kgateway/v2/test/helpers"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

// testingSuite is a suite of tests for testing AgentgatewayPolicy status behavior
type testingSuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, testCases),
	}
}

// TestAgwPolicyClearStaleStatus verifies that stale status is cleared when targetRef becomes invalid
func (s *testingSuite) TestAgwPolicyClearStaleStatus() {
	agwControllerName := wellknown.DefaultAgwControllerName
	otherControllerName := "other-controller.example.com/controller"

	// Add fake ancestor status from another controller
	s.addAncestorStatus("example-policy", "agentgateway-base", "other-gw", otherControllerName)

	// Verify both agw and other controller statuses exist
	s.assertAncestorStatuses("gateway", map[string]bool{
		agwControllerName: true,
	})
	s.assertAncestorStatuses("other-gw", map[string]bool{
		otherControllerName: true,
	})

	// Apply policy with missing gateway target
	err := s.TestInstallation.Actions.Kubectl().ApplyFile(
		s.Ctx,
		policyWithMissingGwManifest,
	)
	s.Require().NoError(err)

	// Verify agw status cleared, other remains
	s.assertAncestorStatuses("gateway", map[string]bool{
		agwControllerName: false,
	})
	s.assertAncestorStatuses("other-gw", map[string]bool{
		otherControllerName: true,
	})
}

func (s *testingSuite) addAncestorStatus(policyName, policyNamespace, gwName, controllerName string) {
	currentTimeout, pollingInterval := helpers.GetTimeouts()
	s.TestInstallation.Assertions.Gomega.Eventually(func(g gomega.Gomega) {
		policy := &agentgateway.AgentgatewayPolicy{}
		err := s.TestInstallation.ClusterContext.Client.Get(
			s.Ctx,
			types.NamespacedName{Name: policyName, Namespace: policyNamespace},
			policy,
		)
		g.Expect(err).NotTo(gomega.HaveOccurred())

		// Add fake ancestor status
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
		err = s.TestInstallation.ClusterContext.Client.Status().Update(s.Ctx, policy)
		g.Expect(err).NotTo(gomega.HaveOccurred())
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

func (s *testingSuite) assertAncestorStatuses(ancestorName string, expectedControllers map[string]bool) {
	currentTimeout, pollingInterval := helpers.GetTimeouts()
	s.TestInstallation.Assertions.Gomega.Eventually(func(g gomega.Gomega) {
		policy := &agentgateway.AgentgatewayPolicy{}
		err := s.TestInstallation.ClusterContext.Client.Get(
			s.Ctx,
			types.NamespacedName{Name: "example-policy", Namespace: "agentgateway-base"},
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
