package agentgateway

import (
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

// Control-plane Authorization rules not specific to policies:
// +kubebuilder:rbac:groups=authentication.k8s.io,resources=tokenreviews,verbs=create

// Selects one object by `name` and, optionally, `namespace`.
// You can target only one object at a time.
type NamespacedObjectReference struct {
	// The name of the target resource.
	// +required
	Name gwv1.ObjectName `json:"name"`

	// The namespace of the target resource.
	// If not set, defaults to the namespace of the parent object.
	// +optional
	Namespace *gwv1.Namespace `json:"namespace,omitempty"`
}

// Selects one same-namespace object by `group`, `kind`, and `name`.
// The object must be in the same namespace as the policy.
type LocalPolicyTargetReference struct {
	// The API group of the target resource.
	// For Kubernetes Gateway API resources, the group is `gateway.networking.k8s.io`.
	// +required
	Group gwv1.Group `json:"group"`

	// The API kind of the target resource, such as `Gateway` or `HTTPRoute`.
	// +required
	Kind gwv1.Kind `json:"kind"`

	// The name of the target resource.
	// +required
	Name gwv1.ObjectName `json:"name"`
}

// Selects one same-namespace object by `group`, `kind`, `name`, and,
// optionally, `sectionName`.
// The object must be in the same namespace as the policy.
type LocalPolicyTargetReferenceWithSectionName struct {
	LocalPolicyTargetReference `json:",inline"`

	// The named section of the target resource.
	// +optional
	SectionName *gwv1.SectionName `json:"sectionName,omitempty"`
}

// Selects same-namespace objects by `group`, `kind`, and `matchLabels`.
// The object must be in the same namespace as the policy and match the
// specified labels.
type LocalPolicyTargetSelector struct {
	// The API group of the target resource.
	// For Kubernetes Gateway API resources, the group is `gateway.networking.k8s.io`.
	// +required
	Group gwv1.Group `json:"group"`

	// The API kind of the target resource, such as `Gateway` or `HTTPRoute`.
	// +required
	Kind gwv1.Kind `json:"kind"`

	// Labels that must be present on each selected target resource.
	// +required
	MatchLabels map[string]string `json:"matchLabels"`
}

// Selects same-namespace objects by `group`, `kind`, `matchLabels`, and,
// optionally, `sectionName`.
// Each selected object must be in the same namespace as the policy and match
// the specified labels.
// Prefer `targetRefs` when reconciliation latency is important, especially
// when many policies target the same resource.
type LocalPolicyTargetSelectorWithSectionName struct {
	LocalPolicyTargetSelector `json:",inline"`

	// The named section of each selected target resource.
	// +optional
	SectionName *gwv1.SectionName `json:"sectionName,omitempty"`
}

type PolicyStatus struct {
	// The current condition state for the policy.
	// +optional
	// +listType=map
	// +listMapKey=type
	// +kubebuilder:validation:MaxItems=8
	Conditions []metav1.Condition `json:"conditions,omitempty"`

	// Status for each ancestor that is affected by this policy.
	// +kubebuilder:validation:MaxItems=16
	// +required
	Ancestors []PolicyAncestorStatus `json:"ancestors"`
}

type PolicyAncestorStatus struct {
	// The ancestor resource that this status entry describes.
	// +required
	AncestorRef gwv1.ParentReference `json:"ancestorRef"`

	// The controller that wrote this status entry.
	//
	// Example: `example.net/gateway-controller`.
	// +required
	ControllerName string `json:"controllerName"`

	// Conditions for this policy's effect on the specified ancestor.
	//
	// +optional
	// +listType=map
	// +listMapKey=type
	// +kubebuilder:validation:MinItems=1
	// +kubebuilder:validation:MaxItems=8
	Conditions []metav1.Condition `json:"conditions,omitempty"`
}

// Modifies request and response headers.
// +kubebuilder:validation:AtLeastOneFieldSet
type HeaderModifiers struct {
	// Header changes to apply before forwarding a request.
	// +optional
	Request *gwv1.HTTPHeaderFilter `json:"request,omitempty"`

	// Header changes to apply before returning a response.
	// +optional
	Response *gwv1.HTTPHeaderFilter `json:"response,omitempty"`
}

// References a same-namespace credential.
// Set only `name` to reference a Kubernetes Secret.
//
// +structType=atomic
// +kubebuilder:validation:XValidation:rule="(!has(self.group) || size(self.group) == 0) ? (!has(self.kind) || size(self.kind) == 0 || self.kind == 'Secret') : (has(self.kind) && size(self.kind) > 0)",message="custom credential refs must set both group and kind"
type LocalSecretObjectRef struct {
	// The name of the referenced credential.
	// +required
	Name gwv1.ObjectName `json:"name"`

	// The API group of the referenced credential.
	// Empty selects the core API group.
	// +optional
	Group string `json:"group,omitempty"`

	// The kind of the referenced credential.
	// Empty defaults to `Secret`.
	// +optional
	Kind string `json:"kind,omitempty"`
}
