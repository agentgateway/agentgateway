//go:build e2e

package e2e_test

import (
	"fmt"
	"net/http"
	"testing"

	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/retry"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
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
	err := t.TestInstallation.ClusterContext.ControllerClient.Get(t.Ctx, client.ObjectKeyFromObject(backendTLSPolicy), backendTLSPolicy)
	assert.NoError(t, err)

	t.Send("example.com", base.ExpectOK())
	t.Send("example2.com", base.ExpectOK())
	t.Send("foo.com", base.Expect(http.StatusMovedPermanently))

	assertBackendTLSPolicyStatus(t, backendTLSPolicy, metav1.Condition{
		Type:               string(agentgateway.PolicyConditionAccepted),
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
	retry.UntilSuccessOrFail(t, func() error {
		tlsPol := &gwv1.BackendTLSPolicy{}
		objKey := client.ObjectKeyFromObject(policy)
		if err := t.TestInstallation.ClusterContext.ControllerClient.Get(t.Ctx, objKey, tlsPol); err != nil {
			return err
		}

		if len(tlsPol.Status.Ancestors) != 1 {
			return fmt.Errorf("ancestors length = %d, want 1", len(tlsPol.Status.Ancestors))
		}
		expectedAncestorRefs := []gwv1.ParentReference{
			{
				Group: ptr.To(gwv1.Group("gateway.networking.k8s.io")),
				Kind:  ptr.To(gwv1.Kind("Gateway")),
				Name:  gwv1.ObjectName("gateway"),
			},
		}

		for i, ancestor := range tlsPol.Status.Ancestors {
			expectedRef := expectedAncestorRefs[i]
			if ancestor.AncestorRef.Group == nil || expectedRef.Group == nil || *ancestor.AncestorRef.Group != *expectedRef.Group ||
				ancestor.AncestorRef.Kind == nil || expectedRef.Kind == nil || *ancestor.AncestorRef.Kind != *expectedRef.Kind ||
				ancestor.AncestorRef.Name != expectedRef.Name {
				return fmt.Errorf("ancestor ref = %+v, want %+v", ancestor.AncestorRef, expectedRef)
			}

			if len(ancestor.Conditions) != 2 {
				return fmt.Errorf("ancestor conditions length = %d, want 2", len(ancestor.Conditions))
			}
			cond := meta.FindStatusCondition(ancestor.Conditions, inCondition.Type)
			if cond == nil {
				return fmt.Errorf("policy should have condition %s", inCondition.Type)
			}
			if cond.Status != inCondition.Status || cond.Reason != inCondition.Reason || cond.ObservedGeneration != inCondition.ObservedGeneration {
				return fmt.Errorf("condition = %+v, want status=%s reason=%s observedGeneration=%d", cond, inCondition.Status, inCondition.Reason, inCondition.ObservedGeneration)
			}
		}
		return nil
	})
}
