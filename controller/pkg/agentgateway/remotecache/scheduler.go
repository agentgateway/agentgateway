package remotecache

import (
	"container/heap"
	"sync"
	"time"

	"istio.io/istio/pkg/util/sets"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

// FetchAt tracks a scheduled fetch event.
type FetchAt struct {
	At           time.Time
	RequestKey   remotehttp.FetchKey
	Generation   uint64
	RetryAttempt int
	Index        int
}

type fetchHeap []*FetchAt

func (h fetchHeap) Len() int           { return len(h) }
func (h fetchHeap) Less(i, j int) bool { return h[i].At.Before(h[j].At) }
func (h fetchHeap) Swap(i, j int) {
	h[i], h[j] = h[j], h[i]
	h[i].Index = i
	h[j].Index = j
}

func (h *fetchHeap) Push(x any) {
	entry := x.(*FetchAt)
	entry.Index = len(*h)
	*h = append(*h, entry)
}

func (h *fetchHeap) Pop() any {
	old := *h
	n := len(old)
	entry := old[n-1]
	entry.Index = -1
	old[n-1] = nil
	*h = old[:n-1]
	return entry
}

// Schedule manages a heap of scheduled fetch requests.
type Schedule struct {
	heap      fetchHeap
	scheduled map[remotehttp.FetchKey]*FetchAt
}

func NewSchedule() *Schedule {
	s := &Schedule{
		heap:      make(fetchHeap, 0),
		scheduled: make(map[remotehttp.FetchKey]*FetchAt),
	}
	heap.Init(&s.heap)
	return s
}

// Peek returns a copy of the next scheduled entry, or zero+false if empty.
// Returning by value prevents external callers from mutating heap internals.
func (s *Schedule) Peek() (FetchAt, bool) {
	if len(s.heap) == 0 {
		return FetchAt{}, false
	}
	return *s.heap[0], true
}

func (s *Schedule) Len() int {
	return len(s.heap)
}

func (s *Schedule) PopDue(now time.Time) []FetchAt {
	var due []FetchAt
	for {
		next, ok := s.Peek()
		if !ok || next.At.After(now) {
			return due
		}
		entry := heap.Pop(&s.heap).(*FetchAt)
		delete(s.scheduled, entry.RequestKey)
		due = append(due, *entry)
	}
}

func (s *Schedule) Schedule(requestKey remotehttp.FetchKey, generation uint64, at time.Time, retryAttempt int) {
	if scheduled := s.scheduled[requestKey]; scheduled != nil {
		scheduled.At = at
		scheduled.Generation = generation
		scheduled.RetryAttempt = retryAttempt
		heap.Fix(&s.heap, scheduled.Index)
		return
	}

	entry := &FetchAt{
		At:           at,
		RequestKey:   requestKey,
		Generation:   generation,
		RetryAttempt: retryAttempt,
		Index:        -1,
	}
	heap.Push(&s.heap, entry)
	s.scheduled[requestKey] = entry
}

func (s *Schedule) Remove(requestKey remotehttp.FetchKey) {
	if scheduled := s.scheduled[requestKey]; scheduled != nil {
		heap.Remove(&s.heap, scheduled.Index)
		delete(s.scheduled, requestKey)
	}
}

// Retry backoff parameters.
const (
	InitialRetryDelay = 100 * time.Millisecond
	MaxRetryDelay     = 15 * time.Second
	MaxRetryShift     = 30
)

// NextRetryDelay calculates the exponential backoff for the next retry attempt.
func NextRetryDelay(retryAttempt int) time.Duration {
	shift := min(retryAttempt+1, MaxRetryShift)

	next := InitialRetryDelay * time.Duration(1<<shift)
	if next > MaxRetryDelay {
		return MaxRetryDelay
	}
	return next
}

// UpdateFanout manages notification to multiple subscribers. Subscribe and
// Notify are safe to call concurrently from multiple goroutines.
type UpdateFanout struct {
	mu          sync.Mutex
	subscribers []chan sets.Set[remotehttp.FetchKey]
}

func NewUpdateFanout() *UpdateFanout {
	return &UpdateFanout{
		subscribers: make([]chan sets.Set[remotehttp.FetchKey], 0),
	}
}

func (f *UpdateFanout) Subscribe() <-chan sets.Set[remotehttp.FetchKey] {
	f.mu.Lock()
	defer f.mu.Unlock()
	subscriber := make(chan sets.Set[remotehttp.FetchKey], 1)
	f.subscribers = append(f.subscribers, subscriber)
	return subscriber
}

func (f *UpdateFanout) Notify(updates sets.Set[remotehttp.FetchKey]) {
	f.mu.Lock()
	defer f.mu.Unlock()
	for _, subscriber := range f.subscribers {
		merged := cloneRequestKeySet(updates)
		select {
		case existing := <-subscriber:
			merged.Merge(existing)
		default:
		}
		subscriber <- merged
	}
}

func cloneRequestKeySet(updates sets.Set[remotehttp.FetchKey]) sets.Set[remotehttp.FetchKey] {
	if updates == nil {
		return sets.New[remotehttp.FetchKey]()
	}
	return updates.Copy()
}

func SignalWake(wake chan<- struct{}) {
	select {
	case wake <- struct{}{}:
	default:
	}
}

func DrainTimer(timer *time.Timer) {
	if !timer.Stop() {
		select {
		case <-timer.C:
		default:
		}
	}
}
