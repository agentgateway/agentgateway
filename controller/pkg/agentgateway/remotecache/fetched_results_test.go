package remotecache

import (
	"sync"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

type fetchedResultsTestResult struct {
	Key       remotehttp.FetchKey
	FetchedAt time.Time
	Value     string
}

func (r fetchedResultsTestResult) RemoteRequestKey() remotehttp.FetchKey { return r.Key }
func (r fetchedResultsTestResult) RemoteFetchedAt() time.Time            { return r.FetchedAt }
func (r fetchedResultsTestResult) ResourceName() string                  { return string(r.Key) }
func (r fetchedResultsTestResult) Equals(other fetchedResultsTestResult) bool {
	return r.Key == other.Key &&
		r.FetchedAt.Equal(other.FetchedAt) &&
		r.Value == other.Value
}

func TestFetchedResultsPublishesKRTEvents(t *testing.T) {
	results := NewFetchedResults[fetchedResultsTestResult]()

	var (
		mu      sync.Mutex
		batches [][]krt.Event[fetchedResultsTestResult]
	)
	registration := results.Collection().RegisterBatch(func(events []krt.Event[fetchedResultsTestResult]) {
		mu.Lock()
		batches = append(batches, events)
		mu.Unlock()
	}, false)
	defer registration.UnregisterHandler()
	require.True(t, registration.WaitUntilSynced(t.Context().Done()))

	result := fetchedResultsTestResult{Key: "issuer-a", FetchedAt: time.Unix(100, 0), Value: "v1"}
	results.Put(result)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		mu.Lock()
		defer mu.Unlock()
		if assert.NotEmpty(c, batches) {
			latest := batches[len(batches)-1]
			assert.Len(c, latest, 1)
			assert.NotNil(c, latest[0].New)
			assert.Equal(c, result, *latest[0].New)
		}
	}, eventuallyTimeout, eventuallyPoll)

	got, ok := results.Get("issuer-a")
	require.True(t, ok)
	require.Equal(t, result, got)
}

func TestFetchedResultsDeleteRemovesRecord(t *testing.T) {
	results := NewFetchedResults[fetchedResultsTestResult]()
	results.Put(fetchedResultsTestResult{Key: "issuer-a", FetchedAt: time.Unix(100, 0), Value: "v1"})

	results.Delete("issuer-a")
	_, ok := results.Get("issuer-a")
	require.False(t, ok)
	results.Delete("issuer-a") // absent key is a safe no-op
}

func TestFetchedResultsDeleteObjectsRemovesMatchingRecords(t *testing.T) {
	results := NewFetchedResults[fetchedResultsTestResult]()
	results.Put(fetchedResultsTestResult{Key: "keep", FetchedAt: time.Unix(100, 0), Value: "keep"})
	results.Put(fetchedResultsTestResult{Key: "delete", FetchedAt: time.Unix(100, 0), Value: "delete"})

	results.DeleteObjects(func(record fetchedResultsTestResult) bool {
		return record.Value == "delete"
	})

	_, ok := results.Get("delete")
	require.False(t, ok)
	got, ok := results.Get("keep")
	require.True(t, ok)
	require.Equal(t, "keep", got.Value)
}

func TestFetchedResultsResetReplacesSnapshot(t *testing.T) {
	results := NewFetchedResults[fetchedResultsTestResult]()
	results.Put(fetchedResultsTestResult{Key: "stale", FetchedAt: time.Unix(100, 0), Value: "stale"})

	fresh := fetchedResultsTestResult{Key: "fresh", FetchedAt: time.Unix(200, 0), Value: "fresh"}
	results.Reset([]fetchedResultsTestResult{fresh})

	_, ok := results.Get("stale")
	require.False(t, ok)
	got, ok := results.Get("fresh")
	require.True(t, ok)
	require.Equal(t, fresh, got)
}
