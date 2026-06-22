package agentgatewaybackend

import (
	"testing"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

func TestTranslateGuardrailBackend(t *testing.T) {
	loc := agentgateway.ShortString("us-east1")
	tests := []struct {
		name    string
		in      *agentgateway.GuardrailBackend
		wantErr bool
		check   func(t *testing.T, gb *api.GuardrailBackend)
	}{
		{
			name: "bedrock",
			in: &agentgateway.GuardrailBackend{Bedrock: &agentgateway.GuardrailBedrockProvider{
				GuardrailIdentifier: "gid",
				GuardrailVersion:    "DRAFT",
				Region:              "us-west-2",
			}},
			check: func(t *testing.T, gb *api.GuardrailBackend) {
				b := gb.GetBedrock()
				if b == nil {
					t.Fatal("expected bedrock provider")
				}
				if b.Identifier != "gid" || b.Version != "DRAFT" || b.Region != "us-west-2" {
					t.Fatalf("unexpected bedrock provider: %+v", b)
				}
			},
		},
		{
			name: "googleModelArmor with location",
			in: &agentgateway.GuardrailBackend{GoogleModelArmor: &agentgateway.GuardrailGoogleModelArmorProvider{
				TemplateID: "tid",
				ProjectID:  "pid",
				Location:   &loc,
			}},
			check: func(t *testing.T, gb *api.GuardrailBackend) {
				g := gb.GetGoogleModelArmor()
				if g == nil {
					t.Fatal("expected googleModelArmor provider")
				}
				if g.TemplateId != "tid" || g.ProjectId != "pid" || g.GetLocation() != "us-east1" {
					t.Fatalf("unexpected googleModelArmor provider: %+v", g)
				}
			},
		},
		{
			name: "googleModelArmor without location leaves location unset",
			in: &agentgateway.GuardrailBackend{GoogleModelArmor: &agentgateway.GuardrailGoogleModelArmorProvider{
				TemplateID: "tid",
				ProjectID:  "pid",
			}},
			check: func(t *testing.T, gb *api.GuardrailBackend) {
				if g := gb.GetGoogleModelArmor(); g.Location != nil {
					t.Fatalf("expected nil location, got %q", *g.Location)
				}
			},
		},
		{
			name: "openAIModeration",
			in:   &agentgateway.GuardrailBackend{OpenAIModeration: &agentgateway.GuardrailOpenAIModerationProvider{}},
			check: func(t *testing.T, gb *api.GuardrailBackend) {
				if gb.GetOpenaiModeration() == nil {
					t.Fatal("expected openAIModeration provider")
				}
			},
		},
		{
			name:    "no provider is an error",
			in:      &agentgateway.GuardrailBackend{},
			wantErr: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gb, err := translateGuardrailBackend(tt.in)
			if tt.wantErr {
				if err == nil {
					t.Fatal("expected error, got nil")
				}
				return
			}
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			tt.check(t, gb)
		})
	}
}

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
