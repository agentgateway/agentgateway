package krtutil

import (
	"fmt"

	"istio.io/istio/pkg/kube/krt"
)

// UnnamedIndex creates a simple index, keyed by key K, over a collection for O. This is similar to
// Informer.AddIndex, but is easier to use and can be added after an informer has already started.
//
// This differs from krt.NewIndex in that it does not require a name. A name can be passed to dedupe indexes by the same name;
// however, when not intended to dedupe, this can lead to accidental deduping.
func UnnamedIndex[K comparable, O any](
	c krt.Collection[O],
	extract func(o O) []K,
) krt.Index[K, O] {
	// We just need some unique key, any will do
	key := fmt.Sprintf("%p", extract)

	return krt.NewIndex(c, key, extract)
}
