package remotecache

import (
	"testing"
	"time"

	"github.com/stretchr/testify/require"
)

type stubEntry struct {
	fetchedAt time.Time
	name      string
	namespace string
}

func (e stubEntry) GetFetchedAt() time.Time { return e.fetchedAt }
func (e stubEntry) GetName() string         { return e.name }
func (e stubEntry) GetNamespace() string    { return e.namespace }

var _ Hydratable = stubEntry{}

func TestBestHydrationEntryPrefersLatestFetchedAt(t *testing.T) {
	old := stubEntry{fetchedAt: time.Unix(1000, 0), name: "old"}
	fresh := stubEntry{fetchedAt: time.Unix(2000, 0), name: "fresh"}

	best := BestHydrationEntry([]stubEntry{old, fresh}, "canonical")
	require.Equal(t, "fresh", best.name)

	best = BestHydrationEntry([]stubEntry{fresh, old}, "canonical")
	require.Equal(t, "fresh", best.name)
}

func TestBestHydrationEntryPrefersCanonicalOnTie(t *testing.T) {
	at := time.Unix(1000, 0)
	other := stubEntry{fetchedAt: at, name: "other"}
	canonical := stubEntry{fetchedAt: at, name: "canonical"}

	best := BestHydrationEntry([]stubEntry{other, canonical}, "canonical")
	require.Equal(t, "canonical", best.name)
}

func TestBestHydrationEntryLexicographicNameTieBreak(t *testing.T) {
	at := time.Unix(1000, 0)
	z := stubEntry{fetchedAt: at, name: "z"}
	a := stubEntry{fetchedAt: at, name: "a"}

	best := BestHydrationEntry([]stubEntry{z, a}, "canonical")
	require.Equal(t, "a", best.name)
}

func TestBestHydrationEntryNamespaceTieBreak(t *testing.T) {
	at := time.Unix(1000, 0)
	nsB := stubEntry{fetchedAt: at, name: "same", namespace: "b"}
	nsA := stubEntry{fetchedAt: at, name: "same", namespace: "a"}

	best := BestHydrationEntry([]stubEntry{nsB, nsA}, "canonical")
	require.Equal(t, "a", best.namespace)
}

func TestBestHydrationEntryEmptyReturnsZero(t *testing.T) {
	best := BestHydrationEntry[stubEntry](nil, "canonical")
	require.Equal(t, stubEntry{}, best)
}
