package profile

import (
	"context"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestProfileURL(t *testing.T) {
	tests := []struct {
		name    string
		kind    profileKind
		seconds int
		want    string
	}{
		{
			name:    "cpu",
			kind:    profileKindCPU,
			seconds: 30,
			want:    "http://127.0.0.1:15000/debug/pprof/profile?seconds=30",
		},
		{
			name: "heap",
			kind: profileKindHeap,
			want: "http://127.0.0.1:15000/debug/pprof/heap",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := profileURL("127.0.0.1:15000", tt.kind, tt.seconds)
			if got != tt.want {
				t.Fatalf("got %q, want %q", got, tt.want)
			}
		})
	}
}

func TestDefaultOutputFile(t *testing.T) {
	now := time.Date(2026, 6, 6, 13, 45, 12, 0, time.UTC)
	got := defaultOutputFile(profileKindCPU, now)
	want := "agentgateway-cpu-20260606-134512.pb.gz"
	if got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

func TestProfileAdminAddressUsesIPv4ForLocalTarget(t *testing.T) {
	got, closeFn, err := profileAdminAddress(&profileTarget{Local: true}, 15000)
	if err != nil {
		t.Fatal(err)
	}
	defer closeFn()

	want := "127.0.0.1:15000"
	if got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

func TestProfileTimeout(t *testing.T) {
	tests := []struct {
		name    string
		kind    profileKind
		seconds int
		want    time.Duration
	}{
		{
			name:    "cpu",
			kind:    profileKindCPU,
			seconds: 30,
			want:    40 * time.Second,
		},
		{
			name: "heap",
			kind: profileKindHeap,
			want: 30 * time.Second,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := profileTimeout(tt.kind, tt.seconds)
			if got != tt.want {
				t.Fatalf("got %s, want %s", got, tt.want)
			}
		})
	}
}

func TestDownloadProfileWritesResponseBody(t *testing.T) {
	body := []byte("profile-bytes")
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/debug/pprof/profile" {
			t.Fatalf("got path %q, want /debug/pprof/profile", r.URL.Path)
		}
		if got := r.URL.Query().Get("seconds"); got != "17" {
			t.Fatalf("got seconds %q, want 17", got)
		}
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write(body)
	}))
	t.Cleanup(server.Close)

	outputFile := filepath.Join(t.TempDir(), "profile.pb.gz")
	if err := downloadProfile(context.Background(), server.Listener.Addr().String(), profileKindCPU, 17, outputFile); err != nil {
		t.Fatal(err)
	}

	got, err := os.ReadFile(outputFile)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != string(body) {
		t.Fatalf("got %q, want %q", got, body)
	}
}

func TestDownloadProfileReportsHTTPError(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		http.Error(w, "pprof disabled", http.StatusServiceUnavailable)
	}))
	t.Cleanup(server.Close)

	outputFile := filepath.Join(t.TempDir(), "profile.pb.gz")
	err := downloadProfile(context.Background(), server.Listener.Addr().String(), profileKindHeap, 0, outputFile)
	if err == nil {
		t.Fatal("expected error")
	}
	if _, statErr := os.Stat(outputFile); !os.IsNotExist(statErr) {
		t.Fatalf("output file should not be created, stat err: %v", statErr)
	}
}

func TestDownloadProfileRemovesPartialFileOnWriteFailure(t *testing.T) {
	originalClient := profileHTTPClient
	t.Cleanup(func() {
		profileHTTPClient = originalClient
	})

	profileHTTPClient = &http.Client{
		Transport: roundTripFunc(func(*http.Request) (*http.Response, error) {
			return &http.Response{
				StatusCode: http.StatusOK,
				Status:     "200 OK",
				Body:       io.NopCloser(&failingReader{}),
			}, nil
		}),
	}

	outputFile := filepath.Join(t.TempDir(), "profile.pb.gz")
	err := downloadProfile(context.Background(), "127.0.0.1:15000", profileKindHeap, 0, outputFile)
	if err == nil {
		t.Fatal("expected error")
	}
	if _, statErr := os.Stat(outputFile); !os.IsNotExist(statErr) {
		t.Fatalf("partial output file should be removed, stat err: %v", statErr)
	}
}

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(req *http.Request) (*http.Response, error) {
	return f(req)
}

type failingReader struct {
	readOnce bool
}

func (r *failingReader) Read(p []byte) (int, error) {
	if r.readOnce {
		return 0, errors.New("read failed")
	}
	r.readOnce = true
	return copy(p, "partial"), nil
}
