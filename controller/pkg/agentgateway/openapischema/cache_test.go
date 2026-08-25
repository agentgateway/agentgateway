package openapischema

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"

	"istio.io/istio/pkg/kube/krt"
)

// TestFetchConcurrencyIsBounded proves the gap a naive "spawn a goroutine
// per distinct URL" design would have: without a shared limiter, N distinct
// AgentgatewayBackend objects with N distinct schema.url values would open
// N concurrent outbound connections. maxConcurrentFetches must cap that
// regardless of how many distinct URLs are requested.
func TestFetchConcurrencyIsBounded(t *testing.T) {
	const totalURLs = maxConcurrentFetches + 5

	var (
		inFlight    atomic.Int64
		maxObserved atomic.Int64
	)
	unblock := make(chan struct{})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		cur := inFlight.Add(1)
		defer inFlight.Add(-1)
		for {
			old := maxObserved.Load()
			if cur <= old || maxObserved.CompareAndSwap(old, cur) {
				break
			}
		}
		<-unblock
		_, _ = w.Write([]byte(`{"openapi":"3.0.0"}`))
	}))
	t.Cleanup(srv.Close)
	t.Cleanup(func() { close(unblock) })

	c := NewCache(make(chan struct{}))
	for i := 0; i < totalURLs; i++ {
		u := fmt.Sprintf("%s/%d", srv.URL, i)
		if _, ready, _ := c.Get(krt.TestingDummyContext{}, u); ready {
			t.Fatalf("unexpected immediate ready for %s", u)
		}
	}

	deadline := time.Now().Add(2 * time.Second)
	for inFlight.Load() < maxConcurrentFetches && time.Now().Before(deadline) {
		time.Sleep(5 * time.Millisecond)
	}
	if got := inFlight.Load(); got != maxConcurrentFetches {
		t.Fatalf("expected exactly %d requests in flight once saturated, got %d", maxConcurrentFetches, got)
	}
	if got := maxObserved.Load(); got > maxConcurrentFetches {
		t.Fatalf("concurrency bound violated: observed %d concurrent fetches, want <= %d", got, maxConcurrentFetches)
	}
}

func TestGetNeverBlocksOnSlowUpstream(t *testing.T) {
	unblock := make(chan struct{})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		<-unblock // hangs until the test explicitly releases it
		_, _ = w.Write([]byte(`{"openapi":"3.0.0"}`))
	}))
	t.Cleanup(srv.Close)
	t.Cleanup(func() { close(unblock) })

	c := NewCache(make(chan struct{}))

	start := time.Now()
	schema, ready, err := c.Get(krt.TestingDummyContext{}, srv.URL)
	elapsed := time.Since(start)

	if elapsed > 500*time.Millisecond {
		t.Fatalf("Get blocked for %v on a hung upstream; it must return immediately", elapsed)
	}
	if ready {
		t.Fatalf("expected ready=false on first call (fetch just scheduled), got ready=true, schema=%q", schema)
	}
	if err == nil {
		t.Fatal("expected a 'will retry automatically' error on first call, got nil")
	}
}

func TestGetReturnsSchemaOnceFetchCompletes(t *testing.T) {
	const wantSchema = `{"openapi":"3.0.0","info":{"title":"t","version":"1"},"paths":{}}`
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(wantSchema))
	}))
	t.Cleanup(srv.Close)

	c := NewCache(make(chan struct{}))

	schema, ready, err := pollUntilReady(t, c, srv.URL)
	if err != nil {
		t.Fatalf("unexpected error after polling: %v", err)
	}
	if !ready {
		t.Fatal("expected ready=true after polling for completion")
	}
	if schema != wantSchema {
		t.Fatalf("schema mismatch: got %q, want %q", schema, wantSchema)
	}
}

func TestGetSurfacesFetchFailure(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusInternalServerError)
	}))
	t.Cleanup(srv.Close)

	c := NewCache(make(chan struct{}))

	deadline := time.Now().Add(2 * time.Second)
	var err error
	for time.Now().Before(deadline) {
		_, ready, e := c.Get(krt.TestingDummyContext{}, srv.URL)
		err = e
		if ready {
			t.Fatal("expected the fetch to fail (server returns 500), got ready=true")
		}
		if err != nil && !isPending(err) {
			break // got the real failure, not just "pending"
		}
		time.Sleep(10 * time.Millisecond)
	}
	if err == nil || isPending(err) {
		t.Fatalf("expected a concrete fetch-failure error within the deadline, got %v", err)
	}
}

func TestGetRejectsMalformedURL(t *testing.T) {
	c := NewCache(make(chan struct{}))

	cases := []string{"not-a-url", "ftp://example.com/spec.json", "https:///no-host"}
	for _, u := range cases {
		_, ready, err := c.Get(krt.TestingDummyContext{}, u)
		if ready {
			t.Errorf("%q: expected ready=false for a malformed URL", u)
		}
		if err == nil {
			t.Errorf("%q: expected an error for a malformed URL", u)
		}
	}
}

func TestGetEnforcesSizeLimit(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		chunk := make([]byte, 1<<20) // 1MB per write
		for written := 0; written < maxSchemaSize+2<<20; written += len(chunk) {
			if _, err := w.Write(chunk); err != nil {
				return
			}
		}
	}))
	t.Cleanup(srv.Close)

	c := NewCache(make(chan struct{}))

	deadline := time.Now().Add(2 * time.Second)
	var err error
	for time.Now().Before(deadline) {
		_, ready, e := c.Get(krt.TestingDummyContext{}, srv.URL)
		err = e
		if ready {
			t.Fatal("expected the fetch to fail (response exceeds size limit), got ready=true")
		}
		if err != nil && !isPending(err) {
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	if err == nil || isPending(err) {
		t.Fatalf("expected a size-limit error within the deadline, got %v", err)
	}
}

func isPending(err error) bool {
	return err != nil && err.Error() != "" && containsWillRetry(err.Error())
}

func containsWillRetry(s string) bool {
	const marker = "will retry automatically"
	for i := 0; i+len(marker) <= len(s); i++ {
		if s[i:i+len(marker)] == marker {
			return true
		}
	}
	return false
}

func pollUntilReady(t *testing.T, c *Cache, rawURL string) (schema string, ready bool, err error) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		schema, ready, err = c.Get(krt.TestingDummyContext{}, rawURL)
		if ready || (err != nil && !isPending(err)) {
			return schema, ready, err
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for fetch of %s to complete", rawURL)
	return "", false, nil
}
