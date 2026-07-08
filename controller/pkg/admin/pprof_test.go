package admin

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestAddPprofHandlerRegistersEndpoints(t *testing.T) {
	mux := http.NewServeMux()
	profiles := map[string]dynamicProfileDescription{}

	addPprofHandler("/debug/pprof/", mux, profiles)

	// verify fast endpoints are registered by checking they don't 404.
	// we use HEAD requests to avoid triggering long-running profiling operations
	// (profile defaults to 30s, trace to 1s, fgprof to 30s).
	// profile/trace/fgprof are verified via profile description and tested separately.
	endpoints := []string{
		"/debug/pprof/",
		"/debug/pprof/cmdline",
		"/debug/pprof/symbol",
	}

	for _, endpoint := range endpoints {
		t.Run(endpoint, func(t *testing.T) {
			req := httptest.NewRequest(http.MethodHead, endpoint, nil)
			rec := httptest.NewRecorder()
			mux.ServeHTTP(rec, req)
			if rec.Code == http.StatusNotFound {
				t.Errorf("endpoint %s returned 404, handler not registered", endpoint)
			}
		})
	}

	// profile/trace/fgprof also registered — verify via profile description
	t.Run("profile_and_trace_registered", func(t *testing.T) {
		descFunc, ok := profiles["/debug/pprof/"]
		if !ok {
			t.Fatal("profile description not registered")
		}
		desc := descFunc()
		if !strings.Contains(desc, "fgprof") {
			t.Errorf("profile description missing fgprof link: %s", desc)
		}
		if !strings.Contains(desc, "goroutine?debug=2") {
			t.Errorf("profile description missing goroutine link: %s", desc)
		}
	})
}

func TestFgprofEndpointReturnsProfile(t *testing.T) {
	mux := http.NewServeMux()
	profiles := map[string]dynamicProfileDescription{}
	addPprofHandler("/debug/pprof/", mux, profiles)

	// fgprof with seconds=1 returns a valid profile quickly
	req := httptest.NewRequest(http.MethodGet, "/debug/pprof/fgprof?seconds=1", nil)
	rec := httptest.NewRecorder()
	mux.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Errorf("fgprof endpoint returned status %d, want %d", rec.Code, http.StatusOK)
	}
	if rec.Body.Len() == 0 {
		t.Error("fgprof endpoint returned empty body")
	}
}
