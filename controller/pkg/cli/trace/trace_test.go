package trace

import (
	"bytes"
	"context"
	"encoding/base64"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/spf13/cobra"
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

func TestRunRawFromTraceFile(t *testing.T) {
	line := `{"eventEnd":1,"severity":"INFO","message":{"type":"event","message":"hello"}}`
	filename := filepath.Join(t.TempDir(), "trace.jsonl")
	if err := os.WriteFile(filename, []byte(line+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	cmd := &cobra.Command{}
	cmd.SetContext(context.Background())
	var output bytes.Buffer
	cmd.SetOut(&output)

	err := run(cmd, &traceFlags{traceFile: filename, raw: true}, "", nil)
	if err != nil {
		t.Fatal(err)
	}
	if got := output.String(); got != line+"\n" {
		t.Fatalf("got %q, want %q", got, line+"\n")
	}
}

func TestConsumeTraceAcceptsStructuredPolicyEventDetails(t *testing.T) {
	line := `{"eventEnd":1,"severity":"info","message":{"type":"policyEvent","kind":"llm_cost","details":{"provider":"openai","model":"gpt-4o-mini","status":"exact"}}}`

	var got traceEnvelope
	err := consumeTrace(strings.NewReader(line+"\n"), func(_ string, envelope traceEnvelope) error {
		got = envelope
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}

	if got.Message.Kind != "llm_cost" {
		t.Fatalf("got kind %q, want llm_cost", got.Message.Kind)
	}
	summary := summarizeEnvelope(got)
	if !strings.Contains(summary, `"status":"exact"`) {
		t.Fatalf("got summary %q, want structured details", summary)
	}
}

func TestSummarizePolicyEventStringDetails(t *testing.T) {
	line := `{"eventEnd":1,"severity":"info","message":{"type":"policyEvent","kind":"cors","details":"request has no Origin header"}}`

	var got traceEnvelope
	err := consumeTrace(strings.NewReader(line+"\n"), func(_ string, envelope traceEnvelope) error {
		got = envelope
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}

	summary := summarizeEnvelope(got)
	if summary != "cors: request has no Origin header" {
		t.Fatalf("got summary %q", summary)
	}
}

func TestTraceStreamURLEncodesExpression(t *testing.T) {
	got := traceStreamURL("127.0.0.1:15000", `request.path == "/healthz"`)
	want := "http://127.0.0.1:15000/debug/trace?expression=request.path+%3D%3D+%22%2Fhealthz%22"
	if got != want {
		t.Fatalf("got %q, want %q", got, want)
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

func TestSnapshotJSONPrettyPrints(t *testing.T) {
	got := snapshotJSON([]byte(`{"request":{"body":"eyJoZWxsbyI6IndvcmxkIn0="},"empty":null}`))

	if strings.Contains(got, `"empty"`) {
		t.Fatalf("got %s, want nil top-level fields omitted", got)
	}
	if !strings.Contains(got, "\n  ") {
		t.Fatalf("got %s, want pretty-printed JSON", got)
	}
	if !strings.Contains(got, `"body": {`) || !strings.Contains(got, `"hello": "world"`) {
		t.Fatalf("got %s, want normalized JSON body", got)
	}
}

func TestEventJSONPrettyPrints(t *testing.T) {
	got := eventJSON(`{"message":{"requestState":{"request":{"body":"aGVsbG8="}}}}`)

	if !strings.Contains(got, "\n  ") {
		t.Fatalf("got %s, want pretty-printed JSON", got)
	}
	if !strings.Contains(got, `"body": "hello"`) {
		t.Fatalf("got %s, want normalized request body", got)
	}
}

func TestHighlightJSON(t *testing.T) {
	got := highlightJSON("{\n  \"name\": \"agentgateway\",\n  \"count\": 2,\n  \"enabled\": true,\n  \"missing\": null\n}")

	for _, want := range []string{
		`[teal]"name"[-]: [green]"agentgateway",[-]`,
		`[teal]"count"[-]: [yellow]2,[-]`,
		`[teal]"enabled"[-]: [yellow]true,[-]`,
		`[teal]"missing"[-]: [gray]null[-]`,
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("got %s, want highlighted fragment %s", got, want)
		}
	}
}
