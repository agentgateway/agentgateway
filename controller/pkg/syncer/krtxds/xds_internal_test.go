package krtxds

import "testing"

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
