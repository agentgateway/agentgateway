//go:build e2e

package agentgateway

import (
	"net/http"
	"testing"

	"github.com/onsi/gomega"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	"github.com/agentgateway/agentgateway/controller/test/helpers"
)

func TestBackendTLSPolicyAndStatus(t *testing.T) {
	agw := New(t)
	agw.ApplyConfig(base.TestCase{
		Manifests: []string{
			manifest("backendtls", "configmap.yaml"),
			manifest("backendtls", "base.yaml"),
		},
		MinGwApiVersion: base.GwApiRequireBackendTLSPolicy,
	})

	backendTLSPolicy := &gwv1.BackendTLSPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "tls-policy",
			Namespace: base.Namespace,
		},
	}
	err := agw.TestInstallation.ClusterContext.Client.Get(agw.Ctx, client.ObjectKeyFromObject(backendTLSPolicy), backendTLSPolicy)
	agw.Require().NoError(err)

	agw.Send("example.com", base.ExpectOK())
	agw.Send("example2.com", base.ExpectOK())
	agw.Send("foo.com", base.Expect(http.StatusMovedPermanently))

	assertBackendTLSPolicyStatus(t, agw, backendTLSPolicy, metav1.Condition{
		Type:               string(shared.PolicyConditionAccepted),
		Status:             metav1.ConditionTrue,
		Reason:             string(gwv1.PolicyReasonAccepted),
		ObservedGeneration: backendTLSPolicy.Generation,
	})

	agw.Delete(manifest("backendtls", "configmap.yaml"))

	assertBackendTLSPolicyStatus(t, agw, backendTLSPolicy, metav1.Condition{
		Type:               string(gwv1.PolicyConditionAccepted),
		Status:             metav1.ConditionFalse,
		Reason:             string(gwv1.BackendTLSPolicyReasonNoValidCACertificate),
		ObservedGeneration: backendTLSPolicy.Generation,
	})
}

func assertBackendTLSPolicyStatus(t *testing.T, agw *base.BaseTestingSuite, policy *gwv1.BackendTLSPolicy, inCondition metav1.Condition) {
	t.Helper()
	currentTimeout, pollingInterval := helpers.GetTimeouts()
	p := agw.TestInstallation.AssertionsT(t)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		tlsPol := &gwv1.BackendTLSPolicy{}
		objKey := client.ObjectKeyFromObject(policy)
		err := agw.TestInstallation.ClusterContext.Client.Get(agw.Ctx, objKey, tlsPol)
		g.Expect(err).NotTo(gomega.HaveOccurred(), "failed to get BackendTLSPolicy %s", objKey)

		g.Expect(tlsPol.Status.Ancestors).To(gomega.HaveLen(1), "ancestors didn't have length of 1")
		expectedAncestorRefs := []gwv1.ParentReference{
			{
				Group: ptr.To(gwv1.Group("gateway.networking.k8s.io")),
				Kind:  ptr.To(gwv1.Kind("Gateway")),
				Name:  gwv1.ObjectName("gateway"),
			},
		}

		for i, ancestor := range tlsPol.Status.Ancestors {
			expectedRef := expectedAncestorRefs[i]
			g.Expect(ancestor.AncestorRef).To(gomega.BeEquivalentTo(expectedRef))

			g.Expect(ancestor.Conditions).To(gomega.HaveLen(2), "ancestors conditions wasn't length of 2")
			cond := meta.FindStatusCondition(ancestor.Conditions, inCondition.Type)
			g.Expect(cond).NotTo(gomega.BeNil(), "policy should have condition "+inCondition.Type)
			g.Expect(cond.Status).To(gomega.Equal(inCondition.Status), "policy accepted condition should be true")
			g.Expect(cond.Reason).To(gomega.Equal(inCondition.Reason), "policy reason should be accepted")
			g.Expect(cond.ObservedGeneration).To(gomega.Equal(inCondition.ObservedGeneration))
		}
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}
