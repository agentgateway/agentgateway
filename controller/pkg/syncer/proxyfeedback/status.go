package proxyfeedback

import (
	"fmt"
	"slices"
	"strings"

	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

// maxConditionMessages bounds how many reported problems one condition lists.
const maxConditionMessages = 10

// WithPolicyDiagnostics adds the problems proxies reported for an AgentgatewayPolicy to the
// Accepted condition of each Gateway ancestor whose proxies reported them. It runs after
// translation, so a report changes status without translating the policy again.
func WithPolicyDiagnostics(
	statuses krt.StatusCollection[controllers.Object, any],
	published Published,
	controllerName string,
	opts ...krt.CollectionOption,
) krt.StatusCollection[controllers.Object, any] {
	controller := gwv1.GatewayController(controllerName)
	return krt.NewCollection(statuses, func(ctx krt.HandlerContext, o krt.ObjectWithStatus[controllers.Object, any]) *krt.ObjectWithStatus[controllers.Object, any] {
		policy, ok := o.Obj.(*agentgateway.AgentgatewayPolicy)
		if !ok {
			return &o
		}
		desired, ok := o.Status.(gwv1.PolicyStatus)
		if !ok {
			return &o
		}
		source := Source{Kind: wellknown.AgentgatewayPolicyGVK.Kind, Namespace: policy.Namespace, Name: policy.Name}
		replicas := krt.Fetch(ctx, published.Collection, krt.FilterIndex(published.BySource, source))
		byGateway := diagnosticsByGateway(replicas, source)
		if len(byGateway) == 0 {
			return &o
		}
		o.Status = applyDiagnostics(policy, desired, byGateway, controller)
		return &o
	}, opts...)
}

func diagnosticsByGateway(replicas []ReplicaDiagnostics, source Source) map[types.NamespacedName][]Diagnostic {
	out := map[types.NamespacedName][]Diagnostic{}
	for _, replica := range replicas {
		for _, d := range replica.Diagnostics {
			if d.Source == source {
				out[d.Gateway] = append(out[d.Gateway], d)
			}
		}
	}
	return out
}

func applyDiagnostics(
	policy *agentgateway.AgentgatewayPolicy,
	desired gwv1.PolicyStatus,
	byGateway map[types.NamespacedName][]Diagnostic,
	controller gwv1.GatewayController,
) gwv1.PolicyStatus {
	out := gwv1.PolicyStatus{Ancestors: slices.Clone(desired.Ancestors)}
	for i, ancestor := range out.Ancestors {
		if ancestor.ControllerName != controller {
			continue
		}
		gateway, ok := gatewayOf(ancestor.AncestorRef, policy.Namespace)
		if !ok {
			continue
		}
		diagnostics := byGateway[gateway]
		if len(diagnostics) == 0 {
			continue
		}
		accepted := acceptedCondition(meta.FindStatusCondition(ancestor.Conditions, agentgateway.PolicyConditionAccepted), diagnostics)
		if accepted == nil {
			continue
		}
		// Build from the live status so an unchanged condition keeps its transition time and
		// does not cause a status write.
		updated := plugins.SetAncestorStatus(ancestor.AncestorRef, &policy.Status, policy.Generation, map[string]*plugins.Condition{
			agentgateway.PolicyConditionAccepted: accepted,
		}, controller)
		out.Ancestors[i].Conditions = replaceCondition(ancestor.Conditions, *meta.FindStatusCondition(updated.Conditions, agentgateway.PolicyConditionAccepted))
	}
	return out
}

func replaceCondition(conditions []metav1.Condition, condition metav1.Condition) []metav1.Condition {
	out := slices.Clone(conditions)
	i := slices.IndexFunc(out, func(c metav1.Condition) bool { return c.Type == condition.Type })
	if i < 0 {
		return append(out, condition)
	}
	out[i] = condition
	return out
}

func gatewayOf(ref gwv1.ParentReference, defaultNamespace string) (types.NamespacedName, bool) {
	if ref.Group != nil && string(*ref.Group) != wellknown.GatewayGVK.Group {
		return types.NamespacedName{}, false
	}
	if ref.Kind != nil && string(*ref.Kind) != wellknown.GatewayGVK.Kind {
		return types.NamespacedName{}, false
	}
	namespace := defaultNamespace
	if ref.Namespace != nil {
		namespace = string(*ref.Namespace)
	}
	return types.NamespacedName{Namespace: namespace, Name: string(ref.Name)}, true
}

// acceptedCondition returns the Accepted condition with the reported problems, or nil when the
// controller already rejected the policy. A rejected resource makes the policy Invalid; a warning
// leaves it accepted but PartiallyValid.
func acceptedCondition(current *metav1.Condition, diagnostics []Diagnostic) *plugins.Condition {
	var errors, warnings []string
	for _, d := range diagnostics {
		if d.Severity == SeverityError {
			errors = append(errors, d.Message)
		} else {
			warnings = append(warnings, d.Message)
		}
	}
	if len(errors) > 0 {
		return &plugins.Condition{
			Status:  metav1.ConditionFalse,
			Reason:  agentgateway.PolicyReasonInvalid,
			Message: reportedMessage("rejected", errors),
		}
	}
	if current != nil && current.Status == metav1.ConditionFalse {
		return nil
	}
	message := reportedMessage("reported", warnings)
	if current != nil && current.Reason == agentgateway.PolicyReasonPartiallyValid && current.Message != "" {
		message = current.Message + "\n" + message
	}
	return &plugins.Condition{
		Status:  metav1.ConditionTrue,
		Reason:  agentgateway.PolicyReasonPartiallyValid,
		Message: message,
	}
}

func reportedMessage(verb string, messages []string) string {
	slices.Sort(messages)
	messages = slices.Compact(messages)
	extra := 0
	if len(messages) > maxConditionMessages {
		extra = len(messages) - maxConditionMessages
		messages = messages[:maxConditionMessages]
	}
	lines := make([]string, 0, len(messages)+1)
	for _, m := range messages {
		lines = append(lines, fmt.Sprintf("Proxy %s configuration: %s", verb, m))
	}
	if extra > 0 {
		lines = append(lines, fmt.Sprintf("and %d more", extra))
	}
	return strings.Join(lines, "\n")
}
