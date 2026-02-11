package jwks

import (
	"container/heap"
	"context"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"reflect"
	"sync"
	"time"

	"github.com/go-jose/go-jose/v4"
	"sigs.k8s.io/controller-runtime/pkg/log"
)

// JwksFetcher is used for fetching and periodic updates of jwks.
// Fetched jwks are stored in jwksCache. When a jwks is updated, registered subscribers are sent an update.
type JwksFetcher struct {
	mu                sync.Mutex
	cache             *jwksCache
	defaultJwksClient JwksHttpClient
	keysetSources     map[string]*JwksSource
	schedule          FetchingSchedule
	subscribers       []chan map[string]string
}

type FetchingSchedule []fetchAt

//go:generate go tool github.com/golang/mock/mockgen -destination mocks/mock_jwks_http_client.go -package mocks -source ./jwks_fetcher.go
type JwksHttpClient interface {
	FetchJwks(ctx context.Context, jwksURL string) (jose.JSONWebKeySet, error)
}

type JwksSource struct {
	JwksURL   string
	Ttl       time.Duration
	Deleted   bool
	TlsConfig *tls.Config
}

func (js JwksSource) ResourceName() string {
	return js.JwksURL
}

func (js JwksSource) Equals(other JwksSource) bool {
	return js.JwksURL == other.JwksURL &&
		js.Ttl == other.Ttl && js.Deleted == other.Deleted && reflect.DeepEqual(js.TlsConfig, other.TlsConfig)
}

type fetchAt struct {
	at           time.Time
	keysetSource *JwksSource
	retryAttempt int
}

type jwksHttpClientImpl struct {
	Client *http.Client
}

func NewJwksFetcher(cache *jwksCache) *JwksFetcher {
	toret := &JwksFetcher{
		cache:             cache,
		defaultJwksClient: &jwksHttpClientImpl{Client: &http.Client{}},
		keysetSources:     make(map[string]*JwksSource),
		schedule:          make([]fetchAt, 0),
		subscribers:       make([]chan map[string]string, 0),
	}
	heap.Init(&toret.schedule)

	return toret
}

// heap implementation
func (s FetchingSchedule) Len() int           { return len(s) }
func (s FetchingSchedule) Less(i, j int) bool { return s[i].at.Before(s[j].at) }
func (s FetchingSchedule) Swap(i, j int)      { s[i], s[j] = s[j], s[i] }
func (s *FetchingSchedule) Push(x any) {
	*s = append(*s, x.(fetchAt))
}
func (s *FetchingSchedule) Pop() any {
	old := *s
	n := len(old)
	x := old[n-1]
	*s = old[0 : n-1]
	return x
}
func (s FetchingSchedule) Peek() *fetchAt {
	if len(s) == 0 {
		return nil
	}
	return &s[0]
}

func (f *JwksFetcher) Run(ctx context.Context) {
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

func (f *JwksFetcher) maybeFetchJwks(ctx context.Context) {
	updates := make(map[string]string)

	f.mu.Lock()
	defer f.mu.Unlock()

	now := time.Now()
	for {
		maybeFetch := f.schedule.Peek()
		if maybeFetch == nil || maybeFetch.at.After(now) {
			break
		}

		fetch := heap.Pop(&f.schedule).(fetchAt)
		if fetch.keysetSource.Deleted {
			continue
		}

		logger.Debug("fetching remote jwks", "jwks_uri", fetch.keysetSource.JwksURL)

		jwks, err := f.fetchJwks(ctx, fetch.keysetSource.JwksURL, fetch.keysetSource.TlsConfig)
		if err != nil {
			logger.Error("error fetching jwks", "jwks_uri", fetch.keysetSource.JwksURL, "error", err)
			if fetch.retryAttempt < 5 { // backoff by 5s * retry attempt number
				heap.Push(&f.schedule, fetchAt{at: now.Add(time.Duration(5*(fetch.retryAttempt+1)) * time.Second), keysetSource: fetch.keysetSource, retryAttempt: fetch.retryAttempt + 1})
			} else {
				// give up retrying and schedule an update at a later time
				heap.Push(&f.schedule, fetchAt{at: now.Add(fetch.keysetSource.Ttl), keysetSource: fetch.keysetSource})
			}
			continue
		}

		updatedJwks, err := f.cache.addJwks(fetch.keysetSource.JwksURL, jwks)
		// error serializing jwks, shouldn't happen, retry
		if err != nil {
			logger.Error("error adding jwks", "jwks_uri", fetch.keysetSource.JwksURL, "error", err)
			heap.Push(&f.schedule, fetchAt{at: now.Add(time.Duration(5*(fetch.retryAttempt+1)) * time.Second), keysetSource: fetch.keysetSource, retryAttempt: fetch.retryAttempt + 1})
			continue
		}

		heap.Push(&f.schedule, fetchAt{at: now.Add(fetch.keysetSource.Ttl), keysetSource: fetch.keysetSource})
		updates[fetch.keysetSource.JwksURL] = updatedJwks
	}

	if len(updates) > 0 {
		for _, s := range f.subscribers {
			s <- updates
		}
	}
}

func (f *JwksFetcher) SubscribeToUpdates() chan map[string]string {
	f.mu.Lock()
	defer f.mu.Unlock()

	subscriber := make(chan map[string]string)
	f.subscribers = append(f.subscribers, subscriber)

	return subscriber
}

// handle http, https jwks source (default http(s) client), or a client with tls.Options
func (f *JwksFetcher) AddOrUpdateKeyset(source JwksSource) error {
	if _, err := url.Parse(source.JwksURL); err != nil {
		return fmt.Errorf("error parsing jwks url %w", err)
	}

	f.mu.Lock()
	defer f.mu.Unlock()

	if existingKeysetSource, ok := f.keysetSources[source.JwksURL]; ok {
		delete(f.keysetSources, source.JwksURL)
		existingKeysetSource.Deleted = true
	}

	addedKeysetSource := source
	f.keysetSources[source.JwksURL] = &addedKeysetSource
	heap.Push(&f.schedule, fetchAt{at: time.Now(), keysetSource: &addedKeysetSource}) // schedule an immediate fetch

	return nil
}

func (f *JwksFetcher) RemoveKeyset(source JwksSource) {
	f.mu.Lock()
	defer f.mu.Unlock()

	if beingDeleted, ok := f.keysetSources[source.JwksURL]; ok {
		delete(f.keysetSources, source.JwksURL)
		f.cache.deleteJwks(source.JwksURL)
		beingDeleted.Deleted = true

		for _, s := range f.subscribers {
			s <- map[string]string{source.JwksURL: ""}
		}
	}
}

func (f *JwksFetcher) fetchJwks(ctx context.Context, jwksURL string, tlsConfig *tls.Config) (jose.JSONWebKeySet, error) {
	if tlsConfig != nil {
		c := &jwksHttpClientImpl{Client: &http.Client{Transport: &http.Transport{TLSClientConfig: tlsConfig}}}
		return c.FetchJwks(ctx, jwksURL)
	}
	return f.defaultJwksClient.FetchJwks(ctx, jwksURL)
}

func (c *jwksHttpClientImpl) FetchJwks(ctx context.Context, jwksURL string) (jose.JSONWebKeySet, error) {
	log := log.FromContext(ctx)
	log.Info("fetching jwks", "url", jwksURL)

	request, err := http.NewRequest(http.MethodGet, jwksURL, nil)
	if err != nil {
		return jose.JSONWebKeySet{}, fmt.Errorf("could not build request to get JWKS: %w", err)
	}

	// TODO (dmitri-d) control the size here maybe?
	response, err := c.Client.Do(request)
	if err != nil {
		return jose.JSONWebKeySet{}, err
	}
	defer response.Body.Close() //nolint:errcheck

	if response.StatusCode != http.StatusOK {
		return jose.JSONWebKeySet{}, fmt.Errorf("unexpected status code from jwks endpoint at %s: %d", jwksURL, response.StatusCode)
	}

	var jwks jose.JSONWebKeySet
	if err := json.NewDecoder(response.Body).Decode(&jwks); err != nil {
		return jose.JSONWebKeySet{}, fmt.Errorf("could not decode jwks: %w", err)
	}

	return jwks, nil
}
