package oidc

import (
	"container/heap"
	"context"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"io"
	"math"
	"net"
	"net/http"
	"net/url"
	"sync"
	"time"

	"sigs.k8s.io/controller-runtime/pkg/log"
)

type ProviderFetcher struct {
	mu            sync.Mutex
	cache         *providerCache
	defaultClient *http.Client
	sources       map[string]*ProviderSource
	schedule      providerFetchingSchedule
	subscribers   []chan map[string]StoredProvider
}

type providerFetchingSchedule []providerFetchAt

type providerFetchAt struct {
	at           time.Time
	source       *ProviderSource
	retryAttempt int
}

const (
	providerFetchTimeout      = 30 * time.Second
	providerSubscriberBufSize = 1
)

func NewProviderFetcher(cache *providerCache) *ProviderFetcher {
	fetcher := &ProviderFetcher{
		cache:         cache,
		defaultClient: makeProviderHTTPClient(nil),
		sources:       make(map[string]*ProviderSource),
		schedule:      make([]providerFetchAt, 0),
		subscribers:   make([]chan map[string]StoredProvider, 0),
	}
	heap.Init(&fetcher.schedule)
	return fetcher
}

func makeProviderHTTPClient(tlsConfig *tls.Config) *http.Client {
	return &http.Client{
		Timeout: providerFetchTimeout,
		Transport: &http.Transport{
			TLSClientConfig: tlsConfig,
			DialContext: (&net.Dialer{
				Timeout: 5 * time.Second,
			}).DialContext,
			DisableKeepAlives: true,
		},
	}
}

func (s providerFetchingSchedule) Len() int           { return len(s) }
func (s providerFetchingSchedule) Less(i, j int) bool { return s[i].at.Before(s[j].at) }
func (s providerFetchingSchedule) Swap(i, j int)      { s[i], s[j] = s[j], s[i] }
func (s *providerFetchingSchedule) Push(x any) {
	*s = append(*s, x.(providerFetchAt))
}
func (s *providerFetchingSchedule) Pop() any {
	old := *s
	n := len(old)
	x := old[n-1]
	*s = old[:n-1]
	return x
}
func (s providerFetchingSchedule) Peek() *providerFetchAt {
	if len(s) == 0 {
		return nil
	}
	return &s[0]
}

func (f *ProviderFetcher) Run(ctx context.Context) {
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			f.maybeFetchProviders(ctx)
		}
	}
}

func (f *ProviderFetcher) maybeFetchProviders(ctx context.Context) {
	updates := make(map[string]StoredProvider)

	f.mu.Lock()
	now := time.Now()
	for {
		maybeFetch := f.schedule.Peek()
		if maybeFetch == nil || maybeFetch.at.After(now) {
			break
		}

		fetch := heap.Pop(&f.schedule).(providerFetchAt)
		if fetch.source.Deleted {
			continue
		}

		provider, err := f.fetchProvider(ctx, fetch.source)
		if err != nil {
			multiplier := time.Duration(math.Pow(2, float64(fetch.retryAttempt+1)))
			next := min(100*time.Millisecond*multiplier, 15*time.Second)
			logger.Error(
				"error fetching oidc provider discovery metadata",
				"issuer", fetch.source.Issuer,
				"error", err,
				"retryAttempt", fetch.retryAttempt,
				"next", next.String(),
			)
			heap.Push(&f.schedule, providerFetchAt{
				at:           now.Add(next),
				source:       fetch.source,
				retryAttempt: fetch.retryAttempt + 1,
			})
			continue
		}

		f.cache.AddProvider(fetch.source.ResourceKey, provider)
		heap.Push(&f.schedule, providerFetchAt{
			at:     now.Add(fetch.source.Ttl),
			source: fetch.source,
		})
		updates[fetch.source.ResourceKey] = provider
	}

	subscribers := append([]chan map[string]StoredProvider(nil), f.subscribers...)
	f.mu.Unlock()

	if len(updates) > 0 {
		for _, subscriber := range subscribers {
			subscriber <- updates
		}
	}
}

func (f *ProviderFetcher) SubscribeToUpdates() chan map[string]StoredProvider {
	f.mu.Lock()
	defer f.mu.Unlock()

	subscriber := make(chan map[string]StoredProvider, providerSubscriberBufSize)
	f.subscribers = append(f.subscribers, subscriber)
	return subscriber
}

func (f *ProviderFetcher) AddOrUpdateSource(source ProviderSource) error {
	if source.ResourceKey == "" {
		return fmt.Errorf("oidc provider source resource key is required")
	}
	if source.Issuer == "" {
		return fmt.Errorf("oidc provider source issuer is required")
	}
	if _, err := url.Parse(source.RequestURL); err != nil {
		return fmt.Errorf("error parsing oidc provider request url: %w", err)
	}
	if source.Ttl <= 0 {
		source.Ttl = DefaultProviderStoreTTL
	}

	f.mu.Lock()
	defer f.mu.Unlock()

	if existing, ok := f.sources[source.ResourceKey]; ok {
		delete(f.sources, source.ResourceKey)
		existing.Deleted = true
	}

	addedSource := source
	f.sources[source.ResourceKey] = &addedSource
	heap.Push(&f.schedule, providerFetchAt{
		at:     time.Now(),
		source: &addedSource,
	})
	return nil
}

func (f *ProviderFetcher) RemoveSource(source ProviderSource) {
	var subscribers []chan map[string]StoredProvider
	var update map[string]StoredProvider

	f.mu.Lock()
	if existing, ok := f.sources[source.ResourceKey]; ok {
		delete(f.sources, source.ResourceKey)
		f.cache.DeleteProvider(source.ResourceKey)
		existing.Deleted = true
		subscribers = append([]chan map[string]StoredProvider(nil), f.subscribers...)
		update = map[string]StoredProvider{source.ResourceKey: {}}
	}
	f.mu.Unlock()

	for _, subscriber := range subscribers {
		subscriber <- update
	}
}

func (f *ProviderFetcher) fetchProvider(
	ctx context.Context,
	source *ProviderSource,
) (StoredProvider, error) {
	logger := log.FromContext(ctx)
	logger.Info("fetching oidc discovery metadata", "issuer", source.Issuer, "url", source.RequestURL)

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, source.RequestURL, nil)
	if err != nil {
		return StoredProvider{}, fmt.Errorf("failed building request: %w", err)
	}
	if source.HostOverride != "" {
		req.Host = source.HostOverride
	}

	client := f.defaultClient
	if source.TlsConfig != nil {
		client = makeProviderHTTPClient(source.TlsConfig)
	}

	resp, err := client.Do(req)
	if err != nil {
		return StoredProvider{}, fmt.Errorf("request failed: %w", err)
	}
	defer resp.Body.Close() //nolint:errcheck

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return StoredProvider{}, fmt.Errorf(
			"request failed with status %d",
			resp.StatusCode,
		)
	}

	var metadata DiscoveryDocument
	if err := json.NewDecoder(io.LimitReader(resp.Body, 2<<20)).Decode(&metadata); err != nil {
		return StoredProvider{}, fmt.Errorf("failed decoding response body: %w", err)
	}
	if err := ValidateDiscoveryIssuer(source.Issuer, metadata.Issuer); err != nil {
		return StoredProvider{}, err
	}
	if err := ValidateDiscoveryMetadataEndpoints(&metadata); err != nil {
		return StoredProvider{}, fmt.Errorf(
			"invalid oidc discovery metadata for issuer %q: %w",
			source.Issuer,
			err,
		)
	}
	if metadata.AuthorizationEndpoint == "" || metadata.TokenEndpoint == "" || metadata.JwksURI == "" {
		return StoredProvider{}, fmt.Errorf(
			"oidc discovery document missing required endpoints for issuer %q",
			source.Issuer,
		)
	}

	return StoredProviderFromDiscovery(source.ResourceKey, metadata), nil
}
