package translator

import (
	"strings"
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

	t.Run("upstream overrides", func(t *testing.T) {
		providerType := agentgateway.ModelProviderOpenAI
		tests := []struct {
			name       string
			model      agentgateway.AgentgatewayModelSpec
			host       string
			port       int32
			path       string
			pathPrefix string
			wantErr    string
		}{
			{
				name: "https default port",
				model: agentgateway.AgentgatewayModelSpec{
					Provider:          &providerType,
					UpstreamOverrides: &agentgateway.UpstreamOverrides{BaseURL: new(agentgateway.LongString("https://provider.example.com"))},
				},
				host: "provider.example.com",
				port: 443,
			},
			{
				name: "http explicit port and path prefix",
				model: agentgateway.AgentgatewayModelSpec{
					Provider:          &providerType,
					UpstreamOverrides: &agentgateway.UpstreamOverrides{BaseURL: new(agentgateway.LongString("http://provider.example.com:8443/api/chat"))},
				},
				host:       "provider.example.com",
				port:       8443,
				pathPrefix: "/api/chat",
			},
			{
				name: "trailing slash is omitted from path prefix",
				model: agentgateway.AgentgatewayModelSpec{
					Provider:          &providerType,
					UpstreamOverrides: &agentgateway.UpstreamOverrides{BaseURL: new(agentgateway.LongString("https://provider.example.com/v1/"))},
				},
				host:       "provider.example.com",
				port:       443,
				pathPrefix: "/v1",
			},
			{
				name: "invalid base URL",
				model: agentgateway.AgentgatewayModelSpec{
					Provider:          &providerType,
					UpstreamOverrides: &agentgateway.UpstreamOverrides{BaseURL: new(agentgateway.LongString("ftp://provider.example.com"))},
				},
				wantErr: "must use http or https",
			},
			{
				name: "base URL query is rejected",
				model: agentgateway.AgentgatewayModelSpec{
					Provider:          &providerType,
					UpstreamOverrides: &agentgateway.UpstreamOverrides{BaseURL: new(agentgateway.LongString("https://provider.example.com/v1?api-version=2025-01-01"))},
				},
				wantErr: "cannot include user info, query parameters, or a fragment",
			},
		}

		for _, tt := range tests {
			t.Run(tt.name, func(t *testing.T) {
				provider, err := translateModelLLMProvider(RouteContext{}, "default", &tt.model, "openai", nil)
				if tt.wantErr != "" {
					if err == nil || !strings.Contains(err.Error(), tt.wantErr) {
						t.Fatalf("error = %v, want containing %q", err, tt.wantErr)
					}
					return
				}
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
