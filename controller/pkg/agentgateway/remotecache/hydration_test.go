package remotecache

import (
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

type hydrationTestResult struct {
	Key       remotehttp.FetchKey
	FetchedAt time.Time
}

func (r hydrationTestResult) RemoteRequestKey() remotehttp.FetchKey { return r.Key }
func (r hydrationTestResult) RemoteFetchedAt() time.Time            { return r.FetchedAt }
func (r hydrationTestResult) Equals(other hydrationTestResult) bool {
	return r.Key == other.Key && r.FetchedAt.Equal(other.FetchedAt)
}

func entry(name string) Entry[hydrationTestResult] {
	return entryWithFetchedAt(name, time.Time{})
}

func entryWithFetchedAt(name string, fetchedAt time.Time) Entry[hydrationTestResult] {
	payload := hydrationTestResult{Key: "test-key", FetchedAt: fetchedAt}
	return Entry[hydrationTestResult]{
		NamespacedName: types.NamespacedName{Name: name},
		Payload:        &payload,
	}
}

func TestBestHydrationEntryPrefersCanonical(t *testing.T) {
	other := entry("other")
	canonical := entry("canonical")

	best := BestHydrationEntry([]Entry[hydrationTestResult]{other, canonical}, "canonical")
	require.Equal(t, "canonical", best.GetName())
}

func TestBestHydrationEntryPrefersFreshestPayload(t *testing.T) {
	canonical := entryWithFetchedAt("canonical", time.Unix(100, 0).UTC())
	other := entryWithFetchedAt("other", time.Unix(200, 0).UTC())

	best := BestHydrationEntry([]Entry[hydrationTestResult]{canonical, other}, "canonical")
	require.Equal(t, "other", best.GetName())
}

func TestBestHydrationEntryFallsBackToFirstWhenNoCanonical(t *testing.T) {
	first := entry("a")
	second := entry("b")

	best := BestHydrationEntry([]Entry[hydrationTestResult]{first, second}, "canonical")
	require.Equal(t, "a", best.GetName())
}

func TestBestHydrationEntryEmptyReturnsZero(t *testing.T) {
	best := BestHydrationEntry[hydrationTestResult](nil, "canonical")
	require.Equal(t, Entry[hydrationTestResult]{}, best)
}
