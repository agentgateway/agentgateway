package agentgatewaybackend

import (
	"testing"

	"github.com/agentgateway/agentgateway/api"
)

func TestParseAzureEndpoint(t *testing.T) {
	tests := []struct {
		name             string
		endpoint         string
		wantName         string
		wantResourceType api.AIBackend_AzureResourceType
	}{
		{
			name:             "openai endpoint",
			endpoint:         "my-resource.openai.azure.com",
			wantName:         "my-resource",
			wantResourceType: api.AIBackend_OPEN_AI,
		},
		{
			name:             "foundry endpoint without -resource suffix",
			endpoint:         "myproject.services.ai.azure.com",
			wantName:         "myproject",
			wantResourceType: api.AIBackend_FOUNDRY,
		},
		{
			// Azure portal's "Foundry legacy" template generates resource
			// names that end in "-resource". That suffix is part of the
			// user's resource name, NOT part of the hostname suffix the
			// parser should strip.
			name:             "foundry endpoint with legacy -resource resource name preserved",
			endpoint:         "myproject-resource.services.ai.azure.com",
			wantName:         "myproject-resource",
			wantResourceType: api.AIBackend_FOUNDRY,
		},
		{
			name:             "unknown suffix falls back to whole endpoint with OpenAI type",
			endpoint:         "something.example.com",
			wantName:         "something.example.com",
			wantResourceType: api.AIBackend_OPEN_AI,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotName, gotResourceType := parseAzureEndpoint(tt.endpoint)
			if gotName != tt.wantName {
				t.Errorf("parseAzureEndpoint(%q) name = %q, want %q", tt.endpoint, gotName, tt.wantName)
			}
			if gotResourceType != tt.wantResourceType {
				t.Errorf("parseAzureEndpoint(%q) resourceType = %v, want %v", tt.endpoint, gotResourceType, tt.wantResourceType)
			}
		})
	}
}
