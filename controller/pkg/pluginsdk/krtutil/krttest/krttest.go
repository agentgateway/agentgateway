// Package krttest provides KRT test helpers shared across subsystems.
// Subsystem-specific fixtures live alongside their owning package.
package krttest

import (
	"errors"
	"net/http"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

// EventuallyTimeout is the upper bound for assert.Eventually-style polls.
// Long enough to absorb krt collection settling and ConfigMap informer warm-up.
const EventuallyTimeout = 2 * time.Second

// EventuallyPoll is the polling interval for assert.Eventually-style waits.
const EventuallyPoll = 20 * time.Millisecond

// AlwaysSynced is a krt collection sync handle that reports synced
// immediately. Use as the first arg to krt.NewStaticCollection in tests where
// no real informer-style waiting is needed.
type AlwaysSynced struct{}

func (AlwaysSynced) HasSynced() bool                      { return true }
func (AlwaysSynced) WaitUntilSynced(<-chan struct{}) bool { return true }

// OfflineTransport is an http.RoundTripper that fails every request without
// touching the network. Use it as the DefaultClient transport when a test
// exercises lifecycle behaviour and must not attempt a real fetch.
type OfflineTransport struct{}

func (OfflineTransport) RoundTrip(*http.Request) (*http.Response, error) {
	return nil, errors.New("offline")
}

// KrtOptions returns a krtutil.KrtOptions wired to t.Context().Done() so the
// stop channel closes when the test ends.
func KrtOptions(t *testing.T) krtutil.KrtOptions {
	t.Helper()
	return krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
}

// NewStaticCollection constructs a krt.StaticCollection seeded with items,
// pre-synced via AlwaysSynced and registered under the given observability
// name. Tests use the returned StaticCollection's Reset method to mutate the
// collection across phases.
func NewStaticCollection[T any](
	t *testing.T,
	items []T,
	krtOpts krtutil.KrtOptions,
	name string,
) krt.StaticCollection[T] {
	t.Helper()
	return krt.NewStaticCollection(AlwaysSynced{}, items, krtOpts.ToOptions(name)...)
}

// Await polls a krt collection until it contains exactly expectedLen items
// or EventuallyTimeout elapses. Returns the snapshot at the moment the
// length condition is met.
func Await[T any](t *testing.T, collection krt.Collection[T], expectedLen int) []T {
	t.Helper()

	var snapshot []T
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		snapshot = collection.List()
		assert.Len(c, snapshot, expectedLen)
	}, EventuallyTimeout, EventuallyPoll)
	return snapshot
}
