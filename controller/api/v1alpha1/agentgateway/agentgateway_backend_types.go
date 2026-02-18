package agentgateway

import (
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewaybackends,verbs=get;list;watch
// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewaybackends/status,verbs=get;update;patch

// +kubebuilder:printcolumn:name="Accepted",type=string,JSONPath=".status.conditions[?(@.type=='Accepted')].status",description="Backend configuration acceptance status"
// +kubebuilder:printcolumn:name="Age",type=date,JSONPath=".metadata.creationTimestamp",description="The age of the backend."

// +genclient
// +kubebuilder:object:root=true
// +kubebuilder:metadata:labels={app=kgateway,app.kubernetes.io/name=kgateway}
// +kubebuilder:resource:categories=kgateway,shortName=agbe
// +kubebuilder:subresource:status
type AgentgatewayBackend struct {
	metav1.TypeMeta `json:",inline"`
	// metadata for the object
	// More info: https://git.k8s.io/community/contributors/devel/sig-architecture/api-conventions.md#metadata
	// +optional
	metav1.ObjectMeta `json:"metadata,omitempty"`

	// spec defines the desired state of AgentgatewayBackend.
	// +required
	Spec AgentgatewayBackendSpec `json:"spec"`

	// status defines the current state of AgentgatewayBackend.
	// +optional
	Status AgentgatewayBackendStatus `json:"status,omitempty"`
	// TODO: embed this into a typed Status field when
	// https://github.com/kubernetes/kubernetes/issues/131533 is resolved
}

// AgentgatewayBackend defines the observed state of AgentgatewayBackend.
type AgentgatewayBackendStatus struct {
	// Conditions is the list of conditions for the backend.
	// +listType=map
	// +listMapKey=type
	// +kubebuilder:validation:MaxItems=8
	// +optional
	Conditions []metav1.Condition `json:"conditions,omitempty"`
}

// +kubebuilder:object:root=true
type AgentgatewayBackendList struct {
	metav1.TypeMeta `json:",inline"`
	metav1.ListMeta `json:"metadata,omitempty"`
	Items           []AgentgatewayBackend `json:"items"`
}

// +kubebuilder:validation:ExactlyOneOf=ai;static;dynamicForwardProxy;mcp
// +kubebuilder:validation:XValidation:rule="has(self.policies) && has(self.policies.ai) ? has(self.ai) : true",message="AI policies require AI backend"
// +kubebuilder:validation:XValidation:rule="has(self.policies) && has(self.policies.mcp) ? has(self.mcp) : true",message="MCP policies require MCP backend"
type AgentgatewayBackendSpec struct {
	// static represents a static hostname.
	// +optional
	Static *StaticBackend `json:"static,omitempty"`

	// ai represents a LLM backend.
	// +optional
	AI *AIBackend `json:"ai,omitempty"`

	// mcp represents an MCP backend
	// +optional
	MCP *MCPBackend `json:"mcp,omitempty"`

	// dynamicForwardProxy configures the proxy to dynamically send requests to the destination based on the incoming
	// request HTTP host header, or TLS SNI for TLS traffic.
	//
	// Note: this Backend type enables users to send trigger the proxy to send requests to arbitrary destinations. Proper
	// access controls must be put in place when using this backend type.
	// +optional
	DynamicForwardProxy *DynamicForwardProxyBackend `json:"dynamicForwardProxy,omitempty"`

	// policies controls policies for communicating with this backend. Policies may also be set in AgentgatewayPolicy;
	// policies are merged on a field-level basis, with policies on the Backend (this field) taking precedence.
	// +optional
	Policies *BackendFull `json:"policies,omitempty"`
}

type DynamicForwardProxyBackend struct {
}

type StaticBackend struct {
	// host to connect to.
	// +required
	Host ShortString `json:"host"`
	// port to connect to.
	// +kubebuilder:validation:Minimum=1
	// +kubebuilder:validation:Maximum=65535
	// +required
	Port int32 `json:"port"`
}

// AIBackend specifies the AI backend configuration
// +kubebuilder:validation:ExactlyOneOf=provider;groups
type AIBackend struct {
	// provider specifies configuration for how to reach the configured LLM provider.
	// +optional
	LLM *LLMProvider `json:"provider,omitempty"`

	// groups specifies a list of groups in priority order where each group defines
	// a set of LLM providers. The priority determines the priority of the backend endpoints chosen.
	// Note: provider names must be unique across all providers in all priority groups. Backend policies
	// may target a specific provider by name using targetRefs[].sectionName.
	//
	// Example configuration with two priority groups:
	// ```yaml
	// groups:
	// - providers:
	//   - azureopenai:
	//       deploymentName: gpt-4o-mini
	//       apiVersion: 2024-02-15-preview
	//       endpoint: ai-gateway.openai.azure.com
	// - providers:
	//   - azureopenai:
	//       deploymentName: gpt-4o-mini-2
	//       apiVersion: 2024-02-15-preview
	//       endpoint: ai-gateway-2.openai.azure.com
	//      policies:
	//        auth:
	//          secretRef:
	//            name: azure-secret
	// ```
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=32
	// +optional
	// TODO: enable this rule when we don't need to support older k8s versions where this rule breaks // +kubebuilder:validation:XValidation:message="provider names must be unique across groups",rule="self.map(pg, pg.providers.map(pp, pp.name)).map(p, self.map(pg, pg.providers.map(pp, pp.name)).filter(cp, cp != p).exists(cp, p.exists(pn, pn in cp))).exists(p, !p)"
	PriorityGroups []PriorityGroup `json:"groups,omitempty"`
}

type PriorityGroup struct {
	// providers specifies a list of LLM providers within this group. Each provider is treated equally in terms of priority,
	// with automatic weighting based on health.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=32
	// +kubebuilder:validation:XValidation:message="provider names must be unique within a group",rule="self.all(p1, self.exists_one(p2, p1.name == p2.name))"
	// +required
	Providers []NamedLLMProvider `json:"providers"`
}

type NamedLLMProvider struct {
	// Name of the provider. Policies can target this provider by name.
	// +required
	Name gwv1.SectionName `json:"name"`

	// policies controls policies for communicating with this backend. Policies may also be set in AgentgatewayPolicy, or
	// in the top level AgentgatewayBackend. policies are merged on a field-level basis, with order: AgentgatewayPolicy <
	// AgentgatewayBackend < AgentgatewayBackend LLM provider (this field).
	// +optional
	Policies *BackendWithAI `json:"policies,omitempty"`

	LLMProvider `json:",inline"`
}

// LLMProvider specifies the target large language model provider that the backend should route requests to.
// +kubebuilder:validation:ExactlyOneOf=openai;azureopenai;anthropic;gemini;vertexai;bedrock
// +kubebuilder:validation:XValidation:rule="has(self.host) || has(self.port) ? has(self.host) && has(self.port) : true",message="both host and port must be set together"
type LLMProvider struct {
	// OpenAI provider
	// +optional
	OpenAI *OpenAIConfig `json:"openai,omitempty"`

	// Azure OpenAI provider
	// +optional
	AzureOpenAI *AzureOpenAIConfig `json:"azureopenai,omitempty"`

	// Anthropic provider
	// +optional
	Anthropic *AnthropicConfig `json:"anthropic,omitempty"`

	// Gemini provider
	// +optional
	Gemini *GeminiConfig `json:"gemini,omitempty"`

	// Vertex AI provider
	// +optional
	VertexAI *VertexAIConfig `json:"vertexai,omitempty"`

	// Bedrock provider
	// +optional
	Bedrock *BedrockConfig `json:"bedrock,omitempty"`

	// Host specifies the hostname to send the requests to.
	// If not specified, the default hostname for the provider is used.
	// +optional
	Host ShortString `json:"host,omitempty"`

	// Port specifies the port to send the requests to.
	// +kubebuilder:validation:Minimum=1
	// +kubebuilder:validation:Maximum=65535
	// +optional
	Port int32 `json:"port,omitempty"`

	// Path specifies the URL path to use for the LLM provider API requests.
	// This is useful when you need to route requests to a different API endpoint while maintaining
	// compatibility with the original provider's API structure.
	// If not specified, the default path for the provider is used.
	// +optional
	Path LongString `json:"path,omitempty"`
}

// OpenAIConfig settings for the [OpenAI](https://developers.openai.com/api/docs/guides/streaming-responses) LLM provider.
type OpenAIConfig struct {
	// Optional: Override the model name, such as `gpt-4o-mini`.
	// If unset, the model name is taken from the request.
	// +optional
	Model *ShortString `json:"model,omitempty"`
}

// AzureOpenAIConfig settings for the [Azure OpenAI](https://learn.microsoft.com/en-us/azure/ai-foundry/?view=foundry-classic) LLM provider.
// +kubebuilder:validation:XValidation:message="deploymentName is required for this apiVersion",rule="!has(self.apiVersion) || self.apiVersion == 'v1' ? true : has(self.deploymentName)"
type AzureOpenAIConfig struct {
	// The endpoint for the Azure OpenAI API to use, such as `my-endpoint.openai.azure.com`.
	// If the scheme is included, it is stripped.
	// +required
	Endpoint ShortString `json:"endpoint"`

	// The name of the Azure OpenAI model deployment to use.
	// For more information, see the [Azure OpenAI model docs](https://learn.microsoft.com/en-us/azure/ai-foundry/foundry-models/concepts/models-sold-directly-by-azure?view=foundry-classic).
	// This is required if ApiVersion is not 'v1'. For v1, the model can be set in the request.
	// +optional
	DeploymentName *ShortString `json:"deploymentName,omitempty"`

	// The version of the Azure OpenAI API to use.
	// For more information, see the [Azure OpenAI API version reference](https://learn.microsoft.com/en-us/azure/ai-foundry/?view=foundry-classicreference#api-specs).
	// If unset, defaults to "v1"
	// +optional
	ApiVersion *TinyString `json:"apiVersion,omitempty"`
}

// GeminiConfig settings for the [Gemini](https://ai.google.dev/gemini-api/docs) LLM provider.
type GeminiConfig struct {
	// Optional: Override the model name, such as `gemini-2.5-pro`.
	// If unset, the model name is taken from the request.
	// +optional
	Model *ShortString `json:"model,omitempty"`
}

// VertexAIConfig settings for the [Vertex AI](https://docs.cloud.google.com/vertex-ai/docs) LLM provider.
type VertexAIConfig struct {
	// Optional: Override the model name, such as `gpt-4o-mini`.
	// If unset, the model name is taken from the request.
	// +optional
	Model *ShortString `json:"model,omitempty"`

	// The ID of the Google Cloud Project that you use for the Vertex AI.
	// +required
	ProjectId TinyString `json:"projectId"`

	// The location of the Google Cloud Project that you use for the Vertex AI.
	// +required
	Region TinyString `json:"region"`
}

// AnthropicConfig settings for the [Anthropic](https://platform.claude.com/docs/en/release-notes/overview) LLM provider.
type AnthropicConfig struct {
	// Optional: Override the model name, such as `gpt-4o-mini`.
	// If unset, the model name is taken from the request.
	// +optional
	Model *ShortString `json:"model,omitempty"`
}

type BedrockConfig struct {
	// Region is the AWS region to use for the backend.
	// Defaults to us-east-1 if not specified.
	// +optional
	// +kubebuilder:default=us-east-1
	// +kubebuilder:validation:MinLength=1
	// +kubebuilder:validation:MaxLength=63
	// +kubebuilder:validation:Pattern="^[a-z0-9-]+$"
	Region string `json:"region,omitempty"`

	// Optional: Override the model name, such as `gpt-4o-mini`.
	// If unset, the model name is taken from the request.
	// +optional
	Model *ShortString `json:"model,omitempty"`

	// Guardrail configures the Guardrail policy to use for the backend. See <https://docs.aws.amazon.com/bedrock/latest/userguide/guardrails.html>
	// If not specified, the AWS Guardrail policy will not be used.
	// +optional
	Guardrail *AWSGuardrailConfig `json:"guardrail,omitempty"`
}

type AWSGuardrailConfig struct {
	// GuardrailIdentifier is the identifier of the Guardrail policy to use for the backend.
	// +required
	GuardrailIdentifier ShortString `json:"identifier"`

	// GuardrailVersion is the version of the Guardrail policy to use for the backend.
	// +required
	GuardrailVersion ShortString `json:"version"`
}

// MCPBackend configures mcp backends
type MCPBackend struct {
	// Targets is a list of MCPBackend targets to use for this backend.
	// Policies targeting MCPBackend targets must use targetRefs[].sectionName
	// to select the target by name.
	// +listType=map
	// +listMapKey=name
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=32
	// +kubebuilder:validation:XValidation:message="target names must be unique",rule="self.all(t1, self.exists_one(t2, t1.name == t2.name))"
	// +required
	Targets []McpTargetSelector `json:"targets"`

	// SessionRouting configures MCP session behavior for requests.
	// Defaults to Stateful if not set.
	// +optional
	SessionRouting SessionRouting `json:"sessionRouting,omitempty"`
}

// McpTargetSelector defines the MCPBackend target to use for this backend.
// +kubebuilder:validation:ExactlyOneOf=selector;static
type McpTargetSelector struct {
	// Name of the MCPBackend target.
	// +required
	Name gwv1.SectionName `json:"name"`

	// selector is a label selector is the selector to use to select Services.
	// If policies are needed on a per-service basis, AgentgatewayPolicy can target the desired Service.
	// +optional
	Selector *McpSelector `json:"selector,omitempty"`

	// static configures a static MCP destination. When connecting to in-cluster Services, it is recommended to use
	// 'selector' instead.
	// +optional
	Static *McpTarget `json:"static,omitempty"`
}

const (
	// Stateful mode creates an MCP session (via mcp-session-id) and internally
	// ensures requests for that session are routed to a consistent backend replica.
	Stateful  SessionRouting = "Stateful"
	Stateless SessionRouting = "Stateless"
)

// +kubebuilder:validation:Enum=Stateful;Stateless
type SessionRouting string

// +kubebuilder:validation:AtLeastOneOf=namespaces;services
type McpSelector struct {
	// namespace is the label selector in which namespaces Services should be selected from.
	// If unset, only the namespace of the AgentgatewayBackend is searched.
	// +optional
	Namespace *metav1.LabelSelector `json:"namespaces,omitempty"`

	// services is the label selector for which Services should be selected.
	// +optional
	Service *metav1.LabelSelector `json:"services,omitempty"`
}

// McpTarget defines a single MCPBackend target configuration.
type McpTarget struct {
	// Host is the hostname or IP address of the MCPBackend target.
	// +required
	Host ShortString `json:"host"`

	// Port is the port number of the MCPBackend target.
	// +kubebuilder:validation:Minimum=1
	// +kubebuilder:validation:Maximum=65535
	// +required
	Port int32 `json:"port"`

	// Path is the URL path of the MCPBackend target endpoint.
	// Defaults to "/sse" for SSE protocol or "/mcp" for StreamableHTTP protocol if not specified.
	// +optional
	Path *LongString `json:"path,omitempty"`

	// Protocol is the protocol to use for the connection to the MCPBackend target.
	// +optional
	Protocol *MCPProtocol `json:"protocol,omitempty"`

	// policies controls policies for communicating with this backend. Policies may also be set in AgentgatewayPolicy, or
	// in the top level AgentgatewayBackend. Policies are merged on a field-level basis, with order: AgentgatewayPolicy <
	// AgentgatewayBackend < AgentgatewayBackend MCP (this field).
	// +optional
	Policies *BackendWithMCP `json:"policies,omitempty"`
}

// MCPProtocol defines the protocol to use for the MCPBackend target
// +kubebuilder:validation:Enum=StreamableHTTP;SSE
type MCPProtocol string

const (

	// MCPProtocolStreamableHTTP specifies Streamable HTTP must be used as the protocol
	MCPProtocolStreamableHTTP MCPProtocol = "StreamableHTTP"

	// MCPProtocolSSE specifies Server-Sent Events (SSE) must be used as the protocol
	MCPProtocolSSE MCPProtocol = "SSE"
)
