package config

import (
	"bytes"
	"strings"
	"testing"
)

func TestParseBackendRowsIgnoresUnneededFields(t *testing.T) {
	raw := []byte(`{
		"unknown": "ignored",
		"services": [
			{
				"name": "echo",
				"namespace": "default",
				"hostname": "echo.default.svc.cluster.local",
				"endpoints": [
					{
						"active": {
							"//Pod/default/echo-b": {
								"endpoint": {
									"workloadUid": "//Pod/default/echo-b",
									"port": {"80": 80},
									"status": "Healthy"
								},
								"info": {
									"health": 1.0,
									"request_latency": 0.30290916037250004,
									"total_requests": 5
								}
							},
							"//Pod/default/echo-a": {
								"endpoint": {
									"workloadUid": "//Pod/default/echo-a",
									"status": "Healthy"
								},
								"info": {
									"health": 1.0,
									"request_latency": 0.0013023867314999999,
									"total_requests": 0
								}
							}
						},
						"rejected": {"ignored": true}
					}
				]
			}
		]
	}`)

	rows, err := parseBackendRows(raw, true)
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 2 {
		t.Fatalf("got %d rows, want 2", len(rows))
	}
	if rows[0].Endpoint != "echo-a" {
		t.Fatalf("rows not sorted by endpoint: %#v", rows)
	}
	if rows[0].Name != "echo" || rows[0].Namespace != "default" || rows[0].Health != "1.00" {
		t.Fatalf("unexpected row fields: %#v", rows[0])
	}
	if got, want := rows[0].Requests, int64(0); got != want {
		t.Fatalf("requests = %d, want %d", got, want)
	}
	if got, want := formatLatencyMS(rows[0]), ""; got != want {
		t.Fatalf("latency = %q, want %q", got, want)
	}
	if got, want := rows[1].Requests, int64(5); got != want {
		t.Fatalf("requests = %d, want %d", got, want)
	}
	if got, want := formatLatencyMS(rows[1]), "302.91ms"; got != want {
		t.Fatalf("latency = %q, want %q", got, want)
	}
}

func TestParseBackendRowsFiltersZeroRequestEndpointsByDefault(t *testing.T) {
	raw := []byte(`{
		"services": [
			{
				"name": "echo",
				"namespace": "default",
				"endpoints": [
					{
						"active": {
							"//Pod/default/echo-a": {
								"endpoint": {"workloadUid": "//Pod/default/echo-a"},
								"info": {
									"health": 1.0,
									"request_latency": 0.001,
									"total_requests": 0
								}
							},
							"//Pod/default/echo-b": {
								"endpoint": {"workloadUid": "//Pod/default/echo-b"},
								"info": {
									"health": 1.0,
									"request_latency": 0.302,
									"total_requests": 5
								}
							}
						}
					}
				]
			}
		]
	}`)

	rows, err := parseBackendRows(raw, false)
	if err != nil {
		t.Fatal(err)
	}
	if len(rows) != 1 {
		t.Fatalf("got %d rows, want 1", len(rows))
	}
	if rows[0].Endpoint != "echo-b" {
		t.Fatalf("unexpected endpoint: %#v", rows[0])
	}
}

func TestFormatEndpointName(t *testing.T) {
	tests := []struct {
		name      string
		endpoint  string
		namespace string
		want      string
	}{
		{
			name:      "matching pod namespace",
			endpoint:  "//Pod/default/echo-8fbd95d6d-fmrnq",
			namespace: "default",
			want:      "echo-8fbd95d6d-fmrnq",
		},
		{
			name:      "prefixed matching pod namespace",
			endpoint:  "/foo/Pod/default/echo-8fbd95d6d-fmrnq",
			namespace: "default",
			want:      "echo-8fbd95d6d-fmrnq",
		},
		{
			name:      "prefixed non matching pod namespace",
			endpoint:  "/foo/Pod/other/echo-8fbd95d6d-fmrnq",
			namespace: "default",
			want:      "other/echo-8fbd95d6d-fmrnq",
		},
		{
			name:      "non matching pod namespace",
			endpoint:  "//Pod/other/echo-8fbd95d6d-fmrnq",
			namespace: "default",
			want:      "other/echo-8fbd95d6d-fmrnq",
		},
		{
			name:      "non pod endpoint",
			endpoint:  "//Service/default/echo",
			namespace: "default",
			want:      "Service/default/echo",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := formatEndpointName(tt.endpoint, tt.namespace); got != tt.want {
				t.Fatalf("formatEndpointName(%q, %q) = %q, want %q", tt.endpoint, tt.namespace, got, tt.want)
			}
		})
	}
}

func TestPrintBackendTable(t *testing.T) {
	var out bytes.Buffer
	printBackendTable(&out, []backendRow{{
		Name:      "agentgateway",
		Namespace: "agentgateway-system",
		Endpoint:  "//Pod/agentgateway-system/agentgateway-6cbd9c8cb7-7t5xv",
		Health:    "1.00",
		Requests:  0,
		LatencyMS: 0,
	}})

	got := out.String()
	for _, want := range []string{"NAME", "NAMESPACE", "ENDPOINT", "HEALTH", "REQUESTS", "LATENCY"} {
		if !strings.Contains(got, want) {
			t.Fatalf("table output missing %q:\n%s", want, got)
		}
	}
	if strings.Contains(got, "0.00ms") {
		t.Fatalf("table output should not show latency for zero requests:\n%s", got)
	}
}
