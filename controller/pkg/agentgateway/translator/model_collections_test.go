package translator

import (
	"testing"

	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestModelReferenceIgnoresListenerHostname(t *testing.T) {
	parent := &ParentInfo{
		AllowedKinds: []gwv1.RouteGroupKind{toRouteKind(wellknown.AgentgatewayModelGVK)},
		Hostnames:    []string{"other-namespace/models.example.com"},
	}

	if err := ReferenceAllowed(
		RouteContext{},
		parent,
		wellknown.AgentgatewayModelGVK,
		ParentReference{},
		nil,
		"default",
	); err != nil {
		t.Fatalf("model attachment should not depend on listener hostname: %v", err)
	}
}

func TestModelLLMProvider(t *testing.T) {
	t.Run("default provider", func(t *testing.T) {
		providerType := agentgateway.ModelProviderOpenAI
		provider, err := modelLLMProvider(&agentgateway.AgentgatewayModelSpec{Provider: &providerType})
		if err != nil {
			t.Fatal(err)
		}
		if provider.OpenAI == nil {
			t.Fatal("expected OpenAI provider")
		}
	})

	t.Run("provider configuration", func(t *testing.T) {
		providerType := agentgateway.ModelProviderBedrock
		provider, err := modelLLMProvider(&agentgateway.AgentgatewayModelSpec{
			Provider: &providerType,
			Bedrock:  &agentgateway.BedrockConfig{Region: "us-west-2"},
		})
		if err != nil {
			t.Fatal(err)
		}
		if provider.Bedrock == nil || provider.Bedrock.Region != "us-west-2" {
			t.Fatalf("unexpected Bedrock provider: %#v", provider.Bedrock)
		}
	})

	t.Run("missing provider configuration", func(t *testing.T) {
		providerType := agentgateway.ModelProviderBedrock
		_, err := modelLLMProvider(&agentgateway.AgentgatewayModelSpec{Provider: &providerType})
		if err == nil {
			t.Fatal("expected Bedrock provider without configuration to be rejected")
		}
	})

}

func TestTranslatePresetProviderOverrides(t *testing.T) {
	providerType := agentgateway.ModelProviderOllama
	baseURL := agentgateway.LongString("https://ollama.example/v2")
	model := agentgateway.ShortString("llama3.3")
	provider, err := translateModelLLMProvider(
		RouteContext{},
		"default",
		&agentgateway.AgentgatewayModelSpec{
			Provider: &providerType,
			UpstreamOverrides: &agentgateway.UpstreamOverrides{
				BaseURL: &baseURL,
				Model:   &model,
			},
		},
		"ollama",
		nil,
	)
	if err != nil {
		t.Fatal(err)
	}
	if provider.GetProviderPreset() != api.AIBackend_PROVIDER_PRESET_OLLAMA {
		t.Fatalf("provider preset = %v, want Ollama", provider.GetProviderPreset())
	}
	if provider.GetBaseUrl() != string(baseURL) {
		t.Errorf("base URL = %q, want %q", provider.GetBaseUrl(), baseURL)
	}
	if provider.GetModelOverride() != string(model) {
		t.Errorf("model override = %q, want %q", provider.GetModelOverride(), model)
	}
}
