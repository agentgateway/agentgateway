package plugins

import (
	"istio.io/istio/pilot/pkg/model/kstatus"
	"istio.io/istio/pkg/maps"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

type condition struct {
	// reason defines the reason to report on success. Ignored if error is set
	reason string
	// message defines the message to report on success. Ignored if error is set
	message string
	// status defines the status to report on success. The inverse will be set if error is set
	// If not set, will default to StatusTrue
	status metav1.ConditionStatus
	// error defines an error state; the reason and message will be replaced with that of the error and
	// the status inverted
	error *ConfigError
	// setOnce, if enabled, will only set the condition if it is not yet present or set to this reason
	setOnce string
}

// ConfigError represents an invalid configuration that will be reported back to the user.
type ConfigError struct {
	Reason  string
	Message string
}

// mergeAncestors merges an existing ancestor with in incoming one. We preserve order, prune stale references set by our controller,
// and add any new references from our controller.
func mergeAncestors(controllerName string, existing []gwv1.PolicyAncestorStatus, incoming []gwv1.PolicyAncestorStatus) []gwv1.PolicyAncestorStatus {
	n := 0
	for _, x := range existing {
		if controllerName != string(x.ControllerName) {
			// Keep it as-is
			existing[n] = x
			n++
			continue
		}
		replacement := slices.IndexFunc(incoming, func(status gwv1.PolicyAncestorStatus) bool {
			return parentRefEqual(status.AncestorRef, x.AncestorRef)
		})
		if replacement != -1 {
			// We found a replacement!
			existing[n] = incoming[replacement]
			incoming = slices.Delete(incoming, replacement)
			n++
		}
		// Else, do nothing and it will be filtered
	}
	existing = existing[:n]
	// Add all remaining ones.
	existing = append(existing, incoming...)
	// There is a max of 16
	return existing[:min(len(existing), 16)]
}

func parentRefEqual(a, b gwv1.ParentReference) bool {
	return ptr.Equal(a.Group, b.Group) &&
		ptr.Equal(a.Kind, b.Kind) &&
		a.Name == b.Name &&
		ptr.Equal(a.Namespace, b.Namespace) &&
		ptr.Equal(a.SectionName, b.SectionName) &&
		ptr.Equal(a.Port, b.Port)
}

func setAncestorStatus(
	pr gwv1.ParentReference,
	status *gwv1.PolicyStatus,
	generation int64,
	conds map[string]*condition,
	controller gwv1.GatewayController,
) gwv1.PolicyAncestorStatus {
	currentAncestor := slices.FindFunc(status.Ancestors, func(ex gwv1.PolicyAncestorStatus) bool {
		return parentRefEqual(ex.AncestorRef, pr)
	})
	var currentConds []metav1.Condition
	if currentAncestor != nil {
		currentConds = currentAncestor.Conditions
	}
	return gwv1.PolicyAncestorStatus{
		AncestorRef:    pr,
		ControllerName: controller,
		Conditions:     setConditions(generation, currentConds, conds),
	}
}

// setConditions sets the existingConditions with the new conditions
func setConditions(generation int64, existingConditions []metav1.Condition, conditions map[string]*condition) []metav1.Condition {
	// Sort keys for deterministic ordering
	for _, k := range slices.Sort(maps.Keys(conditions)) {
		cond := conditions[k]
		setter := kstatus.UpdateConditionIfChanged
		if cond.setOnce != "" {
			setter = func(conditions []metav1.Condition, condition metav1.Condition) []metav1.Condition {
				return kstatus.CreateCondition(conditions, condition, cond.setOnce)
			}
		}
		// A condition can be "negative polarity" (ex: ListenerInvalid) or "positive polarity" (ex:
		// ListenerValid), so in order to determine the status we should set each `condition` defines its
		// default positive status. When there is an error, we will invert that. Example: If we have
		// condition ListenerInvalid, the status will be set to StatusFalse. If an error is reported, it
		// will be inverted to StatusTrue to indicate listeners are invalid. See
		// https://github.com/kubernetes/community/blob/master/contributors/devel/sig-architecture/api-conventions.md#typical-status-properties
		// for more information
		if cond.error != nil {
			existingConditions = setter(existingConditions, metav1.Condition{
				Type:               k,
				Status:             kstatus.InvertStatus(cond.status),
				ObservedGeneration: generation,
				LastTransitionTime: metav1.Now(),
				Reason:             cond.error.Reason,
				Message:            cond.error.Message,
			})
		} else {
			status := cond.status
			if status == "" {
				status = kstatus.StatusTrue
			}
			existingConditions = setter(existingConditions, metav1.Condition{
				Type:               k,
				Status:             status,
				ObservedGeneration: generation,
				LastTransitionTime: metav1.Now(),
				Reason:             cond.reason,
				Message:            cond.message,
			})
		}
	}
	return existingConditions
}
