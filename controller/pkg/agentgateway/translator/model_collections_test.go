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

func TestAgentgatewayModelSupportedKindsFeatureGate(t *testing.T) {
	modelKind := toRouteKind(wellknown.AgentgatewayModelGVK)
	listener := gwv1.Listener{
		Protocol: gwv1.HTTPProtocolType,
		AllowedRoutes: &gwv1.AllowedRoutes{
			Kinds: []gwv1.RouteGroupKind{modelKind},
		},
	}

	for _, tt := range []struct {
		name      string
		enabled   bool
		want      bool
		wantValid bool
	}{
		{name: "disabled", enabled: false, want: false, wantValid: false},
		{name: "enabled", enabled: true, want: true, wantValid: true},
	} {
		t.Run(tt.name, func(t *testing.T) {
			supported, valid := GenerateSupportedKinds(listener, tt.enabled)
			if valid != tt.wantValid {
				t.Errorf("listener valid = %t, want %t", valid, tt.wantValid)
			}
			found := false
			for _, kind := range supported {
				found = routeGroupKindEqual(kind, modelKind)
				if found {
					break
				}
			}
			if found != tt.want {
				t.Errorf("AgentgatewayModel supported = %t, want %t", found, tt.want)
			}
		})
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

	t.Run("provider configuration is required", func(t *testing.T) {
		providerType := agentgateway.ModelProviderBedrock
		_, err := modelLLMProvider(&agentgateway.AgentgatewayModelSpec{Provider: &providerType})
		if err == nil || err.Error() != "bedrock provider requires bedrock configuration" {
			t.Fatalf("error = %v, want missing Bedrock configuration error", err)
		}
	})
}

func TestModelProviderInlinePolicies(t *testing.T) {
	providerType := agentgateway.ModelProviderOpenAI
	apiKey := "test-api-key"
	model := &agentgateway.AgentgatewayModelSpec{
		Provider: &providerType,
		Policies: &agentgateway.ModelPolicies{
			Transformations: []agentgateway.FieldTransformation{{Field: "temperature", Expression: "0.5"}},
			Auth:            &agentgateway.ModelBackendAuth{InlineKey: &apiKey},
			Health:          &agentgateway.Health{UnhealthyCondition: new(agentgateway.CELExpression("response.code >= 500"))},
			TLS:             &agentgateway.BackendTLS{InsecureSkipVerify: new(agentgateway.InsecureTLSModeAll)},
			Headers: &agentgateway.HeaderModifiers{
				Request:  &gwv1.HTTPHeaderFilter{Add: []gwv1.HTTPHeader{{Name: "x-model-request-policy", Value: "enabled"}}},
				Response: &gwv1.HTTPHeaderFilter{Add: []gwv1.HTTPHeader{{Name: "x-model-response-policy", Value: "enabled"}}},
			},
			PromptGuard: &agentgateway.AIPromptGuard{Request: []agentgateway.PromptguardRequest{{
				Regex: &agentgateway.Regex{Action: new(agentgateway.Action(agentgateway.REJECT)), Matches: []agentgateway.LongString{"blocked"}},
			}}},
		},
	}

	provider, err := translateModelLLMProvider(RouteContext{}, "default", model, "openai", nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(provider.InlinePolicies) != 6 {
		t.Fatalf("inline policies = %d, want 6", len(provider.InlinePolicies))
	}
	if provider.InlinePolicies[0].GetBackendTls() == nil {
		t.Errorf("TLS policy = %#v, want backend TLS", provider.InlinePolicies[0])
	}
	if provider.InlinePolicies[1].GetHealth() == nil {
		t.Errorf("health policy = %#v, want health", provider.InlinePolicies[1])
	}
	if ai := provider.InlinePolicies[2].GetAi(); ai == nil || ai.GetPromptGuard() == nil {
		t.Errorf("AI policy = %#v, want prompt guard", provider.InlinePolicies[2])
	}
	if provider.InlinePolicies[3].GetAuth() == nil {
		t.Errorf("auth policy = %#v, want backend auth", provider.InlinePolicies[3])
	}
	routePolicy, err := translateModelRouteAIPolicy(RouteContext{}, "default", model.Policies)
	if err != nil {
		t.Fatal(err)
	}
	if got := routePolicy.GetTransformations()["temperature"]; got != "0.5" {
		t.Errorf("temperature transformation = %q, want %q", got, "0.5")
	}
	if provider.InlinePolicies[4].GetRequestHeaderModifier() == nil {
		t.Errorf("request header policy = %#v, want request header modifier", provider.InlinePolicies[4])
	}
	if provider.InlinePolicies[5].GetResponseHeaderModifier() == nil {
		t.Errorf("response header policy = %#v, want response header modifier", provider.InlinePolicies[5])
	}
}

func TestModelAuthorization(t *testing.T) {
	providerType := agentgateway.ModelProviderOpenAI
	model := &agentgateway.AgentgatewayModel{
		Spec: agentgateway.AgentgatewayModelSpec{
			Provider: &providerType,
			Policies: &agentgateway.ModelPolicies{
				Authorization: &agentgateway.Authorization{
					Policy: agentgateway.AuthorizationPolicy{MatchExpressions: []agentgateway.CELExpression{"request.headers['x-model-access'] == 'allowed'"}},
				},
			},
		},
	}
	parent := RouteParentReference{ListenerKey: "default/gateway.llm"}
	resources, err := convertAgentgatewayModel(RouteContext{}, model, parent)
	if err != nil {
		t.Fatal(err)
	}
	if len(resources) != 2 {
		t.Fatalf("resources = %d, want 2", len(resources))
	}
	route := resources[1].GetModelRoute()
	if route == nil || route.GetAuthorization() == nil {
		t.Fatalf("model route authorization = %#v, want RBAC policy", route)
	}
	if got := route.GetAuthorization().GetAllow(); len(got) != 1 || got[0] != "request.headers['x-model-access'] == 'allowed'" {
		t.Errorf("authorization allow = %#v, want model access rule", got)
	}
}

func TestValidateModelBaseURL(t *testing.T) {
	tests := []struct {
		name     string
		provider agentgateway.ModelProvider
		baseURL  *agentgateway.LongString
		wantErr  string
	}{
		{name: "public address", provider: agentgateway.ModelProviderOpenAI, baseURL: new(agentgateway.LongString("https://api.example.com/v1"))},
		{name: "ollama requires base URL", provider: agentgateway.ModelProviderOllama, wantErr: "ollama requires baseURL"},
		{name: "localhost", provider: agentgateway.ModelProviderOllama, baseURL: new(agentgateway.LongString("http://localhost:11434/v1")), wantErr: "cannot target localhost, loopback, link-local, or unspecified"},
		{name: "loopback IPv4", provider: agentgateway.ModelProviderOpenAI, baseURL: new(agentgateway.LongString("https://127.0.0.1/v1")), wantErr: "cannot target localhost, loopback, link-local, or unspecified"},
		{name: "loopback IPv6", provider: agentgateway.ModelProviderOpenAI, baseURL: new(agentgateway.LongString("https://[::1]/v1")), wantErr: "cannot target localhost, loopback, link-local, or unspecified"},
		{name: "link local", provider: agentgateway.ModelProviderOpenAI, baseURL: new(agentgateway.LongString("http://169.254.169.254/latest/meta-data")), wantErr: "cannot target localhost, loopback, link-local, or unspecified"},
		{name: "unspecified", provider: agentgateway.ModelProviderOpenAI, baseURL: new(agentgateway.LongString("http://0.0.0.0")), wantErr: "cannot target localhost, loopback, link-local, or unspecified"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			model := &agentgateway.AgentgatewayModelSpec{Provider: &tt.provider}
			model.BaseURL = tt.baseURL
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

func TestTranslatePresetProviderBaseURL(t *testing.T) {
	providerType := agentgateway.ModelProviderOllama
	baseURL := agentgateway.LongString("https://ollama.example/v2")
	provider, err := translateModelLLMProvider(
		RouteContext{},
		"default",
		&agentgateway.AgentgatewayModelSpec{
			Provider: &providerType,
			BaseURL:  &baseURL,
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
}
