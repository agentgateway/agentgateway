package krtxds

import (
	"testing"

	discovery "github.com/envoyproxy/go-control-plane/envoy/service/discovery/v3"
	"google.golang.org/genproto/googleapis/rpc/status"
	"istio.io/istio/pilot/pkg/model"

	"github.com/agentgateway/agentgateway/controller/pkg/metrics/metricstest"
)

func TestDiagnosticFeedback(t *testing.T) {
	const typeURL = "type.googleapis.com/agentgateway.dev.resource.Resource"
	for _, tt := range []struct {
		name         string
		message      string
		nonce        string
		acknowledged bool
		warnings     float64
		rejects      float64
	}{
		{
			name: "warnings", nonce: "nonce", acknowledged: true, warnings: 1,
			message: `[{"key":"policy/default","warn":"invalid expression"},{"key":"bind/default","warn":"cipher skipped"}]`,
		},
		{
			name: "mixed", nonce: "nonce", warnings: 1, rejects: 1,
			message: `[{"key":"policy/default","warn":"invalid expression"},{"key":"bind/default","error":"invalid enum"}]`,
		},
		{
			name: "error", nonce: "nonce", rejects: 1,
			message: `[{"key":"bind/default","error":"invalid enum"}]`,
		},
		{name: "legacy", nonce: "nonce", rejects: 1, message: "invalid backend"},
		{name: "malformed", nonce: "nonce", rejects: 1, message: `[{"key":"bind/default"}]`},
		{
			name: "stale warning", nonce: "old", warnings: 1,
			message: `[{"key":"policy/default","warn":"invalid expression"}]`,
		},
		{
			name: "stale error", nonce: "old", rejects: 1,
			message: `[{"key":"bind/default","error":"invalid enum"}]`,
		},
	} {
		t.Run(tt.name, func(t *testing.T) {
			xdsRejectsTotal.Reset()
			xdsWarningsTotal.Reset()
			xdsRejectsTotal.Add(0)
			xdsWarningsTotal.Add(0)
			con := &Connection{proxy: &Proxy{WatchedResources: map[string]*model.WatchedResource{
				typeURL: {TypeUrl: typeURL, NonceSent: "nonce", LastError: "previous error"},
			}}}
			req := &discovery.DeltaDiscoveryRequest{
				TypeUrl: typeURL, ResponseNonce: tt.nonce,
				ErrorDetail: &status.Status{Message: tt.message},
			}
			if shouldRespondDelta(con, req, nil) {
				t.Fatal("feedback should not trigger another response")
			}
			wr := con.proxy.GetWatchedResource(typeURL)
			if tt.acknowledged {
				if wr.NonceAcked != tt.nonce || wr.LastError != "" {
					t.Fatalf("warning should acknowledge config and clear rejection state: %#v", wr)
				}
			} else {
				expectedError := tt.message
				if tt.rejects == 0 {
					expectedError = "previous error"
				}
				if wr.NonceAcked != "" || wr.LastError != expectedError {
					t.Fatalf("unexpected rejection state: %#v", wr)
				}
			}
			m := metricstest.MustGatherMetrics(t)
			m.AssertMetric("agentgateway_xds_warnings_total", &metricstest.ExpectedMetric{Value: tt.warnings})
			m.AssertMetric("agentgateway_xds_rejects_total", &metricstest.ExpectedMetric{Value: tt.rejects})
		})
	}
}

func TestParseNackDiagnosticsStructuredJSON(t *testing.T) {
	message := `[{"key":"bind/default","warn":"cipher skipped"},{"key":"route/default","error":"invalid backend"}]`

	diagnostics, ok := parseNackDiagnostics(message)
	if !ok {
		t.Fatal("expected structured diagnostics to parse")
	}
	if len(diagnostics) != 2 {
		t.Fatalf("expected 2 diagnostics, got %d", len(diagnostics))
	}
	if diagnostics[0].Key != "bind/default" || diagnostics[0].Warn != "cipher skipped" {
		t.Fatalf("unexpected first diagnostic: %#v", diagnostics[0])
	}
	if diagnostics[1].Key != "route/default" || diagnostics[1].Error != "invalid backend" {
		t.Fatalf("unexpected second diagnostic: %#v", diagnostics[1])
	}
}

func TestParseNackDiagnosticsFallsBackForLegacyMessage(t *testing.T) {
	if diagnostics, ok := parseNackDiagnostics("bind/default: failed to parse"); ok || diagnostics != nil {
		t.Fatalf("expected legacy message to skip structured parsing, got %#v", diagnostics)
	}
}
