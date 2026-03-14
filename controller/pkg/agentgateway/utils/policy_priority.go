package utils

import (
	"cmp"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

// HigherPriorityPolicy reports whether a should win over b using the controller's
// standard policy precedence rules: oldest creation timestamp first, then
// namespace/name lexical order as a stable tie breaker.
func HigherPriorityPolicy(a, b metav1.Object) bool {
	ts := a.GetCreationTimestamp().Compare(b.GetCreationTimestamp().Time)
	if ts < 0 {
		return true
	}
	if ts > 0 {
		return false
	}

	ns := cmp.Compare(a.GetNamespace(), b.GetNamespace())
	if ns < 0 {
		return true
	}
	if ns > 0 {
		return false
	}
	return a.GetName() < b.GetName()
}
