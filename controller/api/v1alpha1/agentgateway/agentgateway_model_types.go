package agentgateway

import (
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewaymodels,verbs=get;list;watch
// +kubebuilder:rbac:groups=agentgateway.dev,resources=agentgatewaymodels/status,verbs=get;update;patch

// +kubebuilder:printcolumn:name="Model Match",type=string,JSONPath=".spec.modelMatch",description="Model name matched from client requests"
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
// +kubebuilder:validation:XValidation:rule="has(self.provider) || !has(self.providerModel)",message="providerModel requires provider"
// +kubebuilder:validation:XValidation:rule="has(self.provider) || !has(self.transformations)",message="transformations require provider"
// +kubebuilder:validation:XValidation:rule="!has(self.virtualModel) || self.visibility != 'Internal'",message="virtual models must be public"
type AgentgatewayModelSpec struct {
	// Gateways and listeners to which this model attaches.
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=16
	// +required
	ParentRefs []gwv1.ParentReference `json:"parentRefs"`

	// Model name or wildcard expression matched against client requests.
	// When omitted, the model matches metadata.name exactly.
	// +optional
	ModelMatch *LongString `json:"modelMatch,omitempty"`

	// Controls whether clients can request this model directly. Internal models
	// can only be selected by virtual models. Defaults to Public.
	// +kubebuilder:default=Public
	// +optional
	Visibility ModelVisibility `json:"visibility,omitempty"`

	// Provider serving this concrete model.
	// +optional
	Provider *LLMProvider `json:"provider,omitempty"`

	// Fixed model name sent to the provider. When omitted, the model selected by
	// modelMatch is sent to the provider.
	// +optional
	ProviderModel *ShortString `json:"providerModel,omitempty"`

	// CEL transformations applied to fields in the provider request body.
	// Transformations take precedence over providerModel for the same field.
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
	// when modelRef points to a wildcard modelMatch. When omitted, the referenced
	// model's effective modelMatch is used.
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
