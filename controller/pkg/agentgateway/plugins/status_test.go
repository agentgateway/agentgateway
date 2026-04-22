package plugins

import (
	"testing"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

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

	got := mergeAncestors(controllerName, nil, incoming)

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
		t.Fatalf("expected one summary condition, got %d", len(got[15].Conditions))
	}
	cond := got[15].Conditions[0]
	if cond != (metav1.Condition{
		Type:    "StatusSummarized",
		Status:  metav1.ConditionTrue,
		Reason:  "StatusSummary",
		Message: "2 AncestorRefs ignored due to max status size",
	}) {
		t.Fatalf("unexpected summary condition: %#v", cond)
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

	got := mergeAncestors(controllerName, nil, incoming)

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
