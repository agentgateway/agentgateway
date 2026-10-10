package krtutil

import (
	"istio.io/istio/pkg/kube/krt"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
)

// SpecOnly exposes the identity and spec of an API object fetched by generation.
type SpecOnly[S any] struct {
	types.NamespacedName
	Spec S

	uid        types.UID
	generation int64
}

// FetchOneSpec tracks spec changes through generation and object recreation through UID.
// Use it for API objects whose generation changes with their spec.
func FetchOneSpec[T metav1.Object, S any](
	ctx krt.HandlerContext,
	collection krt.Collection[T],
	spec func(T) S,
	opts ...krt.FetchOption,
) *SpecOnly[S] {
	results := krt.PartialFetch(ctx, collection, func(obj T) SpecOnly[S] {
		return SpecOnly[S]{
			Name: obj.GetName(), Namespace: obj.GetNamespace(),
			Spec:       spec(obj),
			uid:        obj.GetUID(),
			generation: obj.GetGeneration(),
		}
	}, func(a, b SpecOnly[S]) bool {
		return a.NamespacedName == b.NamespacedName && a.uid == b.uid && a.generation == b.generation
	}, opts...)
	switch len(results) {
	case 0:
		return nil
	case 1:
		return &results[0]
	default:
		panic("FetchOneSpec found more than one object")
	}
}
