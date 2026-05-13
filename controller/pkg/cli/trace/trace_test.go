package trace

import (
	"encoding/base64"
	"reflect"
	"testing"
)

func TestNormalizeTraceRequestStateBodies(t *testing.T) {
	tests := []struct {
		name string
		body []byte
		want any
	}{
		{
			name: "utf8 string",
			body: []byte("hello"),
			want: "hello",
		},
		{
			name: "json object",
			body: []byte(`{"hello":"world"}`),
			want: map[string]any{"hello": "world"},
		},
		{
			name: "json array",
			body: []byte(`[{"hello":"world"}]`),
			want: []any{map[string]any{"hello": "world"}},
		},
		{
			name: "binary",
			body: []byte{0xff},
			want: "/w==",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			value := map[string]any{
				"request": map[string]any{
					"body": base64.StdEncoding.EncodeToString(tt.body),
				},
			}

			normalizeTraceRequestStateBodies(value)

			got := value["request"].(map[string]any)["body"]
			if !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("got %#v, want %#v", got, tt.want)
			}
		})
	}
}

func TestNormalizeTraceRequestStateBodiesKeepsInvalidBase64(t *testing.T) {
	value := map[string]any{
		"response": map[string]any{
			"body": "not base64",
		},
	}

	normalizeTraceRequestStateBodies(value)

	got := value["response"].(map[string]any)["body"]
	if got != "not base64" {
		t.Fatalf("got %#v, want invalid base64 to remain unchanged", got)
	}
}

func TestNormalizeTraceRequestStateBodiesOnlyTouchesRequestAndResponseBody(t *testing.T) {
	encoded := base64.StdEncoding.EncodeToString([]byte("hello"))
	value := map[string]any{
		"request": map[string]any{
			"body": encoded,
			"nested": map[string]any{
				"request": map[string]any{
					"body": encoded,
				},
			},
		},
		"backend": map[string]any{
			"request": map[string]any{
				"body": encoded,
			},
		},
	}

	normalizeTraceRequestStateBodies(value)

	request := value["request"].(map[string]any)
	if got := request["body"]; got != "hello" {
		t.Fatalf("got request.body %#v, want decoded string", got)
	}
	gotNested := request["nested"].(map[string]any)["request"].(map[string]any)["body"]
	if gotNested != encoded {
		t.Fatalf("got nested request.body %#v, want unchanged base64", gotNested)
	}
	gotBackend := value["backend"].(map[string]any)["request"].(map[string]any)["body"]
	if gotBackend != encoded {
		t.Fatalf("got backend request.body %#v, want unchanged base64", gotBackend)
	}
}

func TestNormalizeTraceEventRequestStateBodiesOnlyTouchesMessageRequestState(t *testing.T) {
	encoded := base64.StdEncoding.EncodeToString([]byte("hello"))
	value := map[string]any{
		"message": map[string]any{
			"requestState": map[string]any{
				"request": map[string]any{
					"body": encoded,
				},
			},
		},
		"requestState": map[string]any{
			"request": map[string]any{
				"body": encoded,
			},
		},
	}

	normalizeTraceEventRequestStateBodies(value)

	gotMessage := value["message"].(map[string]any)["requestState"].(map[string]any)["request"].(map[string]any)["body"]
	if gotMessage != "hello" {
		t.Fatalf("got message.requestState.request.body %#v, want decoded string", gotMessage)
	}
	gotTopLevel := value["requestState"].(map[string]any)["request"].(map[string]any)["body"]
	if gotTopLevel != encoded {
		t.Fatalf("got top-level requestState.request.body %#v, want unchanged base64", gotTopLevel)
	}
}
