package agentgateway

import (
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewaybackends,verbs=get;list;watch
// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewaybackends/status,verbs=get;update;patch

// +kubebuilder:printcolumn:name="Accepted",type=string,JSONPath=".status.conditions[?(@.type=='Accepted')].status",description="Backend configuration acceptance status"
// +kubebuilder:printcolumn:name="Age",type=date,JSONPath=".metadata.creationTimestamp",description="The age of the backend."

// +genclient
// +kubebuilder:object:root=true
// +kubebuilder:metadata:labels={app=agentgateway,app.kubernetes.io/name=agentgateway}
// +kubebuilder:resource:categories=agentgateway,shortName=agbe
// +kubebuilder:subresource:status
type AgentgatewayBackend struct {
	metav1.TypeMeta `json:",inline"`
	// metadata for the object
	// More info: https://git.k8s.io/community/contributors/devel/sig-architecture/api-conventions.md#metadata
	// +optional
	metav1.ObjectMeta `json:"metadata,omitempty"`

	// Desired backend configuration.
	// +required
	Spec AgentgatewayBackendSpec `json:"spec"`

	// Current backend status.
	// +optional
	Status AgentgatewayBackendStatus `json:"status,omitempty"`
	// TODO: embed this into a typed Status field when
	// https://github.com/kubernetes/kubernetes/issues/131533 is resolved
}

// Current backend status.
type AgentgatewayBackendStatus struct {
	// Current condition state for the backend.
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

// +kubebuilder:validation:ExactlyOneOf=ai;static;dynamicForwardProxy;mcp;aws;a2a
// +kubebuilder:validation:XValidation:rule="has(self.policies) && has(self.policies.ai) ? has(self.ai) : true",message="AI policies require AI backend"
// +kubebuilder:validation:XValidation:rule="has(self.policies) && has(self.policies.mcp) ? has(self.mcp) : true",message="MCP policies require MCP backend"
type AgentgatewayBackendSpec struct {
	// Static hostname, IP address, or Unix Domain Socket backend.
	// +optional
	Static *StaticBackend `json:"static,omitempty"`

	// A2A backend.
	// +optional
	A2A *A2ABackend `json:"a2a,omitempty"`

	// LLM backend.
	// +optional
	AI *AIBackend `json:"ai,omitempty"`

	// MCP backend.
	// +optional
	MCP *MCPBackend `json:"mcp,omitempty"`

	// Dynamically sends requests to the destination based on the incoming
	// request HTTP host header, or TLS SNI for TLS traffic.
	//
	// Warning: this backend type can send requests to arbitrary destinations. Proper
	// access controls must be put in place when using this backend type.
	// +optional
	DynamicForwardProxy *DynamicForwardProxyBackend `json:"dynamicForwardProxy,omitempty"`

	// AWS service backend, such as AgentCore.
	// +optional
	Aws *AwsBackend `json:"aws,omitempty"`

	// Policies for communicating with this backend. Policies may also be set
	// with AgentgatewayPolicy. Backend policies take precedence over policy
	// resources when they set the same field.
	// +optional
	Policies *BackendFull `json:"policies,omitempty"`
}

type DynamicForwardProxyBackend struct {
}

// Configures an AWS service backend.
// +kubebuilder:validation:ExactlyOneOf=agentCore
type AwsBackend struct {
	// Amazon Bedrock AgentCore backend settings.
	// +optional
	AgentCore *AwsAgentCoreBackend `json:"agentCore,omitempty"`
}

// Configures Amazon Bedrock AgentCore.
type AwsAgentCoreBackend struct {
	// ARN of the AgentCore runtime.
	// +required
	AgentRuntimeArn string `json:"agentRuntimeArn"`
	// Alias or version qualifier.
	// +optional
	Qualifier *string `json:"qualifier,omitempty"`
}

// Static backend endpoint, either TCP (`host` and `port`) or Unix Domain Socket.
// +kubebuilder:validation:XValidation:rule="has(self.unixPath) || (has(self.host) && has(self.port))",message="must specify either unixPath or both host and port"
// +kubebuilder:validation:XValidation:rule="!has(self.unixPath) || (!has(self.host) && !has(self.port))",message="unixPath and host/port are mutually exclusive"
type StaticBackend struct {
	// Host to connect to for TCP backends.
	// +optional
	Host ShortString `json:"host,omitempty"`
	// Port to connect to for TCP backends.
	// +kubebuilder:validation:Minimum=1
	// +kubebuilder:validation:Maximum=65535
	// +optional
	Port int32 `json:"port,omitempty"`
	// Filesystem path to a Unix Domain Socket. The gateway pod
	// must share a volume with the target (e.g., via emptyDir sidecar pattern).
	// Mutually exclusive with host/port.
	// +kubebuilder:validation:MinLength=1
	// +optional
	UnixPath *string `json:"unixPath,omitempty"`
}

// A2A backend endpoint.
type A2ABackend struct {
	// Hostname or IP address of the A2A backend.
	// +required
	Host ShortString `json:"host"`

	// Port number of the A2A backend.
	// +kubebuilder:validation:Minimum=1
	// +kubebuilder:validation:Maximum=65535
	// +required
	Port int32 `json:"port"`
}

// AI backend configuration.
// +kubebuilder:validation:ExactlyOneOf=provider;groups
type AIBackend struct {
	// Configuration for how to reach the configured LLM
	// provider.
	// +optional
	LLM *LLMProvider `json:"provider,omitempty"`

	// Groups in priority order, where each group
	// defines a set of LLM providers. The priority determines the priority of
	// the backend endpoints chosen.
	// Note: provider names must be unique across all providers in all priority
	// groups. Backend policies may target a specific provider by name using
	// `targetRefs[].sectionName`.
	//
	// Example configuration with two priority groups:
	//
	//	groups:
	//	- providers:
	//	  - azureopenai:
	//	      deploymentName: gpt-4o-mini
	//	      apiVersion: 2024-02-15-preview
	//	      endpoint: ai-gateway.openai.azure.com
	//	- providers:
	//	  - azureopenai:
	//	      deploymentName: gpt-4o-mini-2
	//	      apiVersion: 2024-02-15-preview
	//	      endpoint: ai-gateway-2.openai.azure.com
	//	     policies:
	//	       auth:
	//	         secretRef:
	//	           name: azure-secret
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=8
	// +optional
	// TODO: enable this rule when we don't need to support older k8s versions where this rule breaks // +kubebuilder:validation:XValidation:message="provider names must be unique across groups",rule="self.map(pg, pg.providers.map(pp, pp.name)).map(p, self.map(pg, pg.providers.map(pp, pp.name)).filter(cp, cp != p).exists(cp, p.exists(pn, pn in cp))).exists(p, !p)"
	PriorityGroups []PriorityGroup `json:"groups,omitempty"`
}

type PriorityGroup struct {
	// LLM providers within this group. Each provider is treated equally in terms of priority,
	// with automatic weighting based on health.
	//
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +kubebuilder:validation:XValidation:message="provider names must be unique within a group",rule="self.all(p1, self.exists_one(p2, p1.name == p2.name))"
	// +required
	Providers []NamedLLMProvider `json:"providers"`
}

type NamedLLMProvider struct {
	// Name of the provider. Policies can target this provider by name.
	// +required
	Name gwv1.SectionName `json:"name"`

	// Policies for communicating with this backend.
	// Policies may also be set in `AgentgatewayPolicy`, or in the top-level
	// `AgentgatewayBackend`. Policies are merged on a field-level basis, with
	// order: `AgentgatewayPolicy` < `AgentgatewayBackend` < `AgentgatewayBackend`
	// LLM provider (this field).
	// +optional
	Policies *BackendWithAI `json:"policies,omitempty"`

	LLMProvider `json:",inline"`
}

// Large language model provider that the backend routes requests to.
// +kubebuilder:validation:ExactlyOneOf=openai;azureopenai;azure;anthropic;gemini;vertexai;bedrock;custom
// +kubebuilder:validation:XValidation:rule="has(self.host) || has(self.port) ? has(self.host) && has(self.port) : true",message="both host and port must be set together"
// +kubebuilder:validation:XValidation:rule="has(self.custom) ? has(self.custom.backendRef) != has(self.host) : true",message="custom providers must specify exactly one of backendRef or host and port"
// +kubebuilder:validation:XValidation:rule="!(has(self.path) && has(self.pathPrefix))",message="path and pathPrefix are mutually exclusive"
// +kubebuilder:validation:XValidation:rule="!(has(self.custom) && self.custom.formats.exists(f, has(f.path)) && (has(self.path) || has(self.pathPrefix)))",message="path, pathPrefix, and custom format paths are mutually exclusive"
// +kubebuilder:validation:XValidation:rule="has(self.pathPrefix) ? has(self.host) : true",message="pathPrefix requires host to be set"
type LLMProvider struct {
	// OpenAI provider settings.
	// +optional
	OpenAI *OpenAIConfig `json:"openai,omitempty"`

	// Azure OpenAI provider settings.
	// +optional
	AzureOpenAI *AzureOpenAIConfig `json:"azureopenai,omitempty"`

	// Azure provider with resource-based configuration.
	// Supports both Azure OpenAI and Azure AI Foundry resource types.
	// +optional
	Azure *AzureConfig `json:"azure,omitempty"`

	// Anthropic provider settings.
	// +optional
	Anthropic *AnthropicConfig `json:"anthropic,omitempty"`

	// Gemini provider settings.
	// +optional
	Gemini *GeminiConfig `json:"gemini,omitempty"`

	// Vertex AI provider settings.
	// +optional
	VertexAI *VertexAIConfig `json:"vertexai,omitempty"`

	// Bedrock provider settings.
	// +optional
	Bedrock *BedrockConfig `json:"bedrock,omitempty"`

	// Custom provider configures a non-managed or self-hosted LLM provider.
	// Use this when the provider target and API formats should be declared
	// explicitly instead of inferred from a managed provider such as OpenAI or
	// Anthropic.
	// +optional
	Custom *CustomProvider `json:"custom,omitempty"`

	// Hostname to send requests to.
	// For custom providers without backendRef, host and port specify the target.
	// For managed providers, host and port override the provider default.
	// +optional
	Host ShortString `json:"host,omitempty"`

	// Port to send requests to.
	// +kubebuilder:validation:Minimum=1
	// +kubebuilder:validation:Maximum=65535
	// +optional
	Port int32 `json:"port,omitempty"`

	// URL path to use for LLM provider API requests.
	// This is useful when you need to route requests to a different API endpoint while maintaining
	// compatibility with the original provider's API structure.
	// If not specified, the default path for the provider is used.
	// +optional
	Path LongString `json:"path,omitempty"`

	// Overrides the default base path prefix, such as `/v1`, for upstream requests.
	// Path translation for cross-format requests still applies using this prefix.
	// Only supported for OpenAI and Anthropic providers.
	// +optional
	PathPrefix LongString `json:"pathPrefix,omitempty"`
}

// References a namespace-local backend resource.
// +kubebuilder:validation:XValidation:rule="(size(self.group) == 0 && self.kind == 'Service') ? has(self.port) : true",message="Must have port for Service reference"
type LocalBackendObjectReference struct {
	// API group of the referenced resource. For example, `gateway.networking.k8s.io`.
	// When unspecified or empty string, core API group is inferred.
	// +kubebuilder:default=""
	// +kubebuilder:validation:MaxLength=253
	// +kubebuilder:validation:Pattern=`^$|^[a-z0-9]([-a-z0-9]*[a-z0-9])?(\.[a-z0-9]([-a-z0-9]*[a-z0-9])?)*$`
	// +optional
	Group *string `json:"group,omitempty"`

	// Kind of the referenced resource. For example, `Service`.
	// Defaults to "Service" when not specified.
	// +kubebuilder:default=Service
	// +kubebuilder:validation:MaxLength=63
	// +kubebuilder:validation:MinLength=1
	// +kubebuilder:validation:Pattern=`^[a-zA-Z]([-a-zA-Z0-9]*[a-zA-Z0-9])?$`
	// +optional
	Kind *string `json:"kind,omitempty"`

	// Name of the referenced resource.
	// +kubebuilder:validation:MaxLength=253
	// +kubebuilder:validation:MinLength=1
	// +required
	Name string `json:"name"`

	// Destination port number to use for this resource.
	// Required when the referenced resource is a Kubernetes Service.
	// +kubebuilder:validation:Minimum=1
	// +kubebuilder:validation:Maximum=65535
	// +optional
	Port *int32 `json:"port,omitempty"`
}

// Provider with explicit API format support and an explicit target.
// Use this for local, self-hosted, or OpenAI-compatible providers whose
// supported request/response formats are not fully described by the managed
// provider types.
// +kubebuilder:validation:XValidation:rule="!has(self.backendRef) || (((!has(self.backendRef.group) || self.backendRef.group == \"\") && (!has(self.backendRef.kind) || self.backendRef.kind == 'Service')) || (has(self.backendRef.group) && self.backendRef.group == 'inference.networking.k8s.io' && has(self.backendRef.kind) && self.backendRef.kind == 'InferencePool'))",message="custom provider backendRef may target only Service or InferencePool"
type CustomProvider struct {
	// Kubernetes backend that serves this provider.
	// `backendRef` may target only a namespace-local Service or InferencePool.
	// If unset, host and port must be set on the parent provider.
	// +optional
	BackendRef *LocalBackendObjectReference `json:"backendRef,omitempty"`

	// Model name override, such as `gpt-oss`.
	// If unset, the model name is taken from the request.
	// +optional
	Model *ShortString `json:"model,omitempty"`

	// Provider-native API formats this provider supports.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=6
	// +listType=map
	// +listMapKey=type
	// +required
	Formats []ProviderFormatConfig `json:"formats"`
}

// Provider-native LLM API format settings.
// +kubebuilder:validation:XValidation:rule="!has(self.path) || self.path.startsWith('/')",message="path must start with /"
type ProviderFormatConfig struct {
	// Provider-native API format.
	// +required
	Type ProviderFormat `json:"type"`

	// Default upstream path override for this format.
	// If unset, agentgateway uses the default path for the format.
	// +optional
	Path LongString `json:"path,omitempty"`
}

// Provider-native LLM API format.
// +k8s:enum
type ProviderFormat string

const (
	// ProviderFormatCompletions is the OpenAI-compatible chat completions API.
	ProviderFormatCompletions ProviderFormat = "Completions"

	// ProviderFormatMessages is the Anthropic-compatible messages API.
	ProviderFormatMessages ProviderFormat = "Messages"

	// ProviderFormatResponses is the OpenAI responses API.
	ProviderFormatResponses ProviderFormat = "Responses"

	// ProviderFormatEmbeddings is the OpenAI-compatible embeddings API.
	ProviderFormatEmbeddings ProviderFormat = "Embeddings"

	// ProviderFormatAnthropicTokenCount is the Anthropic token-count API.
	ProviderFormatAnthropicTokenCount ProviderFormat = "AnthropicTokenCount" //nolint:gosec // G101: False positive - this is an API format name, not credentials

	// ProviderFormatRealtime is the OpenAI-compatible realtime API.
	ProviderFormatRealtime ProviderFormat = "Realtime"

	// ProviderFormatRerank is the Cohere-compatible rerank API.
	ProviderFormatRerank ProviderFormat = "Rerank"
)

// Settings for the [OpenAI](https://developers.openai.com/api/docs/guides/streaming-responses) LLM provider.
type OpenAIConfig struct {
	// Model name override, such as `gpt-4o-mini`.
	// If unset, the model name is taken from the request.
	// +optional
	Model *ShortString `json:"model,omitempty"`
}

// Settings for the [Azure OpenAI](https://learn.microsoft.com/en-us/azure/foundry/?view=foundry-classic) LLM provider.
// +kubebuilder:validation:XValidation:message="deploymentName is required for this apiVersion",rule="!has(self.apiVersion) || self.apiVersion == 'v1' ? true : has(self.deploymentName)"
type AzureOpenAIConfig struct {
	// The endpoint for the Azure OpenAI API to use, such as `my-endpoint.openai.azure.com`.
	// If the scheme is included, it is stripped.
	// +required
	Endpoint ShortString `json:"endpoint"`

	// The name of the Azure OpenAI model deployment to use.
	// For more information, see the [Azure OpenAI model docs](https://learn.microsoft.com/en-us/azure/foundry/foundry-models/concepts/models-sold-directly-by-azure?view=foundry-classic).
	// This is required if `apiVersion` is not `v1`. For `v1`, the model can be
	// set in the request.
	// +optional
	DeploymentName *ShortString `json:"deploymentName,omitempty"`

	// The version of the Azure OpenAI API to use.
	// For more information, see the [Azure OpenAI API version reference](https://learn.microsoft.com/en-us/azure/foundry/openai/reference).
	// If unset, defaults to `v1`.
	// +optional
	ApiVersion *TinyString `json:"apiVersion,omitempty"`
}

// Type of Azure endpoint.
// +k8s:enum
type AzureResourceType string

const (
	// AzureResourceTypeOpenAI uses the Azure OpenAI endpoint: {resourceName}.openai.azure.com
	AzureResourceTypeOpenAI AzureResourceType = "OpenAI"
	// AzureResourceTypeFoundry uses the Azure AI Foundry endpoint: {resourceName}.services.ai.azure.com
	AzureResourceTypeFoundry AzureResourceType = "Foundry"
)

// Settings for Azure AI backends, supporting both Azure OpenAI and Azure AI Foundry.
// +kubebuilder:validation:XValidation:message="projectName is required when resourceType is Foundry",rule="self.resourceType != 'Foundry' || has(self.projectName)"
type AzureConfig struct {
	// The Azure resource name used to construct the endpoint host.
	// For OpenAI: {resourceName}.openai.azure.com
	// For Foundry: {resourceName}.services.ai.azure.com
	// Note: when the Azure portal "Foundry legacy" template was used, the
	// generated resource name may end in "-resource" (e.g. "myproject-resource");
	// that suffix is part of the resource name as the user configured it, not
	// part of the hostname suffix agentgateway should append.
	// +required
	ResourceName ShortString `json:"resourceName"`

	// The type of Azure endpoint. Determines the host suffix.
	// +required
	ResourceType AzureResourceType `json:"resourceType"`

	// Model name override, such as `gpt-4o-mini`.
	// If unset, the model name is taken from the request.
	// +optional
	Model *ShortString `json:"model,omitempty"`

	// The version of the Azure OpenAI API to use.
	// If unset, defaults to `v1`.
	// +optional
	ApiVersion *TinyString `json:"apiVersion,omitempty"`

	// The Foundry project name, required when `resourceType` is `Foundry`.
	// Used to construct paths: /api/projects/{projectName}/openai/v1/...
	// +optional
	ProjectName *ShortString `json:"projectName,omitempty"`
}

// Settings for the [Gemini](https://ai.google.dev/gemini-api/docs) LLM provider.
type GeminiConfig struct {
	// Model name override, such as `gemini-2.5-pro`.
	// If unset, the model name is taken from the request.
	// +optional
	Model *ShortString `json:"model,omitempty"`
}

// Settings for the [Vertex AI](https://docs.cloud.google.com/gemini-enterprise-agent-platform) LLM provider.
type VertexAIConfig struct {
	// Model name override, such as `gpt-4o-mini`.
	// If unset, the model name is taken from the request.
	// +optional
	Model *ShortString `json:"model,omitempty"`

	// The ID of the Google Cloud Project that you use for the Vertex AI.
	// +required
	ProjectId TinyString `json:"projectId"`

	// The location of the Google Cloud Project that you use for the Vertex AI.
	// Special values: `global` uses the global endpoint, while `us` and `eu` use restricted
	// multi-region endpoints. Other values are treated as regional locations.
	// Defaults to `global` if not specified.
	// +optional
	// +kubebuilder:default=global
	Region TinyString `json:"region,omitempty"`
}

// Settings for the [Anthropic](https://platform.claude.com/docs/en/release-notes/overview) LLM provider.
type AnthropicConfig struct {
	// Model name override, such as `gpt-4o-mini`.
	// If unset, the model name is taken from the request.
	// +optional
	Model *ShortString `json:"model,omitempty"`
}

type BedrockConfig struct {
	// AWS region to use for the backend.
	// Defaults to `us-east-1` if not specified.
	// +optional
	// +kubebuilder:default=us-east-1
	// +kubebuilder:validation:MinLength=1
	// +kubebuilder:validation:MaxLength=63
	// +kubebuilder:validation:Pattern="^[a-z0-9-]+$"
	Region string `json:"region,omitempty"`

	// Model name override, such as `gpt-4o-mini`.
	// If unset, the model name is taken from the request.
	// +optional
	Model *ShortString `json:"model,omitempty"`

	// Guardrail policy to use for the backend. See
	// <https://docs.aws.amazon.com/bedrock/latest/userguide/guardrails.html>.
	// If not specified, the AWS Guardrail policy will not be used.
	// +optional
	Guardrail *AWSGuardrailConfig `json:"guardrail,omitempty"`
}

type AWSGuardrailConfig struct {
	// Identifier of the Guardrail policy to use for the backend.
	// +required
	GuardrailIdentifier ShortString `json:"identifier"`

	// Version of the Guardrail policy to use for the backend.
	// +required
	GuardrailVersion ShortString `json:"version"`
}

// MCP backend settings.
type MCPBackend struct {
	// MCP targets to use for this backend. Policies
	// targeting MCP targets must use `targetRefs[].sectionName` to select
	// the target by name.
	//
	// +listType=map
	// +listMapKey=name
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=32
	// +kubebuilder:validation:XValidation:message="target names must be unique",rule="self.all(t1, self.exists_one(t2, t1.name == t2.name))"
	// +required
	Targets []McpTargetSelector `json:"targets"`

	// MCP session routing behavior.
	// Defaults to `Stateful` if not set.
	// +optional
	SessionRouting SessionRouting `json:"sessionRouting,omitempty"`

	// Behavior when MCP targets fail to initialize or
	// become unavailable at runtime. `FailOpen` skips failed targets and
	// continues serving from healthy ones. `FailClosed` (default) fails the
	// entire session if any target fails.
	// +optional
	FailureMode FailureMode `json:"failureMode,omitempty"`
}

const (
	// FailClosed fails the entire MCP session if any target fails.
	FailClosed FailureMode = "FailClosed"
	// FailOpen skips failed targets and continues serving from healthy ones.
	FailOpen FailureMode = "FailOpen"
)

// +k8s:enum
type FailureMode string

// MCP target selection for this backend.
// +kubebuilder:validation:ExactlyOneOf=selector;static
type McpTargetSelector struct {
	// Name of the MCP target.
	// +required
	Name gwv1.SectionName `json:"name"`

	// Label selector used to select `Service` resources.
	// If policies are needed on a per-service basis, `AgentgatewayPolicy` can
	// target the desired `Service`.
	// +optional
	Selector *McpSelector `json:"selector,omitempty"`

	// Static MCP destination. When connecting to
	// in-cluster `Service` resources, it is recommended to use `selector`
	// instead.
	// +optional
	Static *McpTarget `json:"static,omitempty"`
}

const (
	// `Stateful` mode creates an MCP session (via `mcp-session-id`) and
	// internally
	// ensures requests for that session are routed to a consistent backend replica.
	Stateful  SessionRouting = "Stateful"
	Stateless SessionRouting = "Stateless"
)

// +k8s:enum
type SessionRouting string

// +kubebuilder:validation:AtLeastOneFieldSet
type McpSelector struct {
	// `namespace` is the label selector for namespaces that `Service`
	// resources should be selected from. If unset, only the namespace of the
	// `AgentgatewayBackend` is searched.
	// +optional
	Namespace *metav1.LabelSelector `json:"namespaces,omitempty"`

	// `services` is the label selector for which `Service` resources should be
	// selected.
	// +optional
	Service *metav1.LabelSelector `json:"services,omitempty"`
}

// MCP target configuration.
// +kubebuilder:validation:ExactlyOneOf=host;backendRef
// +kubebuilder:validation:XValidation:rule="!has(self.backendRef) || !has(self.policies)",message="mcp target policies may not be used with backendRef"
type McpTarget struct {
	// Hostname or IP address of the MCP target.
	// +optional
	Host *ShortString `json:"host,omitempty"`

	// Namespace-local `Service` resource by name.
	// When set, this replaces `host` only; `port`, `path`, and `protocol`
	// remain configured on this target.
	// +optional
	BackendRef *corev1.LocalObjectReference `json:"backendRef,omitempty"`

	// Port number of the MCP target.
	// +kubebuilder:validation:Minimum=1
	// +kubebuilder:validation:Maximum=65535
	// +required
	Port int32 `json:"port"`

	// URL path of the MCP target endpoint.
	// Defaults to `"/sse"` for the `SSE` protocol or `"/mcp"` for the
	// `StreamableHTTP` protocol if not specified.
	// +optional
	Path *LongString `json:"path,omitempty"`

	// Protocol to use for the connection to the MCP
	// target.
	// +optional
	Protocol *MCPProtocol `json:"protocol,omitempty"`

	// Policies for communicating with this backend.
	// Policies may also be set in `AgentgatewayPolicy`, or in the top-level
	// `AgentgatewayBackend`. Policies are merged on a field-level basis, with
	// order: `AgentgatewayPolicy` < `AgentgatewayBackend` < `AgentgatewayBackend` MCP (this field).
	// This field may only be used with host-based static targets, not
	// `backendRef`.
	// +optional
	Policies *BackendSimple `json:"policies,omitempty"`
}

// Protocol to use for an MCP target.
// +k8s:enum
type MCPProtocol string

const (
	// MCPProtocolStreamableHTTP specifies that `StreamableHTTP` must be used as
	// the protocol.
	MCPProtocolStreamableHTTP MCPProtocol = "StreamableHTTP"

	// MCPProtocolSSE specifies that Server-Sent Events (`SSE`) must be used as
	// the protocol.
	MCPProtocolSSE MCPProtocol = "SSE"
)
