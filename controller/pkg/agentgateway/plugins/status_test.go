package plugins

import (
	"errors"
	"testing"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

func TestPolicyConditionMapAlwaysReportsAcceptedAndAttached(t *testing.T) {
	errInvalid := errors.New("invalid policy configuration")
	tests := []struct {
		name                  string
		err                   error
		hasTranslatedPolicies bool
		acceptedStatus        metav1.ConditionStatus
		acceptedReason        agentgateway.PolicyConditionReason
		attachedStatus        metav1.ConditionStatus
		attachedReason        agentgateway.PolicyConditionReason
		attachedMessage       string
	}{
		{
			name:                  "partially valid",
			err:                   errInvalid,
			hasTranslatedPolicies: true,
			acceptedStatus:        metav1.ConditionTrue,
			acceptedReason:        agentgateway.PolicyReasonPartiallyValid,
			attachedStatus:        metav1.ConditionTrue,
			attachedReason:        agentgateway.PolicyReasonAttached,
			attachedMessage:       "Policy is attached with invalid configuration: " + errInvalid.Error(),
		},
		{
			name:           "invalid",
			err:            errInvalid,
			acceptedStatus: metav1.ConditionFalse,
			acceptedReason: agentgateway.PolicyReasonInvalid,
			attachedStatus: metav1.ConditionFalse,
			attachedReason: agentgateway.PolicyReasonPending,
		},
		{
			name:                  "valid",
			hasTranslatedPolicies: true,
			acceptedStatus:        metav1.ConditionTrue,
			acceptedReason:        agentgateway.PolicyReasonValid,
			attachedStatus:        metav1.ConditionTrue,
			attachedReason:        agentgateway.PolicyReasonAttached,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := PolicyConditionMap(tt.err, tt.hasTranslatedPolicies)
			if len(got) != 2 {
				t.Fatalf("PolicyConditionMap() returned %d conditions, want 2", len(got))
			}
			accepted := got[agentgateway.PolicyConditionAccepted]
			if accepted == nil || accepted.Status != tt.acceptedStatus || accepted.Reason != tt.acceptedReason {
				t.Fatalf("Accepted condition = %#v, want status %q and reason %q", accepted, tt.acceptedStatus, tt.acceptedReason)
			}
			attached := got[agentgateway.PolicyConditionAttached]
			if attached == nil || attached.Status != tt.attachedStatus || attached.Reason != tt.attachedReason {
				t.Fatalf("Attached condition = %#v, want status %q and reason %q", attached, tt.attachedStatus, tt.attachedReason)
			}
			if tt.attachedMessage != "" && attached.Message != tt.attachedMessage {
				t.Fatalf("Attached message = %q, want %q", attached.Message, tt.attachedMessage)
			}
		})
	}
}

func TestMergeAncestorsSummarizesWhenOwnedAncestorFitsInFirst16(t *testing.T) {
	const controllerName = "agentgateway.dev/controller"

	var incoming []gwv1.PolicyAncestorStatus
	for i := range 17 {
		controller := "other.dev/controller"
		name := gwv1.ObjectName("other-" + string(rune('a'+i)))
		if i == 14 {
			controller = controllerName
			name = "ours"
		}
		incoming = append(incoming, gwv1.PolicyAncestorStatus{
			AncestorRef:    gwv1.ParentReference{Name: name},
			ControllerName: gwv1.GatewayController(controller),
		})
	}

	got := MergeAncestors(controllerName, nil, incoming)

	if len(got) != 16 {
		t.Fatalf("expected 16 ancestors, got %d", len(got))
	}
	if got[15].AncestorRef.Name != "StatusSummary" {
		t.Fatalf("expected final ancestor to be StatusSummary, got %q", got[15].AncestorRef.Name)
	}
	if got[15].ControllerName != gwv1.GatewayController(controllerName) {
		t.Fatalf("expected summary controller %q, got %q", controllerName, got[15].ControllerName)
	}
	if got[15].AncestorRef.Group == nil || *got[15].AncestorRef.Group != gwv1.Group("agentgateway.dev") {
		t.Fatalf("expected summary group agentgateway.dev, got %#v", got[15].AncestorRef.Group)
	}
	if len(got[15].Conditions) != 1 {
		t.Fatalf("expected one summary Condition, got %d", len(got[15].Conditions))
	}
	cond := got[15].Conditions[0]
	if cond != (metav1.Condition{
		Type:    "StatusSummarized",
		Status:  metav1.ConditionTrue,
		Reason:  "StatusSummary",
		Message: "2 AncestorRefs ignored due to max status size",
	}) {
		t.Fatalf("unexpected summary Condition: %#v", cond)
	}
	for _, ancestor := range got[:15] {
		if ancestor.AncestorRef.Name == "ours" {
			t.Fatalf("expected owned ancestor to be replaced by summary")
		}
	}
}

func TestMergeAncestorsTruncatesWhenNoOwnedAncestorFitsInFirst16(t *testing.T) {
	const controllerName = "agentgateway.dev/controller"

	var incoming []gwv1.PolicyAncestorStatus
	for i := range 17 {
		controller := "other.dev/controller"
		if i == 16 {
			controller = controllerName
		}
		incoming = append(incoming, gwv1.PolicyAncestorStatus{
			AncestorRef:    gwv1.ParentReference{Name: gwv1.ObjectName("ancestor-" + string(rune('a'+i)))},
			ControllerName: gwv1.GatewayController(controller),
		})
	}

	got := MergeAncestors(controllerName, nil, incoming)

	if len(got) != 16 {
		t.Fatalf("expected 16 ancestors, got %d", len(got))
	}
	if got[15].AncestorRef.Name == "StatusSummary" {
		t.Fatalf("did not expect summary ancestor when first 16 are not ours")
	}
	for _, ancestor := range got {
		if ancestor.AncestorRef.Name == "ancestor-q" {
			t.Fatalf("expected 17th ancestor to be truncated")
		}
	}
}
