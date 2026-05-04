package jwks

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/util/sets"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil/krttest"
)

func TestAddKeysetToFetcher(t *testing.T) {
	expected := testSharedJwksRequest("https://test/jwks")

	f := NewFetcher(NewCache())
	f.AddOrUpdate(expected)

	fetch, ok := f.NextFetchForTest()
	assert.True(t, ok)
	assert.Equal(t, expected.RequestKey, fetch.RequestKey)
	assert.Equal(t, 1, f.RequestCountForTest())
}

func TestRemoveKeysetFromFetcher(t *testing.T) {
	source := testSharedJwksRequest("https://test/jwks")
	cache := NewCache()
	f := NewFetcher(cache)

	f.AddOrUpdate(source)
	seedJwksCacheForTest(cache, source.RequestKey, source.Target.URL)

	f.Remove(source.RequestKey)

	assert.Equal(t, 0, f.RequestCountForTest())
	_, ok := cache.Get(source.RequestKey)
	assert.False(t, ok)
}

func TestRemoveKeysetClearsCacheEvenWithoutRequest(t *testing.T) {
	source := testSharedJwksRequest("https://test/jwks")
	cache := NewCache()
	f := NewFetcher(cache)
	seedJwksCacheForTest(cache, source.RequestKey, source.Target.URL)

	f.Remove(source.RequestKey)

	_, ok := cache.Get(source.RequestKey)
	assert.False(t, ok, "cache should be cleared even when request was not tracked")
}

func TestRetireKeysetKeepsCacheThenSweptOnSuccessfulFetch(t *testing.T) {
	ctx := t.Context()
	oldSource := testSharedJwksRequest("https://test/old-jwks")
	newSource := testSharedJwksRequest("https://test/new-jwks")

	cache := NewCache()
	f := NewFetcher(cache)
	f.AddOrUpdate(oldSource)
	seedJwksCacheForTest(cache, oldSource.RequestKey, oldSource.Target.URL)

	f.Retire(oldSource.RequestKey)

	assert.Equal(t, 0, f.RequestCountForTest(), "retired key should be removed from requests")
	_, inCache := cache.Get(oldSource.RequestKey)
	assert.True(t, inCache, "retired key should remain in cache")

	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		fmt.Fprint(w, sampleJWKS)
	}))
	defer backend.Close()
	newSource.Target.URL = backend.URL
	newSource.RequestKey = newSource.Target.Key()

	f.AddOrUpdate(newSource)

	updates := f.SubscribeToUpdates()
	go f.MaybeFetch(ctx)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		select {
		case update := <-updates:
			assert.True(c, update.Contains(newSource.RequestKey), "new key should be in updates")
			assert.True(c, update.Contains(oldSource.RequestKey), "swept old key should be in updates")
		default:
			assert.Fail(c, "no updates yet")
		}
	}, krttest.EventuallyTimeout, krttest.EventuallyPoll)

	_, inCache = cache.Get(oldSource.RequestKey)
	assert.False(t, inCache, "retired key should be swept after successful fetch")
}

func TestSuccessfulJwksFetch(t *testing.T) {
	ctx := t.Context()

	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		fmt.Fprint(w, sampleJWKS)
	}))
	defer backend.Close()

	cache := NewCache()
	f := NewFetcher(cache)
	source := testSharedJwksRequest(backend.URL)
	f.AddOrUpdate(source)
	updates := f.SubscribeToUpdates()

	go f.MaybeFetch(ctx)

	awaitJwksUpdate(t, updates, source.RequestKey)
	keyset := awaitStoredKeyset(t, cache, source.RequestKey)
	assert.Equal(t, sampleJWKS, keyset.JwksJSON)
}

func testSharedJwksRequest(requestURL string) SharedJwksRequest {
	target := remotehttp.FetchTarget{URL: requestURL}
	return SharedJwksRequest{
		RequestKey: target.Key(),
		Target:     target,
		TTL:        5 * time.Minute,
	}
}

func seedJwksCacheForTest(cache *JwksCache, requestKey remotehttp.FetchKey, url string) {
	cache.Put(Keyset{
		RequestKey: requestKey,
		URL:        url,
		JwksJSON:   `{"keys":[]}`,
	})
}

func awaitJwksUpdate(t *testing.T, updates <-chan sets.Set[remotehttp.FetchKey], requestKey remotehttp.FetchKey) {
	t.Helper()

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		select {
		case update := <-updates:
			assert.True(c, update.Contains(requestKey))
		default:
			assert.Fail(c, "no updates yet")
		}
	}, krttest.EventuallyTimeout, krttest.EventuallyPoll)
}

func awaitStoredKeyset(t *testing.T, cache *JwksCache, requestKey remotehttp.FetchKey) Keyset {
	t.Helper()

	var keyset Keyset
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		var ok bool
		keyset, ok = cache.Get(requestKey)
		assert.True(c, ok)
	}, krttest.EventuallyTimeout, krttest.EventuallyPoll)

	return keyset
}

var sampleJWKS = `{"keys":[]}`
