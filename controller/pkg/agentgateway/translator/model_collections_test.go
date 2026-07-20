package translator

import (
	"testing"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

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

	t.Run("top-level overrides", func(t *testing.T) {
		providerType := agentgateway.ModelProviderOpenAI
		tests := []struct {
			name       string
			model      agentgateway.AgentgatewayModelSpec
			host       string
			port       int32
			path       string
			pathPrefix string
		}{
			{
				name: "default port",
				model: agentgateway.AgentgatewayModelSpec{
					Provider: &providerType,
					Host:     "provider.example.com",
				},
				host: "provider.example.com",
				port: 443,
			},
			{
				name: "host port and path override",
				model: agentgateway.AgentgatewayModelSpec{
					Provider: &providerType,
					Host:     "provider.example.com",
					Port:     8443,
					Path:     "/api/chat",
				},
				host: "provider.example.com",
				port: 8443,
				path: "/api/chat",
			},
			{
				name: "path prefix override",
				model: agentgateway.AgentgatewayModelSpec{
					Provider:   &providerType,
					Host:       "provider.example.com",
					PathPrefix: "/v1",
				},
				host:       "provider.example.com",
				port:       443,
				pathPrefix: "/v1",
			},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				provider, err := translateModelLLMProvider(RouteContext{}, "default", &tt.model, "openai", nil)
				if err != nil {
					t.Fatal(err)
				}
				if got := provider.GetHostOverride().GetHost(); got != tt.host {
					t.Errorf("host = %q, want %q", got, tt.host)
				}
				if got := provider.GetHostOverride().GetPort(); got != tt.port {
					t.Errorf("port = %d, want %d", got, tt.port)
				}
				if got := provider.GetPathOverride(); got != tt.path {
					t.Errorf("path override = %q, want %q", got, tt.path)
				}
				if got := provider.GetPathPrefix(); got != tt.pathPrefix {
					t.Errorf("path prefix = %q, want %q", got, tt.pathPrefix)
				}
			})
		}
	})
}
