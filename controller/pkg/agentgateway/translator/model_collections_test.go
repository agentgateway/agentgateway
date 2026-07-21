package translator

import (
	"strings"
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
			Bedrock:  &agentgateway.BedrockSettings{Region: "us-west-2"},
		})
		if err != nil {
			t.Fatal(err)
		}
		if provider.Bedrock == nil || provider.Bedrock.Region != "us-west-2" {
			t.Fatalf("unexpected Bedrock provider: %#v", provider.Bedrock)
		}
	})

	t.Run("provider defaults", func(t *testing.T) {
		providerType := agentgateway.ModelProviderBedrock
		provider, err := modelLLMProvider(&agentgateway.AgentgatewayModelSpec{Provider: &providerType})
		if err != nil {
			t.Fatal(err)
		}
		if provider.Bedrock == nil || provider.Bedrock.Region != "us-east-1" {
			t.Fatalf("unexpected default Bedrock provider: %#v", provider.Bedrock)
		}
	})

}

func TestValidateModelBaseURL(t *testing.T) {
	tests := []struct {
		name     string
		provider agentgateway.ModelProvider
		baseURL  *agentgateway.LongString
		wantErr  string
	}{
		{name: "public address", provider: agentgateway.ModelProviderOpenAI, baseURL: new(agentgateway.LongString("https://api.example.com/v1"))},
		{name: "ollama requires override", provider: agentgateway.ModelProviderOllama, wantErr: "ollama requires upstreamOverrides.baseURL"},
		{name: "localhost", provider: agentgateway.ModelProviderOllama, baseURL: new(agentgateway.LongString("http://localhost:11434/v1")), wantErr: "cannot target localhost, loopback, or link-local"},
		{name: "loopback IPv4", provider: agentgateway.ModelProviderOpenAI, baseURL: new(agentgateway.LongString("https://127.0.0.1/v1")), wantErr: "cannot target localhost, loopback, or link-local"},
		{name: "loopback IPv6", provider: agentgateway.ModelProviderOpenAI, baseURL: new(agentgateway.LongString("https://[::1]/v1")), wantErr: "cannot target localhost, loopback, or link-local"},
		{name: "link local", provider: agentgateway.ModelProviderOpenAI, baseURL: new(agentgateway.LongString("http://169.254.169.254/latest/meta-data")), wantErr: "cannot target localhost, loopback, or link-local"},
		{name: "link local IPv6 zone", provider: agentgateway.ModelProviderOpenAI, baseURL: new(agentgateway.LongString("http://[fe80::1%25eth0]/v1")), wantErr: "cannot target localhost, loopback, or link-local"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			model := &agentgateway.AgentgatewayModelSpec{Provider: &tt.provider}
			if tt.baseURL != nil {
				model.UpstreamOverrides = &agentgateway.UpstreamOverrides{BaseURL: tt.baseURL}
			}
			err := validateModelBaseURL(model)
			if tt.wantErr == "" {
				if err != nil {
					t.Fatal(err)
				}
				return
			}
			if err == nil || !strings.Contains(err.Error(), tt.wantErr) {
				t.Errorf("error = %v, want %q", err, tt.wantErr)
			}
		})
	}
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
