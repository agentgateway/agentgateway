package plugins

import (
	"errors"
	"fmt"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

func TestPolicyConditionMap(t *testing.T) {
	configErr := &ConfigError{Reason: "UnsupportedValue", Message: "unsupported policy setting"}
	for _, tt := range []struct {
		name     string
		err      error
		emitted  bool
		accepted metav1.ConditionStatus
		reason   string
		attached metav1.ConditionStatus
	}{
		{
			name: "valid", emitted: true,
			accepted: metav1.ConditionTrue, reason: agentgateway.PolicyReasonValid, attached: metav1.ConditionTrue,
		},
		{
			name: "partial translation", err: errors.New("translation failed"), emitted: true,
			accepted: metav1.ConditionTrue, reason: agentgateway.PolicyReasonPartiallyValid,
		},
		{
			name: "failed translation", err: errors.New("translation failed"),
			accepted: metav1.ConditionFalse, reason: agentgateway.PolicyReasonInvalid, attached: metav1.ConditionFalse,
		},
		{
			name: "invalid configuration with output", err: configErr, emitted: true,
			accepted: metav1.ConditionFalse, reason: agentgateway.PolicyReasonInvalid, attached: metav1.ConditionTrue,
		},
		{
			name: "invalid configuration without output", err: configErr,
			accepted: metav1.ConditionFalse, reason: agentgateway.PolicyReasonInvalid, attached: metav1.ConditionFalse,
		},
		{
			name: "joined validation and translation errors",
			err:  errors.Join(errors.New("translation failed"), fmt.Errorf("policy: %w", configErr)), emitted: true,
			accepted: metav1.ConditionFalse, reason: agentgateway.PolicyReasonInvalid, attached: metav1.ConditionTrue,
		},
	} {
		t.Run(tt.name, func(t *testing.T) {
			conditions := PolicyConditionMap(tt.err, tt.emitted)
			accepted := conditions[agentgateway.PolicyConditionAccepted]
			require.Equal(t, tt.accepted, accepted.Status)
			require.Equal(t, tt.reason, accepted.Reason)
			if tt.err != nil {
				require.Equal(t, tt.err.Error(), accepted.Message)
			}
			attached := conditions[agentgateway.PolicyConditionAttached]
			if tt.attached == "" {
				require.Nil(t, attached)
			} else {
				require.NotNil(t, attached)
				require.Equal(t, tt.attached, attached.Status)
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

// The API server defaults ancestorRef.kind to Gateway on write, so the stored StatusSummary ref differs from
// the one the translator builds without a kind. The match must still succeed or every translation recreates
// the conditions with a fresh lastTransitionTime and rewrites the object forever.
func TestSetAncestorStatusMatchesDefaultedKind(t *testing.T) {
	const controllerName = "agentgateway.dev/controller"
	computed := gwv1.ParentReference{
		Group: new(gwv1.Group("agentgateway.dev")),
		Name:  "StatusSummary",
	}
	stored := computed
	stored.Kind = new(gwv1.Kind("Gateway"))
	then := metav1.NewTime(time.Date(2026, 4, 22, 17, 57, 23, 0, time.UTC))
	existing := &gwv1.PolicyStatus{Ancestors: []gwv1.PolicyAncestorStatus{{
		AncestorRef:    stored,
		ControllerName: controllerName,
		Conditions: []metav1.Condition{{
			Type: "Attached", Status: metav1.ConditionFalse, Reason: "Pending", Message: "stale",
			ObservedGeneration: 1, LastTransitionTime: then,
		}},
	}}}

	got := SetAncestorStatus(computed, existing, 1, map[string]*Condition{
		"Attached": {Status: metav1.ConditionFalse, Reason: "Pending", Message: "stale"},
	}, controllerName)

	if len(got.Conditions) != 1 || !got.Conditions[0].LastTransitionTime.Equal(&then) {
		t.Fatalf("expected the existing condition to be kept with its transition time, got %#v", got.Conditions)
	}
	if got.AncestorRef.Kind == nil || *got.AncestorRef.Kind != "Gateway" {
		t.Fatalf("expected the stored ancestorRef with its defaulted kind to be kept, got %#v", got.AncestorRef)
	}
}
