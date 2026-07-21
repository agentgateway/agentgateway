package agentgateway

import (
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewaymodels,verbs=get;list;watch
// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewaymodels/status,verbs=get;update;patch

// +kubebuilder:printcolumn:name="Model Match",type=string,JSONPath=".spec.match.model",description="Model name matched from client requests"
// +kubebuilder:printcolumn:name="Age",type=date,JSONPath=".metadata.creationTimestamp",description="The age of the model."

// +genclient
// +kubebuilder:object:root=true
// +kubebuilder:metadata:labels={app=agentgateway,app.kubernetes.io/name=agentgateway}
// +kubebuilder:resource:categories=agentgateway,shortName=agmodel
// +kubebuilder:subresource:status
type AgentgatewayModel struct {
	metav1.TypeMeta `json:",inline"`
	// metadata for the object
	// More info: https://git.k8s.io/community/contributors/devel/sig-architecture/api-conventions.md#metadata
	// +optional
	metav1.ObjectMeta `json:"metadata,omitempty"`

	// Desired model configuration.
	// +required
	Spec AgentgatewayModelSpec `json:"spec"`

	// Current model attachment status.
	// +optional
	Status AgentgatewayModelStatus `json:"status,omitempty"`
}

// +kubebuilder:object:root=true
type AgentgatewayModelList struct {
	metav1.TypeMeta `json:",inline"`
	metav1.ListMeta `json:"metadata,omitempty"`
	Items           []AgentgatewayModel `json:"items"`
}

// +kubebuilder:validation:ExactlyOneOf=provider;virtualModel
// +kubebuilder:validation:XValidation:rule="has(self.provider) || !has(self.upstreamOverrides)",message="upstreamOverrides requires provider"
// +kubebuilder:validation:XValidation:rule="has(self.provider) || !has(self.transformations)",message="transformations require provider"
// +kubebuilder:validation:XValidation:rule="!has(self.virtualModel) || self.visibility != 'Internal'",message="virtual models must be public"
// +kubebuilder:validation:XValidation:rule="!has(self.virtualModel) || !has(self.match) || !has(self.match.model) || !self.match.model.contains('*')",message="virtual model match.model must be an exact name"
// +kubebuilder:validation:XValidation:rule="!has(self.provider) || self.provider != 'Ollama' || (has(self.upstreamOverrides) && has(self.upstreamOverrides.baseURL))",message="ollama requires upstreamOverrides.baseURL"
// +kubebuilder:validation:XValidation:rule="has(self.azure) == (has(self.provider) && self.provider == 'Azure')",message="azure configuration is required only for the Azure provider"
// +kubebuilder:validation:XValidation:rule="has(self.vertexai) == (has(self.provider) && self.provider == 'VertexAI')",message="vertexai configuration is required only for the VertexAI provider"
// +kubebuilder:validation:XValidation:rule="has(self.bedrock) == (has(self.provider) && self.provider == 'Bedrock')",message="bedrock configuration is required only for the Bedrock provider"
// +kubebuilder:validation:XValidation:rule="has(self.custom) == (has(self.provider) && self.provider == 'Custom')",message="custom configuration is required only for the Custom provider"
type AgentgatewayModelSpec struct {
	// Gateways and listeners to which this model attaches.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +required
	ParentRefs []gwv1.ParentReference `json:"parentRefs"`

	// Conditions for selecting this model from client requests.
	// +optional
	Match *ModelMatch `json:"match,omitempty"`

	// Controls whether clients can request this model directly. Internal models
	// can only be selected by virtual models. Defaults to Public.
	// +kubebuilder:default=Public
	// +optional
	Visibility ModelVisibility `json:"visibility,omitempty"`

	// Provider serving this concrete model. Provider-specific configuration is
	// set by the corresponding field below when needed.
	// +optional
	Provider *ModelProvider `json:"provider,omitempty"`

	// Provider-specific settings for Azure AI.
	// +optional
	Azure *AzureSettings `json:"azure,omitempty"`

	// Provider-specific settings for Vertex AI.
	// +optional
	VertexAI *VertexAISettings `json:"vertexai,omitempty"`

	// Provider-specific settings for Amazon Bedrock.
	// +optional
	Bedrock *BedrockSettings `json:"bedrock,omitempty"`

	// Provider-specific settings for a custom provider.
	// +optional
	Custom *CustomProviderSettings `json:"custom,omitempty"`

	// Shared overrides for requests sent to the selected provider.
	// +optional
	UpstreamOverrides *UpstreamOverrides `json:"upstreamOverrides,omitempty"`

	// CEL transformations applied to fields in the provider request body.
	// Transformations take precedence over upstreamOverrides.model for the same field.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +listType=map
	// +listMapKey=field
	// +optional
	Transformations []FieldTransformation `json:"transformations,omitempty"`

	// Request-time routing among concrete AgentgatewayModel resources.
	// +optional
	VirtualModel *VirtualModel `json:"virtualModel,omitempty"`
}

// Shared overrides for requests sent to an LLM provider.
// +kubebuilder:validation:XValidation:rule="!has(self.baseURL) || self.baseURL.startsWith('http://') || self.baseURL.startsWith('https://')",message="upstreamOverrides.baseURL must use http or https"
// +kubebuilder:validation:XValidation:rule="!has(self.baseURL) || !self.baseURL.matches(\"(?i)^https?://(localhost|[^/]+\\\\.localhost)(:[0-9]+)?(/|$)\")",message="upstreamOverrides.baseURL cannot target localhost, loopback, or link-local addresses"
// +kubebuilder:validation:XValidation:rule="!has(self.baseURL) || !self.baseURL.matches(\"^https?://127(\\\\.[0-9]{1,3}){0,3}(:[0-9]+)?(/|$)\")",message="upstreamOverrides.baseURL cannot target localhost, loopback, or link-local addresses"
// +kubebuilder:validation:XValidation:rule="!has(self.baseURL) || !self.baseURL.matches(\"^https?://169\\\\.254\\\\.[0-9]{1,3}\\\\.[0-9]{1,3}(:[0-9]+)?(/|$)\")",message="upstreamOverrides.baseURL cannot target localhost, loopback, or link-local addresses"
// +kubebuilder:validation:XValidation:rule="!has(self.baseURL) || !self.baseURL.matches(\"(?i)^https?://\\\\[(::1|fe[89ab][0-9a-f:]*)\\\\](:[0-9]+)?(/|$)\")",message="upstreamOverrides.baseURL cannot target localhost, loopback, or link-local addresses"
type UpstreamOverrides struct {
	// BaseURL overrides the provider address and base path prefix. It must use the
	// http or https scheme. Backend policies may override the default
	// TLS configuration.
	//
	// Query parameters, fragments, and user info are not supported.
	// +kubebuilder:validation:Format=uri
	// +optional
	BaseURL *LongString `json:"baseURL,omitempty"`

	// Fixed model name sent to the provider. When omitted, the model selected by
	// match.model is sent to the provider.
	// +optional
	Model *ShortString `json:"model,omitempty"`
}

// ModelMatch contains conditions for selecting a model.
type ModelMatch struct {
	// Model name or wildcard expression matched against client requests.
	// When omitted, the model matches metadata.name exactly.
	// +optional
	Model *LongString `json:"model,omitempty"`
}

// ModelProvider identifies the LLM provider serving a concrete model.
// +k8s:enum
type ModelProvider string

const (
	ModelProviderOpenAI      ModelProvider = "OpenAI"
	ModelProviderAzure       ModelProvider = "Azure"
	ModelProviderAnthropic   ModelProvider = "Anthropic"
	ModelProviderGemini      ModelProvider = "Gemini"
	ModelProviderVertexAI    ModelProvider = "VertexAI"
	ModelProviderBedrock     ModelProvider = "Bedrock"
	ModelProviderCohere      ModelProvider = "Cohere"
	ModelProviderOllama      ModelProvider = "Ollama"
	ModelProviderBaseten     ModelProvider = "Baseten"
	ModelProviderCerebras    ModelProvider = "Cerebras"
	ModelProviderDeepinfra   ModelProvider = "Deepinfra"
	ModelProviderDeepseek    ModelProvider = "Deepseek"
	ModelProviderGroq        ModelProvider = "Groq"
	ModelProviderHuggingface ModelProvider = "Huggingface"
	ModelProviderMistral     ModelProvider = "Mistral"
	ModelProviderOpenrouter  ModelProvider = "Openrouter"
	ModelProviderTogetherAI  ModelProvider = "TogetherAI"
	ModelProviderXAI         ModelProvider = "XAI"
	ModelProviderFireworks   ModelProvider = "Fireworks"
	ModelProviderCustom      ModelProvider = "Custom"
)

// Visibility of a model to direct client requests.
// +k8s:enum
type ModelVisibility string

const (
	// ModelVisibilityPublic allows direct client requests and includes the model
	// in model discovery responses.
	ModelVisibilityPublic ModelVisibility = "Public"

	// ModelVisibilityInternal allows selection only by virtual models.
	ModelVisibilityInternal ModelVisibility = "Internal"
)

// +kubebuilder:validation:ExactlyOneOf=weighted;failover;conditional
type VirtualModel struct {
	// Weight-based model selection.
	// +optional
	Weighted *WeightedModelRouting `json:"weighted,omitempty"`

	// Priority-based model selection with failover between priority groups.
	// +optional
	Failover *FailoverModelRouting `json:"failover,omitempty"`

	// Ordered condition-based model selection.
	// +optional
	Conditional *ConditionalModelRouting `json:"conditional,omitempty"`
}

type WeightedModelRouting struct {
	// Concrete model targets and their relative weights.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +required
	Targets []WeightedModelTarget `json:"targets"`
}

type WeightedModelTarget struct {
	ModelTargetReference `json:",inline"`

	// Relative traffic weight. Defaults to 1.
	// +kubebuilder:default=1
	// +kubebuilder:validation:Minimum=1
	// +kubebuilder:validation:Maximum=1000000
	// +optional
	Weight int32 `json:"weight,omitempty"`
}

type FailoverModelRouting struct {
	// Concrete model targets grouped by priority. Lower values are preferred.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +required
	Targets []FailoverModelTarget `json:"targets"`
}

type FailoverModelTarget struct {
	ModelTargetReference `json:",inline"`

	// Priority of this target. Lower values are preferred.
	// +kubebuilder:validation:Minimum=0
	// +kubebuilder:validation:Maximum=1000000
	// +required
	Priority int32 `json:"priority"`
}

type ConditionalModelRouting struct {
	// Concrete model targets evaluated in order. The first matching condition is
	// selected. One final target may omit when to act as the fallback.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=64
	// +kubebuilder:validation:XValidation:message="conditional targets without when must be last",rule="self.filter(e, !has(e.when)).size() <= 1 && (!self.exists(e, !has(e.when)) || !has(self[size(self) - 1].when))"
	// +required
	Targets []ConditionalModelTarget `json:"targets"`
}

type ConditionalModelTarget struct {
	ModelTargetReference `json:",inline"`

	// CEL expression that must evaluate to true for this target to be selected.
	// Omit only on the final fallback target.
	// +optional
	When *CELExpression `json:"when,omitempty"`
}

type ModelTargetReference struct {
	// Same-namespace AgentgatewayModel resource selected by this target.
	// +required
	ModelRef corev1.LocalObjectReference `json:"modelRef"`

	// Concrete model name selected through the referenced model. This is needed
	// when modelRef points to a wildcard match.model. When omitted, the referenced
	// model's effective match.model is used.
	// +optional
	Model *LongString `json:"model,omitempty"`
}

// Current attachment status for an AgentgatewayModel.
type AgentgatewayModelStatus struct {
	// Status for each Gateway parent to which this model is attached.
	// +kubebuilder:validation:MaxItems=16
	// +optional
	Parents []gwv1.RouteParentStatus `json:"parents,omitempty"`
}
