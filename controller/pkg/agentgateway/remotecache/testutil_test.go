package remotecache

import (
	"testing"
	"time"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/retry"
)

// eventuallyTimeout bounds assert.Eventually-style polls in this package's
// tests. Long enough to absorb krt collection settling.
const eventuallyTimeout = 2 * time.Second

// eventuallyPoll is the polling interval for assert.Eventually-style waits.
const eventuallyPoll = 20 * time.Millisecond

// await polls a krt collection until it contains exactly expectedLen items
// or eventuallyTimeout elapses. Returns the snapshot at the moment the
// length condition is met.
func await[T any](t *testing.T, collection krt.Collection[T], expectedLen int) []T {
	t.Helper()

	var snapshot []T
	assert.EventuallyEqual(t, func() int {
		snapshot = collection.List()
		return len(snapshot)
	}, expectedLen, retry.Timeout(eventuallyTimeout), retry.BackoffDelay(eventuallyPoll))
	return snapshot
}
