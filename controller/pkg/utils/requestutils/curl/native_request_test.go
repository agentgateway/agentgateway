package curl_test

import (
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strconv"
	"testing"
	"time"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
)

func TestExecuteRequestTimeoutContextStaysAliveUntilBodyClose(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
		if f, ok := w.(http.Flusher); ok {
			f.Flush()
		}
		time.Sleep(100 * time.Millisecond)
		_, _ = w.Write([]byte("ok"))
	}))
	t.Cleanup(srv.Close)

	parsed, err := url.Parse(srv.URL)
	if err != nil {
		t.Fatalf("parse server URL: %v", err)
	}
	port, err := strconv.Atoi(parsed.Port())
	if err != nil {
		t.Fatalf("parse server port: %v", err)
	}

	resp, err := curl.ExecuteRequest(
		curl.WithHost(parsed.Hostname()),
		curl.WithPort(port),
		curl.WithPath("/"),
		curl.WithConnectionTimeout(2),
	)
	if err != nil {
		t.Fatalf("execute request: %v", err)
	}
	t.Cleanup(func() {
		_ = resp.Body.Close()
	})

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatalf("read response body: %v", err)
	}
	if got := string(body); got != "ok" {
		t.Fatalf("unexpected body: got %q, want %q", got, "ok")
	}
}
