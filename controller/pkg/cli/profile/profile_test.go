package profile

import (
	"context"
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
