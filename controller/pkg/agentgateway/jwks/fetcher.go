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
	"istio.io/istio/pkg/util/sets"
	"sigs.k8s.io/controller-runtime/pkg/log"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

const (
	initialRetryDelay = 100 * time.Millisecond
	maxRetryDelay     = 15 * time.Second
	maxRetryShift     = 30
	clientTimeout     = 10 * time.Second
)

type fetchAt struct {
	At           time.Time
	RequestKey   remotehttp.FetchKey
	Generation   uint64
	RetryAttempt int
	index        int
}

type fetchHeap []*fetchAt

func (h fetchHeap) Len() int           { return len(h) }
func (h fetchHeap) Less(i, j int) bool { return h[i].At.Before(h[j].At) }
func (h fetchHeap) Swap(i, j int) {
	h[i], h[j] = h[j], h[i]
	h[i].index = i
	h[j].index = j
}

func (h *fetchHeap) Push(x any) {
	entry := x.(*fetchAt)
	entry.index = len(*h)
	*h = append(*h, entry)
}

func (h *fetchHeap) Pop() any {
	old := *h
	n := len(old)
	entry := old[n-1]
	entry.index = -1
	old[n-1] = nil
	*h = old[:n-1]
	return entry
}

type fetchSchedule struct {
	heap      fetchHeap
	scheduled map[remotehttp.FetchKey]*fetchAt
}

func newFetchSchedule() *fetchSchedule {
	s := &fetchSchedule{
		heap:      make(fetchHeap, 0),
		scheduled: make(map[remotehttp.FetchKey]*fetchAt),
	}
	heap.Init(&s.heap)
	return s
}

func (s *fetchSchedule) Len() int {
	return len(s.heap)
}

func (s *fetchSchedule) Peek() *fetchAt {
	if len(s.heap) == 0 {
		return nil
	}
	return s.heap[0]
}

func (s *fetchSchedule) PopDue(now time.Time) []fetchAt {
	var due []fetchAt
	for {
		next := s.Peek()
		if next == nil || next.At.After(now) {
			return due
		}
		entry := heap.Pop(&s.heap).(*fetchAt)
		delete(s.scheduled, entry.RequestKey)
		due = append(due, *entry)
	}
}

func (s *fetchSchedule) Schedule(requestKey remotehttp.FetchKey, generation uint64, at time.Time, retryAttempt int) {
	if scheduled := s.scheduled[requestKey]; scheduled != nil {
		scheduled.At = at
		scheduled.Generation = generation
		scheduled.RetryAttempt = retryAttempt
		heap.Fix(&s.heap, scheduled.index)
		return
	}

	entry := &fetchAt{
		At:           at,
		RequestKey:   requestKey,
		Generation:   generation,
		RetryAttempt: retryAttempt,
		index:        -1,
	}
	heap.Push(&s.heap, entry)
	s.scheduled[requestKey] = entry
}

func (s *fetchSchedule) Remove(requestKey remotehttp.FetchKey) {
	if scheduled := s.scheduled[requestKey]; scheduled != nil {
		heap.Remove(&s.heap, scheduled.index)
		delete(s.scheduled, requestKey)
	}
}

func nextRetryDelay(retryAttempt int) time.Duration {
	shift := min(retryAttempt+1, maxRetryShift)

	next := initialRetryDelay * time.Duration(1<<shift)
	if next > maxRetryDelay {
		return maxRetryDelay
	}
	return next
}

func makeFetchClient(tlsConfig *tls.Config, proxyURL string, proxyTLSConfig *tls.Config) (*http.Client, error) {
	dialer := &net.Dialer{Timeout: 5 * time.Second}
	transport := &http.Transport{
		TLSClientConfig:   tlsConfig,
		DialContext:       dialer.DialContext,
		DisableKeepAlives: true,
	}
	if proxyURL != "" {
		parsed, err := url.Parse(proxyURL)
		if err != nil {
			return nil, fmt.Errorf("error parsing proxy URL %q: %w", proxyURL, err)
		}
		if proxyTLSConfig != nil {
			// Downgrade the proxy URL scheme to http so that Go's transport
			// does not attempt its own TLS handshake to the proxy. Our custom
			// DialContext handles TLS with the proxy-specific configuration.
			httpProxy := *parsed
			httpProxy.Scheme = "http"
			transport.Proxy = http.ProxyURL(&httpProxy)
			transport.DialContext = proxyTLSDialContext(dialer, proxyTLSConfig)
		} else {
			transport.Proxy = http.ProxyURL(parsed)
		}
	}
	return &http.Client{
		Timeout:   clientTimeout,
		Transport: transport,
	}, nil
}

// proxyTLSDialContext returns a DialContext function that wraps TCP connections
// in TLS using the given proxy TLS configuration. This is used when the tunnel
// proxy backend has a TLS policy, so the CONNECT request is sent over TLS.
func proxyTLSDialContext(dialer *net.Dialer, proxyTLSConfig *tls.Config) func(ctx context.Context, network, addr string) (net.Conn, error) {
	return func(ctx context.Context, network, addr string) (net.Conn, error) {
		conn, err := dialer.DialContext(ctx, network, addr)
		if err != nil {
			return nil, err
		}
		tlsConn := tls.Client(conn, proxyTLSConfig.Clone())
		if err := tlsConn.HandshakeContext(ctx); err != nil {
			conn.Close() //nolint:errcheck
			return nil, err
		}
		return tlsConn, nil
	}
}

func drainTimer(timer *time.Timer) {
	if !timer.Stop() {
		select {
		case <-timer.C:
		default:
		}
	}
}

func signalWake(wake chan<- struct{}) {
	select {
	case wake <- struct{}{}:
	default:
	}
}

// Fetcher fetches and periodically refreshes remote JWKS keysets.
// Fetched keysets are stored in JwksCache and updates are sent to subscribers.
type Fetcher struct {
	mu                sync.Mutex
	cache             *JwksCache
	defaultJwksClient JwksHttpClient
	requests          map[remotehttp.FetchKey]fetchState
	schedule          *fetchSchedule
	subscribers       []chan sets.Set[remotehttp.FetchKey]
	wake              chan struct{}
}

type fetchState struct {
	source     JwksSource
	generation uint64
}

type JwksHttpClient interface {
	FetchJwks(ctx context.Context, target remotehttp.FetchTarget) (jose.JSONWebKeySet, error)
}

type jwksHttpClientImpl struct {
	Client *http.Client
}

func NewFetcher(cache *JwksCache) *Fetcher {
	// Default client has no TLS or proxy config, so makeFetchClient cannot fail.
	defaultClient, _ := makeFetchClient(nil, "", nil)
	return &Fetcher{
		cache:             cache,
		defaultJwksClient: &jwksHttpClientImpl{Client: defaultClient},
		requests:          make(map[remotehttp.FetchKey]fetchState),
		schedule:          newFetchSchedule(),
		subscribers:       make([]chan sets.Set[remotehttp.FetchKey], 0),
		wake:              make(chan struct{}, 1),
	}
}

func (f *Fetcher) Run(ctx context.Context) {
	timer := time.NewTimer(time.Hour)
	if !timer.Stop() {
		<-timer.C
	}
	defer timer.Stop()

	for {
		f.maybeFetchJwks(ctx)

		f.mu.Lock()
		next := f.schedule.Peek()
		var delay time.Duration
		if next == nil {
			delay = time.Hour
		} else {
			delay = time.Until(next.At)
		}
		f.mu.Unlock()

		if delay < 0 {
			delay = 0
		}
		timer.Reset(delay)

		select {
		case <-ctx.Done():
			return
		case <-timer.C:
		case <-f.wake:
			// Drain the timer if it fired concurrently with wake so the next loop
			// iteration can safely reset it after a request was added, updated, or removed.
			drainTimer(timer)
		}
	}
}

func (f *Fetcher) maybeFetchJwks(ctx context.Context) {
	now := time.Now()
	due := f.popDue(now)
	if len(due) == 0 {
		return
	}

	updates := sets.New[remotehttp.FetchKey]()
	for _, fetch := range due {
		state, ok := f.lookup(fetch.RequestKey)
		if !ok || state.generation != fetch.Generation {
			continue
		}

		logger.Debug("fetching jwks", "request_key", fetch.RequestKey, "target", state.source.Target.URL)

		requestURL, jwks, err := f.fetchJwks(ctx, state.source)
		if err != nil {
			next := nextRetryDelay(fetch.RetryAttempt)
			logger.Error("error fetching jwks", "request_key", fetch.RequestKey, "target", state.source.Target.URL, "error", err, "retryAttempt", fetch.RetryAttempt, "next", next.String())
			f.scheduleAt(fetch.RequestKey, state.generation, now.Add(next), fetch.RetryAttempt+1)
			continue
		}

		keyset, err := buildKeyset(fetch.RequestKey, requestURL, jwks)
		if err != nil {
			logger.Error("error adding jwks", "request_key", fetch.RequestKey, "jwks_uri", requestURL, "error", err)
			next := nextRetryDelay(fetch.RetryAttempt)
			f.scheduleAt(fetch.RequestKey, state.generation, now.Add(next), fetch.RetryAttempt+1)
			continue
		}

		if !f.commitFetchResult(fetch.RequestKey, fetch.Generation, keyset, now.Add(state.source.TTL)) {
			continue
		}
		updates.Insert(fetch.RequestKey)
	}

	if !updates.IsEmpty() {
		f.notifySubscribers(updates)
	}
}

func (f *Fetcher) SubscribeToUpdates() <-chan sets.Set[remotehttp.FetchKey] {
	f.mu.Lock()
	defer f.mu.Unlock()

	subscriber := make(chan sets.Set[remotehttp.FetchKey], 1)
	f.subscribers = append(f.subscribers, subscriber)

	return subscriber
}

func (f *Fetcher) AddOrUpdateKeyset(source JwksSource) error {
	if _, err := url.Parse(source.Target.URL); err != nil {
		return fmt.Errorf("error parsing jwks url %w", err)
	}

	nextFetchAt := time.Now()
	if cached, ok := f.cache.GetJwks(source.RequestKey); ok && !cached.FetchedAt.IsZero() {
		expiresAt := cached.FetchedAt.Add(source.TTL)
		if expiresAt.After(nextFetchAt) {
			nextFetchAt = expiresAt
		}
	}

	f.mu.Lock()
	defer f.mu.Unlock()

	state := f.requests[source.RequestKey]
	state.generation++
	state.source = source
	f.requests[source.RequestKey] = state
	f.scheduleAtLocked(source.RequestKey, state.generation, nextFetchAt, 0)

	return nil
}

// commitFetchResult publishes a freshly fetched keyset to the cache and
// re-schedules the next fetch, atomically under f.mu so that a concurrent
// RemoveKeyset cannot interleave between the liveness check and the cache
// write and leave a stale keyset behind. Returns false if the request has
// been removed or superseded by a newer generation since the fetch was
// dispatched; in that case the result is discarded.
func (f *Fetcher) commitFetchResult(requestKey remotehttp.FetchKey, generation uint64, keyset Keyset, nextFetchAt time.Time) bool {
	f.mu.Lock()
	defer f.mu.Unlock()

	state, ok := f.requests[requestKey]
	if !ok || state.generation != generation {
		return false
	}

	f.cache.putKeyset(keyset)
	f.scheduleAtLocked(requestKey, generation, nextFetchAt, 0)
	return true
}

// SweepOrphans drops any cache entries that do not correspond to a live
// request. Intended to be called once at startup after the request collection
// has synced, to reconcile persisted keysets whose owning policies were
// deleted while the controller was down.
func (f *Fetcher) SweepOrphans() {
	f.mu.Lock()
	orphans := sets.New[remotehttp.FetchKey]()
	for _, key := range f.cache.Keys() {
		if _, ok := f.requests[key]; !ok {
			orphans.Insert(key)
			f.cache.deleteJwks(key)
		}
	}
	f.mu.Unlock()

	if orphans.IsEmpty() {
		return
	}

	f.notifySubscribers(orphans)
}

func (f *Fetcher) RemoveKeyset(requestKey remotehttp.FetchKey) {
	f.mu.Lock()
	_, hadRequest := f.requests[requestKey]
	if hadRequest {
		delete(f.requests, requestKey)
		f.schedule.Remove(requestKey)
	}
	hadCache := f.cache.deleteJwks(requestKey)
	f.mu.Unlock()

	if !hadRequest && !hadCache {
		return
	}

	f.notifySubscribers(sets.New(requestKey))
	if hadRequest {
		signalWake(f.wake)
	}
}

func (f *Fetcher) fetchJwks(ctx context.Context, source JwksSource) (string, jose.JSONWebKeySet, error) {
	jwks, err := f.fetchJwksFromTarget(ctx, source.TLSConfig, source.Target, source.ProxyTLSConfig)
	if err != nil {
		return "", jose.JSONWebKeySet{}, err
	}
	return source.Target.URL, jwks, nil
}

func (f *Fetcher) fetchJwksFromTarget(ctx context.Context, tlsConfig *tls.Config, target remotehttp.FetchTarget, proxyTLSConfig *tls.Config) (jose.JSONWebKeySet, error) {
	if tlsConfig != nil || target.ProxyURL != "" {
		client, err := makeFetchClient(tlsConfig, target.ProxyURL, proxyTLSConfig)
		if err != nil {
			return jose.JSONWebKeySet{}, err
		}
		return (&jwksHttpClientImpl{Client: client}).FetchJwks(ctx, target)
	}
	return f.defaultJwksClient.FetchJwks(ctx, target)
}

func (c *jwksHttpClientImpl) FetchJwks(ctx context.Context, target remotehttp.FetchTarget) (jose.JSONWebKeySet, error) {
	log := log.FromContext(ctx)
	log.Info("fetching jwks", "url", target.URL)

	request, err := http.NewRequestWithContext(ctx, http.MethodGet, target.URL, nil)
	if err != nil {
		return jose.JSONWebKeySet{}, fmt.Errorf("could not build request to get JWKS: %w", err)
	}

	response, err := c.Client.Do(request)
	if err != nil {
		return jose.JSONWebKeySet{}, err
	}
	defer response.Body.Close() //nolint:errcheck

	if response.StatusCode != http.StatusOK {
		return jose.JSONWebKeySet{}, fmt.Errorf("unexpected status code from jwks endpoint at %s: %d", target.URL, response.StatusCode)
	}

	var jwks jose.JSONWebKeySet
	if err := json.NewDecoder(io.LimitReader(response.Body, 1<<20)).Decode(&jwks); err != nil {
		return jose.JSONWebKeySet{}, fmt.Errorf("could not decode jwks: %w", err)
	}

	return jwks, nil
}

func (f *Fetcher) popDue(now time.Time) []fetchAt {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.schedule.PopDue(now)
}

func (f *Fetcher) lookup(requestKey remotehttp.FetchKey) (fetchState, bool) {
	f.mu.Lock()
	defer f.mu.Unlock()

	state, ok := f.requests[requestKey]
	return state, ok
}

func (f *Fetcher) scheduleAt(requestKey remotehttp.FetchKey, generation uint64, at time.Time, retryAttempt int) {
	f.mu.Lock()
	defer f.mu.Unlock()

	f.scheduleAtLocked(requestKey, generation, at, retryAttempt)
}

func (f *Fetcher) scheduleAtLocked(requestKey remotehttp.FetchKey, generation uint64, at time.Time, retryAttempt int) {
	if _, ok := f.requests[requestKey]; !ok {
		return
	}

	f.schedule.Schedule(requestKey, generation, at, retryAttempt)
	signalWake(f.wake)
}

func (f *Fetcher) notifySubscribers(updates sets.Set[remotehttp.FetchKey]) {
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
