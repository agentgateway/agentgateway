//go:build e2e

package e2e_test

import (
	"net/http"
	"testing"

	"github.com/onsi/gomega"
	"istio.io/istio/pkg/test/util/assert"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/helpers"
)

func TestBackendTLSPolicyAndStatus(tt *testing.T) {
	t := New(tt, base.WithMinGwApiVersion(base.GwApiRequireBackendTLSPolicy))
	t.Apply(
		manifest("backendtls", "configmap.yaml"),
		manifest("backendtls", "base.yaml"),
	)

	backendTLSPolicy := &gwv1.BackendTLSPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "tls-policy",
			Namespace: base.Namespace,
		},
	}
	err := t.TestInstallation.ClusterContext.Client.Get(t.Ctx, client.ObjectKeyFromObject(backendTLSPolicy), backendTLSPolicy)
	assert.NoError(t, err)

	t.Send("example.com", base.ExpectOK())
	t.Send("example2.com", base.ExpectOK())
	t.Send("foo.com", base.Expect(http.StatusMovedPermanently))

	assertBackendTLSPolicyStatus(t, backendTLSPolicy, metav1.Condition{
		Type:               string(shared.PolicyConditionAccepted),
		Status:             metav1.ConditionTrue,
		Reason:             string(gwv1.PolicyReasonAccepted),
		ObservedGeneration: backendTLSPolicy.Generation,
	})

	t.Delete(manifest("backendtls", "configmap.yaml"))

	assertBackendTLSPolicyStatus(t, backendTLSPolicy, metav1.Condition{
		Type:               string(gwv1.PolicyConditionAccepted),
		Status:             metav1.ConditionFalse,
		Reason:             string(gwv1.BackendTLSPolicyReasonNoValidCACertificate),
		ObservedGeneration: backendTLSPolicy.Generation,
	})
}

func assertBackendTLSPolicyStatus(t base.Test, policy *gwv1.BackendTLSPolicy, inCondition metav1.Condition) {
	t.Helper()
	currentTimeout, pollingInterval := helpers.GetTimeouts()
	p := t.TestInstallation.AssertionsT(t)
	p.Gomega.Eventually(func(g gomega.Gomega) {
		tlsPol := &gwv1.BackendTLSPolicy{}
		objKey := client.ObjectKeyFromObject(policy)
		err := t.TestInstallation.ClusterContext.Client.Get(t.Ctx, objKey, tlsPol)
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
