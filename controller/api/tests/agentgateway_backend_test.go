package tests

import (
	"fmt"
	"testing"

	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/tmpl"
)

func TestCustomLLMProviderValidation(t *testing.T) {
	tm := `apiVersion: agentgateway.dev/v1alpha1
kind: AgentgatewayBackend
metadata:
  name: custom-provider
spec:
  ai:
    provider:
{{ .provider | nindent 6 }}
`

	cases := []struct {
		name      string
		provider  string
		wantValid bool
	}{
		{
			name: "service backendRef",
			provider: `custom:
  backendRef:
    name: llm-service
    port: 8080
  formats:
  - type: Completions`,
			wantValid: true,
		},
		{
			name: "inferencepool backendRef",
			provider: `custom:
  backendRef:
    group: inference.networking.k8s.io
    kind: InferencePool
    name: llama-pool
  formats:
  - type: Completions`,
			wantValid: true,
		},
		{
			name: "direct host port",
			provider: `custom:
  formats:
  - type: Messages
    path: /api/chat
host: llm.example.com
port: 443`,
			wantValid: true,
		},
		{
			name: "missing formats",
			provider: `custom:
  backendRef:
    name: llm-service
    port: 8080`,
			wantValid: false,
		},
		{
			name: "empty formats",
			provider: `custom:
  backendRef:
    name: llm-service
    port: 8080
  formats: []`,
			wantValid: false,
		},
		{
			name: "unsupported format",
			provider: `custom:
  backendRef:
    name: llm-service
    port: 8080
  formats:
  - type: Detect`,
			wantValid: false,
		},
		{
			name: "path and custom format path",
			provider: `custom:
  backendRef:
    name: llm-service
    port: 8080
  formats:
  - type: Messages
    path: /api/messages
path: /v1/messages`,
			wantValid: false,
		},
		{
			name: "pathPrefix and custom format path",
			provider: `custom:
  formats:
  - type: Messages
    path: /api/messages
host: llm.example.com
port: 443
pathPrefix: /custom`,
			wantValid: false,
		},
		{
			name: "missing target",
			provider: `custom:
  formats:
  - type: Completions`,
			wantValid: false,
		},
		{
			name: "backendRef and direct host port",
			provider: `custom:
  backendRef:
    name: llm-service
    port: 8080
  formats:
  - type: Completions
host: llm.example.com
port: 443`,
			wantValid: false,
		},
		{
			name: "cross namespace backendRef",
			provider: `custom:
  backendRef:
    name: llm-service
    namespace: other
    port: 8080
  formats:
  - type: Completions`,
			wantValid: false,
		},
		{
			name: "unsupported backendRef kind",
			provider: `custom:
  backendRef:
    group: agentgateway.dev
    kind: AgentgatewayBackend
    name: other-backend
  formats:
  - type: Completions`,
			wantValid: false,
		},
	}

	v := NewAgentgatewayValidator(t)
	for _, tt := range cases {
		t.Run(tt.name, func(t *testing.T) {
			res := tmpl.EvaluateOrFail(t, tm, map[string]any{"provider": tt.provider})
			err := v.ValidateCustomResourceYAML(res, nil)
			assert.Equal(t, tt.wantValid, err == nil, fmt.Sprintf("validation error: %v", err))
		})
	}
}
