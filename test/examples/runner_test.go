//go:build examples

package examples

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
	"github.com/stretchr/testify/require"
	"sigs.k8s.io/yaml"
)

// A smoke test is declared entirely by a smoke.yaml next to an example's
// config.yaml. TestExamples discovers every examples/*/smoke.yaml and runs it
// through this one generic runner, so adding coverage for a new example means
// writing a small data file, not Go.

// Spec is the schema of examples/<name>/smoke.yaml.
type Spec struct {
	// ReadyAddr is host:port of the gateway readiness server. Defaults to
	// 127.0.0.1:15021 (agentgateway's default); set it when the example config
	// overrides readinessAddr.
	ReadyAddr string `json:"readyAddr,omitempty"`
	// Env holds extra environment variables for the gateway process (e.g. dummy
	// API keys the config expands, or npm_config_yes for npx targets).
	Env map[string]string `json:"env,omitempty"`
	// Mocks are plain HTTP upstreams to start before the gateway, standing in for
	// backends the example routes to (each replies 200 "ok").
	Mocks []MockSpec `json:"mocks,omitempty"`
	// MockLLM starts a mock LLM provider and rewrites the config so every model's
	// baseUrl points at it. The mock reflects the request's prompt back as the
	// completion, so a probe can assert on a string it chose itself.
	MockLLM bool `json:"mockLLM,omitempty"`
	// Probes are the requests to send once the gateway is ready.
	Probes []Probe `json:"probes"`
}

// MockSpec is a plain HTTP upstream bound to a fixed address.
type MockSpec struct {
	Listen string `json:"listen"`
}

// Probe is exactly one of http or mcp.
type Probe struct {
	HTTP *HTTPProbe `json:"http,omitempty"`
	MCP  *MCPProbe  `json:"mcp,omitempty"`
}

// HTTPProbe sends one HTTP request and asserts on the response.
type HTTPProbe struct {
	Method             string            `json:"method,omitempty"` // default GET
	URL                string            `json:"url"`
	Headers            map[string]string `json:"headers,omitempty"`
	Body               string            `json:"body,omitempty"`
	ExpectStatus       int               `json:"expectStatus,omitempty"` // default 200
	ExpectBody         string            `json:"expectBody,omitempty"`   // exact match
	ExpectBodyContains []string          `json:"expectBodyContains,omitempty"`
}

// MCPProbe drives the MCP handshake against an endpoint, optionally asserting a
// tool is listed and calling it.
type MCPProbe struct {
	Endpoint     string   `json:"endpoint"`
	ToolsContain []string `json:"toolsContain,omitempty"`
	Call         *MCPCall `json:"call,omitempty"`
}

// MCPCall calls a tool and asserts on its textual result.
type MCPCall struct {
	Name                 string         `json:"name"`
	Arguments            map[string]any `json:"arguments,omitempty"`
	ExpectResultContains []string       `json:"expectResultContains,omitempty"`
}

func TestExamples(t *testing.T) {
	root := repoRoot(t)
	specs, err := filepath.Glob(filepath.Join(root, "examples", "*", "smoke.yaml"))
	require.NoError(t, err)
	require.NotEmpty(t, specs, "no examples/*/smoke.yaml found")

	for _, specPath := range specs {
		name := filepath.Base(filepath.Dir(specPath))
		t.Run(name, func(t *testing.T) {
			runSpec(t, filepath.Dir(specPath), specPath)
		})
	}
}

func runSpec(t *testing.T, exampleDir, specPath string) {
	raw, err := os.ReadFile(specPath)
	require.NoError(t, err, "read spec")
	var spec Spec
	require.NoError(t, yaml.UnmarshalStrict(raw, &spec), "parse spec")
	require.NotEmpty(t, spec.Probes, "spec has no probes")

	for _, m := range spec.Mocks {
		startHTTPUpstream(t, m.Listen)
	}

	config := filepath.Join(exampleDir, "config.yaml")
	if spec.MockLLM {
		config = writeLLMOverlay(t, config, startMockLLM(t))
	}

	readyAddr := spec.ReadyAddr
	if readyAddr == "" {
		readyAddr = "127.0.0.1:15021"
	}
	startGateway(t, config, "http://"+readyAddr+"/healthz/ready", envList(spec.Env)...)

	for i, probe := range spec.Probes {
		switch {
		case probe.HTTP != nil:
			runHTTPProbe(t, fmt.Sprintf("probe[%d]", i), probe.HTTP)
		case probe.MCP != nil:
			runMCPProbe(t, fmt.Sprintf("probe[%d]", i), probe.MCP)
		default:
			t.Fatalf("probe[%d] declares neither http nor mcp", i)
		}
	}
}

func envList(m map[string]string) []string {
	out := make([]string, 0, len(m))
	for k, v := range m {
		out = append(out, k+"="+v)
	}
	return out
}

func runHTTPProbe(t *testing.T, label string, p *HTTPProbe) {
	method := p.Method
	if method == "" {
		method = http.MethodGet
	}
	wantStatus := p.ExpectStatus
	if wantStatus == 0 {
		wantStatus = http.StatusOK
	}

	var bodyReader io.Reader
	if p.Body != "" {
		bodyReader = strings.NewReader(p.Body)
	}
	req, err := http.NewRequest(method, p.URL, bodyReader)
	require.NoErrorf(t, err, "%s: build request", label)
	for k, v := range p.Headers {
		req.Header.Set(k, v)
	}

	client := &http.Client{Timeout: 15 * time.Second}
	resp, err := client.Do(req)
	require.NoErrorf(t, err, "%s: %s %s", label, method, p.URL)
	body, err := io.ReadAll(resp.Body)
	_ = resp.Body.Close()
	require.NoErrorf(t, err, "%s: read body", label)

	require.Equalf(t, wantStatus, resp.StatusCode, "%s: status; body: %s", label, body)
	if p.ExpectBody != "" {
		require.Equalf(t, p.ExpectBody, string(body), "%s: body", label)
	}
	for _, want := range p.ExpectBodyContains {
		require.Containsf(t, string(body), want, "%s: body should contain %q", label, want)
	}
}

func runMCPProbe(t *testing.T, label string, p *MCPProbe) {
	// Generous deadline: the first call may spawn a stdio server (npx download).
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Minute)
	defer cancel()

	client := mcp.NewClient(&mcp.Implementation{Name: "smoke", Version: "0"}, nil)
	session, err := client.Connect(ctx, &mcp.StreamableClientTransport{Endpoint: p.Endpoint}, nil)
	require.NoErrorf(t, err, "%s: connect %s", label, p.Endpoint)
	defer func() { _ = session.Close() }()

	if len(p.ToolsContain) > 0 {
		tools, err := session.ListTools(ctx, nil)
		require.NoErrorf(t, err, "%s: tools/list", label)
		var names []string
		for _, tool := range tools.Tools {
			names = append(names, tool.Name)
		}
		for _, want := range p.ToolsContain {
			require.Containsf(t, names, want, "%s: tools should include %q; got %v", label, want, names)
		}
	}

	if p.Call != nil {
		res, err := session.CallTool(ctx, &mcp.CallToolParams{Name: p.Call.Name, Arguments: p.Call.Arguments})
		require.NoErrorf(t, err, "%s: tools/call %s", label, p.Call.Name)
		require.Falsef(t, res.IsError, "%s: tool %s returned an error", label, p.Call.Name)
		var text strings.Builder
		for _, content := range res.Content {
			if tc, ok := content.(*mcp.TextContent); ok {
				text.WriteString(tc.Text)
			}
		}
		for _, want := range p.Call.ExpectResultContains {
			require.Containsf(t, text.String(), want, "%s: result should contain %q", label, want)
		}
	}
}

// startMockLLM serves a provider-shaped response that reflects the request's
// prompt back as the completion, keyed on the request path (the gateway
// rewrites the request before forwarding, so the path is what distinguishes the
// providers). Reflecting the prompt lets a spec assert on a string it chose,
// with no coupling to the mock's internals. Returns the mock's base URL.
func startMockLLM(t *testing.T) string {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		reqBody, _ := io.ReadAll(r.Body)
		echo := lastPromptText(reqBody)
		if echo == "" {
			echo = "smoke-llm-ok"
		}
		w.Header().Set("content-type", "application/json")
		if strings.Contains(r.URL.Path, "/messages") {
			_, _ = io.WriteString(w, anthropicResponse(echo))
		} else {
			_, _ = io.WriteString(w, openAIResponse(echo))
		}
	}))
	t.Cleanup(srv.Close)
	return srv.URL
}

// lastPromptText pulls the text of the last message from an OpenAI- or
// Anthropic-style chat request, tolerating both string and array content.
func lastPromptText(body []byte) string {
	var req struct {
		Messages []struct {
			Content json.RawMessage `json:"content"`
		} `json:"messages"`
	}
	if err := json.Unmarshal(body, &req); err != nil || len(req.Messages) == 0 {
		return ""
	}
	raw := req.Messages[len(req.Messages)-1].Content
	var s string
	if json.Unmarshal(raw, &s) == nil {
		return s
	}
	var parts []struct {
		Text string `json:"text"`
	}
	if json.Unmarshal(raw, &parts) == nil {
		var b strings.Builder
		for _, part := range parts {
			b.WriteString(part.Text)
		}
		return b.String()
	}
	return ""
}

func openAIResponse(content string) string {
	b, _ := json.Marshal(content)
	return `{"id":"chatcmpl-smoke","object":"chat.completion","created":0,` +
		`"model":"gpt-3.5-turbo-0125","choices":[{"index":0,"message":{"role":"assistant",` +
		`"content":` + string(b) + `},"logprobs":null,"finish_reason":"stop"}],` +
		`"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}`
}

func anthropicResponse(text string) string {
	b, _ := json.Marshal(text)
	return `{"id":"msg_smoke","type":"message","role":"assistant",` +
		`"model":"claude-3-5-haiku-latest","content":[{"type":"text","text":` + string(b) + `}],` +
		`"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}`
}

// writeLLMOverlay copies the example config and injects a baseUrl under every
// model's params so all providers resolve to the mock. The example's
// routing/transformation config is otherwise used verbatim.
func writeLLMOverlay(t *testing.T, srcConfig, baseURL string) string {
	t.Helper()
	data, err := os.ReadFile(srcConfig)
	require.NoError(t, err, "read example config")

	var out []string
	injected := 0
	for _, line := range strings.Split(string(data), "\n") {
		out = append(out, line)
		if line == "    params:" {
			out = append(out, "      baseUrl: "+baseURL)
			injected++
		}
	}
	require.Positive(t, injected, "expected to inject at least one baseUrl into %s", srcConfig)

	dst := filepath.Join(t.TempDir(), "config.yaml")
	require.NoError(t, os.WriteFile(dst, []byte(strings.Join(out, "\n")), 0o600))
	return dst
}
