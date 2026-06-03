package agentgateway

import (
	"encoding/json"
	"math"
	"testing"

	"istio.io/istio/pkg/ptr"
	"sigs.k8s.io/yaml"
)

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
