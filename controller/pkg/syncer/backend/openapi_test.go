package agentgatewaybackend

import "testing"

func TestSplitOpenAPIHost(t *testing.T) {
	cases := []struct {
		name     string
		in       string
		wantHost string
		wantPort int32
		wantErr  string
	}{
		{name: "host and port", in: "localhost:8080", wantHost: "localhost", wantPort: 8080},
		{name: "ip and port", in: "10.0.0.5:9090", wantHost: "10.0.0.5", wantPort: 9090},
		{name: "bracketed ipv6 and port", in: "[::1]:8080", wantHost: "::1", wantPort: 8080},
		{name: "bare hostname defaults to port 80", in: "localhost", wantHost: "localhost", wantPort: 80},
		{name: "bare ip defaults to port 80", in: "10.0.0.5", wantHost: "10.0.0.5", wantPort: 80},
		{name: "fqdn defaults to port 80", in: "petstore.example.com", wantHost: "petstore.example.com", wantPort: 80},
		{name: "scheme prefix is rejected, not guessed", in: "https://localhost:8080", wantErr: "must not include a scheme"},
		{name: "http scheme prefix is also rejected", in: "http://localhost", wantErr: "must not include a scheme"},
		{name: "invalid port", in: "localhost:not-a-port", wantErr: "invalid port"},
		{name: "port zero is rejected", in: "localhost:0", wantErr: "invalid port"},
		{name: "empty string", in: "", wantErr: "must be in"},
		{name: "path-like value is rejected, not treated as a hostname", in: "localhost/openapi.json", wantErr: "must be in"},
		{name: "trailing colon with no port", in: "localhost:", wantErr: "invalid port"},
	}

	for _, tt := range cases {
		t.Run(tt.name, func(t *testing.T) {
			host, port, err := splitOpenAPIHost(tt.in)
			if tt.wantErr != "" {
				if err == nil {
					t.Fatalf("expected error containing %q, got host=%q port=%d", tt.wantErr, host, port)
				}
				if !containsSubstring(err.Error(), tt.wantErr) {
					t.Fatalf("expected error containing %q, got %q", tt.wantErr, err.Error())
				}
				return
			}
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if host != tt.wantHost || port != tt.wantPort {
				t.Fatalf("got host=%q port=%d, want host=%q port=%d", host, port, tt.wantHost, tt.wantPort)
			}
		})
	}
}

func TestValidateOpenAPISchema(t *testing.T) {
	cases := []struct {
		name    string
		schema  string
		wantErr string
	}{
		{
			name:   "valid minimal schema",
			schema: `{"openapi":"3.0.0","info":{"title":"t","version":"1"},"paths":{}}`,
		},
		{
			name:    "not JSON or YAML",
			schema:  "https://example.com/openapi.json",
			wantErr: "not valid JSON or YAML",
		},
		{
			name:    "missing openapi field",
			schema:  `{"info":{"title":"t","version":"1"},"paths":{}}`,
			wantErr: `missing required top-level "openapi"`,
		},
		{
			name:    "missing info",
			schema:  `{"openapi":"3.0.0","paths":{}}`,
			wantErr: `missing required "info.title"`,
		},
		{
			name:    "missing info.version",
			schema:  `{"openapi":"3.0.0","info":{"title":"t"},"paths":{}}`,
			wantErr: `missing required "info.title"`,
		},
		{
			name:    "missing paths",
			schema:  `{"openapi":"3.0.0","info":{"title":"t","version":"1"}}`,
			wantErr: `missing required top-level "paths"`,
		},
		{
			name:   "YAML form is accepted, not just JSON",
			schema: "openapi: \"3.0.0\"\ninfo:\n  title: t\n  version: \"1\"\npaths: {}\n",
		},
	}

	for _, tt := range cases {
		t.Run(tt.name, func(t *testing.T) {
			err := validateOpenAPISchema(tt.schema)
			if tt.wantErr == "" {
				if err != nil {
					t.Fatalf("unexpected error: %v", err)
				}
				return
			}
			if err == nil {
				t.Fatalf("expected error containing %q, got nil", tt.wantErr)
			}
			if !containsSubstring(err.Error(), tt.wantErr) {
				t.Fatalf("expected error containing %q, got %q", tt.wantErr, err.Error())
			}
		})
	}
}

func containsSubstring(s, substr string) bool {
	for i := 0; i+len(substr) <= len(s); i++ {
		if s[i:i+len(substr)] == substr {
			return true
		}
	}
	return false
}
