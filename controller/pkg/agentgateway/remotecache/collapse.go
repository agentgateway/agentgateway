package remotecache

import (
	"cmp"
	"time"

	"istio.io/istio/pkg/slices"
)

// CollapseSources merges per-owner sources sharing a fetch key into a single
// canonical record. It returns the source with the lowest ownerKey (so output
// is stable across input order) and the minimum TTL across all sources (so
// the shared fetch refreshes as often as the most aggressive owner requires).
//
// Callers must pre-filter empty input — CollapseSources panics on len 0.
func CollapseSources[S any](sources []S, ownerKey func(S) string, ttl func(S) time.Duration) (S, time.Duration) {
	sorted := append([]S(nil), sources...)
	slices.SortFunc(sorted, func(a, b S) int {
		return cmp.Compare(ownerKey(a), ownerKey(b))
	})
	primary := sorted[0]
	minTTL := ttl(primary)
	for _, s := range sorted[1:] {
		if t := ttl(s); t < minTTL {
			minTTL = t
		}
	}
	return primary, minTTL
}
