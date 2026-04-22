package jwks

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	"github.com/go-jose/go-jose/v4"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/util/sets"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

const (
	testEventuallyTimeout = 2 * time.Second
	testEventuallyPoll    = 20 * time.Millisecond
)

func TestAddKeysetToFetcher(t *testing.T) {
	expected := testSource()

	f := NewFetcher(NewCache())
	assert.NoError(t, f.AddOrUpdateKeyset(expected))

	f.mu.Lock()
	defer f.mu.Unlock()

	fetch := f.schedule.Peek()
	assert.NotNil(t, fetch)
	assert.Equal(t, expected.RequestKey, fetch.RequestKey)
	state, ok := f.requests[expected.RequestKey]
	assert.True(t, ok)
	assert.Equal(t, expected, state.source)
	assert.Equal(t, 1, f.schedule.Len())
}

func TestRemoveKeysetFromFetcher(t *testing.T) {
	source := testSource()
	f := NewFetcher(NewCache())

	assert.NoError(t, f.AddOrUpdateKeyset(source))
	seedJwksCacheForTest(f.cache, source.RequestKey, source.Target.URL)

	f.RemoveKeyset(source.RequestKey)

	f.mu.Lock()
	_, ok := f.requests[source.RequestKey]
	assert.Equal(t, 0, f.schedule.Len())
	f.mu.Unlock()
	assert.False(t, ok)
	_, ok = f.cache.GetJwks(source.RequestKey)
	assert.False(t, ok)
}

// RemoveKeyset must clear the cache even when f.requests didn't own the key.
// The cache can be seeded by LoadPersistedKeysets at startup without a
// corresponding Fetcher request; if a later event (e.g. manual CM deletion)
// drives RemoveKeyset, the early-return path leaves the cache stale.
func TestRemoveKeysetClearsCacheEvenWithoutRequest(t *testing.T) {
	source := testSource()
	f := NewFetcher(NewCache())
	// Simulate LoadPersistedKeysets populating the cache without the fetcher
	// ever seeing an AddOrUpdateKeyset.
	seedJwksCacheForTest(f.cache, source.RequestKey, source.Target.URL)

	f.RemoveKeyset(source.RequestKey)

	_, ok := f.cache.GetJwks(source.RequestKey)
	assert.False(t, ok, "cache should be cleared even when request was not tracked")
}

func TestAddOrUpdateKeysetReplacesExistingScheduleEntry(t *testing.T) {
	f := NewFetcher(NewCache())
	source := testSource()

	assert.NoError(t, f.AddOrUpdateKeyset(source))
	assert.NoError(t, f.AddOrUpdateKeyset(source))

	f.mu.Lock()
	defer f.mu.Unlock()

	assert.Equal(t, 1, f.schedule.Len())
	fetch := f.schedule.Peek()
	assert.NotNil(t, fetch)
	assert.Equal(t, source.RequestKey, fetch.RequestKey)
	assert.Equal(t, uint64(2), fetch.Generation)
}

func TestFetcherWithEmptyJwksFetchSchedule(t *testing.T) {
	ctx := t.Context()

	f := NewFetcher(NewCache())
	updates := f.SubscribeToUpdates()
	go f.maybeFetchJwks(ctx)

	assert.Never(t, func() bool {
		select {
		case <-updates:
			return true
		default:
			return false
		}
	}, 1*time.Second, 100*time.Millisecond)
}

func TestSuccessfulJwksFetch(t *testing.T) {
	ctx := t.Context()

	f := NewFetcher(NewCache())
	source := testSource()
	assert.NoError(t, f.AddOrUpdateKeyset(source))
	updates := f.SubscribeToUpdates()

	expectedJwks := jose.JSONWebKeySet{}
	err := json.Unmarshal([]byte(sampleJWKS), &expectedJwks)
	assert.NoError(t, err)

	f.defaultJwksClient = stubJwksClient{
		t:           t,
		expectedReq: source.Target,
		result:      expectedJwks,
	}
	go f.maybeFetchJwks(ctx)

	awaitJwksUpdate(t, updates, source.RequestKey)
	keyset := awaitStoredKeyset(t, f.cache, source.RequestKey)
	assert.Equal(t, sampleJWKS, keyset.JwksJSON)

	retry := awaitJwksRetry(t, f)
	assert.WithinDuration(t, time.Now().Add(5*time.Minute), retry.At, 3*time.Second)
}

func TestFetchJwksWithError(t *testing.T) {
	ctx := t.Context()

	f := NewFetcher(NewCache())
	source := testSource()
	assert.NoError(t, f.AddOrUpdateKeyset(source))
	updates := f.SubscribeToUpdates()

	f.defaultJwksClient = stubJwksClient{
		t:           t,
		expectedReq: source.Target,
		err:         fmt.Errorf("boom!"),
	}
	go f.maybeFetchJwks(ctx)

	assert.Never(t, func() bool {
		select {
		case <-updates:
			return true
		default:
			return false
		}
	}, 250*time.Millisecond, 10*time.Millisecond)

	retry := awaitJwksRetryAttempt(t, f, source.RequestKey, 1)
	assert.WithinDuration(t, time.Now().Add(200*time.Millisecond), retry.At, 2*time.Second)
}

func TestFetcherDiscardedFetchDoesNotRepopulateRemovedKeyset(t *testing.T) {
	ctx := t.Context()

	f := NewFetcher(NewCache())
	source := testSource()
	assert.NoError(t, f.AddOrUpdateKeyset(source))

	expectedJwks := jose.JSONWebKeySet{}
	err := json.Unmarshal([]byte(sampleJWKS), &expectedJwks)
	assert.NoError(t, err)

	started := make(chan struct{})
	release := make(chan struct{})
	f.defaultJwksClient = stubJwksClient{
		t:           t,
		expectedReq: source.Target,
		result:      expectedJwks,
		started:     started,
		release:     release,
	}

	done := make(chan struct{})
	go func() {
		f.maybeFetchJwks(ctx)
		close(done)
	}()

	<-started
	f.RemoveKeyset(source.RequestKey)
	close(release)
	<-done

	_, ok := f.cache.GetJwks(source.RequestKey)
	assert.False(t, ok)
}

func TestNotifySubscribersMergesPendingRequestKeyUpdates(t *testing.T) {
	f := NewFetcher(NewCache())
	updates := f.SubscribeToUpdates()
	first := testSource()
	second := testSourceWithURL("https://test/other-jwks")

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

func TestFetchJwksViaProxy(t *testing.T) {
	// Backend serves JWKS.
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		fmt.Fprint(w, sampleJWKS)
	}))
	defer backend.Close()

	// Forward proxy records the request and forwards it to the backend.
	var proxyRequestCount atomic.Int32
	proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		proxyRequestCount.Add(1)
		outReq, err := http.NewRequestWithContext(r.Context(), r.Method, r.URL.String(), r.Body)
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		resp, err := http.DefaultTransport.RoundTrip(outReq)
		if err != nil {
			http.Error(w, err.Error(), http.StatusBadGateway)
			return
		}
		defer resp.Body.Close()
		for k, vv := range resp.Header {
			for _, v := range vv {
				w.Header().Add(k, v)
			}
		}
		w.WriteHeader(resp.StatusCode)
		io.Copy(w, resp.Body) //nolint:errcheck
	}))
	defer proxy.Close()

	target := remotehttp.FetchTarget{
		URL:      backend.URL,
		ProxyURL: proxy.URL,
	}
	source := JwksSource{
		OwnerKey:   testOwnerKey(),
		RequestKey: target.Key(),
		Target:     target,
		TTL:        5 * time.Minute,
	}

	ctx := t.Context()
	f := NewFetcher(NewCache())
	require.NoError(t, f.AddOrUpdateKeyset(source))
	updates := f.SubscribeToUpdates()

	go f.maybeFetchJwks(ctx)

	awaitJwksUpdate(t, updates, source.RequestKey)
	keyset := awaitStoredKeyset(t, f.cache, source.RequestKey)
	assert.Equal(t, sampleJWKS, keyset.JwksJSON)
	assert.Equal(t, int32(1), proxyRequestCount.Load(), "request should have been routed through the proxy")
}

func TestFetchJwksViaProxyWithTLS(t *testing.T) {
	// Backend serves JWKS over TLS.
	backend := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		fmt.Fprint(w, sampleJWKS)
	}))
	defer backend.Close()

	// Forward proxy that handles CONNECT for HTTPS targets.
	var connectCount atomic.Int32
	proxy := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodConnect {
			http.Error(w, "expected CONNECT", http.StatusMethodNotAllowed)
			return
		}
		connectCount.Add(1)

		destConn, err := net.Dial("tcp", r.Host)
		if err != nil {
			http.Error(w, err.Error(), http.StatusBadGateway)
			return
		}
		w.WriteHeader(http.StatusOK)
		hijacker, ok := w.(http.Hijacker)
		if !ok {
			http.Error(w, "hijack not supported", http.StatusInternalServerError)
			return
		}
		clientConn, clientBuf, err := hijacker.Hijack()
		if err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		go func() {
			defer destConn.Close()
			io.Copy(destConn, clientBuf) //nolint:errcheck
		}()
		defer clientConn.Close()
		io.Copy(clientConn, destConn) //nolint:errcheck
	}))
	defer proxy.Close()

	// Use the test server's TLS config so the client trusts the backend cert.
	tlsConfig := &tls.Config{
		InsecureSkipVerify: true, //nolint:gosec // test only
	}

	target := remotehttp.FetchTarget{
		URL:      backend.URL,
		ProxyURL: proxy.URL,
	}
	source := JwksSource{
		OwnerKey:   testOwnerKey(),
		RequestKey: target.Key(),
		Target:     target,
		TLSConfig:  tlsConfig,
		TTL:        5 * time.Minute,
	}

	ctx := t.Context()
	f := NewFetcher(NewCache())
	require.NoError(t, f.AddOrUpdateKeyset(source))
	updates := f.SubscribeToUpdates()

	go f.maybeFetchJwks(ctx)

	awaitJwksUpdate(t, updates, source.RequestKey)
	keyset := awaitStoredKeyset(t, f.cache, source.RequestKey)
	assert.Equal(t, sampleJWKS, keyset.JwksJSON)
	assert.Equal(t, int32(1), connectCount.Load(), "HTTPS request should have used CONNECT through the proxy")
}

func TestMakeFetchClientRejectsInvalidProxyURL(t *testing.T) {
	_, err := makeFetchClient(nil, "://missing-scheme", nil)
	assert.Error(t, err)
	assert.Contains(t, err.Error(), "error parsing proxy URL")
}

func TestProxyURLAffectsFetchKey(t *testing.T) {
	a := remotehttp.FetchTarget{URL: "https://example.com/jwks"}
	b := remotehttp.FetchTarget{URL: "https://example.com/jwks", ProxyURL: "http://proxy:8080"}
	assert.NotEqual(t, a.Key(), b.Key(), "different proxy URLs should produce different fetch keys")
}

func testOwnerKey() JwksOwnerID {
	return JwksOwnerID{
		Kind:      OwnerKindPolicy,
		Namespace: "default",
		Name:      "test",
		Path:      "spec.traffic.jwtAuthentication.providers[0].jwks.remote",
	}
}

func testSource() JwksSource {
	return testSourceWithURL("https://test/jwks")
}

func testSourceWithURL(requestURL string) JwksSource {
	target := remotehttp.FetchTarget{URL: requestURL}
	return JwksSource{
		OwnerKey:   testOwnerKey(),
		RequestKey: target.Key(),
		Target:     target,
		TTL:        5 * time.Minute,
	}
}

// seedJwksCacheForTest writes a synthetic keyset into the cache through
// putKeyset so the cache lock is respected, rather than reaching into the
// unexported map directly.
func seedJwksCacheForTest(cache *JwksCache, requestKey remotehttp.FetchKey, url string) {
	cache.putKeyset(Keyset{
		RequestKey: requestKey,
		URL:        url,
		JwksJSON:   `{"keys":[]}`,
	})
}

type stubJwksClient struct {
	t           *testing.T
	expectedReq remotehttp.FetchTarget
	result      jose.JSONWebKeySet
	err         error
	started     chan<- struct{}
	release     <-chan struct{}
}

func (s stubJwksClient) FetchJwks(_ context.Context, req remotehttp.FetchTarget) (jose.JSONWebKeySet, error) {
	assert.Equal(s.t, s.expectedReq, req)
	if s.started != nil {
		close(s.started)
	}
	if s.release != nil {
		<-s.release
	}
	return s.result, s.err
}

var sampleJWKS = `{"keys":[{"use":"sig","kty":"RSA","kid":"JWxVLtipR-Q6wF2zmQKEoxbFhqwibK2aKNLyRqNxdj4","alg":"RS256","n":"5ApthhEwr6U00Coa0_572OytJXbVZKgl-myirM2m4GSrVfaKus41GEPHHXMzyGDPgHU7Rb4o0yzB-obkgz0zo2jnjv1zSx88BgdhhdE0BX2ULFDj67jVYdFZdCOoBr1_xJ5LEjQArHxfywZxW4a0egc3JaIwo-3qSSlRnD1KV2uzTG9FoDpvJLn1ZzdMgoTHuxIMla6WdgPDswVD8nrQM0I_1VGyGC0l2dICUEiqN0QrZen--U70J6EU6hd8vi_9qmALhjoSEASH2Z2sHco4Shv_aVx0BM-zN5UJWz4VF51Ag_KgcePS5Co7iVM0FUwMNWauWhPDPLWiXoUJvUWVPw","e":"AQAB","x5c":["MIICozCCAYsCBgGYyKDydjANBgkqhkiG9w0BAQsFADAVMRMwEQYDVQQDDAprYWdlbnQtZGV2MB4XDTI1MDgyMDE3NTU0N1oXDTM1MDgyMDE3NTcyN1owFTETMBEGA1UEAwwKa2FnZW50LWRldjCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAOQKbYYRMK+lNNAqGtP+e9jsrSV21WSoJfpsoqzNpuBkq1X2irrONRhDxx1zM8hgz4B1O0W+KNMswfqG5IM9M6No5479c0sfPAYHYYXRNAV9lCxQ4+u41WHRWXQjqAa9f8SeSxI0AKx8X8sGcVuGtHoHNyWiMKPt6kkpUZw9Sldrs0xvRaA6byS59Wc3TIKEx7sSDJWulnYDw7MFQ/J60DNCP9VRshgtJdnSAlBIqjdEK2Xp/vlO9CehFOoXfL4v/apgC4Y6EhAEh9mdrB3KOEob/2lcdATPszeVCVs+FRedQIPyoHHj0uQqO4lTNBVMDDVmrloTwzy1ol6FCb1FlT8CAwEAATANBgkqhkiG9w0BAQsFAAOCAQEAxElyp6gak62xC3yEw0nRUZNI0nsu0Oeow8ZwbmfTSa2hRKFQQe2sjMzm6L4Eyg2IInVn0spkw9BVJ07i8mDmvChjRNra7t6CX1dIykUUtxtNwglX0YBRjMl/heG7dC/dyDRVW6EUrPopMQ9QibzmH5XOBLDanTfK6tPwe5ezG5JF3JCx2Z3dtmAMtpCp7Nnr/gj48z7j4V8EHSB8hgITHBPcLOmiVglS3LF2/D+PK6efRWnVaDtcPmuh/0JmdmKxwJcvvuZD7tp5UFRbw9cgx5Pvv+mOWVCp/E2L+P17Gu0C/MC4Wnbn3Pi6Tgt0GNUMngCCyBnfcTpljUddW6Kheg=="],"x5t":"SmEthIFV9ehf3ggduek6QLfXxyU","x5t#S256":"XNGenWvGVC_sxSOTW0j_d7zwQlbGzkFj5XGCgPrLNJA"},{"use":"enc","kty":"RSA","kid":"hb2m-EP6nG_ktqHJOna_rnadxRaOtzArOecAJlNSmqU","alg":"RSA-OAEP","n":"xYU8uN6rXI6l6LAQ5inpylE4qiFqshbV92VnPrUO8gNff_TuZjvq19f0zXpVnnu88bCL5Q6DjRqRP4a2brAsYYBjSjwKGF3dd7jda6uavU1br2NFppZ6GSisOlKuKqMAUitQuYgAzYP-E2FasQOskrZ8HQ8S8hff7rNZH84VL5lNwTMHiwL1O8jBmxJE-ABM0To-2a9YosRkRa_uVzY720lSAir1UNiUSR1PypS2ixWyO04AVMJf8JgYU8rsUHNkZenYSRySzYzIxE57RCYnuZoc1hSVBtN2cFXXSqTwGMI7tfzTAtG11Z7zkiWmP0Tk7xabh5xfdXhZtJfHT6id5w","e":"AQAB","x5c":["MIICozCCAYsCBgGYyKD0zDANBgkqhkiG9w0BAQsFADAVMRMwEQYDVQQDDAprYWdlbnQtZGV2MB4XDTI1MDgyMDE3NTU0OFoXDTM1MDgyMDE3NTcyOFowFTETMBEGA1UEAwwKa2FnZW50LWRldjCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAMWFPLjeq1yOpeiwEOYp6cpROKoharIW1fdlZz61DvIDX3/07mY76tfX9M16VZ57vPGwi+UOg40akT+Gtm6wLGGAY0o8Chhd3Xe43Wurmr1NW69jRaaWehkorDpSriqjAFIrULmIAM2D/hNhWrEDrJK2fB0PEvIX3+6zWR/OFS+ZTcEzB4sC9TvIwZsSRPgATNE6PtmvWKLEZEWv7lc2O9tJUgIq9VDYlEkdT8qUtosVsjtOAFTCX/CYGFPK7FBzZGXp2Ekcks2MyMROe0QmJ7maHNYUlQbTdnBV10qk8BjCO7X80wLRtdWe85Ilpj9E5O8Wm4ecX3V4WbSXx0+onecCAwEAATANBgkqhkiG9w0BAQsFAAOCAQEAWuRnoKtKhCqLaz3Ze2q8hRykke7JwNrNxqDPn7eToa1MKsfsrtE678kzXhnfdivK/1F/8dr7Thn/WX7ZUJW2jsmbP1sCJjK02yY2setJ1jJKvJZcib8y7LAsqoACYZ4FM/KLrdywGn7KSenqWCLRMqeT04dWlmJexEszb5fgCKCFIZLKjaGJZIuLhsJBLyYHEVFpacr69cZ/ZjNpshHIiV0l/I434vcW39S9+uMfxf1glLTEPifmwK4gMRem3QQLqK21vBcjuS0GBQXQinaztcNaiu1invyTZd5s+3u5yORsip6YhbGhe08TbbtN7yLlZFITDQL4oFrXVGXX+4dp8w=="],"x5t":"BMlhx-2TUdiyftY8aR_zt7xECEI","x5t#S256":"YTTj8SxySpGgVFl5ZQqniLPnmg0gWHgBhissHXQCZ8k"}]}`

func awaitJwksUpdate(t *testing.T, updates <-chan sets.Set[remotehttp.FetchKey], requestKey remotehttp.FetchKey) {
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

func awaitStoredKeyset(t *testing.T, cache *JwksCache, requestKey remotehttp.FetchKey) Keyset {
	t.Helper()

	var keyset Keyset
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		var ok bool
		keyset, ok = cache.GetJwks(requestKey)
		assert.True(c, ok)
	}, testEventuallyTimeout, testEventuallyPoll)

	return keyset
}

func awaitJwksRetry(t *testing.T, f *Fetcher) fetchAt {
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

func awaitJwksRetryAttempt(t *testing.T, f *Fetcher, requestKey remotehttp.FetchKey, retryAttempt int) fetchAt {
	t.Helper()

	var retry fetchAt
	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		retry = awaitJwksRetryNoWait(f)
		assert.Equal(c, requestKey, retry.RequestKey)
		assert.Equal(c, retryAttempt, retry.RetryAttempt)
	}, testEventuallyTimeout, testEventuallyPoll)

	return retry
}

func awaitJwksRetryNoWait(f *Fetcher) fetchAt {
	f.mu.Lock()
	defer f.mu.Unlock()

	scheduled := f.schedule.Peek()
	if scheduled == nil {
		return fetchAt{}
	}
	return *scheduled
}
