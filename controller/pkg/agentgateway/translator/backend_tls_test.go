package translator_test

import (
	"encoding/json"
	"testing"

	"istio.io/istio/pilot/pkg/config/kube/crd"
	"istio.io/istio/pkg/test/util/assert"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestBackendTLSPolicyWithoutGateway(t *testing.T) {
	const policy = `apiVersion: gateway.networking.k8s.io/v1
kind: BackendTLSPolicy
metadata:
  name: entra-jwks
  namespace: agentgateway-system
  generation: 2
  annotations:
    argocd.argoproj.io/tracking-id: agentgateway:gateway.networking.k8s.io/BackendTLSPolicy:agentgateway-system/entra-jwks
spec:
  targetRefs:
    - group: agentgateway.dev
      kind: AgentgatewayBackend
      name: entra-jwks
  validation:
    hostname: login.microsoftonline.com
    wellKnownCACertificates: System
`
	const backend = `apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayBackend
metadata:
  name: entra-jwks
  namespace: agentgateway-system
  annotations:
    argocd.argoproj.io/tracking-id: agentgateway:agentgateway.dev/AgentgatewayBackend:agentgateway-system/entra-jwks
spec:
  static:
    host: login.microsoftonline.com
    port: 443
`

	readStatus := func(t *testing.T, queue *testutils.TestStatusQueue) gwv1.PolicyStatus {
		t.Helper()
		updates := queue.Dump()
		assert.Equal(t, len(updates), 1)
		update := updates[0].(crd.IstioKind)
		assert.Equal(t, update.Name, "entra-jwks")
		assert.Equal(t, update.Namespace, "agentgateway-system")

		var status gwv1.PolicyStatus
		assert.NoError(t, json.Unmarshal(*update.Status, &status))
		return status
	}

	t.Run("direct backend", func(t *testing.T) {
		ctx := testutils.BuildMockPolicyContext(t, []any{policy, backend})
		queue, syncer := testutils.Syncer(t, ctx, "BackendTLSPolicy")
		for _, resource := range syncer.Outputs.Resources.List() {
			if resource.Resource.GetPolicy().GetBackend().GetBackendTls() != nil {
				t.Fatal("BackendTLSPolicy without a Gateway generated a Gateway TLS policy")
			}
		}
		status := readStatus(t, queue)
		if len(status.Ancestors) != 1 {
			t.Fatalf("BackendTLSPolicy status.ancestors has %d entries, want 1", len(status.Ancestors))
		}
		ancestor := status.Ancestors[0]
		assert.Equal(t, ancestor.AncestorRef, gwv1.ParentReference{
			Group: new(gwv1.Group("agentgateway.dev")),
			Kind:  new(gwv1.Kind("AgentgatewayBackend")),
			Name:  "entra-jwks",
		})
		assert.Equal(t, ancestor.ControllerName, gwv1.GatewayController(wellknown.DefaultAgwControllerName))
		assert.Equal(t, len(ancestor.Conditions), 2)
		for i, conditionType := range []string{"Accepted", "ResolvedRefs"} {
			condition := ancestor.Conditions[i]
			assert.Equal(t, condition.Type, conditionType)
			assert.Equal(t, condition.Reason, conditionType)
			assert.Equal(t, condition.Status, metav1.ConditionTrue)
			assert.Equal(t, condition.ObservedGeneration, int64(2))
		}
	})

	t.Run("missing backend", func(t *testing.T) {
		ctx := testutils.BuildMockPolicyContext(t, []any{policy})
		queue, _ := testutils.Syncer(t, ctx, "BackendTLSPolicy")
		assert.Equal(t, len(readStatus(t, queue).Ancestors), 0)
	})
}
