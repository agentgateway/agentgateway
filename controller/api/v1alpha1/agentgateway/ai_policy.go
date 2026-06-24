package agentgateway

import (
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

// Enriches requests sent to the LLM provider by appending and prepending system prompts.
//
// Prompt enrichment allows you to add additional context to the prompt before sending it to the model.
// Unlike RAG or other dynamic context methods, prompt enrichment is static and is applied to every request.
//
// **Note**: Some providers, including Anthropic, do not support `SYSTEM`
// role messages, and instead have a dedicated `system` field in the input
// JSON. In this case, use the [`defaults` setting](#fielddefault) to set the
// `system` field.
//
// The following example prepends a system prompt of
// `Answer all questions in French.` and appends
// `Describe the painting as if you were a famous art critic from the 17th century.`
// to each request that is sent to the `openai` `HTTPRoute`.
//
//	name: openai-opt
//	namespace: agentgateway-system
//
// spec:
//
//	targetRefs:
//	- group: gateway.networking.k8s.io
//	  kind: HTTPRoute
//	  name: openai
//	ai:
//	    promptEnrichment:
//	      prepend:
//	      - role: SYSTEM
//	        content: "Answer all questions in French."
//	      append:
//	      - role: USER
//	        content: "Describe the painting as if you were a famous art critic from the 17th century."
type AIPromptEnrichment struct {
	// Messages to prepend to the prompt sent by the client.
	// +optional
	Prepend []Message `json:"prepend,omitempty"`

	// Messages to append to the prompt sent by the client.
	// +optional
	Append []Message `json:"append,omitempty"`
}

// An entry for a message to prepend or append to each prompt.
type Message struct {
	// Role of the message. The available roles depend on the backend
	// LLM provider model, such as `SYSTEM` or `USER` in the OpenAI API.
	// +required
	Role string `json:"role"`

	// String content of the message.
	// +required
	Content string `json:"content"`
}

// Built-in regex patterns for specific types of strings in prompts.
// For example, if you specify `CreditCard`, any credit card numbers
// in the request or response are matched.
// +k8s:enum
type BuiltIn string

const (
	// Default regex matching for Social Security numbers.
	SSN BuiltIn = "Ssn"

	// Default regex matching for credit card numbers.
	CREDIT_CARD BuiltIn = "CreditCard"

	// Default regex matching for phone numbers.
	PHONE_NUMBER BuiltIn = "PhoneNumber"

	// Default regex matching for email addresses.
	EMAIL BuiltIn = "Email"

	// Default regex matching for Canadian Social Insurance Numbers.
	CA_SIN BuiltIn = "CaSin"
)

// Action to take if a regex pattern is matched in a request or response.
// This setting applies only to request matches. `PromptguardResponse`
// matches are always masked by default.
// +k8s:enum
type Action string

const (
	// Mask the matched data in the request.
	MASK Action = "Mask"

	// Reject the request if the regex matches content in the request.
	REJECT Action = "Reject"
)

// Streaming prompt guard mode.
// +k8s:enum
type PromptGuardStreamingMode string

const (
	// Enable prompt guards for streaming responses and realtime websocket messages.
	PromptGuardStreamingModeEnabled PromptGuardStreamingMode = "Enabled"
)

// Regular expression matching for prompt guards and data masking.
type Regex struct {
	// Regex patterns to match against the request or response.
	// Matches and built-ins are additive.
	// +optional
	Matches []LongString `json:"matches,omitempty"`

	// Built-in regex patterns to match against the request or response.
	// Matches and built-ins are additive.
	// +optional
	Builtins []BuiltIn `json:"builtins,omitempty"`

	// The action to take if a regex pattern is matched in a request or response.
	// This setting applies only to request matches. `PromptguardResponse`
	// matches are always masked by default.
	// Defaults to `Mask`.
	// +kubebuilder:default=Mask
	// +optional
	Action *Action `json:"action,omitempty"`
}

// Webhook for prompt guard request or response checks.
type Webhook struct {
	// Webhook server to reach.
	//
	// Supported types: Service and Backend.
	// +required
	BackendRef gwv1.BackendObjectReference `json:"backendRef"`

	// HTTP header matches used to select the headers to forward to the webhook.
	// Request headers are used when forwarding requests and response headers
	// are used when forwarding responses.
	// By default, no headers are forwarded.
	// +optional
	ForwardHeaderMatches []gwv1.HTTPHeaderMatch `json:"forwardHeaderMatches,omitempty"`

	// Behavior when the webhook guardrail is unavailable
	// or returns an error. `FailOpen` allows the request to continue.
	// `FailClosed` (default) rejects the request.
	// +optional
	FailureMode FailureMode `json:"failureMode,omitempty"`
}

// Response to return to the client if request content
// is matched against a regex pattern and the action is `REJECT`.
// +kubebuilder:validation:AtLeastOneFieldSet
type CustomResponse struct {
	// Custom response message to return to the client. If not specified, defaults to
	// `The request was rejected due to inappropriate content`.
	// +kubebuilder:default="The request was rejected due to inappropriate content"
	// +optional
	Message string `json:"message,omitempty"`

	// Status code to return to the client. Defaults to 403.
	// +kubebuilder:default=403
	// +kubebuilder:validation:Minimum=200
	// +kubebuilder:validation:Maximum=599
	// +optional
	StatusCode int32 `json:"statusCode,omitempty"`
}

type OpenAIModeration struct {
	// Moderation model to use. For example,
	// `omni-moderation`.
	// +optional
	Model *string `json:"model,omitempty"`
	// Policies for communicating with OpenAI.
	// +kubebuilder:validation:AtLeastOneFieldSet
	// +optional
	Policies *BackendSimple `json:"policies,omitempty"`
}

type BedrockGuardrails struct {
	// Identifier of the Guardrail policy to use for the backend.
	// +required
	GuardrailIdentifier ShortString `json:"identifier"`

	// Version of the Guardrail policy to use for the backend.
	// +required
	GuardrailVersion ShortString `json:"version"`

	// AWS region where the guardrail is deployed, for example
	// `us-west-2`).
	// +required
	Region ShortString `json:"region"`

	// Policies for communicating with AWS Bedrock Guardrails.
	// +kubebuilder:validation:AtLeastOneFieldSet
	// +optional
	Policies *BackendSimple `json:"policies,omitempty"`
}

type GoogleModelArmor struct {
	// Template ID for Google Model Armor.
	// +required
	TemplateID ShortString `json:"templateId"`

	// Google Cloud project ID.
	// +required
	ProjectID ShortString `json:"projectId"`

	// Google Cloud location, for example `us-central1`.
	// Defaults to `us-central1` if not specified.
	// +kubebuilder:default="us-central1"
	// +optional
	Location *ShortString `json:"location,omitempty"`

	// Policies for communicating with Google Model Armor.
	// +kubebuilder:validation:AtLeastOneFieldSet
	// +optional
	Policies *BackendSimple `json:"policies,omitempty"`
}

// Prompt guards to apply to requests sent by the client.
// +kubebuilder:validation:ExactlyOneOf=regex;webhook;openAIModeration;bedrockGuardrails;googleModelArmor
type PromptguardRequest struct {
	// Custom response message to return to the client. If not specified, defaults to
	// `The request was rejected due to inappropriate content`.
	// +optional
	CustomResponse *CustomResponse `json:"response,omitempty"`

	// Regular expression (regex) matching for prompt guards and data masking.
	// +optional
	Regex *Regex `json:"regex,omitempty"`

	// Webhook that receives requests for prompt guarding.
	// +optional
	Webhook *Webhook `json:"webhook,omitempty"`

	// Passes prompt data through the OpenAI Moderations
	// endpoint.
	// See https://developers.openai.com/api/reference/resources/moderations for more information.
	// +optional
	OpenAIModeration *OpenAIModeration `json:"openAIModeration,omitempty"`

	// AWS Bedrock Guardrails settings for prompt
	// guarding.
	// +optional
	BedrockGuardrails *BedrockGuardrails `json:"bedrockGuardrails,omitempty"`

	// Google Model Armor settings for prompt guarding.
	// +optional
	GoogleModelArmor *GoogleModelArmor `json:"googleModelArmor,omitempty"`
}

// Prompt guards to apply to responses returned by the LLM provider.
// +kubebuilder:validation:ExactlyOneOf=regex;webhook;bedrockGuardrails;googleModelArmor
type PromptguardResponse struct {
	// Custom response message to return to the client. If not specified, defaults to
	// `The response was rejected due to inappropriate content`.
	// +optional
	CustomResponse *CustomResponse `json:"response,omitempty"`

	// Regular expression (regex) matching for prompt guards and data masking.
	// +optional
	Regex *Regex `json:"regex,omitempty"`

	// Webhook that receives responses for prompt guarding.
	// +optional
	Webhook *Webhook `json:"webhook,omitempty"`

	// AWS Bedrock Guardrails settings for prompt
	// guarding.
	// +optional
	BedrockGuardrails *BedrockGuardrails `json:"bedrockGuardrails,omitempty"`

	// Google Model Armor settings for prompt guarding.
	// +optional
	GoogleModelArmor *GoogleModelArmor `json:"googleModelArmor,omitempty"`
}

// Prompt guards that block unwanted requests to the LLM provider and mask sensitive data.
// Prompt guards can be used to reject requests based on the content of the prompt, as well as
// mask responses based on the content of the response.
//
// This example rejects any request prompts that contain
// the string "credit card", and masks any credit card numbers in the response.
//
//	promptGuard:
//		request:
//		- response:
//		    message: "Rejected due to inappropriate content"
//		  regex:
//		    action: REJECT
//		    matches:
//		    - pattern: "credit card"
//		      name: "CC"
//		response:
//		- regex:
//		    builtins:
//		    - CREDIT_CARD
//		    action: MASK
//
// +kubebuilder:validation:AtLeastOneFieldSet:fields=request;response
type AIPromptGuard struct {
	// Apply prompt guards to streaming responses and realtime websocket messages.
	// Defaults to disabled to preserve streaming throughput unless explicitly enabled.
	// +optional
	Streaming PromptGuardStreamingMode `json:"streaming,omitempty"`

	// Prompt guards to apply to requests sent by the client.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=8
	// +optional
	Request []PromptguardRequest `json:"request,omitempty"`

	// Prompt guards to apply to responses returned by the LLM provider.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=8
	// +optional
	Response []PromptguardResponse `json:"response,omitempty"`
}

// Default value for a field in the JSON request body sent to the LLM provider.
// These defaults are merged with the user-provided request to ensure missing fields are populated.
//
// User input fields here refer to the fields in the JSON request body that a client sends when making a request to the LLM provider.
// Defaults set here do _not_ override those user-provided values unless you explicitly set `override` to `true`.
//
// Example: Setting a default system field for Anthropic, which does not support system role messages:
//
//	defaults:
//	  - field: "system"
//	    value: "answer all questions in French"
//
// Example: Setting a default temperature and overriding `max_tokens`:
//
//	defaults:
//	  - field: "temperature"
//	    value: "0.5"
//	  - field: "max_tokens"
//	    value: "100"
//	    override: true
//
// Example: Setting custom lists fields:
//
//	defaults:
//	  - field: "custom_integer_list"
//	    value: [1,2,3]
//
//	overrides:
//	  - field: "custom_string_list"
//	    value: ["one","two","three"]
//
// Note: The `field` values correspond to keys in the JSON request body, not fields in this CRD.
type FieldDefault struct {
	// Name of the field.
	// +kubebuilder:validation:MinLength=1
	// +required
	Field ShortString `json:"field"`

	// Default value for the field. This can be any JSON data type.
	// +kubebuilder:pruning:PreserveUnknownFields
	// +required
	Value apiextensionsv1.JSON `json:"value"`
}

// Maps a request JSON field to a CEL expression.
// The expression is evaluated against the current request body and its result
// is assigned to the configured field.
type FieldTransformation struct {
	// Name of the field to set.
	// +kubebuilder:validation:MinLength=1
	// +required
	Field ShortString `json:"field"`

	// CEL expression used to compute the field value.
	// +required
	Expression CELExpression `json:"expression"`
}

// Automatic prompt caching for supported LLM providers.
// Currently only AWS Bedrock supports this feature (Claude 3+ and Nova models).
//
// When enabled, the gateway automatically inserts cache points at strategic locations
// to reduce API costs. Bedrock charges lower rates for cached tokens (90% discount).
//
// Example:
//
//	promptCaching:
//	  cacheSystem: true
//	  cacheMessages: true
//	  cacheTools: false
//
// Cost savings example:
// - Without caching: 10,000 tokens × $3/MTok = $0.03
// - With caching (90% cached): 1,000 × $3/MTok + 9,000 × $0.30/MTok = $0.0057 (81% savings)
type PromptCachingConfig struct {
	// Enables caching for system prompts.
	// Inserts a cache point after all system messages.
	// +optional
	// +kubebuilder:default=true
	CacheSystem bool `json:"cacheSystem,omitempty"`

	// Enables caching for conversation messages.
	// Caches all messages in the conversation for cost savings.
	// +optional
	// +kubebuilder:default=true
	CacheMessages bool `json:"cacheMessages,omitempty"`

	// Enables caching for tool definitions.
	// Inserts a cache point after all tool specifications.
	// +optional
	// +kubebuilder:default=false
	CacheTools bool `json:"cacheTools,omitempty"`

	// Minimum estimated token count
	// before caching is enabled. Uses rough heuristic (word count × 1.3) to estimate tokens.
	// Bedrock requires at least 1,024 tokens for caching to be effective.
	// +optional
	// +kubebuilder:validation:Minimum=0
	// +kubebuilder:default=1024
	MinTokens int `json:"minTokens,omitempty"`

	// Shifts the message cache point further back in the
	// conversation. 0 (default) places it at the second-to-last message.
	// Higher values move it N additional messages towards the start, clamped
	// to bounds.
	// +optional
	// +kubebuilder:validation:Minimum=0
	// +kubebuilder:default=0
	CacheMessageOffset int `json:"cacheMessageOffset,omitempty"`
}
