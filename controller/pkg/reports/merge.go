package reports

import (
	"cmp"

	"istio.io/istio/pkg/slices"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

// MergeOwnedStatuses merges this controller's desired status entries into the existing list.
//
// The existing order is preserved: entries of other controllers are untouched, our entries are replaced in
// place, our stale entries are dropped, and only entries new to the list are appended, in sorted order.
// Appending our entries after everyone else's instead would make two controllers rewrite each other's order
// forever, since each one would move its own entries to the end.
func MergeOwnedStatuses[T any](existing, desired []T, owned func(T) bool, sameRef func(a, b T) bool, compare func(a, b T) int) []T {
	ours := make([]T, 0, len(desired))
	for _, d := range desired {
		if owned(d) {
			ours = append(ours, d)
		}
	}
	slices.SortFunc(ours, compare)

	out := make([]T, 0, len(existing)+len(ours))
	for _, e := range existing {
		if !owned(e) {
			out = append(out, e)
			continue
		}
		if i := slices.IndexFunc(ours, func(d T) bool { return sameRef(e, d) }); i != -1 {
			out = append(out, ours[i])
			ours = slices.Delete(ours, i)
		}
	}
	return append(out, ours...)
}

// CompareParentReference orders ParentReferences with their defaults canonicalized, so nil and explicitly
// set default values compare equal and do not introduce ordering churn.
func CompareParentReference(a, b gwv1.ParentReference) int {
	if c := cmp.Compare(parentRefGroupOrDefault(a.Group), parentRefGroupOrDefault(b.Group)); c != 0 {
		return c
	}
	if c := cmp.Compare(parentRefKindOrDefault(a.Kind), parentRefKindOrDefault(b.Kind)); c != 0 {
		return c
	}
	if c := cmp.Compare(derefStringPtr(a.Namespace), derefStringPtr(b.Namespace)); c != 0 {
		return c
	}
	if c := cmp.Compare(string(a.Name), string(b.Name)); c != 0 {
		return c
	}
	if c := cmp.Compare(derefStringPtr(a.SectionName), derefStringPtr(b.SectionName)); c != 0 {
		return c
	}
	return comparePortNumberPtr(a.Port, b.Port)
}

// CompareInferencePoolParentReference is CompareParentReference for the InferencePool parent type.
func CompareInferencePoolParentReference(a, b inf.ParentReference) int {
	if c := cmp.Compare(inferencePoolParentRefGroupOrDefault(a.Group), inferencePoolParentRefGroupOrDefault(b.Group)); c != 0 {
		return c
	}
	if c := cmp.Compare(inferencePoolParentRefKindOrDefault(a.Kind), inferencePoolParentRefKindOrDefault(b.Kind)); c != 0 {
		return c
	}
	if c := cmp.Compare(string(a.Namespace), string(b.Namespace)); c != 0 {
		return c
	}
	return cmp.Compare(string(a.Name), string(b.Name))
}

func inferencePoolParentRefGroupOrDefault(g *inf.Group) string {
	if g == nil {
		return string(gwv1.GroupName)
	}
	return string(*g)
}

func inferencePoolParentRefKindOrDefault(k inf.Kind) string {
	if k == "" {
		return "Gateway"
	}
	return string(k)
}

func parentRefGroupOrDefault(g *gwv1.Group) string {
	if g == nil {
		return string(gwv1.GroupName)
	}
	return string(*g)
}

func parentRefKindOrDefault(k *gwv1.Kind) string {
	if k == nil {
		return "Gateway"
	}
	return string(*k)
}

func derefStringPtr[S ~string](p *S) string {
	if p == nil {
		return ""
	}
	return string(*p)
}

func comparePortNumberPtr(a, b *gwv1.PortNumber) int {
	switch {
	case a == nil && b == nil:
		return 0
	case a == nil:
		return -1
	case b == nil:
		return 1
	default:
		return cmp.Compare(int(*a), int(*b))
	}
}
