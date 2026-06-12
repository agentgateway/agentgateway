package jwks

import (
	"errors"
	"net/http"
	"testing"
	"time"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/retry"
)

// eventuallyTimeout bounds assert.Eventually-style polls in this package's
// tests. Long enough to absorb krt collection settling and ConfigMap
// informer warm-up.
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

// offlineTransport is an http.RoundTripper that fails every request without
// touching the network. Use it as the DefaultClient transport when a test
// exercises lifecycle behaviour and must not attempt a real fetch.
type offlineTransport struct{}

func (offlineTransport) RoundTrip(*http.Request) (*http.Response, error) {
	return nil, errors.New("offline")
}
