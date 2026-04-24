package oidc

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/util/sets"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

const (
	testEventuallyTimeout = 2 * time.Second
	testEventuallyPoll    = 20 * time.Millisecond
)

// sampleJWKS is a minimal JWKS JSON payload for test fixtures.
const sampleJWKS = `{"keys":[{"use":"sig","kty":"RSA","kid":"test","alg":"RS256","n":"test","e":"AQAB"}]}`

func TestAddProviderToFetcher(t *testing.T) {
	source := testOidcSource("https://issuer.example/.well-known/openid-configuration")

	f := NewFetcher(NewCache())
	assert.NoError(t, f.AddOrUpdateProvider(source))

	f.mu.Lock()
	defer f.mu.Unlock()

	fetch := f.schedule.Peek()
	assert.NotNil(t, fetch)
	assert.Equal(t, source.RequestKey, fetch.RequestKey)
	state, ok := f.requests[source.RequestKey]
	assert.True(t, ok)
	assert.Equal(t, source, state.source)
	assert.Equal(t, 1, f.schedule.Len())
}

func TestRemoveOidcFromFetcher(t *testing.T) {
	source := testOidcSource("https://issuer.example/.well-known/openid-configuration")
	f := NewFetcher(NewCache())

	assert.NoError(t, f.AddOrUpdateProvider(source))
	seedOidcCacheForTest(f.cache, source.RequestKey)

	f.RemoveOidc(source.RequestKey)

	f.mu.Lock()
	_, ok := f.requests[source.RequestKey]
	assert.Equal(t, 0, f.schedule.Len())
	f.mu.Unlock()
	assert.False(t, ok)
	_, ok = f.cache.GetProvider(source.RequestKey)
	assert.False(t, ok)
}

// RemoveOidc must clear the cache even when f.requests didn't own the key.
// Per #1618: a cache entry seeded by LoadPersistedProviders at startup without
// a corresponding fetcher request must still be evicted on RemoveOidc.
func TestRemoveOidcClearsCacheEvenWithoutRequest(t *testing.T) {
	source := testOidcSource("https://issuer.example/.well-known/openid-configuration")
	f := NewFetcher(NewCache())
	// Simulate LoadPersistedProviders populating the cache without the fetcher
	// ever seeing an AddOrUpdateProvider.
	seedOidcCacheForTest(f.cache, source.RequestKey)

	f.RemoveOidc(source.RequestKey)

	_, ok := f.cache.GetProvider(source.RequestKey)
	assert.False(t, ok, "cache should be cleared even when request was not tracked")
}

func TestAddOrUpdateProviderReplacesExistingScheduleEntry(t *testing.T) {
	f := NewFetcher(NewCache())
	source := testOidcSource("https://issuer.example/.well-known/openid-configuration")

	assert.NoError(t, f.AddOrUpdateProvider(source))
	assert.NoError(t, f.AddOrUpdateProvider(source))

	f.mu.Lock()
	defer f.mu.Unlock()

	assert.Equal(t, 1, f.schedule.Len())
	fetch := f.schedule.Peek()
	assert.NotNil(t, fetch)
	assert.Equal(t, source.RequestKey, fetch.RequestKey)
	assert.Equal(t, uint64(2), fetch.Generation)
}

// Per #1629: when the cache already has a fresh entry, AddOrUpdateProvider
// must schedule the next fetch at cached.FetchedAt + TTL, not time.Now().
func TestAddOrUpdateProviderUsesFreshCachedFetchedAtToDelayStartupRefresh(t *testing.T) {
	f := NewFetcher(NewCache())
	source := testOidcSource("https://issuer.example/.well-known/openid-configuration")
	freshFetchedAt := time.Now().Add(-1 * time.Minute).UTC()
	f.cache.providers[source.RequestKey] = DiscoveredProvider{
		RequestKey: source.RequestKey,
		IssuerURL:  "https://issuer.example",
		JwksJSON:   sampleJWKS,
		FetchedAt:  freshFetchedAt,
	}

	assert.NoError(t, f.AddOrUpdateProvider(source))

	f.mu.Lock()
	defer f.mu.Unlock()

	fetch := f.schedule.Peek()
	require.NotNil(t, fetch)
	assert.Equal(t, source.RequestKey, fetch.RequestKey)
	assert.WithinDuration(t, freshFetchedAt.Add(source.TTL), fetch.At, time.Second)
}

func TestAddOrUpdateProviderImmediatelyRefreshesStaleEntry(t *testing.T) {
	f := NewFetcher(NewCache())
	source := testOidcSource("https://issuer.example/.well-known/openid-configuration")
	f.cache.providers[source.RequestKey] = DiscoveredProvider{
		RequestKey: source.RequestKey,
		IssuerURL:  "https://issuer.example",
		JwksJSON:   sampleJWKS,
		FetchedAt:  time.Now().Add(-2 * source.TTL).UTC(),
	}

	before := time.Now()
	assert.NoError(t, f.AddOrUpdateProvider(source))
	after := time.Now()

	f.mu.Lock()
	defer f.mu.Unlock()

	fetch := f.schedule.Peek()
	require.NotNil(t, fetch)
	assert.Equal(t, source.RequestKey, fetch.RequestKey)
	assert.False(t, fetch.At.Before(before))
	assert.False(t, fetch.At.After(after))
}

func TestFetcherWithEmptySchedule(t *testing.T) {
	ctx := t.Context()

	f := NewFetcher(NewCache())
	updates := f.SubscribeToUpdates()
	go f.maybeFetchOidc(ctx)

	assert.Never(t, func() bool {
		select {
		case <-updates:
			return true
		default:
			return false
		}
	}, 1*time.Second, 100*time.Millisecond)
}

func TestSuccessfulOidcDiscoveryAndJwksFetch(t *testing.T) {
	ctx := t.Context()

	issuer := "https://issuer.example"
	discovery := discoveryDocument{
		Issuer:                "https://issuer.example",
		AuthorizationEndpoint: "https://issuer.example/auth",
		TokenEndpoint:         "https://issuer.example/token",
		JwksURI:               "https://issuer.example/jwks",
	}

	f := NewFetcher(NewCache())
	source := testOidcSource("https://issuer.example/.well-known/openid-configuration")
	assert.NoError(t, f.AddOrUpdateProvider(source))
	updates := f.SubscribeToUpdates()

	f.defaultClient = stubOidcClient{
		t:           t,
		issuer:      issuer,
		discovery:   discovery,
		jwksPayload: sampleJWKS,
	}
	go f.maybeFetchOidc(ctx)

	awaitOidcUpdate(t, updates, source.RequestKey)
	provider := awaitStoredProvider(t, f.cache, source.RequestKey)
	assert.Equal(t, sampleJWKS, provider.JwksJSON)
	assert.Equal(t, "https://issuer.example", provider.IssuerURL)
}

func TestOidcDiscoveryValidatesConfiguredIssuerNotFetchURL(t *testing.T) {
	ctx := t.Context()
	target := remotehttp.FetchTarget{URL: "http://dummy-idp.default:8081/realms/master/.well-known/openid-configuration"}
	source := OidcSource{
		OwnerKey:       testOidcOwnerKey(),
		RequestKey:     oidcRequestKey(target, "https://issuer.example/realms/master"),
		ExpectedIssuer: "https://issuer.example/realms/master",
		Target:         target,
		TTL:            5 * time.Minute,
	}

	f := NewFetcher(NewCache())
	assert.NoError(t, f.AddOrUpdateProvider(source))
	updates := f.SubscribeToUpdates()

	f.defaultClient = stubOidcClient{
		t: t,
		discovery: discoveryDocument{
			Issuer:        "https://issuer.example/realms/master",
			JwksURI:       "https://issuer.example/realms/master/jwks",
			TokenEndpoint: "https://issuer.example/realms/master/token",
		},
		jwksPayload: sampleJWKS,
	}
	go f.maybeFetchOidc(ctx)

	awaitOidcUpdate(t, updates, source.RequestKey)
	provider := awaitStoredProvider(t, f.cache, source.RequestKey)
	assert.Equal(t, source.ExpectedIssuer, provider.IssuerURL)
}

func TestIssuerMismatchReturnsError(t *testing.T) {
	ctx := t.Context()

	f := NewFetcher(NewCache())
	source := testOidcSource("https://issuer.example/.well-known/openid-configuration")
	assert.NoError(t, f.AddOrUpdateProvider(source))
	updates := f.SubscribeToUpdates()

	f.defaultClient = stubOidcClient{
		t: t,
		discovery: discoveryDocument{
			// Issuer doesn't match the URL.
			Issuer:        "https://other-issuer.example",
			JwksURI:       "https://other-issuer.example/jwks",
			TokenEndpoint: "https://other-issuer.example/token",
		},
		jwksPayload: sampleJWKS,
	}
	go f.maybeFetchOidc(ctx)

	// Should not produce an update on mismatch.
	assert.Never(t, func() bool {
		select {
		case <-updates:
			return true
		default:
			return false
		}
	}, 250*time.Millisecond, 10*time.Millisecond)

	// Should have scheduled a retry.
	retry := awaitOidcRetryAttempt(t, f, source.RequestKey, 1)
	assert.WithinDuration(t, time.Now().Add(200*time.Millisecond), retry.At, 2*time.Second)
}

func TestNetworkFailureTriggersRetry(t *testing.T) {
	ctx := t.Context()

	f := NewFetcher(NewCache())
	source := testOidcSource("https://issuer.example/.well-known/openid-configuration")
	assert.NoError(t, f.AddOrUpdateProvider(source))
	updates := f.SubscribeToUpdates()

	f.defaultClient = stubOidcClient{
		t:   t,
		err: fmt.Errorf("network failure"),
	}
	go f.maybeFetchOidc(ctx)

	assert.Never(t, func() bool {
		select {
		case <-updates:
			return true
		default:
			return false
		}
	}, 250*time.Millisecond, 10*time.Millisecond)

	retry := awaitOidcRetryAttempt(t, f, source.RequestKey, 1)
	assert.WithinDuration(t, time.Now().Add(200*time.Millisecond), retry.At, 2*time.Second)
}

// Per #1618: an in-flight fetch that completes after RemoveOidc must not
// repopulate the cache.
func TestFetcherDiscardedFetchDoesNotRepopulateRemovedProvider(t *testing.T) {
	ctx := t.Context()

	f := NewFetcher(NewCache())
	source := testOidcSource("https://issuer.example/.well-known/openid-configuration")
	assert.NoError(t, f.AddOrUpdateProvider(source))

	started := make(chan struct{})
	release := make(chan struct{})
	f.defaultClient = stubOidcClient{
		t: t,
		discovery: discoveryDocument{
			Issuer:        "https://issuer.example",
			JwksURI:       "https://issuer.example/jwks",
			TokenEndpoint: "https://issuer.example/token",
		},
		jwksPayload: sampleJWKS,
		started:     started,
		release:     release,
	}

	done := make(chan struct{})
	go func() {
		f.maybeFetchOidc(ctx)
		close(done)
	}()

	<-started
	f.RemoveOidc(source.RequestKey)
	close(release)
	<-done

	_, ok := f.cache.GetProvider(source.RequestKey)
	assert.False(t, ok)
}

func TestNotifySubscribersMergesPendingRequestKeyUpdates(t *testing.T) {
	f := NewFetcher(NewCache())
	updates := f.SubscribeToUpdates()
	first := testOidcSource("https://issuer-a.example/.well-known/openid-configuration")
	second := testOidcSource("https://issuer-b.example/.well-known/openid-configuration")

	f.notifySubscribers(sets.New(first.RequestKey))
	f.notifySubscribers(sets.New(second.RequestKey))

	actual := <-updates
	assert.True(t, actual.Contains(first.RequestKey))
	assert.True(t, actual.Contains(second.RequestKey))
}

func TestNextRetryDelayCapsWithoutOverflow(t *testing.T) {
	assert.Equal(t, 200*time.Millisecond, nextRetryDelay(0))
	assert.Equal(t, maxRetryDelay, nextRetryDelay(7))
	assert.Equal(t, maxRetryDelay, nextRetryDelay(36))
}

func TestTLSConfigHonoredForDiscovery(t *testing.T) {
	// Start a TLS server serving the discovery document.
	issuer := ""
	var discoveryServer *httptest.Server
	discoveryServer = httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		doc := discoveryDocument{
			Issuer:        issuer,
			JwksURI:       discoveryServer.URL + "/jwks",
			TokenEndpoint: discoveryServer.URL + "/token",
		}
		json.NewEncoder(w).Encode(doc) //nolint:errcheck
	}))
	defer discoveryServer.Close()
	issuer = discoveryServer.URL

	// JWKS server.
	jwksServer := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, sampleJWKS)
	}))
	defer jwksServer.Close()

	// Build a discovery URL: strip any path to get the issuer URL.
	target := remotehttp.FetchTarget{URL: discoveryServer.URL + "/.well-known/openid-configuration"}
	source := OidcSource{
		OwnerKey:       testOidcOwnerKey(),
		RequestKey:     oidcRequestKey(target, issuer),
		ExpectedIssuer: issuer,
		Target:         target,
		TLSConfig:      discoveryServer.Client().Transport.(*http.Transport).TLSClientConfig,
		TTL:            5 * time.Minute,
	}

	// Override the client to point JWKS at the TLS JWKS server.
	f := NewFetcher(NewCache())
	require.NoError(t, f.AddOrUpdateProvider(source))
	updates := f.SubscribeToUpdates()

	// Use a custom client that trusts both test servers.
	tlsCfg := discoveryServer.Client().Transport.(*http.Transport).TLSClientConfig
	client, err := makeFetchClient(tlsCfg, "", nil)
	require.NoError(t, err)
	f.defaultClient = &oidcHttpClientImpl{Client: client}

	go f.maybeFetchOidc(t.Context())

	awaitOidcUpdate(t, updates, source.RequestKey)
	provider := awaitStoredProvider(t, f.cache, source.RequestKey)
	assert.Equal(t, issuer, provider.IssuerURL)
}

func TestClientForReturnsProxyParseError(t *testing.T) {
	f := NewFetcher(NewCache())
	target := remotehttp.FetchTarget{
		URL:      "https://issuer.example/.well-known/openid-configuration",
		ProxyURL: "://missing-scheme",
	}

	client, err := f.clientFor(nil, target, nil)

	assert.Nil(t, client)
	assert.Error(t, err)
}

// ---- helpers ----------------------------------------------------------------

func testOidcOwnerKey() OidcOwnerID {
	return OidcOwnerID{
		Namespace: "default",
		Name:      "test",
		Path:      "spec.traffic.oidc",
	}
}

func testOidcSource(discoveryURL string) OidcSource {
	target := remotehttp.FetchTarget{URL: discoveryURL}
	expectedIssuer := strings.TrimSuffix(discoveryURL, "/.well-known/openid-configuration")
	return OidcSource{
		OwnerKey:       testOidcOwnerKey(),
		RequestKey:     oidcRequestKey(target, expectedIssuer),
		ExpectedIssuer: expectedIssuer,
		Target:         target,
		TTL:            5 * time.Minute,
	}
}

func seedOidcCacheForTest(cache *OidcCache, requestKey remotehttp.FetchKey) {
	cache.putProvider(DiscoveredProvider{
		RequestKey: requestKey,
		IssuerURL:  "https://issuer.example",
		JwksJSON:   sampleJWKS,
	})
}

// stubOidcClient is a test double for OidcHttpClient.
type stubOidcClient struct {
	t           *testing.T
	issuer      string
	discovery   discoveryDocument
	jwksPayload string
	err         error
	started     chan<- struct{}
	release     <-chan struct{}
}

func (s stubOidcClient) FetchDiscovery(_ context.Context, _ remotehttp.FetchTarget) (discoveryDocument, error) {
	if s.started != nil {
		close(s.started)
	}
	if s.release != nil {
		<-s.release
	}
	if s.err != nil {
		return discoveryDocument{}, s.err
	}
	return s.discovery, nil
}

func (s stubOidcClient) FetchJwks(_ context.Context, _ string) (string, error) {
	if s.err != nil {
		return "", s.err
	}
	return s.jwksPayload, nil
}

func awaitOidcUpdate(t *testing.T, updates <-chan sets.Set[remotehttp.FetchKey], requestKey remotehttp.FetchKey) {
	t.Helper()

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		select {
		case update := <-updates:
			assert.True(c, update.Contains(requestKey))
		default:
			assert.Fail(c, "no updates yet")
		}
	}, testEventuallyTimeout, testEventuallyPoll)
}

func awaitStoredProvider(t *testing.T, cache *OidcCache, requestKey remotehttp.FetchKey) DiscoveredProvider {
	t.Helper()

	var provider DiscoveredProvider
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		var ok bool
		provider, ok = cache.GetProvider(requestKey)
		assert.True(c, ok)
	}, testEventuallyTimeout, testEventuallyPoll)

	return provider
}

func awaitOidcRetry(t *testing.T, f *Fetcher) fetchAt {
	t.Helper()

	var retry fetchAt
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		f.mu.Lock()
		defer f.mu.Unlock()

		scheduled := f.schedule.Peek()
		if !assert.NotNil(c, scheduled) {
			return
		}
		retry = *scheduled
	}, testEventuallyTimeout, testEventuallyPoll)

	return retry
}

func awaitOidcRetryAttempt(t *testing.T, f *Fetcher, requestKey remotehttp.FetchKey, retryAttempt int) fetchAt {
	t.Helper()

	var retry fetchAt
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		retry = awaitOidcRetryNoWait(f)
		assert.Equal(c, requestKey, retry.RequestKey)
		assert.Equal(c, retryAttempt, retry.RetryAttempt)
	}, testEventuallyTimeout, testEventuallyPoll)

	return retry
}

func awaitOidcRetryNoWait(f *Fetcher) fetchAt {
	f.mu.Lock()
	defer f.mu.Unlock()

	scheduled := f.schedule.Peek()
	if scheduled == nil {
		return fetchAt{}
	}
	return *scheduled
}

var _ = awaitOidcRetry // referenced in store_test.go helpers
