package jwks

import (
	"container/heap"
	"context"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"sync"
	"time"

	"github.com/go-jose/go-jose/v4"
	"sigs.k8s.io/controller-runtime/pkg/log"
)

// fetcher fetches and periodically refreshes remote JWKS artifacts.
// Fetched artifacts are stored in jwksCache and updates are sent to subscribers.
type fetcher struct {
	mu                sync.Mutex
	cache             *jwksCache
	defaultJwksClient JwksHttpClient
	requests          map[RequestKey]fetchState
	schedule          fetchingSchedule
	scheduled         map[RequestKey]*fetchAt
	subscribers       []chan map[RequestKey]struct{}
}

type fetchState struct {
	source     JwksSource
	generation uint64
}

type fetchingSchedule []*fetchAt

const (
	initialRetryDelay = 100 * time.Millisecond
	maxRetryDelay     = 15 * time.Second
	maxRetryShift     = 30
	clientTimeout     = 10 * time.Second
)

//go:generate mockgen -destination mocks/mock_jwks_http_client.go -package mocks -source ./jwks_fetcher.go
type JwksHttpClient interface {
	FetchJwks(ctx context.Context, req Request) (jose.JSONWebKeySet, error)
}

type fetchAt struct {
	at           time.Time
	requestKey   RequestKey
	generation   uint64
	retryAttempt int
	index        int
}

type jwksHttpClientImpl struct {
	Client *http.Client
}

func newFetcher(cache *jwksCache) *fetcher {
	fetcher := &fetcher{
		cache:             cache,
		defaultJwksClient: &jwksHttpClientImpl{Client: makeClient(nil)},
		requests:          make(map[RequestKey]fetchState),
		schedule:          make([]*fetchAt, 0),
		scheduled:         make(map[RequestKey]*fetchAt),
		subscribers:       make([]chan map[RequestKey]struct{}, 0),
	}
	heap.Init(&fetcher.schedule)

	return fetcher
}

func makeClient(t *tls.Config) *http.Client {
	return &http.Client{
		Timeout: clientTimeout,
		Transport: &http.Transport{
			TLSClientConfig: t,
			DialContext: (&net.Dialer{
				Timeout: 5 * time.Second,
			}).DialContext,
			DisableKeepAlives: true,
		},
	}
}

// heap implementation
func (s fetchingSchedule) Len() int           { return len(s) }
func (s fetchingSchedule) Less(i, j int) bool { return s[i].at.Before(s[j].at) }
func (s fetchingSchedule) Swap(i, j int) {
	s[i], s[j] = s[j], s[i]
	s[i].index = i
	s[j].index = j
}
func (s *fetchingSchedule) Push(x any) {
	entry := x.(*fetchAt)
	entry.index = len(*s)
	*s = append(*s, entry)
}
func (s *fetchingSchedule) Pop() any {
	old := *s
	n := len(old)
	x := old[n-1]
	x.index = -1
	old[n-1] = nil
	*s = old[0 : n-1]
	return x
}
func (s fetchingSchedule) Peek() *fetchAt {
	if len(s) == 0 {
		return nil
	}
	return s[0]
}

func nextRetryDelay(retryAttempt int) time.Duration {
	shift := min(retryAttempt+1, maxRetryShift)

	next := initialRetryDelay * time.Duration(1<<shift)
	if next > maxRetryDelay {
		return maxRetryDelay
	}

	return next
}

func (f *fetcher) Run(ctx context.Context) {
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			f.maybeFetchJwks(ctx)
		}
	}
}

func (f *fetcher) maybeFetchJwks(ctx context.Context) {
	now := time.Now()
	due := f.popDue(now)
	if len(due) == 0 {
		return
	}

	updates := make(map[RequestKey]struct{})
	for _, fetch := range due {
		state, ok := f.lookup(fetch.requestKey)
		if !ok || state.generation != fetch.generation {
			continue
		}

		logger.Debug("fetching remote jwks", "request_key", fetch.requestKey, "jwks_uri", state.source.Request.URL)

		jwks, err := f.fetchJwks(ctx, state.source)
		if err != nil {
			next := nextRetryDelay(fetch.retryAttempt)
			logger.Error("error fetching jwks", "request_key", fetch.requestKey, "jwks_uri", state.source.Request.URL, "error", err, "retryAttempt", fetch.retryAttempt, "next", next.String())
			f.scheduleAt(fetch.requestKey, state.generation, now.Add(next), fetch.retryAttempt+1)
			continue
		}

		state, ok = f.lookup(fetch.requestKey)
		if !ok || state.generation != fetch.generation {
			continue
		}

		if err := f.cache.addJwks(fetch.requestKey, state.source.Request.URL, jwks); err != nil {
			logger.Error("error adding jwks", "request_key", fetch.requestKey, "jwks_uri", state.source.Request.URL, "error", err)
			next := nextRetryDelay(fetch.retryAttempt)
			f.scheduleAt(fetch.requestKey, state.generation, now.Add(next), fetch.retryAttempt+1)
			continue
		}

		f.scheduleAt(fetch.requestKey, state.generation, now.Add(state.source.TTL), 0)
		updates[fetch.requestKey] = struct{}{}
	}

	if len(updates) > 0 {
		f.notifySubscribers(updates)
	}
}

func (f *fetcher) SubscribeToUpdates() <-chan map[RequestKey]struct{} {
	f.mu.Lock()
	defer f.mu.Unlock()

	subscriber := make(chan map[RequestKey]struct{}, 1)
	f.subscribers = append(f.subscribers, subscriber)

	return subscriber
}

func (f *fetcher) AddOrUpdateKeyset(source JwksSource) error {
	if _, err := url.Parse(source.Request.URL); err != nil {
		return fmt.Errorf("error parsing jwks url %w", err)
	}

	f.mu.Lock()
	defer f.mu.Unlock()

	state := f.requests[source.RequestKey]
	state.generation++
	state.source = source
	f.requests[source.RequestKey] = state
	f.scheduleAtLocked(source.RequestKey, state.generation, time.Now(), 0)

	return nil
}

func (f *fetcher) RemoveKeyset(requestKey RequestKey) {
	f.mu.Lock()
	_, ok := f.requests[requestKey]
	if ok {
		delete(f.requests, requestKey)
		if scheduled := f.scheduled[requestKey]; scheduled != nil {
			heap.Remove(&f.schedule, scheduled.index)
			delete(f.scheduled, requestKey)
		}
	}
	f.mu.Unlock()

	if !ok {
		return
	}

	f.cache.deleteJwks(requestKey)
	f.notifySubscribers(map[RequestKey]struct{}{requestKey: {}})
}

func (f *fetcher) fetchJwks(ctx context.Context, source JwksSource) (jose.JSONWebKeySet, error) {
	if source.TLSConfig != nil {
		return (&jwksHttpClientImpl{Client: makeClient(source.TLSConfig)}).FetchJwks(ctx, source.Request)
	}
	return f.defaultJwksClient.FetchJwks(ctx, source.Request)
}

func (c *jwksHttpClientImpl) FetchJwks(ctx context.Context, req Request) (jose.JSONWebKeySet, error) {
	log := log.FromContext(ctx)
	log.Info("fetching jwks", "url", req.URL)

	request, err := http.NewRequestWithContext(ctx, http.MethodGet, req.URL, nil)
	if err != nil {
		return jose.JSONWebKeySet{}, fmt.Errorf("could not build request to get JWKS: %w", err)
	}

	response, err := c.Client.Do(request)
	if err != nil {
		return jose.JSONWebKeySet{}, err
	}
	defer response.Body.Close() //nolint:errcheck

	if response.StatusCode != http.StatusOK {
		return jose.JSONWebKeySet{}, fmt.Errorf("unexpected status code from jwks endpoint at %s: %d", req.URL, response.StatusCode)
	}

	var jwks jose.JSONWebKeySet
	if err := json.NewDecoder(io.LimitReader(response.Body, 1<<20)).Decode(&jwks); err != nil {
		return jose.JSONWebKeySet{}, fmt.Errorf("could not decode jwks: %w", err)
	}

	return jwks, nil
}

func (f *fetcher) popDue(now time.Time) []fetchAt {
	f.mu.Lock()
	defer f.mu.Unlock()

	var due []fetchAt
	for {
		maybeFetch := f.schedule.Peek()
		if maybeFetch == nil || maybeFetch.at.After(now) {
			return due
		}
		fetch := heap.Pop(&f.schedule).(*fetchAt)
		delete(f.scheduled, fetch.requestKey)
		due = append(due, *fetch)
	}
}

func (f *fetcher) lookup(requestKey RequestKey) (fetchState, bool) {
	f.mu.Lock()
	defer f.mu.Unlock()

	state, ok := f.requests[requestKey]
	return state, ok
}

func (f *fetcher) scheduleAt(requestKey RequestKey, generation uint64, at time.Time, retryAttempt int) {
	f.mu.Lock()
	defer f.mu.Unlock()

	f.scheduleAtLocked(requestKey, generation, at, retryAttempt)
}

func (f *fetcher) scheduleAtLocked(requestKey RequestKey, generation uint64, at time.Time, retryAttempt int) {
	if _, ok := f.requests[requestKey]; !ok {
		return
	}

	if scheduled := f.scheduled[requestKey]; scheduled != nil {
		scheduled.at = at
		scheduled.generation = generation
		scheduled.retryAttempt = retryAttempt
		heap.Fix(&f.schedule, scheduled.index)
		return
	}

	entry := &fetchAt{
		at:           at,
		requestKey:   requestKey,
		generation:   generation,
		retryAttempt: retryAttempt,
		index:        -1,
	}
	heap.Push(&f.schedule, entry)
	f.scheduled[requestKey] = entry
}

func (f *fetcher) notifySubscribers(updates map[RequestKey]struct{}) {
	f.mu.Lock()
	defer f.mu.Unlock()

	for _, subscriber := range f.subscribers {
		merged := cloneRequestKeySet(updates)
		select {
		case existing := <-subscriber:
			for requestKey := range existing {
				merged[requestKey] = struct{}{}
			}
		default:
		}
		subscriber <- merged
	}
}

func cloneRequestKeySet(updates map[RequestKey]struct{}) map[RequestKey]struct{} {
	cloned := make(map[RequestKey]struct{}, len(updates))
	for requestKey := range updates {
		cloned[requestKey] = struct{}{}
	}
	return cloned
}
