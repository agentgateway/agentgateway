// Package openapischema provides a non-blocking cache for OpenAPI schemas
// fetched from user-supplied URLs (AgentgatewayBackend openapi.schema.url).
//
// AgentgatewayBackend translation runs on a single shared krt reconciliation
// queue used by every backend object cluster-wide. Fetching a schema
// synchronously on that queue means one slow or unreachable upstream blocks
// reconciliation of every other AgentgatewayBackend until it times out. This
// package moves the actual network fetch onto a background goroutine, keyed
// by URL, and exposes a Get that never blocks: it returns immediately with
// whatever is cached (nothing, a schema, or a prior error) and schedules a
// fetch if needed. Get registers a krt dependency on the cache entry, so the
// AgentgatewayBackend that called it is automatically re-reconciled once the
// background fetch completes.
package openapischema

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"sync"
	"time"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/ssrf"
)

const (
	// fetchTimeout bounds a single fetch attempt.
	fetchTimeout = 10 * time.Second
	// maxSchemaSize caps how much of a response we will read.
	maxSchemaSize = 5 << 20 // 5MB
	// failureCooldown is how long a failed fetch's error is served from cache
	// before a retry is scheduled, so a permanently broken URL isn't retried
	// on every reconcile, but a transient failure eventually self-heals.
	failureCooldown = 30 * time.Second
	// maxConcurrentFetches bounds how many schema fetches run at once,
	// regardless of how many distinct URLs are requested — mirrors the JWKS
	// fetcher's maxConcurrentFetches (controller/pkg/agentgateway/jwks).
	// Without this, a namespace user creating many AgentgatewayBackends with
	// distinct schema.url values could spawn unbounded concurrent outbound
	// connections.
	maxConcurrentFetches = 10
)

// Entry is one cached fetch result, keyed by URL.
type Entry struct {
	Key       string
	Schema    string
	Err       string
	FetchedAt time.Time
}

// ResourceName implements krt.ResourceNamer; it is what keys this entry in
// the backing krt.StaticCollection.
func (e Entry) ResourceName() string { return e.Key }

// Cache fetches and caches OpenAPI schemas from remote URLs. All network
// fetches run on background goroutines; Get never blocks on network I/O.
type Cache struct {
	collection krt.StaticCollection[Entry]
	client     *http.Client
	sem        chan struct{}
	stop       <-chan struct{}

	mu       sync.Mutex
	inFlight map[string]bool
}

// NewCache creates a Cache. Background fetch goroutines are best-effort and
// are abandoned (not explicitly canceled) when stop is closed; one already
// queued waiting for a concurrency slot gives up rather than leaking.
func NewCache(stop <-chan struct{}) *Cache {
	return &Cache{
		collection: krt.NewStaticCollection[Entry](nil, nil, krt.WithStop(stop), krt.WithName("openapi/SchemaCache")),
		client: &http.Client{
			Timeout: fetchTimeout,
			Transport: &http.Transport{
				DialContext: ssrf.SafeDialContext(&net.Dialer{Timeout: 5 * time.Second}),
			},
		},
		sem:      make(chan struct{}, maxConcurrentFetches),
		stop:     stop,
		inFlight: map[string]bool{},
	}
}

// Get returns the cached schema for rawURL. It never blocks on network I/O:
//   - ready=true: schema is populated, fetch succeeded.
//   - ready=false, err=nil: no attempt has completed yet; a fetch has been
//     scheduled (or one is already in flight) and will complete in the
//     background. The caller's krt input is now dependent on this entry and
//     will be re-reconciled automatically once it updates.
//   - ready=false, err!=nil: the most recent attempt failed (or rawURL is
//     malformed). A retry is scheduled automatically once failureCooldown
//     has elapsed since that attempt.
func (c *Cache) Get(ctx krt.HandlerContext, rawURL string) (schema string, ready bool, err error) {
	if _, err := validateSchemaURL(rawURL); err != nil {
		return "", false, err
	}

	entry := krt.FetchOne(ctx, c.collection, krt.FilterKey(rawURL))
	if entry != nil {
		if entry.Err == "" {
			return entry.Schema, true, nil
		}
		if time.Since(entry.FetchedAt) > failureCooldown {
			c.ensureFetch(rawURL)
		}
		return "", false, errors.New(entry.Err)
	}

	c.ensureFetch(rawURL)
	return "", false, fmt.Errorf("fetching openapi schema from %s (will retry automatically)", rawURL)
}

// ensureFetch schedules a background fetch for rawURL unless one is already
// in flight.
func (c *Cache) ensureFetch(rawURL string) {
	c.mu.Lock()
	if c.inFlight[rawURL] {
		c.mu.Unlock()
		return
	}
	c.inFlight[rawURL] = true
	c.mu.Unlock()

	go c.fetch(rawURL)
}

func (c *Cache) fetch(rawURL string) {
	defer func() {
		c.mu.Lock()
		delete(c.inFlight, rawURL)
		c.mu.Unlock()
	}()

	select {
	case c.sem <- struct{}{}:
		defer func() { <-c.sem }()
	case <-c.stop:
		return
	}

	schema, err := c.doFetch(rawURL)
	entry := Entry{Key: rawURL, FetchedAt: time.Now()}
	if err != nil {
		entry.Err = err.Error()
	} else {
		entry.Schema = schema
	}
	c.collection.UpdateObject(entry)
}

func validateSchemaURL(rawURL string) (*url.URL, error) {
	parsed, err := url.Parse(rawURL)
	if err != nil {
		return nil, fmt.Errorf("invalid openapi schema.url: %w", err)
	}
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		return nil, fmt.Errorf("openapi schema.url must use http or https, got %q", parsed.Scheme)
	}
	if parsed.Hostname() == "" {
		return nil, fmt.Errorf("openapi schema.url must include a host")
	}
	return parsed, nil
}

// doFetch performs a single bounded, SSRF-safe, synchronous HTTP fetch. It
// is only ever called from a background goroutine (see fetch), never from a
// krt reconciliation path.
func (c *Cache) doFetch(rawURL string) (string, error) {
	parsed, err := validateSchemaURL(rawURL)
	if err != nil {
		return "", err
	}

	req, err := http.NewRequestWithContext(context.Background(), http.MethodGet, parsed.String(), nil)
	if err != nil {
		return "", fmt.Errorf("building request for openapi schema.url %s: %w", rawURL, err)
	}
	resp, err := c.client.Do(req)
	if err != nil {
		return "", fmt.Errorf("fetching openapi schema from %s: %w", rawURL, err)
	}
	defer resp.Body.Close() //nolint:errcheck

	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("unexpected status %d fetching openapi schema from %s", resp.StatusCode, rawURL)
	}

	body, err := io.ReadAll(io.LimitReader(resp.Body, maxSchemaSize+1))
	if err != nil {
		return "", fmt.Errorf("reading openapi schema from %s: %w", rawURL, err)
	}
	if len(body) > maxSchemaSize {
		return "", fmt.Errorf("openapi schema from %s exceeds %d byte limit", rawURL, maxSchemaSize)
	}
	return string(body), nil
}
