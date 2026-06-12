package remotecache

import (
	"context"
	"log/slog"
	"sync"
	"time"

	"golang.org/x/time/rate"
	"k8s.io/client-go/util/workqueue"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

type Request interface {
	RemoteRequestKey() remotehttp.FetchKey
	RemoteTTL() time.Duration
}

// Result is F-bounded over T so payloadEquals can dispatch through the typed
// Equals method instead of reflect.DeepEqual on every KRT diff.
type Result[T any] interface {
	RemoteRequestKey() remotehttp.FetchKey
	RemoteFetchedAt() time.Time
	Equals(T) bool
}

// newFetchRateLimiter: per-key exponential backoff (100ms → 15s) under a
// 10qps/100-burst global cap shared across all retries.
func newFetchRateLimiter() workqueue.TypedRateLimiter[remotehttp.FetchKey] {
	return workqueue.NewTypedMaxOfRateLimiter[remotehttp.FetchKey](
		workqueue.NewTypedItemExponentialFailureRateLimiter[remotehttp.FetchKey](100*time.Millisecond, 15*time.Second),
		&workqueue.TypedBucketRateLimiter[remotehttp.FetchKey]{Limiter: rate.NewLimiter(rate.Limit(10), 100)},
	)
}

// Fetcher publishes fetched results into FetchedResults; scheduling uses a
// client-go workqueue (AddAfter for TTL refresh, AddRateLimited for backoff).
//
// workqueue.AddAfter is one-directional: a future-scheduled item can be pulled
// earlier but never pushed later, so a pending retry runs even after a fresh
// cached result lands. Net effect: one extra fetch on startup; equivalent
// sources content-hash to the same key and fetch identical material.
//
// Lock order: f.mu may be held across FetchedResults calls; callers MUST NOT
// acquire krt locks before f.mu.
type Fetcher[S Request, R Result[R]] struct {
	mu       sync.Mutex
	Results  *FetchedResults[R]
	Fetch    func(ctx context.Context, source S) (R, error)
	requests map[remotehttp.FetchKey]S
	queue    workqueue.TypedRateLimitingInterface[remotehttp.FetchKey]
	logger   *slog.Logger
}

func NewFetcher[S Request, R Result[R]](
	results *FetchedResults[R],
	fetch func(ctx context.Context, source S) (R, error),
	logger *slog.Logger,
) *Fetcher[S, R] {
	return &Fetcher[S, R]{
		Results:  results,
		Fetch:    fetch,
		requests: make(map[remotehttp.FetchKey]S),
		queue:    workqueue.NewTypedRateLimitingQueue(newFetchRateLimiter()),
		logger:   logger,
	}
}

// fetchWorkers bounds concurrent remote fetches so one slow or hung endpoint
// (up to the client timeout) cannot stall refreshes for every other issuer.
// The workqueue still serializes per key, so a given request is never fetched
// concurrently.
const fetchWorkers = 4

func (f *Fetcher[S, R]) Run(ctx context.Context) {
	done := make(chan struct{})
	defer close(done)
	go func() {
		select {
		case <-ctx.Done():
			f.queue.ShutDown()
		case <-done:
		}
	}()
	var wg sync.WaitGroup
	for range fetchWorkers {
		wg.Go(func() {
			for f.processOne(ctx) {
			}
		})
	}
	wg.Wait()
}

func (f *Fetcher[S, R]) processOne(ctx context.Context) bool {
	key, quit := f.queue.Get()
	if quit {
		return false
	}
	defer f.queue.Done(key)

	source, ok := f.lookup(key)
	if !ok {
		// Request retired between schedule and execute. Forget clears any
		// rate-limiter failure count for this key so a future re-AddOrUpdate
		// starts from a clean retry curve.
		f.queue.Forget(key)
		return true
	}

	f.logger.Debug("fetching remote resource", "request_key", key)
	result, err := f.Fetch(ctx, source)
	if err != nil {
		f.logger.Error("error fetching remote resource",
			"request_key", key,
			"error", err,
			"retryAttempt", f.queue.NumRequeues(key),
		)
		f.queue.AddRateLimited(key)
		return true
	}

	f.commit(key, result)
	return true
}

// commit publishes the result and schedules the next refresh at the CURRENT
// source's TTL, re-read under the lock so a concurrent shorter-TTL update
// isn't lost.
func (f *Fetcher[S, R]) commit(key remotehttp.FetchKey, result R) {
	f.mu.Lock()
	source, ok := f.requests[key]
	if !ok {
		f.mu.Unlock()
		return // retired during fetch
	}
	f.Results.Put(result)
	ttl := source.RemoteTTL()
	f.mu.Unlock()

	f.queue.Forget(key)
	f.queue.AddAfter(key, ttl)
}

func (f *Fetcher[S, R]) lookup(key remotehttp.FetchKey) (S, bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	source, ok := f.requests[key]
	return source, ok
}

func (f *Fetcher[S, R]) AddOrUpdate(source S) {
	key := source.RemoteRequestKey()

	f.mu.Lock()
	f.requests[key] = source
	f.mu.Unlock()

	// Avoid duplicate fetch on hydration: defer to the TTL boundary if a fresh
	// result is already cached.
	delay := time.Duration(0)
	if cached, ok := f.Results.Get(key); ok {
		if fetchedAt := cached.RemoteFetchedAt(); !fetchedAt.IsZero() {
			if remaining := time.Until(fetchedAt.Add(source.RemoteTTL())); remaining > 0 {
				delay = remaining
			}
		}
	}
	f.queue.AddAfter(key, delay) // duration <= 0 ⇒ immediate Add
}

func (f *Fetcher[S, R]) Remove(key remotehttp.FetchKey) {
	f.mu.Lock()
	delete(f.requests, key)
	f.mu.Unlock()

	f.queue.Forget(key)
	f.Results.Delete(key)
}

// SweepOrphans drops Results whose key is no longer live; used post-hydration
// to clear ConfigMaps whose owner has gone away.
func (f *Fetcher[S, R]) SweepOrphans() {
	f.mu.Lock()
	defer f.mu.Unlock()

	f.Results.DeleteObjects(func(record R) bool {
		_, ok := f.requests[record.RemoteRequestKey()]
		return !ok
	})
}

// ForTest methods expose internal state for white-box tests in sibling
// packages; not part of the public API.

// ReadyQueueLenForTest returns the active workqueue length. Items in the
// delayed heap (future AddAfter) are not counted — workqueue does not expose
// that length.
func (f *Fetcher[S, R]) ReadyQueueLenForTest() int {
	return f.queue.Len()
}

func (f *Fetcher[S, R]) RequestCountForTest() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return len(f.requests)
}
