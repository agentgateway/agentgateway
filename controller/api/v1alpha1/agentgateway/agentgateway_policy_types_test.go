package agentgateway

import (
	"encoding/json"
	"math"
	"reflect"
	"strings"
	"testing"

	"istio.io/istio/pkg/ptr"
	"sigs.k8s.io/yaml"
)

// frontendConnectionFields are the Frontend policy fields resolved at the bind
// (gateway-or-port) level by the data plane's frontend_policies path. They may
// only target a Gateway or a port, never a listener (sectionName).
//
// frontendObservabilityFields are resolved per-listener by the data plane's
// listener_frontend_policies path, so they may additionally target a listener.
//
// Both sets are mirrored in the AgentgatewayPolicySpec CEL validation. When a
// new Frontend field is added, classify it here AND update the connection/L4
// XValidation rule in agentgateway_policy_types.go if the new field must not be
// listener-scoped. TestFrontendFieldsAreClassified fails until that is done.
var (
	frontendConnectionFields = map[string]struct{}{
		"tcp":                  {},
		"networkAuthorization": {},
		"tls":                  {},
		"http":                 {},
		"proxyProtocol":        {},
		"connect":              {},
	}
	frontendObservabilityFields = map[string]struct{}{
		"accessLog": {},
		"tracing":   {},
		"metrics":   {},
	}
)

// TestFrontendFieldsAreClassified guards the CEL rules that decide which
// frontend policies may target a listener (sectionName). Every field on the
// Frontend struct must be classified as either a connection/L4 field (no
// sectionName) or an observability field (sectionName allowed). A newly added,
// unclassified field fails this test so the author cannot silently grant it
// listener targeting that the data plane does not honor.
func TestFrontendFieldsAreClassified(t *testing.T) {
	for field := range reflect.TypeFor[Frontend]().Fields() {
		name, _, _ := strings.Cut(field.Tag.Get("json"), ",")
		if name == "" || name == "-" {
			continue
		}
		_, isConn := frontendConnectionFields[name]
		_, isObs := frontendObservabilityFields[name]
		switch {
		case isConn && isObs:
			t.Errorf("Frontend field %q is classified as both connection and observability", name)
		case !isConn && !isObs:
			t.Errorf("Frontend field %q is not classified; add it to frontendConnectionFields or "+
				"frontendObservabilityFields and update the AgentgatewayPolicySpec CEL rules accordingly", name)
		}
	}
}

func TestByteSizeInvalidJSONDecodesAsUnset(t *testing.T) {
	var b ByteSize
	if err := json.Unmarshal([]byte(`"not-a-quantity"`), &b); err != nil {
		t.Fatalf("UnmarshalJSON() error = %v", err)
	}
	if b.Value != nil {
		t.Fatalf("Value = %v, want nil", b.Value)
	}
	if got := b.ClampedValue(); got != nil {
		t.Fatalf("ByteSize = %v, want nil", got)
	}
}

func TestFrontendHTTPInvalidByteSizeDecodesAsUnsetValue(t *testing.T) {
	var http FrontendHTTP
	if err := json.Unmarshal([]byte(`{
		"maxBufferSize": "not-a-quantity",
		"http2WindowSize": "64Ki"
	}`), &http); err != nil {
		t.Fatalf("UnmarshalJSON() error = %v", err)
	}

	if http.MaxBufferSize == nil {
		t.Fatal("MaxBufferSize pointer = nil, want allocated ByteSize")
	}
	if http.MaxBufferSize.Value != nil {
		t.Fatalf("MaxBufferSize.Value = %v, want nil", http.MaxBufferSize.Value)
	}
	if http.MaxBufferSize.ClampedValue() != nil {
		t.Fatalf("MaxBufferSize.ClampedValue() = %v, want nil", http.MaxBufferSize.Value)
	}
	if http.HTTP2WindowSize == nil || http.HTTP2WindowSize.Value == nil {
		t.Fatal("HTTP2WindowSize = unset, want parsed quantity")
	}
	got := http.HTTP2WindowSize.ClampedValue()
	if got == nil || *got != 65536 {
		t.Fatalf("HTTP2WindowSize = %v, want 65536", got)
	}
}

func TestAzureManagedIdentityJSONOmitsSecretRef(t *testing.T) {
	auth := AzureAuth{
		ManagedIdentity: &AzureManagedIdentity{
			ClientID:   "client-id",
			ObjectID:   "object-id",
			ResourceID: "resource-id",
		},
	}

	got, err := json.Marshal(auth)
	if err != nil {
		t.Fatalf("Marshal() error = %v", err)
	}
	want := `{"managedIdentity":{"clientId":"client-id","objectId":"object-id","resourceId":"resource-id"}}`
	if string(got) != want {
		t.Fatalf("Marshal() = %s, want %s", got, want)
	}
}

func TestByteSizeYAMLDecodeClampedValue(t *testing.T) {
	type holder struct {
		Size *ByteSize `json:"size,omitempty"`
	}

	tests := []struct {
		name string
		in   string
		want *uint32
	}{
		{
			name: "missing",
			in:   `{}`,
			want: nil,
		},
		{
			name: "null",
			in:   `size: null`,
			want: nil,
		},
		{
			name: "invalid",
			in:   `size: not-a-quantity`,
			want: nil,
		},
		{
			name: "quantity string",
			in:   `size: 64Ki`,
			want: new(uint32(65536)),
		},
		{
			name: "integer",
			in:   `size: 1024`,
			want: new(uint32(1024)),
		},
		{
			name: "negative",
			in:   `size: -1`,
			want: new(uint32(0)),
		},
		{
			name: "too large",
			in:   `size: 5Gi`,
			want: new(uint32(math.MaxUint32)),
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var h holder
			if err := yaml.Unmarshal([]byte(tt.in), &h); err != nil {
				t.Fatalf("Unmarshal() error = %v", err)
			}

			got := h.Size.ClampedValue()
			if !ptr.Equal(got, tt.want) {
				t.Fatalf("ClampedValue() = %v, want %v", got, tt.want)
			}
		})
	}
}
