package agentgateway

// CredentialRef references a same-namespace credential source.
// A ref with only name set is wire-compatible with corev1.LocalObjectReference
// and resolves to a Kubernetes Secret in the core API group.
//
// +structType=atomic
// +kubebuilder:validation:XValidation:rule="(!has(self.group) || size(self.group) == 0) ? (!has(self.kind) || self.kind == 'Secret') : has(self.kind)",message="custom credential refs must set both group and kind"
type CredentialRef struct {
	// Name of the referenced credential source.
	// This field is effectively required, but defaults to empty for
	// compatibility with corev1.LocalObjectReference. Translation reports
	// empty names instead of rejecting them during admission.
	// +optional
	// +default=""
	// +kubebuilder:default=""
	Name string `json:"name,omitempty"`

	// Group of the referenced credential source. Empty means the Kubernetes
	// core API group.
	// +optional
	Group string `json:"group,omitempty"`

	// Kind of the referenced credential source. Empty defaults to Secret.
	// +optional
	Kind string `json:"kind,omitempty"`
}
