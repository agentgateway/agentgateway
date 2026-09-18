package tests

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestBedrockGuardrailEndpointPreference(t *testing.T) {
	v := NewAgentgatewayValidatorStrict(t)
	for _, location := range []string{"backend", "group", "model"} {
		for _, preference := range []string{"", "RuntimePreferred", "RuntimeOnly", "MantlePreferred", "MantleOnly"} {
			for _, guardrail := range []bool{false, true} {
				bedrock := map[string]any{}
				if preference != "" {
					bedrock["endpointPreference"] = preference
				}
				if guardrail {
					bedrock["guardrail"] = map[string]any{"identifier": "test-guardrail", "version": "1"}
				}
				kind := "AgentgatewayBackend"
				provider := map[string]any{"bedrock": bedrock}
				spec := map[string]any{"ai": map[string]any{"provider": provider}}
				if location == "group" {
					provider["name"] = "bedrock"
					spec = map[string]any{"ai": map[string]any{"groups": []any{map[string]any{"providers": []any{provider}}}}}
				} else if location == "model" {
					kind = "AgentgatewayModel"
					spec = map[string]any{"provider": "Bedrock", "bedrock": bedrock, "parentRefs": []any{map[string]any{"name": "gateway"}}}
				}
				body, err := json.Marshal(map[string]any{
					"apiVersion": "agentgateway.dev/v1alpha1", "kind": kind,
					"metadata": map[string]any{"name": "test"}, "spec": spec,
				})
				if err != nil {
					t.Fatal(err)
				}
				err = v.ValidateCustomResourceYAML(string(body), nil)
				wantError := guardrail && (preference == "MantlePreferred" || preference == "MantleOnly")
				if wantError {
					if err == nil || !strings.Contains(err.Error(), "Bedrock guardrails cannot be used") {
						t.Fatalf("%s/%s guardrail=%v: expected guardrail validation error, got %v", location, preference, guardrail, err)
					}
				} else if err != nil {
					t.Fatalf("%s/%s guardrail=%v: %v", location, preference, guardrail, err)
				}
			}
		}
	}
}
