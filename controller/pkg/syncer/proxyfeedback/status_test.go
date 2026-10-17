package proxyfeedback

import (
	"strings"
	"testing"
	"time"

	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test"
	"istio.io/istio/pkg/test/util/assert"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

const testController = gwv1.GatewayController("agentgateway.dev/agentgateway")

func gatewayRef(name string) gwv1.ParentReference {
	return gwv1.ParentReference{
		Group:     new(gwv1.Group(wellknown.GatewayGVK.Group)),
		Kind:      new(gwv1.Kind(wellknown.GatewayGVK.Kind)),
		Namespace: new(gwv1.Namespace("default")),
		Name:      gwv1.ObjectName(name),
	}
}

func acceptedAncestor(name string, status metav1.ConditionStatus, reason, message string) gwv1.PolicyAncestorStatus {
	return gwv1.PolicyAncestorStatus{
		AncestorRef:    gatewayRef(name),
		ControllerName: testController,
		Conditions: []metav1.Condition{
			{
				Type:               agentgateway.PolicyConditionAccepted,
				Status:             status,
				Reason:             reason,
				Message:            message,
				ObservedGeneration: 2,
				LastTransitionTime: metav1.NewTime(time.Unix(100, 0)),
			},
			{
				Type:               agentgateway.PolicyConditionAttached,
				Status:             metav1.ConditionTrue,
				Reason:             agentgateway.PolicyReasonAttached,
				ObservedGeneration: 2,
				LastTransitionTime: metav1.NewTime(time.Unix(100, 0)),
			},
		},
	}
}

func testPolicy(live ...gwv1.PolicyAncestorStatus) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		Namespace: "default", Name: "auth", Generation: 2,
		Status: gwv1.PolicyStatus{Ancestors: live},
	}
}

func accepted(t *testing.T, status gwv1.PolicyStatus, gateway string) metav1.Condition {
	t.Helper()
	for _, ancestor := range status.Ancestors {
		if string(ancestor.AncestorRef.Name) == gateway {
			return *meta.FindStatusCondition(ancestor.Conditions, agentgateway.PolicyConditionAccepted)
		}
	}
	t.Fatalf("no ancestor for gateway %s", gateway)
	return metav1.Condition{}
}

func reported(severity Severity, message string) map[types.NamespacedName][]Diagnostic {
	return map[types.NamespacedName][]Diagnostic{
		testGateway: {{Gateway: testGateway, Source: testSource, Resource: "policy/auth", Severity: severity, Message: message}},
	}
}

func TestApplyDiagnostics(t *testing.T) {
	valid := acceptedAncestor("gw", metav1.ConditionTrue, agentgateway.PolicyReasonValid, "Policy accepted")
	other := acceptedAncestor("other", metav1.ConditionTrue, agentgateway.PolicyReasonValid, "Policy accepted")
	desired := gwv1.PolicyStatus{Ancestors: []gwv1.PolicyAncestorStatus{valid, other}}

	t.Run("warning", func(t *testing.T) {
		got := applyDiagnostics(testPolicy(valid, other), desired, reported(SeverityWarning, "bad key"), testController)
		cond := accepted(t, got, "gw")
		assert.Equal(t, cond.Status, metav1.ConditionTrue)
		assert.Equal(t, cond.Reason, agentgateway.PolicyReasonPartiallyValid)
		assert.Equal(t, cond.Message, "Proxy reported configuration: bad key")
		assert.Equal(t, cond.ObservedGeneration, int64(2))
		// Status is unchanged, so the transition time is kept.
		assert.Equal(t, cond.LastTransitionTime, metav1.NewTime(time.Unix(100, 0)))
		assert.Equal(t, accepted(t, got, "other").Reason, agentgateway.PolicyReasonValid)
		// The translated status shared with other collections is not modified.
		assert.Equal(t, accepted(t, desired, "gw").Reason, agentgateway.PolicyReasonValid)
	})

	t.Run("error", func(t *testing.T) {
		got := applyDiagnostics(testPolicy(valid, other), desired, reported(SeverityError, "rejected"), testController)
		cond := accepted(t, got, "gw")
		assert.Equal(t, cond.Status, metav1.ConditionFalse)
		assert.Equal(t, cond.Reason, agentgateway.PolicyReasonInvalid)
		assert.Equal(t, cond.Message, "Proxy rejected configuration: rejected")
		assert.Equal(t, cond.LastTransitionTime.After(time.Unix(100, 0)), true)
	})

	t.Run("already reported keeps live condition", func(t *testing.T) {
		live := acceptedAncestor("gw", metav1.ConditionFalse, agentgateway.PolicyReasonInvalid, "Proxy rejected configuration: rejected")
		got := applyDiagnostics(testPolicy(live, other), desired, reported(SeverityError, "rejected"), testController)
		assert.Equal(t, accepted(t, got, "gw"), live.Conditions[0])
	})

	t.Run("controller rejection is kept", func(t *testing.T) {
		invalid := acceptedAncestor("gw", metav1.ConditionFalse, agentgateway.PolicyReasonInvalid, "bad spec")
		desired := gwv1.PolicyStatus{Ancestors: []gwv1.PolicyAncestorStatus{invalid}}
		got := applyDiagnostics(testPolicy(invalid), desired, reported(SeverityWarning, "bad key"), testController)
		assert.Equal(t, accepted(t, got, "gw").Message, "bad spec")
	})

	t.Run("controller partial validity is extended", func(t *testing.T) {
		partial := acceptedAncestor("gw", metav1.ConditionTrue, agentgateway.PolicyReasonPartiallyValid, "missing secret")
		desired := gwv1.PolicyStatus{Ancestors: []gwv1.PolicyAncestorStatus{partial}}
		got := applyDiagnostics(testPolicy(partial), desired, reported(SeverityWarning, "bad key"), testController)
		assert.Equal(t, accepted(t, got, "gw").Message, "missing secret\nProxy reported configuration: bad key")
	})
}

func TestReportedMessageBounded(t *testing.T) {
	var messages []string
	for i := range maxConditionMessages + 3 {
		messages = append(messages, strings.Repeat("x", i+1))
	}
	message := reportedMessage("reported", append(messages, messages[0]))
	lines := strings.Split(message, "\n")
	assert.Equal(t, len(lines), maxConditionMessages+1)
	assert.Equal(t, lines[maxConditionMessages], "and 3 more")
}

func TestWithPolicyDiagnosticsFollowsPublishedSet(t *testing.T) {
	stop := test.NewStop(t)
	valid := acceptedAncestor("gw", metav1.ConditionTrue, agentgateway.PolicyReasonValid, "Policy accepted")
	policy := testPolicy(valid)
	statuses := krt.NewStaticCollection[krt.ObjectWithStatus[controllers.Object, any]](alwaysSynced{}, []krt.ObjectWithStatus[controllers.Object, any]{{
		Obj:    policy,
		Status: gwv1.PolicyStatus{Ancestors: []gwv1.PolicyAncestorStatus{valid}},
	}}, krt.WithStop(stop))

	diagnostics, err := encode([]Diagnostic{{Gateway: testGateway, Source: testSource, Resource: "policy/auth", Severity: SeverityWarning, Message: "bad key"}})
	assert.NoError(t, err)
	replica := &corev1.ConfigMap{
		Namespace: testNamespace,
		Name:      configMapPrefix + "replica-a",
		Data:      map[string]string{dataKey: diagnostics},
	}
	configMaps := krt.NewMutableCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{replica}, krt.WithStop(stop))
	withDiagnostics := WithPolicyDiagnostics(statuses, NewPublishedFromConfigMaps(configMaps.AsCollection(), krt.WithStop(stop)), string(testController), krt.WithStop(stop))
	withDiagnostics.WaitUntilSynced(stop)

	reason := func() string {
		o := withDiagnostics.GetKey(krt.GetKey[controllers.Object](policy))
		if o == nil {
			return ""
		}
		return accepted(t, o.Status.(gwv1.PolicyStatus), "gw").Reason
	}
	assert.EventuallyEqual(t, reason, agentgateway.PolicyReasonPartiallyValid)

	// Once the replica publishes an empty set, the translated status comes back.
	cleared := replica.DeepCopy()
	cleared.Data[dataKey] = "[]"
	configMaps.UpdateObject(cleared)
	assert.EventuallyEqual(t, reason, agentgateway.PolicyReasonValid)
}
